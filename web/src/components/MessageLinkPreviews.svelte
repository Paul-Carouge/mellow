<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";

  import { strings } from "../i18n";
  import type { LinkPreview, Snowflake } from "../lib/protocol";
  import { imagePreviewWidth } from "./attachment-preview";
  import GitHubCard from "./embeds/GitHubCard.svelte";
  import MusicCard from "./embeds/MusicCard.svelte";
  import PageCard from "./embeds/PageCard.svelte";
  import PreviewPicture from "./embeds/PreviewPicture.svelte";
  import VideoEmbed from "./embeds/VideoEmbed.svelte";
  import WikipediaCard from "./embeds/WikipediaCard.svelte";
  import {
    knownLinkEmbedOf,
    linkHostName,
    linkPreviewBarColor,
    linkPreviewImageUrl,
    linkPreviewLayout,
    youTubeChannelNameOf,
    youTubeDurationSecondsOf,
    youTubeEmbedOf,
  } from "./link-previews";

  // Link cards under a message body (ADR-0041, ADR-0044, DESIGN.md 5.4 "Link cards"): a picture
  // link alone, a video as a small screen, a provider's card by its kind, or the page card with the
  // site's seal, title, description and picture. Every text is plain.
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
          pageColor: linkPreviewBarColor(preview.theme_color),
          imageUrl: preview.image ? linkPreviewImageUrl(channelId, messageId, preview.id) : null,
        };
      })
      .filter((shown) => shown.layout !== "none"),
  );

  const failedImageIds = new SvelteSet<Snowflake>();
</script>

{#if shownPreviews.length > 0}
  <div class="flex flex-col items-start gap-(--mw-component-link-preview-gap) pt-(--mw-component-attachment-gap)">
    {#each shownPreviews as { preview, youTubeEmbed, embed, layout, title, pageColor, imageUrl } (preview.id)}
      {@const hasPicture = imageUrl !== null && !failedImageIds.has(preview.id)}
      {@const markPictureFailed = () => failedImageIds.add(preview.id)}
      {#if embed?.kind === "github_repository" || embed?.kind === "github_issue" || embed?.kind === "github_pull_request"}
        <GitHubCard {preview} {embed} {imageUrl} {pageColor} />
      {:else if embed?.kind === "wikipedia_article"}
        <WikipediaCard {preview} {embed} {imageUrl} {pageColor} />
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
          {pageColor}
          {imageUrl}
        />
      {:else if layout === "youtube" && youTubeEmbed !== null}
        <VideoEmbed
          {preview}
          video={youTubeEmbed}
          {messageId}
          {title}
          {imageUrl}
          durationSeconds={youTubeDurationSecondsOf(embed, youTubeEmbed)}
          channelName={youTubeChannelNameOf(preview, embed)}
        />
      {:else if layout === "large-picture" || layout === "thumbnail" || layout === "text-only"}
        <PageCard {preview} {title} {layout} {pageColor} {imageUrl} />
      {/if}
    {/each}
  </div>
{/if}
