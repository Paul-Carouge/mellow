<script lang="ts" module>
  import { lengthTokenInPixels, numberTokenValue } from "./tokens";

  // Read once: the bars' count and proportions are the same in both themes and at any zoom.
  let waveformTokens: { barCount: number; placeholderLevel: number; minimumBarRatio: number } | null = null;

  function waveformTokensOnce() {
    waveformTokens ??= {
      barCount: numberTokenValue("component-audio-player-wave-bar-count"),
      placeholderLevel: numberTokenValue("component-audio-player-wave-placeholder-level"),
      minimumBarRatio:
        lengthTokenInPixels("component-audio-player-wave-bar-min-height") / lengthTokenInPixels("component-audio-player-wave-height"),
    };
    return waveformTokens;
  }
</script>

<script lang="ts">
  import type { Snippet } from "svelte";

  import { formattingLocale, strings } from "../i18n";
  import { playAtRememberedVolume } from "../lib/media-volume";
  import { type Attachment, attachmentUrl } from "../lib/protocol";
  import { forgetMediaWhenItStops, pauseOtherMediaWhenOneStarts } from "./attachment-preview";
  import {
    nextPlaybackSpeed,
    placeholderPeaks,
    playbackFraction,
    playbackSpeedNumber,
    playedBarCount,
    seekFractionAfterKey,
  } from "./audio-player";
  import { readSoundShape, rememberSoundDuration, soundShapeOf } from "./audio-waveforms.svelte";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import { clockDurationLabel, durationParts } from "./link-previews";

  // A sound attached to a message (DESIGN.md 5.4, "Sound player"): a round play key, the sound's
  // waveform as the seek bar, the time, the speed and the volume, under the file's name row. The
  // shape shows once the sound played (read again and decoded, `audio-waveforms.svelte.ts`); until
  // then the line is flat. Nothing loads before play (`preload="none"`: each request decrypts the
  // whole file on the instance) and one sound plays at a time (ADR-0038).
  interface Props {
    attachment: Attachment;
    /** The file's name as the download link and the download key. */
    nameRow: Snippet;
    /** "51,9 Ko": always shown under the waveform, after the time once it is known. */
    sizeLabel: string;
    /** The browser cannot play this file: the list shows its download card instead. */
    onUnplayable: (hadFocus: boolean) => void;
  }

  let { attachment, nameRow, sizeLabel, onUnplayable }: Props = $props();

  const { barCount, placeholderLevel, minimumBarRatio } = waveformTokensOnce();
  const percentFormat = new Intl.NumberFormat(formattingLocale, { style: "percent", maximumFractionDigits: 0 });

  let audio = $state<HTMLAudioElement | null>(null);
  let isPaused = $state(true);
  let currentSeconds = $state(0);
  let elementDuration = $state<number | undefined>(undefined);
  let speed = $state(1);
  let volume = $state(1);
  let isMuted = $state(false);
  let isWaiting = $state(false);
  let problem = $state<"offline" | "stopped" | null>(null);
  // Where a press on the waveform asked to start before the sound's length was known.
  let pendingFraction = $state<number | null>(null);

  const shape = $derived(soundShapeOf(attachment.id));
  const durationSeconds = $derived(
    elementDuration !== undefined && Number.isFinite(elementDuration) && elementDuration > 0
      ? elementDuration
      : (shape?.durationSeconds ?? null),
  );
  const fraction = $derived(pendingFraction ?? playbackFraction(currentSeconds, durationSeconds));
  const levels = $derived(shape?.peaks ?? placeholderPeaks(barCount, placeholderLevel));
  const playedBars = $derived(playedBarCount(fraction, barCount));
  const elapsedSeconds = $derived(durationSeconds === null ? currentSeconds : fraction * durationSeconds);

  function spokenTime(totalSeconds: number): string {
    const { hours, minutes, seconds } = durationParts(Math.floor(totalSeconds));
    return strings.linkPreviews.spokenDuration(hours, minutes, seconds) || strings.messages.zeroSeconds;
  }

  const positionText = $derived(
    durationSeconds === null
      ? spokenTime(elapsedSeconds)
      : strings.messages.soundPositionValue(spokenTime(elapsedSeconds), spokenTime(durationSeconds)),
  );

  function togglePlayback() {
    if (audio === null) {
      return;
    }
    if (!isPaused) {
      audio.pause();
      return;
    }
    if (!navigator.onLine && audio.readyState < HTMLMediaElement.HAVE_FUTURE_DATA) {
      problem = "offline";
      return;
    }
    problem = null;
    if (audio.error !== null) {
      // A lost connection leaves the element in error for good: only loading again retries. The
      // position comes back once the metadata is read again.
      pendingFraction = fraction;
      audio.load();
    }
    isWaiting = audio.readyState < HTMLMediaElement.HAVE_FUTURE_DATA;
    audio.play().catch((error: unknown) => {
      isWaiting = false;
      // A pause before the sound could start rejects the request; nothing went wrong.
      if (!(error instanceof DOMException && error.name === "AbortError")) {
        console.warn("couldn't play a sound", error);
      }
    });
  }

  // Unmounted while playing (scrolled out, edited away): the list must not keep a gone player as
  // the one to pause. An attachment's cleanup gets the element itself, which `bind:this` may
  // already have cleared when an `onDestroy` runs.
  function forgetWhenUnmounted(element: HTMLAudioElement) {
    return () => forgetMediaWhenItStops(element);
  }

  function seekTo(nextFraction: number) {
    if (audio !== null && durationSeconds !== null && elementDuration !== undefined && Number.isFinite(elementDuration)) {
      audio.currentTime = nextFraction * elementDuration;
      currentSeconds = audio.currentTime;
      pendingFraction = null;
    } else {
      pendingFraction = nextFraction;
    }
  }

  function seekWithKey(event: KeyboardEvent) {
    if (event.key === " ") {
      event.preventDefault();
      togglePlayback();
      return;
    }
    const nextFraction = seekFractionAfterKey(event.key, fraction, durationSeconds);
    if (nextFraction !== null) {
      event.preventDefault();
      seekTo(nextFraction);
    }
  }

  function applyMetadata() {
    if (audio === null) {
      return;
    }
    rememberSoundDuration(attachment.id, audio.duration);
    if (pendingFraction !== null && Number.isFinite(audio.duration)) {
      audio.currentTime = pendingFraction * audio.duration;
      pendingFraction = null;
    }
  }

  function stopAfterError() {
    isWaiting = false;
    if (audio?.error?.code === MediaError.MEDIA_ERR_NETWORK) {
      // A busy instance or a dropped connection: the player stays and play resumes.
      problem = "stopped";
      return;
    }
    // The player's card holds the focus while someone uses it.
    onUnplayable(audio?.parentElement?.contains(document.activeElement) ?? false);
  }
</script>

<div
  role="group"
  aria-label={strings.messages.audioPlayer(attachment.file_name)}
  class="flex flex-col gap-(--mw-component-attachment-row-gap)"
>
  {@render nameRow()}
  <div class="flex items-center gap-(--mw-component-attachment-file-gap)">
    <button
      type="button"
      aria-label={isPaused ? strings.messages.playSound(attachment.file_name) : strings.messages.pauseSound(attachment.file_name)}
      class={[
        "grid size-(--mw-component-audio-player-play-button-size) shrink-0 cursor-pointer place-items-center rounded-full transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) focus-visible:focus-ring",
        isPaused
          ? "bg-(--mw-component-audio-player-play-button-background) text-(--mw-component-audio-player-play-button-icon) hover:bg-(--mw-component-audio-player-play-button-background-hover)"
          : "bg-(--mw-component-audio-player-play-button-background-playing) text-(--mw-component-audio-player-play-button-icon-playing) hover:bg-(--mw-component-audio-player-play-button-background-playing-hover)",
      ]}
      onclick={togglePlayback}
    >
      <Icon name={isPaused ? "play" : "pause"} class="size-(--mw-component-audio-player-play-button-icon-size)" />
    </button>
    <!-- The waveform is the seek bar: a native slider laid over the bars, so pointers, touch and
         screen readers get the browser's own; Space plays and pauses from it. -->
    <div
      class="relative h-(--mw-component-audio-player-wave-height) min-w-0 flex-1 rounded-xs has-[input:focus-visible]:focus-ring pointer-coarse:h-(--mw-size-hit-web-min)"
    >
      <div aria-hidden="true" class="absolute inset-x-0 top-1/2 flex h-(--mw-component-audio-player-wave-height) -translate-y-1/2 items-center gap-(--mw-component-audio-player-wave-bar-gap)">
        {#each levels as level, index (index)}
          <span
            style:transform="scaleY({Math.max(level, minimumBarRatio)})"
            class={[
              "h-full min-w-0 flex-1 rounded-full transition-transform duration-(--mw-motion-duration-base) ease-(--mw-motion-easing-enter)",
              index >= playedBars
                ? "bg-(--mw-component-audio-player-wave-unplayed)"
                : isPaused
                  ? "bg-(--mw-component-audio-player-wave-played-paused)"
                  : "bg-(--mw-component-audio-player-wave-played)",
            ]}
          ></span>
        {/each}
      </div>
      <input
        type="range"
        min="0"
        max="1"
        step="any"
        value={fraction}
        aria-label={strings.messages.soundPosition(attachment.file_name)}
        aria-valuetext={positionText}
        class="absolute inset-0 size-full cursor-pointer opacity-0"
        oninput={(event) => seekTo(Number(event.currentTarget.value))}
        onkeydown={seekWithKey}
      />
    </div>
  </div>
  <div class="flex flex-wrap items-center gap-x-(--mw-component-attachment-file-gap) ps-[calc(var(--mw-component-audio-player-play-button-size)+var(--mw-component-attachment-file-gap))]">
    <p class="me-auto flex flex-wrap gap-x-(--mw-component-attachment-file-gap) type-body-sm tabular-nums text-(--mw-component-audio-player-time-text)">
      {#if isWaiting}
        <!-- Only when loading takes a while, so a fast start does not flash it. -->
        <span class="animate-[mw-fade-in_var(--mw-motion-duration-fast)_var(--mw-motion-delay-loading-notice)_both]"
          >{strings.messages.soundLoading}</span
        >
      {:else if durationSeconds !== null}
        <!-- The slider says it in words. -->
        <span aria-hidden="true">{clockDurationLabel(Math.floor(elapsedSeconds))} / {clockDurationLabel(Math.round(durationSeconds))}</span>
      {/if}
      <span>{sizeLabel}</span>
    </p>
    <div class="flex items-center gap-2xs">
      <button
        type="button"
        aria-label={strings.messages.playbackSpeed(playbackSpeedNumber(speed, formattingLocale))}
        title={strings.messages.playbackSpeed(playbackSpeedNumber(speed, formattingLocale))}
        class="grid h-(--mw-component-audio-player-control-size) min-w-(--mw-component-audio-player-control-size) cursor-pointer place-items-center rounded-sm px-2xs type-body-sm font-semibold tabular-nums text-(--mw-component-audio-player-control-text) transition-colors duration-(--mw-motion-duration-fast) hover:bg-(--mw-component-audio-player-control-background-hover) hover:text-(--mw-component-audio-player-control-text-hover) focus-visible:focus-ring pointer-coarse:h-(--mw-size-hit-web-min) pointer-coarse:min-w-(--mw-size-hit-web-min)"
        onclick={() => (speed = nextPlaybackSpeed(speed))}
      >
        <span aria-hidden="true">{playbackSpeedNumber(speed, formattingLocale)}×</span>
      </button>
      <IconButton
        icon={isMuted ? "speaker-x" : "speaker-high"}
        label={isMuted ? strings.messages.unmuteSound : strings.messages.muteSound}
        class="size-(--mw-component-audio-player-control-size) text-(--mw-component-audio-player-control-text) hover:bg-(--mw-component-audio-player-control-background-hover) hover:text-(--mw-component-audio-player-control-text-hover) focus-visible:focus-ring pointer-coarse:size-(--mw-size-hit-web-min)"
        iconClass="size-(--mw-component-attachment-row-icon-size)"
        onclick={() => (isMuted = !isMuted)}
      />
      <!-- Phones have their own volume buttons: no slider under a coarse pointer. -->
      <input
        type="range"
        min="0"
        max="1"
        step="0.01"
        bind:value={volume}
        aria-label={strings.messages.soundVolume}
        aria-valuetext={percentFormat.format(volume)}
        class="w-(--mw-component-audio-player-volume-width) cursor-pointer accent-(--mw-component-audio-player-volume-accent) focus-visible:focus-ring pointer-coarse:hidden"
        oninput={() => (isMuted = false)}
      />
    </div>
  </div>
  {#if problem !== null}
    <p role="alert" class="flex items-center gap-2xs ps-[calc(var(--mw-component-audio-player-play-button-size)+var(--mw-component-attachment-file-gap))] type-body-sm text-(--mw-component-audio-player-error-text)">
      <Icon name="warning-circle" class="size-(--mw-component-attachment-row-icon-size) shrink-0" />
      {problem === "offline" ? strings.messages.soundOffline : strings.messages.soundStopped}
    </p>
  {/if}
  <audio
    bind:this={audio}
    src={attachmentUrl(attachment)}
    preload="none"
    bind:paused={isPaused}
    bind:currentTime={currentSeconds}
    bind:duration={elementDuration}
    bind:playbackRate={speed}
    bind:volume
    bind:muted={isMuted}
    onplay={(event) => pauseOtherMediaWhenOneStarts(event.currentTarget)}
    onpause={(event) => {
      isWaiting = false;
      forgetMediaWhenItStops(event.currentTarget);
    }}
    onwaiting={() => (isWaiting = true)}
    onplaying={() => {
      isWaiting = false;
      if (audio !== null) {
        void readSoundShape(attachment, audio.duration, barCount);
      }
    }}
    onloadedmetadata={applyMetadata}
    onerror={stopAfterError}
    {@attach playAtRememberedVolume}
    {@attach forgetWhenUnmounted}
  ></audio>
</div>
