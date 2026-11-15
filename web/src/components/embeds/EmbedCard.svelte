<script lang="ts">
  import type { Snippet } from "svelte";

  import { strings } from "../../i18n";
  import IconButton from "../IconButton.svelte";
  import SiteLabel from "./SiteLabel.svelte";

  // The page frame provider cards share (DESIGN.md 5.4 "Link cards"): a piece of the ground headed
  // by the site's seal and name, the kind's layout under it. Kinds change what sits inside, never
  // the frame. Videos are screens instead (VideoEmbed).
  interface Props {
    /** The page's colour for the seal, or null for the dusk seal. */
    pageColor: string | null;
    siteName: string;
    /** Names the card for screen readers ("title, artist, service"); none for a plain card. */
    label?: string;
    /** Widens the card so its 16:9 YouTube player gets `video-player-width` when the column has room. */
    hasVideoPlayer?: boolean;
    /** Shows "Close player" on the first line while the card's player is open. */
    onClosePlayer?: () => void;
    children: Snippet;
  }

  let { pageColor, siteName, label, hasVideoPlayer = false, onClosePlayer, children }: Props = $props();
</script>

<div
  role={label === undefined ? undefined : "group"}
  aria-label={label}
  class={[
    "flex w-full flex-col gap-(--mw-component-link-preview-gap) rounded-(--mw-component-link-preview-radius) border-(length:--mw-component-link-preview-border-width) border-(--mw-component-link-preview-border) bg-(--mw-component-link-preview-background) p-(--mw-component-link-preview-padding)",
    // The player's width plus what surrounds it in the card: padding and border.
    hasVideoPlayer
      ? "max-w-[calc(var(--mw-component-link-preview-video-player-width)+2*var(--mw-component-link-preview-padding)+2*var(--mw-component-link-preview-border-width))]"
      : "max-w-(--mw-component-link-preview-max-width)",
  ]}
>
  <SiteLabel {pageColor} label={siteName}>
    {#snippet trailing()}
      {#if onClosePlayer}
        <!-- Negative margins: the key appears on play without pushing the card's content down. -->
        <IconButton
          icon="x"
          label={strings.linkPreviews.closePlayer}
          class="-my-xs size-(--mw-component-button-height-small) text-(--mw-component-link-preview-close-player-icon) hover:bg-state-hover hover:text-text-heading focus-visible:focus-ring pointer-coarse:-my-sm"
          iconClass="size-(--mw-component-link-preview-action-icon-size)"
          onclick={onClosePlayer}
        />
      {/if}
    {/snippet}
  </SiteLabel>
  {@render children()}
</div>
