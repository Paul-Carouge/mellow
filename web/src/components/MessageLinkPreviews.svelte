<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";

  import { strings } from "../i18n";
  import type { LinkPreview, Snowflake } from "../lib/protocol";
  import { imagePreviewWidth } from "./attachment-preview";
  import Icon from "./Icon.svelte";
  import { retryImageOnceThenMarkFailed } from "./image-retry";
  import {
    linkHostName,
    linkPreviewBarColor,
    linkPreviewImageUrl,
    linkPreviewLayout,
    thumbnailWidth,
    youTubeEmbedUrl,
    youTubeVideoIdOf,
  } from "./link-previews";

  // Link previews under a message body (ADR-0041, docs/design/web-link-previews.md): a card per
  // link with the page's colour, name, title and description, its picture large or as a thumbnail;
  // a picture link alone; a YouTube video that plays in place after a click. Every text is plain.
  interface Props {
    previews: readonly LinkPreview[];
    channelId: Snowflake;
    messageId: Snowflake;
  }

  let { previews, channelId, messageId }: Props = $props();

  // Worked out once per previews array: the client keeps it as it was when nothing changed.
  const shownPreviews = $derived(
    previews
      .map((preview) => {
        const youTubeVideoId = youTubeVideoIdOf(preview.url);
        return {
          preview,
          youTubeVideoId,
          layout: linkPreviewLayout(preview, youTubeVideoId),
          title: preview.title ?? linkHostName(preview.url),
          barColor: linkPreviewBarColor(preview.theme_color),
          imageUrl: preview.image ? linkPreviewImageUrl(channelId, messageId, preview.id) : null,
        };
      })
      .filter((shown) => shown.layout !== "none"),
  );

  const playingPreviewIds = new SvelteSet<Snowflake>();
  const retriedImageIds = new SvelteSet<Snowflake>();
  const failedImageIds = new SvelteSet<Snowflake>();

  function focusPlayer(player: HTMLIFrameElement) {
    // The play button that held the focus is gone: the player takes it, so keys reach the video.
    player.focus();
  }
</script>

{#snippet picture(preview: LinkPreview, imageUrl: string, className: string, width: string | undefined, alt: string)}
  {#key retriedImageIds.has(preview.id)}
    <img
      src={imageUrl}
      {alt}
      width={preview.image?.width}
      height={preview.image?.height}
      loading="lazy"
      decoding="async"
      style:width
      class={className}
      onerror={() => retryImageOnceThenMarkFailed(preview.id, retriedImageIds, failedImageIds)}
    />
  {/key}
{/snippet}

{#if shownPreviews.length > 0}
  <div class="flex flex-col items-start gap-(--mw-component-link-preview-gap) pt-(--mw-component-attachment-gap)">
    {#each shownPreviews as { preview, youTubeVideoId, layout, title, barColor, imageUrl } (preview.id)}
      {@const hasPicture = imageUrl !== null && !failedImageIds.has(preview.id)}
      {#if layout === "picture-only"}
        {#if hasPicture}
          <a
            href={preview.url}
            target="_blank"
            rel="noopener noreferrer"
            title={preview.url}
            style:width={imagePreviewWidth(preview.image ?? {})}
            class="block max-w-[min(100%,var(--mw-component-attachment-image-max-width))] overflow-hidden rounded-(--mw-component-attachment-image-radius) bg-(--mw-component-attachment-image-placeholder) focus-visible:focus-ring"
          >
            {@render picture(preview, imageUrl, "block h-auto w-full", undefined, strings.linkPreviews.pictureFrom(linkHostName(preview.url)))}
          </a>
        {/if}
      {:else}
        <div
          class="flex w-full max-w-(--mw-component-link-preview-max-width) gap-(--mw-component-link-preview-gap) rounded-(--mw-component-link-preview-radius) border-(length:--mw-component-link-preview-border-width) border-(--mw-component-link-preview-border) bg-(--mw-component-link-preview-background) p-(--mw-component-link-preview-padding)"
        >
          <!-- The page's colour is decoration only; without one, the quote bar's dusk. -->
          <span
            aria-hidden="true"
            style:background-color={barColor}
            class="w-(--mw-component-link-preview-bar-width) shrink-0 self-stretch rounded-full bg-(--mw-component-link-preview-bar-fallback)"
          ></span>
          <div class="flex min-w-0 flex-1 flex-col gap-(--mw-component-link-preview-gap)">
            <div class="flex items-start gap-(--mw-component-link-preview-gap)">
              <div class="flex min-w-0 flex-1 flex-col gap-(--mw-component-link-preview-text-gap)">
                {#if preview.site_name}
                  <p class="type-body-sm break-words text-(--mw-component-link-preview-site-text)">{preview.site_name}</p>
                {/if}
                <a
                  href={preview.url}
                  target="_blank"
                  rel="noopener noreferrer"
                  class="self-start rounded-xs type-body-lg font-semibold break-words text-(--mw-component-link-preview-title-text) hover:underline focus-visible:focus-ring"
                  >{title}</a
                >
                {#if preview.description}
                  <p class="line-clamp-3 type-body break-words whitespace-pre-line text-(--mw-component-link-preview-description-text)"
                    >{preview.description}</p
                  >
                {/if}
              </div>
              {#if layout === "thumbnail" && hasPicture && preview.image}
                <!-- The title link is the keyboard's way to the page; the picture is a larger pointer target. -->
                <a href={preview.url} target="_blank" rel="noopener noreferrer" tabindex="-1" aria-hidden="true" class="shrink-0">
                  {@render picture(
                    preview,
                    imageUrl,
                    "block h-auto rounded-(--mw-component-link-preview-picture-radius) bg-(--mw-component-link-preview-picture-placeholder)",
                    thumbnailWidth(preview.image),
                    "",
                  )}
                </a>
              {/if}
            </div>
            {#if layout === "large-picture" && hasPicture && preview.image}
              <a href={preview.url} target="_blank" rel="noopener noreferrer" tabindex="-1" aria-hidden="true" class="block max-w-full">
                {@render picture(
                  preview,
                  imageUrl,
                  "block h-auto max-w-full rounded-(--mw-component-link-preview-picture-radius) bg-(--mw-component-link-preview-picture-placeholder)",
                  `min(100%, ${preview.image.width}px)`,
                  "",
                )}
              </a>
            {:else if layout === "youtube" && youTubeVideoId !== null}
              <!-- 16:9 from the start, picture then player, so nothing moves on play. Nothing comes
                   from YouTube before the click (ADR-0041). -->
              <div
                class="relative aspect-video w-full overflow-hidden rounded-(--mw-component-link-preview-picture-radius) bg-(--mw-component-link-preview-player-background)"
              >
                {#if playingPreviewIds.has(preview.id)}
                  <iframe
                    src={youTubeEmbedUrl(youTubeVideoId)}
                    title={strings.linkPreviews.youTubePlayer(title)}
                    allow="autoplay; encrypted-media; picture-in-picture; fullscreen"
                    referrerpolicy="strict-origin-when-cross-origin"
                    class="absolute inset-0 size-full border-0"
                    {@attach focusPlayer}
                  ></iframe>
                {:else}
                  {#if hasPicture}
                    {@render picture(preview, imageUrl, "absolute inset-0 size-full object-cover", undefined, "")}
                  {/if}
                  <button
                    type="button"
                    aria-label={strings.linkPreviews.playYouTubeVideo(title)}
                    title={strings.linkPreviews.playYouTubeVideo(title)}
                    class="group/play absolute inset-0 grid place-items-center [--mw-color-focus:var(--mw-component-link-preview-play-button-focus-ring)] focus-visible:focus-ring-inset"
                    onclick={() => playingPreviewIds.add(preview.id)}
                  >
                    <span
                      class="grid size-(--mw-component-link-preview-play-button-size) place-items-center rounded-full bg-(--mw-component-link-preview-play-button-background) text-(--mw-component-link-preview-play-button-icon) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) group-hover/play:bg-(--mw-component-link-preview-play-button-background-hover)"
                    >
                      <Icon name="play" class="size-(--mw-component-link-preview-play-button-icon-size)" />
                    </span>
                  </button>
                {/if}
              </div>
            {/if}
          </div>
        </div>
      {/if}
    {/each}
  </div>
{/if}
