<script lang="ts">
  import { SvelteSet } from "svelte/reactivity";

  import { language, strings } from "../../i18n";
  import type { LinkPreview, Snowflake, WikipediaArticleEmbed } from "../../lib/protocol";
  import { retryImageOnceThenMarkFailed } from "../image-retry";
  import { thumbnailWidth } from "../link-previews";
  import { wikipediaLanguageName } from "./provider-cards";

  // A Wikipedia article (docs/design/web-provider-embeds.md §4.6): the thumbnail card with the
  // article's summary, six lines, and the wiki's language when it is not the reader's.
  interface Props {
    preview: LinkPreview;
    embed: WikipediaArticleEmbed;
    /** The thumbnail as the instance stored it, or null. */
    imageUrl: string | null;
    barColor: string | null;
  }

  let { preview, embed, imageUrl, barColor }: Props = $props();

  const languageName = $derived(wikipediaLanguageName(embed.language, language));
  const retriedImageIds = new SvelteSet<Snowflake>();
  const failedImageIds = new SvelteSet<Snowflake>();
  const summary = $derived(embed.is_disambiguation ? strings.providerCards.disambiguation : preview.description);
</script>

<div
  role="group"
  aria-label={preview.title ?? preview.url}
  class="flex w-full max-w-(--mw-component-link-preview-max-width) gap-(--mw-component-link-preview-gap) rounded-(--mw-component-link-preview-radius) border-(length:--mw-component-link-preview-border-width) border-(--mw-component-link-preview-border) bg-(--mw-component-link-preview-background) p-(--mw-component-link-preview-padding)"
>
  <!-- The page's colour is decoration only; without one, the quote bar's dusk. -->
  <span
    aria-hidden="true"
    style:background-color={barColor}
    class="w-(--mw-component-link-preview-bar-width) shrink-0 self-stretch rounded-full bg-(--mw-component-link-preview-bar-fallback)"
  ></span>
  <div class="flex min-w-0 flex-1 items-start gap-(--mw-component-link-preview-gap)">
    <div class="flex min-w-0 flex-1 flex-col gap-(--mw-component-link-preview-text-gap)">
      <p class="type-body-sm break-words text-(--mw-component-link-preview-site-text)">
        {languageName === null ? strings.providerCards.wikipedia : strings.providerCards.wikipediaIn(languageName)}
      </p>
      <a
        href={preview.url}
        target="_blank"
        rel="noopener noreferrer"
        class="inline-flex min-h-(--mw-size-hit-desktop-min) items-center self-start rounded-xs type-body-lg font-semibold break-words text-(--mw-component-link-preview-title-text) hover:underline focus-visible:focus-ring pointer-coarse:min-h-(--mw-size-hit-web-min)"
        >{preview.title ?? preview.url}</a
      >
      {#if summary}
        <p
          class="line-clamp-(--mw-component-link-preview-wikipedia-extract-lines) type-body break-words text-(--mw-component-link-preview-description-text)"
        >
          {summary}
        </p>
      {/if}
    </div>
    {#if imageUrl !== null && preview.image && !failedImageIds.has(preview.id)}
      <!-- The title link is the keyboard's way to the article; the picture is a larger pointer target. -->
      <a href={preview.url} target="_blank" rel="noopener noreferrer" tabindex="-1" aria-hidden="true" class="shrink-0">
        {#key retriedImageIds.has(preview.id)}
          <img
            src={imageUrl}
            alt=""
            width={preview.image.width}
            height={preview.image.height}
            loading="lazy"
            decoding="async"
            style:width={thumbnailWidth(preview.image)}
            class="block h-auto rounded-(--mw-component-link-preview-picture-radius) bg-(--mw-component-link-preview-picture-placeholder)"
            onerror={() => retryImageOnceThenMarkFailed(preview.id, retriedImageIds, failedImageIds)}
          />
        {/key}
      </a>
    {/if}
  </div>
</div>
