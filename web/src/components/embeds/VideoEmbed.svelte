<script lang="ts">
  import { onDestroy } from "svelte";

  import { strings } from "../../i18n";
  import type { LinkPreview, Snowflake } from "../../lib/protocol";
  import Icon from "../Icon.svelte";
  import {
    clockDurationLabel,
    durationParts,
    EMBED_PLAYER_SANDBOX,
    VIDEO_PLAYER_PERMISSIONS,
    type YouTubeEmbed,
    youTubeEmbedUrl,
  } from "../link-previews";
  import EmbedCard from "./EmbedCard.svelte";
  import { embedPlayerKey, embedPlayers } from "./embed-players.svelte";
  import EmbedTitleLink from "./EmbedTitleLink.svelte";
  import PreviewPicture from "./PreviewPicture.svelte";

  // A YouTube video or Short (docs/design/web-provider-embeds.md 4.1): the instance's poster in a
  // 16:9 box (9:16 for a Short) that becomes the player on a press, from where the link starts.
  // Nothing comes from YouTube before the press (ADR-0041, ADR-0044).
  interface Props {
    preview: LinkPreview;
    video: YouTubeEmbed;
    messageId: Snowflake;
    title: string;
    barColor: string | null;
    imageUrl: string | null;
  }

  let { preview, video, messageId, title, barColor, imageUrl }: Props = $props();

  const playerKey = $derived(embedPlayerKey(messageId, preview.id));
  const isPlaying = $derived(embedPlayers.isOpen(playerKey));
  let isPictureFailed = $state(false);
  let isOffline = $state(false);
  // Set by "Close player" only: a player closed because another opened leaves the focus alone.
  let shouldFocusPlayKey = false;

  const playLabel = $derived.by(() => {
    const name = video.isShort ? strings.linkPreviews.playYouTubeShort(title) : strings.linkPreviews.playYouTubeVideo(title);
    if (video.startSeconds === null) {
      return name;
    }
    const { hours, minutes, seconds } = durationParts(video.startSeconds);
    return name + strings.linkPreviews.startingAt(strings.linkPreviews.spokenDuration(hours, minutes, seconds));
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

<EmbedCard
  {barColor}
  siteName={preview.site_name}
  label={title}
  isFitContent={video.isShort}
  onClosePlayer={isPlaying ? closePlayer : undefined}
>
  <!-- A Short's texts wrap to its player's width instead of widening the card. -->
  <div class={["flex flex-col gap-(--mw-component-link-preview-text-gap)", video.isShort && "contain-inline-size"]}>
    <EmbedTitleLink href={preview.url} {title} />
    {#if preview.description}
      <p
        class="line-clamp-(--mw-component-link-preview-description-lines) type-body break-words whitespace-pre-line text-(--mw-component-link-preview-description-text)"
        >{preview.description}</p
      >
    {/if}
  </div>
  <!-- The final box from the start, poster then player, so nothing moves on play. -->
  <div
    class={[
      "relative overflow-hidden rounded-(--mw-component-link-preview-picture-radius) bg-(--mw-component-link-preview-player-background)",
      video.isShort
        ? "aspect-9/16 w-[calc(var(--mw-component-link-preview-short-player-max-height)*9/16)] max-w-full"
        : "aspect-video w-full",
    ]}
  >
    {#if isPlaying}
      <iframe
        src={youTubeEmbedUrl(video)}
        title={strings.linkPreviews.youTubePlayer(title)}
        allow={VIDEO_PLAYER_PERMISSIONS}
        sandbox={EMBED_PLAYER_SANDBOX}
        referrerpolicy="strict-origin-when-cross-origin"
        class="absolute inset-0 size-full border-0"
        {@attach focusPlayer}
      ></iframe>
    {:else}
      {#if imageUrl !== null && !isPictureFailed}
        <PreviewPicture
          {preview}
          src={imageUrl}
          alt=""
          class="absolute inset-0 size-full object-cover"
          onFailed={() => (isPictureFailed = true)}
        />
      {/if}
      <button
        type="button"
        aria-label={playLabel}
        title={playLabel}
        class="group/play absolute inset-0 grid place-items-center [--mw-color-focus:var(--mw-component-link-preview-play-button-focus-ring)] focus-visible:focus-ring-inset"
        onclick={play}
        {@attach focusPlayKeyAfterClosing}
      >
        <span
          class="grid size-(--mw-component-link-preview-play-button-size) place-items-center rounded-full bg-(--mw-component-link-preview-play-button-background) text-(--mw-component-link-preview-play-button-icon) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) group-hover/play:bg-(--mw-component-link-preview-play-button-background-hover)"
        >
          <Icon name="play" class="size-(--mw-component-link-preview-play-button-icon-size)" />
        </span>
      </button>
      {#if video.startSeconds !== null}
        <!-- Said in the play key's name; shown for the eye. -->
        <span
          aria-hidden="true"
          class="pointer-events-none absolute start-(--mw-component-link-preview-start-chip-inset) bottom-(--mw-component-link-preview-start-chip-inset) rounded-(--mw-component-link-preview-start-chip-radius) bg-(--mw-component-link-preview-start-chip-background) px-xs py-3xs type-body-sm text-(--mw-component-link-preview-start-chip-text) tabular-nums"
        >
          {strings.linkPreviews.startsAt(clockDurationLabel(video.startSeconds))}
        </span>
      {/if}
    {/if}
  </div>
  {#if isOffline}
    <p role="alert" class="flex items-center gap-2xs type-body-sm text-(--mw-component-link-preview-player-error-text)">
      <Icon name="warning-circle" class="size-(--mw-component-link-preview-action-icon-size) shrink-0" />
      {strings.linkPreviews.offlinePlayer}
    </p>
  {/if}
</EmbedCard>
