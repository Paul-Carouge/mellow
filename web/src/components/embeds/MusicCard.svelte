<script lang="ts">
  import { onDestroy } from "svelte";

  import { strings } from "../../i18n";
  import type { LinkPreview, Snowflake } from "../../lib/protocol";
  import Icon from "../Icon.svelte";
  import { EMBED_PLAYER_SANDBOX, MUSIC_PLAYER_PERMISSIONS, type YouTubeEmbed, youTubeEmbedUrl } from "../link-previews";
  import EmbedCard from "./EmbedCard.svelte";
  import { embedPlayerKey, embedPlayers } from "./embed-players.svelte";
  import EmbedTitleLink from "./EmbedTitleLink.svelte";
  import PreviewPicture from "./PreviewPicture.svelte";

  // A track (docs/design/web-provider-embeds.md 4.2): its cover, title and artist, and a Listen
  // key. YouTube's terms forbid sound without its visible player, so Listen opens the YouTube
  // player under the row, which keeps the track's identity in view while it plays.
  interface Props {
    preview: LinkPreview;
    /** Where the player's address comes from: the link's id, checked strictly. */
    track: YouTubeEmbed;
    artist: string;
    service: string;
    messageId: Snowflake;
    title: string;
    barColor: string | null;
    imageUrl: string | null;
  }

  let { preview, track, artist, service, messageId, title, barColor, imageUrl }: Props = $props();

  const id = $props.id();
  const playerId = `${id}-player`;
  const loadsPlayerId = `${id}-loads-player`;
  const playerKey = $derived(embedPlayerKey(messageId, preview.id));
  const isPlaying = $derived(embedPlayers.isOpen(playerKey));
  let isPictureFailed = $state(false);
  let isOffline = $state(false);

  onDestroy(() => embedPlayers.close(playerKey));

  function togglePlayer() {
    if (isPlaying) {
      embedPlayers.close(playerKey);
      return;
    }
    isOffline = !navigator.onLine;
    if (!isOffline) {
      embedPlayers.open(playerKey);
    }
  }

  function focusPlayer(player: HTMLIFrameElement) {
    player.focus();
  }
</script>

<EmbedCard {barColor} siteName={service} label={`${title}, ${artist}, ${service}`} hasVideoPlayer>
  <!-- The text column keeps text-min-width beside the cover, else wraps under it. -->
  <div class="flex flex-wrap items-start gap-(--mw-component-link-preview-gap)">
    <div
      class="grid aspect-square w-[clamp(var(--mw-component-link-preview-cover-size-min),30%,var(--mw-component-link-preview-cover-size))] shrink-0 place-items-center overflow-hidden rounded-(--mw-component-link-preview-cover-radius) bg-(--mw-component-link-preview-cover-placeholder) text-(--mw-component-link-preview-cover-placeholder-icon)"
    >
      {#if imageUrl !== null && !isPictureFailed}
        <!-- The title link is the keyboard's way to the page; the cover is the art, already named. -->
        <PreviewPicture
          {preview}
          src={imageUrl}
          alt=""
          class="col-start-1 row-start-1 size-full object-cover"
          onFailed={() => (isPictureFailed = true)}
        />
      {:else}
        <Icon name="music-notes" class="size-(--mw-component-link-preview-play-button-icon-size)" />
      {/if}
    </div>
    <div class="flex min-w-0 flex-1 basis-(--mw-component-link-preview-text-min-width) flex-col items-start gap-(--mw-component-link-preview-text-gap)">
      <EmbedTitleLink href={preview.url} {title} />
      <p class="type-body break-words text-(--mw-component-link-preview-description-text)">{artist}</p>
      <button
        type="button"
        aria-label={isPlaying ? undefined : strings.linkPreviews.listenOn(title, artist, service)}
        aria-describedby={isPlaying ? undefined : loadsPlayerId}
        aria-expanded={isPlaying}
        aria-controls={isPlaying ? playerId : undefined}
        title={isPlaying ? undefined : strings.linkPreviews.loadsPlayer(service)}
        class="mt-(--mw-component-link-preview-text-gap) inline-flex h-(--mw-component-button-height-small) items-center gap-2xs rounded-(--mw-component-button-radius) bg-(--mw-component-button-secondary-background) px-sm type-button text-(--mw-component-button-secondary-text) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:bg-(--mw-component-button-secondary-background-hover) focus-visible:focus-ring pointer-coarse:h-(--mw-size-hit-web-min)"
        onclick={togglePlayer}
      >
        <Icon name={isPlaying ? "caret-up" : "play"} class="size-(--mw-component-link-preview-action-icon-size)" />
        {isPlaying ? strings.linkPreviews.hidePlayer : strings.linkPreviews.listen}
      </button>
      <span id={loadsPlayerId} hidden>{strings.linkPreviews.loadsPlayer(service)}</span>
    </div>
  </div>
  {#if isPlaying}
    <div
      id={playerId}
      class="relative aspect-video w-full overflow-hidden rounded-(--mw-component-link-preview-picture-radius) bg-(--mw-component-link-preview-player-background)"
    >
      <iframe
        src={youTubeEmbedUrl(track)}
        title={strings.linkPreviews.playerTitle(service, title)}
        allow={MUSIC_PLAYER_PERMISSIONS}
        sandbox={EMBED_PLAYER_SANDBOX}
        referrerpolicy="strict-origin-when-cross-origin"
        class="absolute inset-0 size-full border-0"
        {@attach focusPlayer}
      ></iframe>
    </div>
  {/if}
  {#if isOffline}
    <p role="alert" class="flex items-center gap-2xs type-body-sm text-(--mw-component-link-preview-player-error-text)">
      <Icon name="warning-circle" class="size-(--mw-component-link-preview-action-icon-size) shrink-0" />
      {strings.linkPreviews.offlinePlayer}
    </p>
  {/if}
</EmbedCard>
