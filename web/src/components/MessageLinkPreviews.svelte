<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";

  import { strings } from "../i18n";
  import type { LinkPreview, Snowflake } from "../lib/protocol";
  import { imagePreviewWidth } from "./attachment-preview";
  import EmbedCard from "./embeds/EmbedCard.svelte";
  import EmbedTitleLink from "./embeds/EmbedTitleLink.svelte";
  import GitHubCard from "./embeds/GitHubCard.svelte";
  import MusicCard from "./embeds/MusicCard.svelte";
  import PreviewPicture from "./embeds/PreviewPicture.svelte";
  import VideoEmbed from "./embeds/VideoEmbed.svelte";
  import WikipediaCard from "./embeds/WikipediaCard.svelte";
  import {
    knownLinkEmbedOf,
    linkHostName,
    linkPreviewBarColor,
    linkPreviewImageUrl,
    linkPreviewLayout,
    thumbnailWidth,
    youTubeEmbedOf,
  } from "./link-previews";

  // Link previews under a message body (ADR-0041, ADR-0044, docs/design/web-link-previews.md and
  // web-provider-embeds.md): a picture link alone, a provider's embed by its kind, or the plain
  // card with the page's colour, name, title, description and picture. Every text is plain.
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
        const youTubeEmbed = youTubeEmbedOf(preview.url);
        const embed = knownLinkEmbedOf(preview);
        return {
          preview,
          youTubeEmbed,
          embed,
          layout: linkPreviewLayout(preview, youTubeEmbed, embed),
          title: preview.title ?? linkHostName(preview.url),
          barColor: linkPreviewBarColor(preview.theme_color),
          imageUrl: preview.image ? linkPreviewImageUrl(channelId, messageId, preview.id) : null,
        };
      })
      .filter((shown) => shown.layout !== "none"),
  );

  const failedImageIds = new SvelteSet<Snowflake>();
</script>

{#if shownPreviews.length > 0}
  <div class="flex flex-col items-start gap-(--mw-component-link-preview-gap) pt-(--mw-component-attachment-gap)">
    {#each shownPreviews as { preview, youTubeEmbed, embed, layout, title, barColor, imageUrl } (preview.id)}
      {@const hasPicture = imageUrl !== null && !failedImageIds.has(preview.id)}
      {@const markPictureFailed = () => failedImageIds.add(preview.id)}
      {#if embed?.kind === "github_repository" || embed?.kind === "github_issue" || embed?.kind === "github_pull_request"}
        <GitHubCard {preview} {embed} {imageUrl} {barColor} />
      {:else if embed?.kind === "wikipedia_article"}
        <WikipediaCard {preview} {embed} {imageUrl} {barColor} />
      {:else if layout === "picture-only"}
        {#if hasPicture}
          <a
            href={preview.url}
            target="_blank"
            rel="noopener noreferrer"
            title={preview.url}
            style:width={imagePreviewWidth(preview.image ?? {})}
            class="block max-w-[min(100%,var(--mw-component-attachment-image-max-width))] overflow-hidden rounded-(--mw-component-attachment-image-radius) bg-(--mw-component-attachment-image-placeholder) focus-visible:focus-ring"
          >
            <PreviewPicture
              {preview}
              src={imageUrl}
              alt={strings.linkPreviews.pictureFrom(linkHostName(preview.url))}
              class="block h-auto w-full"
              onFailed={markPictureFailed}
            />
          </a>
        {/if}
      {:else if layout === "youtube-music" && youTubeEmbed !== null && embed?.kind === "youtube_music_track"}
        <MusicCard
          {preview}
          track={youTubeEmbed}
          artist={embed.artist}
          service={preview.site_name ?? "YouTube Music"}
          {messageId}
          {title}
          {barColor}
          {imageUrl}
        />
      {:else if layout === "youtube" && youTubeEmbed !== null}
        <VideoEmbed {preview} video={youTubeEmbed} {messageId} {title} {barColor} {imageUrl} />
      {:else}
        <EmbedCard {barColor} siteName={undefined}>
          <div class="flex items-start gap-(--mw-component-link-preview-gap)">
            <div class="flex min-w-0 flex-1 flex-col gap-(--mw-component-link-preview-text-gap)">
              {#if preview.site_name}
                <p class="type-body-sm break-words text-(--mw-component-link-preview-site-text)">{preview.site_name}</p>
              {/if}
              <EmbedTitleLink href={preview.url} {title} />
              {#if preview.description}
                <p
                  class="line-clamp-(--mw-component-link-preview-description-lines) type-body break-words whitespace-pre-line text-(--mw-component-link-preview-description-text)"
                  >{preview.description}</p
                >
              {/if}
            </div>
            {#if layout === "thumbnail" && hasPicture && preview.image}
              <!-- The title link is the keyboard's way to the page; the picture is a larger pointer target. -->
              <a href={preview.url} target="_blank" rel="noopener noreferrer" tabindex="-1" aria-hidden="true" class="shrink-0">
                <PreviewPicture
                  {preview}
                  src={imageUrl}
                  alt=""
                  class="block h-auto rounded-(--mw-component-link-preview-picture-radius) bg-(--mw-component-link-preview-picture-placeholder)"
                  width={thumbnailWidth(preview.image)}
                  onFailed={markPictureFailed}
                />
              </a>
            {/if}
          </div>
          {#if layout === "large-picture" && hasPicture && preview.image}
            <a href={preview.url} target="_blank" rel="noopener noreferrer" tabindex="-1" aria-hidden="true" class="block max-w-full">
              <PreviewPicture
                {preview}
                src={imageUrl}
                alt=""
                class="block h-auto max-w-full rounded-(--mw-component-link-preview-picture-radius) bg-(--mw-component-link-preview-picture-placeholder)"
                width={`min(100%, ${preview.image.width}px)`}
                onFailed={markPictureFailed}
              />
            </a>
          {/if}
        </EmbedCard>
      {/if}
    {/each}
  </div>
{/if}
