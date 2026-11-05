<script lang="ts">
  import type { Snippet } from "svelte";

  import { strings } from "../../i18n";
  import IconButton from "../IconButton.svelte";

  // The frame every link preview shares (docs/design/web-provider-embeds.md principle 2): the
  // page's colour as a pill, the site's name on the first line, and the kind's layout under it.
  // Kinds change what sits inside, never the frame.
  interface Props {
    /** The page's colour, or null for the quote bar's dusk. */
    barColor: string | null;
    siteName: string | undefined;
    /** Names the card for screen readers ("title, artist, service"); none for a plain card. */
    label?: string;
    /** A Short's card shrinks to its 9:16 player instead of the column's width. */
    isFitContent?: boolean;
    /** Widens the card so its 16:9 YouTube player gets `video-player-width` when the column has room. */
    hasVideoPlayer?: boolean;
    /** Shows "Close player" on the first line while the card's player is open. */
    onClosePlayer?: () => void;
    children: Snippet;
  }

  let { barColor, siteName, label, isFitContent = false, hasVideoPlayer = false, onClosePlayer, children }: Props = $props();
</script>

<div
  role={label === undefined ? undefined : "group"}
  aria-label={label}
  class={[
    "flex gap-(--mw-component-link-preview-gap) rounded-(--mw-component-link-preview-radius) border-(length:--mw-component-link-preview-border-width) border-(--mw-component-link-preview-border) bg-(--mw-component-link-preview-background) p-(--mw-component-link-preview-padding)",
    isFitContent ? "w-fit" : "w-full",
    // The player's width plus what surrounds it in the card: padding, border, pill and gap.
    hasVideoPlayer
      ? "max-w-[calc(var(--mw-component-link-preview-video-player-width)+2*var(--mw-component-link-preview-padding)+2*var(--mw-component-link-preview-border-width)+var(--mw-component-link-preview-bar-width)+var(--mw-component-link-preview-gap))]"
      : "max-w-(--mw-component-link-preview-max-width)",
  ]}
>
  <!-- The page's colour is decoration only; without one, the quote bar's dusk. -->
  <span
    aria-hidden="true"
    style:background-color={barColor}
    class="w-(--mw-component-link-preview-bar-width) shrink-0 self-stretch rounded-full bg-(--mw-component-link-preview-bar-fallback)"
  ></span>
  <div class="flex min-w-0 flex-1 flex-col gap-(--mw-component-link-preview-gap)">
    {#if siteName || onClosePlayer}
      <div class="flex min-w-0 items-start gap-(--mw-component-link-preview-gap)">
        {#if siteName}
          <p class="min-w-0 flex-1 type-body-sm break-words text-(--mw-component-link-preview-site-text)">{siteName}</p>
        {/if}
        {#if onClosePlayer}
          <!-- Negative margins: the key appears on play without pushing the card's content down. -->
          <IconButton
            icon="x"
            label={strings.linkPreviews.closePlayer}
            class="ms-auto -my-xs pointer-coarse:-my-sm size-(--mw-component-button-height-small) text-(--mw-component-link-preview-close-player-icon) hover:bg-state-hover hover:text-text-heading focus-visible:focus-ring"
            iconClass="size-(--mw-component-link-preview-action-icon-size)"
            onclick={onClosePlayer}
          />
        {/if}
      </div>
    {/if}
    {@render children()}
  </div>
</div>
