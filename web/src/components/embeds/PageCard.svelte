<script lang="ts">
  import type { LinkPreview } from "../../lib/protocol";
  import Icon from "../Icon.svelte";
  import { siteLabelOf, thumbnailWidth } from "../link-previews";
  import PreviewPicture from "./PreviewPicture.svelte";
  import SiteLabel from "./SiteLabel.svelte";

  // A page (DESIGN.md 5.4 "Link cards"): the site's seal and name, the title, the description and
  // the page's picture, as a hero across the top when it is a wide banner or a thumbnail at the
  // trailing top otherwise. The whole card is the link: it lights up under the pointer like a row,
  // and keyboards reach it through its title, its only Tab stop.
  interface Props {
    preview: LinkPreview;
    title: string;
    layout: "large-picture" | "thumbnail" | "text-only";
    pageColor: string | null;
    imageUrl: string | null;
  }

  let { preview, title, layout, pageColor, imageUrl }: Props = $props();

  let isPictureFailed = $state(false);
  const picture = $derived(imageUrl !== null && !isPictureFailed && preview.image ? { src: imageUrl, ...preview.image } : null);
</script>

<div
  class="group/page relative flex w-full max-w-(--mw-component-link-preview-max-width) flex-col overflow-hidden rounded-(--mw-component-link-preview-radius) border-(length:--mw-component-link-preview-border-width) border-(--mw-component-link-preview-border) bg-(--mw-component-link-preview-background) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:bg-(--mw-component-link-preview-card-background-hover) has-[a:focus-visible]:focus-ring"
>
  {#if layout === "large-picture" && picture !== null}
    <!-- Sized before it loads: the picture's own shape across the card's width. -->
    <PreviewPicture
      {preview}
      src={picture.src}
      alt=""
      class="block h-auto w-full bg-(--mw-component-link-preview-picture-placeholder) object-cover"
      onFailed={() => (isPictureFailed = true)}
    />
  {/if}
  <div class="flex items-start gap-(--mw-component-link-preview-gap) p-(--mw-component-link-preview-padding)">
    <div class="flex min-w-0 flex-1 flex-col gap-(--mw-component-link-preview-text-gap)">
      <SiteLabel {pageColor} label={siteLabelOf(preview)}>
        {#snippet trailing()}
          {#if layout !== "thumbnail" || picture === null}
            <Icon name="arrow-square-out" class="size-(--mw-component-link-preview-action-icon-size) shrink-0 text-(--mw-component-link-preview-external-icon)" />
          {/if}
        {/snippet}
      </SiteLabel>
      <!-- Stretched over the card: a press anywhere opens the page, and the card is one Tab stop. -->
      <a
        href={preview.url}
        target="_blank"
        rel="noopener noreferrer"
        title={preview.url}
        class="mt-(--mw-component-link-preview-text-gap) type-body-lg font-semibold break-words text-(--mw-component-link-preview-page-title-text) outline-none group-hover/page:underline after:absolute after:inset-0 after:content-['']"
        >{title}</a
      >
      {#if preview.description}
        <p
          class="line-clamp-(--mw-component-link-preview-description-lines) type-body break-words whitespace-pre-line text-(--mw-component-link-preview-description-text)"
          >{preview.description}</p
        >
      {/if}
    </div>
    {#if layout === "thumbnail" && picture !== null}
      <PreviewPicture
        {preview}
        src={picture.src}
        alt=""
        class="block h-auto shrink-0 rounded-(--mw-component-link-preview-picture-radius) bg-(--mw-component-link-preview-picture-placeholder)"
        width={thumbnailWidth(picture)}
        onFailed={() => (isPictureFailed = true)}
      />
    {/if}
  </div>
</div>
