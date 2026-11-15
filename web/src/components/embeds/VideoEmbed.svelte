<script lang="ts">
  import { onDestroy } from "svelte";

  import { strings } from "../../i18n";
  import type { LinkPreview, Snowflake } from "../../lib/protocol";
  import Icon from "../Icon.svelte";
  import IconButton from "../IconButton.svelte";
  import {
    clockDurationLabel,
    durationParts,
    EMBED_PLAYER_SANDBOX,
    VIDEO_PLAYER_PERMISSIONS,
    type YouTubeEmbed,
    youTubeEmbedUrl,
  } from "../link-previews";
  import { embedPlayerKey, embedPlayers } from "./embed-players.svelte";
  import PreviewPicture from "./PreviewPicture.svelte";
  import { playYouTubeAtRememberedVolume } from "./youtube-player";
  import YouTubeMark from "./YouTubeMark.svelte";

  // A YouTube video or Short as a small screen (DESIGN.md 5.4 "Link cards"): the instance's poster
  // full-bleed on the media stage, fading into a band with the title, the channel and YouTube's
  // mark. A press plays it in the poster's box, from where the link starts. Nothing comes from
  // YouTube before the press (ADR-0041, ADR-0044).
  interface Props {
    preview: LinkPreview;
    video: YouTubeEmbed;
    messageId: Snowflake;
    title: string;
    imageUrl: string | null;
    /** The video's length when the instance knows it, for the badge; else no badge. */
    durationSeconds: number | null;
    /** The channel, from the instance's YouTube embed only (`youTubeChannelNameOf`). */
    channelName: string | null;
  }

  let { preview, video, messageId, title, imageUrl, durationSeconds, channelName }: Props = $props();

  const playerKey = $derived(embedPlayerKey(messageId, preview.id));
  const isPlaying = $derived(embedPlayers.isOpen(playerKey));
  let isPictureFailed = $state(false);
  // Without a picture the key would vanish into the stage: it takes the stage's lifted tone.
  const hasPoster = $derived(imageUrl !== null && !isPictureFailed);
  let isOffline = $state(false);
  // Set by "Close player" only: a player closed because another opened leaves the focus alone.
  let shouldFocusPlayKey = false;

  function spokenTime(totalSeconds: number): string {
    const { hours, minutes, seconds } = durationParts(totalSeconds);
    return strings.linkPreviews.spokenDuration(hours, minutes, seconds);
  }

  const playLabel = $derived.by(() => {
    let name = video.isShort ? strings.linkPreviews.playYouTubeShort(title) : strings.linkPreviews.playYouTubeVideo(title);
    if (channelName !== null) {
      name += strings.linkPreviews.byChannel(channelName);
    }
    if (durationSeconds !== null) {
      name += strings.linkPreviews.lasting(spokenTime(durationSeconds));
    }
    if (video.startSeconds !== null) {
      name += strings.linkPreviews.startingAt(spokenTime(video.startSeconds));
    }
    if (video.playlistId !== null) {
      name += strings.linkPreviews.inPlaylist;
    }
    return name;
  });

  // Unmounted (edited away, scrolled out), the iframe and its sound are gone: so is the claim.
  onDestroy(() => embedPlayers.close(playerKey));

  function play() {
    isOffline = !navigator.onLine;
    if (!isOffline) {
      embedPlayers.open(playerKey);
    }
  }

  function closePlayer() {
    shouldFocusPlayKey = true;
    embedPlayers.close(playerKey);
  }

  function focusPlayer(player: HTMLIFrameElement) {
    // The play key that held the focus is gone: the player takes it, so keys reach the video.
    player.focus();
  }

  function focusPlayKeyAfterClosing(playKey: HTMLButtonElement) {
    if (shouldFocusPlayKey) {
      shouldFocusPlayKey = false;
      playKey.focus();
    }
  }
</script>

{#snippet posterChip(text: string, icon: "clock" | "list-bullets" | null, position: string)}
  <!-- Said in the play key's name; shown for the eye. -->
  <span
    aria-hidden="true"
    class="pointer-events-none absolute inline-flex items-center gap-2xs rounded-(--mw-component-link-preview-start-chip-radius) bg-(--mw-component-link-preview-start-chip-background) px-xs py-3xs type-body-sm text-(--mw-component-link-preview-start-chip-text) tabular-nums {position}"
  >
    {#if icon !== null}
      <Icon name={icon} class="size-(--mw-component-link-preview-action-icon-size)" />
    {/if}
    {text}
  </span>
{/snippet}

<div
  role="group"
  aria-label={channelName === null ? title : `${title}, ${channelName}`}
  class={[
    "flex flex-col overflow-hidden rounded-(--mw-component-link-preview-radius) bg-(--mw-component-link-preview-screen-background) [--mw-color-focus:var(--mw-component-link-preview-screen-focus-ring)]",
    video.isShort
      ? "w-[calc(var(--mw-component-link-preview-short-player-max-height)*9/16)] max-w-full"
      : "w-full max-w-(--mw-component-link-preview-video-player-width)",
  ]}
>
  <!-- The final box from the start, poster then player, so nothing moves on play. -->
  <div class={["group/poster relative w-full overflow-hidden", video.isShort ? "aspect-9/16" : "aspect-video"]}>
    {#if isPlaying}
      <iframe
        src={youTubeEmbedUrl(video, location.origin)}
        title={strings.linkPreviews.youTubePlayer(title)}
        allow={VIDEO_PLAYER_PERMISSIONS}
        sandbox={EMBED_PLAYER_SANDBOX}
        referrerpolicy="strict-origin-when-cross-origin"
        class="absolute inset-0 size-full border-0"
        {@attach focusPlayer}
        {@attach playYouTubeAtRememberedVolume}
      ></iframe>
    {:else}
      {#if hasPoster && imageUrl !== null}
        <PreviewPicture
          {preview}
          src={imageUrl}
          alt=""
          class="absolute inset-0 size-full object-cover transition-transform duration-(--mw-motion-duration-base) ease-(--mw-motion-easing-standard) group-hover/poster:scale-(--mw-component-link-preview-poster-hover-scale) motion-reduce:transition-none motion-reduce:group-hover/poster:scale-100"
          onFailed={() => (isPictureFailed = true)}
        />
      {/if}
      <!-- The picture sets into the stage instead of ending on a hard edge. -->
      <span
        aria-hidden="true"
        class="pointer-events-none absolute inset-x-0 bottom-0 h-[calc(var(--mw-component-link-preview-screen-fade-ratio)*100%)] bg-linear-to-t from-(--mw-component-link-preview-screen-fade) to-transparent"
      ></span>
      <button
        type="button"
        aria-label={playLabel}
        title={playLabel}
        class="group/play absolute inset-0 grid cursor-pointer place-items-center outline-none focus-visible:focus-ring-inset"
        onclick={play}
        {@attach focusPlayKeyAfterClosing}
      >
        <!-- The lamp lights where the pointer or the focus is: the disc settles into a squircle. -->
        <span
          class="grid size-(--mw-component-link-preview-play-button-size) place-items-center rounded-[50%] {hasPoster
            ? 'bg-(--mw-component-link-preview-play-button-background)'
            : 'bg-(--mw-component-link-preview-play-button-background-hover)'} text-(--mw-component-link-preview-play-button-icon) transition-[background-color,color,border-radius,scale] duration-(--mw-motion-duration-settle) ease-(--mw-motion-easing-settle) group-hover/play:rounded-(--mw-component-link-preview-play-button-radius-lit) group-hover/play:bg-(--mw-component-link-preview-play-button-background-lit) group-hover/play:text-(--mw-component-link-preview-play-button-icon-lit) group-focus-visible/play:rounded-(--mw-component-link-preview-play-button-radius-lit) group-focus-visible/play:bg-(--mw-component-link-preview-play-button-background-lit) group-focus-visible/play:text-(--mw-component-link-preview-play-button-icon-lit) group-active/play:scale-(--mw-component-link-preview-play-button-pressed-scale) group-active/play:bg-(--mw-component-link-preview-play-button-background-pressed) group-active/play:duration-(--mw-motion-duration-fast)"
        >
          <Icon name="play" class="size-(--mw-component-link-preview-play-button-icon-size)" />
        </span>
      </button>
      {#if video.playlistId !== null}
        {@render posterChip(strings.linkPreviews.playlist, "list-bullets", "start-(--mw-component-link-preview-start-chip-inset) top-(--mw-component-link-preview-start-chip-inset)")}
      {/if}
      {#if video.startSeconds !== null}
        {@render posterChip(
          strings.linkPreviews.startsAt(clockDurationLabel(video.startSeconds)),
          null,
          "start-(--mw-component-link-preview-start-chip-inset) bottom-(--mw-component-link-preview-start-chip-inset)",
        )}
      {/if}
      {#if durationSeconds !== null}
        {@render posterChip(
          clockDurationLabel(durationSeconds),
          null,
          "end-(--mw-component-link-preview-start-chip-inset) bottom-(--mw-component-link-preview-start-chip-inset)",
        )}
      {/if}
    {/if}
  </div>
  <div class="flex flex-col gap-(--mw-component-link-preview-text-gap) px-(--mw-component-link-preview-screen-padding) pb-(--mw-component-link-preview-screen-padding)">
    <div class="flex items-start gap-(--mw-component-link-preview-gap)">
      <a
        href={preview.url}
        target="_blank"
        rel="noopener noreferrer"
        class="flex min-h-(--mw-size-hit-desktop-min) min-w-0 flex-1 items-center rounded-xs type-body-lg font-semibold break-words text-(--mw-component-link-preview-screen-title-text) hover:underline focus-visible:focus-ring pointer-coarse:min-h-(--mw-size-hit-web-min)"
        ><span class={video.isShort ? "line-clamp-3" : "line-clamp-2"}>{title}</span></a
      >
      {#if isPlaying}
        <IconButton
          icon="x"
          label={strings.linkPreviews.closePlayer}
          class="-me-2xs size-(--mw-component-button-height-small) shrink-0 text-(--mw-component-link-preview-screen-control-icon) hover:bg-(--mw-component-link-preview-screen-control-background-hover) hover:text-(--mw-component-link-preview-screen-control-icon-hover) focus-visible:focus-ring pointer-coarse:size-(--mw-size-hit-web-min)"
          iconClass="size-(--mw-component-link-preview-action-icon-size)"
          onclick={closePlayer}
        />
      {/if}
    </div>
    <div class="flex min-h-(--mw-component-link-preview-screen-avatar-size) items-center gap-(--mw-component-link-preview-gap)">
      <p class="min-w-0 flex-1 truncate type-body-sm text-(--mw-component-link-preview-screen-meta-text)">{channelName ?? ""}</p>
      <YouTubeMark href={preview.url} label={strings.linkPreviews.openOnYouTube(title)} />
    </div>
    {#if isOffline}
      <p role="alert" class="flex items-center gap-2xs type-body-sm text-(--mw-component-link-preview-screen-error-text)">
        <Icon name="warning-circle" class="size-(--mw-component-link-preview-action-icon-size) shrink-0" />
        {strings.linkPreviews.offlinePlayer}
      </p>
    {/if}
  </div>
</div>
