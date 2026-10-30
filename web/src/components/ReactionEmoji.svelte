<script lang="ts">
  import CustomEmojiImage from "./CustomEmojiImage.svelte";
  import type { ReactionAppearance } from "./custom-emoji";

  // The emoji inside a reaction chip (DESIGN.md §5.4, §6.58): a built-in emoji, a server emoji's
  // picture, or a dashed outline where a deleted emoji's picture was. Decorative: the chip names it.
  interface Props {
    appearance: ReactionAppearance;
  }

  let { appearance }: Props = $props();
</script>

{#if appearance.kind === "unicode"}
  <span aria-hidden="true" class="text-(length:--mw-component-reaction-emoji-size) leading-none">{appearance.emoji}</span>
{:else if appearance.kind === "custom"}
  <CustomEmojiImage emoji={appearance.emoji} class="size-(--mw-component-reaction-emoji-size)" isDecorative />
{:else}
  <span
    aria-hidden="true"
    class="size-(--mw-component-reaction-emoji-size) shrink-0 rounded-(--mw-component-custom-emoji-missing-radius) border-(length:--mw-component-custom-emoji-missing-border-width) border-dashed border-(--mw-component-custom-emoji-missing-border)"
  ></span>
{/if}
