<script lang="ts" module>
  import { colorNumberOf, siteSealInkOn, type SiteSealInk } from "../link-previews";
  import { colorTokenValue } from "../tokens";

  // The two inks are the same in both themes, so they are read once.
  let sealInks: { dark: number; light: number } | null = null;

  function sealInkOn(pageColor: string): SiteSealInk | null {
    sealInks ??= {
      dark: colorNumberOf(colorTokenValue("component-link-preview-seal-ink-dark")) ?? 0,
      light: colorNumberOf(colorTokenValue("component-link-preview-seal-ink-light")) ?? 0xffffff,
    };
    const color = colorNumberOf(pageColor);
    return color === null ? null : siteSealInkOn(color, sealInks);
  }
</script>

<script lang="ts">
  import type { Snippet } from "svelte";

  import { siteInitialOf } from "../link-previews";

  // A page card's first line (DESIGN.md 5.4 "Link cards"): the site's seal (its colour and initial,
  // decoration only) and its name, then whatever the card puts at the trailing end.
  interface Props {
    /** The page's `theme-color` as `#rrggbb`, or null for the dusk seal. */
    pageColor: string | null;
    label: string;
    /** Said after the name on the same line ("rust-lang/rust" after "GitHub"). */
    detail?: string;
    trailing?: Snippet;
  }

  let { pageColor, label, detail, trailing }: Props = $props();

  const ink = $derived(pageColor === null ? null : sealInkOn(pageColor));
  const initial = $derived(siteInitialOf(label));
</script>

<div class="flex min-w-0 items-center gap-(--mw-component-link-preview-gap)">
  <span
    aria-hidden="true"
    style:background-color={ink === null ? undefined : pageColor}
    class={[
      "grid size-(--mw-component-link-preview-seal-size) shrink-0 place-items-center rounded-(--mw-component-link-preview-seal-radius) type-badge",
      ink === null && "bg-(--mw-component-link-preview-seal-fallback) text-(--mw-component-link-preview-seal-fallback-text)",
      ink === "dark" && "text-(--mw-component-link-preview-seal-ink-dark)",
      ink === "light" && "text-(--mw-component-link-preview-seal-ink-light)",
    ]}>{initial}</span
  >
  <p class="min-w-0 flex-1 truncate type-body-sm text-(--mw-component-link-preview-site-text)">
    <!-- A real space (Svelte trims a bare one here), so the name and the detail stay two words when
         read or copied. -->
    {label}{#if detail}<span class="ms-2xs whitespace-pre">{` ${detail}`}</span>{/if}
  </p>
  {@render trailing?.()}
</div>
