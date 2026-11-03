<script lang="ts">
  import { onMount } from "svelte";

  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import type { PublicUser, Snowflake } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import { MAXIMUM_VOLUME_PERCENT, MINIMUM_VOLUME_PERCENT } from "../lib/voice-settings";
  import { displayName } from "./format";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import { LIVE_BADGE_CLASS } from "./live-badge";
  import { watchedStreamStatus } from "./screen-share-look";
  import { durationTokenInMilliseconds } from "./tokens";

  // A shared screen, watched large in the conversation sheet's place (ADR-0037,
  // docs/design/web-screen-sharing.md, docs/design/web-stream-watching.md): the chat header's
  // grammar (who, the live chip, then the actions) over a dark stage where the picture is
  // letterboxed, and under it a band as tall as the header for the stream's own sound: mute,
  // volume, and whether any sound comes at all. While the picture comes, the streamer's face and
  // "Connexion à l'écran…" (after 10 s, that it is not coming and what to do); while the instance
  // pauses it, that the connection is too slow; when they stop or leave, one sentence, then the
  // view closes by itself. Esc closes it (in fullscreen, Esc leaves fullscreen first, as browsers do).
  interface Props {
    client: MellowClient;
    userId: Snowflake;
    /** Phones: the navigator is a drawer, opened from the header as in the chat. */
    hasNavigationDrawer: boolean;
    onopennavigation: () => void;
  }

  let { client, userId, hasNavigationDrawer, onopennavigation }: Props = $props();

  let view = $state<HTMLElement | null>(null);
  let isFullscreen = $state(false);
  let hasPicture = $state(false);
  const canGoFullscreen = typeof document !== "undefined" && document.fullscreenEnabled;

  const voice = $derived(client.voice);
  const track = $derived(voice.streamTracks.get(userId));
  const status = $derived(
    watchedStreamStatus(client.voiceStatesByUserId.get(userId), voice.phase === "idle" ? null : voice.channelId, track !== undefined),
  );
  const person = $derived.by((): Pick<PublicUser, "id" | "display_name" | "username" | "avatar"> => {
    const guildId = client.voiceChannel?.guild.id;
    const member = guildId ? client.membersByGuild.get(guildId)?.find((candidate) => candidate.user.id === userId) : undefined;
    return member?.user ?? { id: userId, username: userId, display_name: strings.voice.someone };
  });
  const name = $derived(displayName(person));
  const isPaused = $derived(voice.pausedStreamUserIds.has(userId));
  const isWaitingForPicture = $derived(status === "connecting" || (status === "live" && !hasPicture));
  let isPictureLate = $state(false);
  const stageSentence = $derived.by(() => {
    if (status === "ended") {
      return strings.screenShare.ended(name);
    }
    if (isPaused) {
      return strings.screenShare.paused;
    }
    if (isWaitingForPicture) {
      return isPictureLate ? strings.screenShare.slow(name) : strings.screenShare.connecting;
    }
    return "";
  });

  const soundSetting = $derived(voice.streamSoundSetting(userId));
  const isSoundMuted = $derived(soundSetting.isLocallyMuted);
  const soundStatus = $derived.by((): { icon: "speaker-slash" | "headphones" | null; label: string } | null => {
    if (voice.toggles.isDeafened) {
      return { icon: "headphones", label: strings.screenShare.deafened };
    }
    switch (voice.watchedStreamSound) {
      case "sound":
        return { icon: null, label: strings.screenShare.hasSound };
      case "silent":
        return { icon: "speaker-slash", label: strings.screenShare.noSound };
      default:
        return null;
    }
  });

  // No picture after a while: say it is not coming and what to do, instead of waiting forever.
  $effect(() => {
    if (!isWaitingForPicture) {
      isPictureLate = false;
      return;
    }
    const timer = setTimeout(() => (isPictureLate = true), durationTokenInMilliseconds("component-stream-view-slow-connect-delay"));
    return () => clearTimeout(timer);
  });

  // They stopped: the sentence stays for a reading time, then the view goes. Sharing again
  // meanwhile cancels it.
  $effect(() => {
    if (status !== "ended") {
      return;
    }
    const timer = setTimeout(() => voice.watchStream(null), durationTokenInMilliseconds("component-stream-view-ended-close-delay"));
    return () => clearTimeout(timer);
  });

  onMount(() => {
    // Opened from a live chip: focus follows, and goes back there when the view closes.
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    // `view` is unbound before this cleanup runs: keep the element.
    const element = view;
    element?.focus();
    const noteFullscreen = () => (isFullscreen = document.fullscreenElement !== null && document.fullscreenElement === element);
    document.addEventListener("fullscreenchange", noteFullscreen);
    return () => {
      document.removeEventListener("fullscreenchange", noteFullscreen);
      if (element !== null && document.fullscreenElement === element) {
        void document.exitFullscreen().catch(() => {});
      }
      // Focus was in the view (or fell to the page with it): back to the chip that opened it.
      const focused = document.activeElement;
      if ((focused === document.body || (element?.contains(focused) ?? false)) && opener?.isConnected) {
        opener.focus();
      }
    };
  });

  /** Plays `track` in the video element, and lets it go when the track changes or the view closes. */
  function playTrack(video: HTMLVideoElement) {
    if (track === undefined) {
      return;
    }
    hasPicture = false;
    video.srcObject = new MediaStream([track]);
    return () => {
      video.srcObject = null;
    };
  }

  function toggleFullscreen() {
    if (document.fullscreenElement === view) {
      void document.exitFullscreen().catch(() => {});
    } else {
      // Refused without a user gesture or by the page's permissions: the view stays as it is.
      void view?.requestFullscreen().catch(() => {});
    }
  }

  /** Only while focus is in the view, so Esc keeps its meaning everywhere else (drawers, menus). */
  function closeOnEscape(event: KeyboardEvent) {
    const isInView = event.target instanceof Node && view?.contains(event.target) === true;
    if (event.key === "Escape" && isInView && document.fullscreenElement === null && !event.defaultPrevented) {
      event.preventDefault();
      voice.watchStream(null);
    }
  }

  const headerButtonClass =
    "size-(--mw-component-chat-header-button-size) text-(--mw-component-chat-header-icon) hover:bg-state-hover hover:text-text-normal";
</script>

<svelte:window onkeydown={closeOnEscape} />

<section
  bind:this={view}
  tabindex="-1"
  aria-label={strings.screenShare.screenOf(name)}
  class={[
    "absolute inset-0 flex min-w-0 flex-col overflow-hidden bg-(--mw-component-stream-view-background) outline-none",
    !hasNavigationDrawer && !isFullscreen && "rounded-(--mw-component-sheet-radius)",
  ]}
>
  <header class="flex h-(--mw-component-chat-header-height) shrink-0 items-center gap-xs ps-(--mw-component-chat-header-padding-inline) pe-xs">
    {#if hasNavigationDrawer && !isFullscreen}
      <IconButton
        icon="list"
        label={strings.chat.openNavigation}
        class={headerButtonClass}
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onclick={onopennavigation}
      />
    {/if}
    <Avatar user={person} sizeToken="--mw-component-chat-header-icon-size" />
    <h1 class="min-w-0 truncate type-heading-md text-(--mw-component-chat-header-name-text)">{strings.screenShare.screenOf(name)}</h1>
    {#if status !== "ended"}
      <span aria-hidden="true" class={LIVE_BADGE_CLASS}>{strings.screenShare.live}</span>
    {/if}
    <span class="flex-1"></span>
    {#if canGoFullscreen}
      <IconButton
        icon={isFullscreen ? "corners-in" : "corners-out"}
        label={isFullscreen ? strings.screenShare.exitFullscreen : strings.screenShare.fullscreen}
        aria-keyshortcuts={isFullscreen ? "Escape" : undefined}
        class={headerButtonClass}
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onclick={toggleFullscreen}
      />
    {/if}
    <IconButton
      icon="x"
      label={strings.screenShare.close}
      aria-keyshortcuts="Escape"
      class={headerButtonClass}
      iconClass="size-(--mw-component-chat-header-icon-size)"
      onclick={() => voice.watchStream(null)}
    />
  </header>
  <div
    class={[
      "relative min-h-0 flex-1 overflow-hidden bg-(--mw-component-stream-view-stage)",
      !isFullscreen && "mx-(--mw-component-stream-view-stage-inset) rounded-(--mw-component-stream-view-stage-radius)",
    ]}
  >
    {#if track !== undefined && status !== "ended"}
      <!-- Muted: its sound plays with the voices (`voice-session`), at the stream volume below. The name is the view's. -->
      <video
        {@attach playTrack}
        autoplay
        playsinline
        muted
        aria-hidden="true"
        class={["size-full object-contain", (!hasPicture || isPaused) && "invisible"]}
        onloadeddata={() => (hasPicture = true)}
      ></video>
    {/if}
    {#if stageSentence}
      <div class="absolute inset-0 flex flex-col items-center justify-center gap-sm p-md text-center">
        <Avatar user={person} sizeToken="--mw-component-stream-view-avatar-size" />
        <p class="type-body text-(--mw-component-stream-view-stage-text)">{stageSentence}</p>
      </div>
    {/if}
  </div>
  <!-- The stream's own sound, apart from the person's voice; the band stays when they stop, so nothing jumps. -->
  <div
    class="flex h-(--mw-component-stream-view-sound-bar-height) shrink-0 items-center gap-(--mw-component-stream-view-sound-bar-gap) ps-(--mw-component-stream-view-stage-inset) pe-(--mw-component-chat-header-padding-inline)"
  >
    {#if status !== "ended"}
      <IconButton
        icon={isSoundMuted ? "speaker-x" : "speaker-high"}
        label={strings.screenShare.muteStream}
        aria-pressed={isSoundMuted}
        class="size-(--mw-component-stream-view-control-size) text-(--mw-component-chat-header-icon) hover:bg-state-hover hover:text-text-normal"
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onclick={() => voice.toggleStreamMuted(userId)}
      />
      <input
        type="range"
        min={MINIMUM_VOLUME_PERCENT}
        max={MAXIMUM_VOLUME_PERCENT}
        value={soundSetting.volumePercent}
        aria-label={strings.screenShare.streamVolume}
        aria-valuetext={strings.voiceSettings.percent(soundSetting.volumePercent)}
        class="h-(--mw-component-stream-view-control-size) w-(--mw-component-stream-view-volume-width) min-w-0 shrink cursor-pointer accent-(--mw-component-slider-accent) focus-visible:focus-ring"
        oninput={(event) => voice.setStreamVolume(userId, Number(event.currentTarget.value))}
      />
      <span aria-hidden="true" class="shrink-0 type-body-sm tabular-nums text-(--mw-component-slider-value-text)">
        {strings.voiceSettings.percent(soundSetting.volumePercent)}
      </span>
      <span class="flex-1"></span>
      {#if soundStatus}
        <p
          class="flex shrink-0 items-center gap-2xs type-body-sm text-(--mw-component-stream-view-sound-status-text)"
          title={voice.watchedStreamSound === "silent" && !voice.toggles.isDeafened ? strings.screenShare.noSoundDetail(name) : undefined}
        >
          {#if soundStatus.icon}<Icon name={soundStatus.icon} class="size-(--mw-component-voice-panel-signal-icon-size) shrink-0" />{/if}
          <span class="truncate">{soundStatus.label}</span>
        </p>
      {/if}
    {/if}
  </div>
  <!-- Exist from the start, so "connecting" then "stopped" are read out as they come. -->
  <p class="sr-only" aria-live="polite">{stageSentence}</p>
  <p class="sr-only" aria-live="polite">
    {status !== "ended" && voice.watchedStreamSound === "silent" && !voice.toggles.isDeafened ? strings.screenShare.noSoundDetail(name) : ""}
  </p>
</section>
