<script lang="ts">
  import type { CustomEmoji, Snowflake } from "../lib/protocol";
  import CustomEmojiImage from "./CustomEmojiImage.svelte";
  import { reactionAppearance } from "./custom-emoji";
  import Icon from "./Icon.svelte";

  // A soundboard sound's picture (docs/design/web-soundboard.md): its emoji (a reaction key), else
  // the speaker glyph, so every sound has a shape to find it by. Always decorative: the sound's
  // name is said by the control around it.
  interface Props {
    emojiKey: string | null | undefined;
    /** The custom emoji of the user's servers (`MellowClient.knownCustomEmojis`). */
    customEmojis: ReadonlyMap<Snowflake, CustomEmoji>;
    /** A length token, as a CSS custom property name: the picture's size. */
    sizeToken: string;
    /** Colour of the speaker glyph. */
    glyphClass?: string;
  }

  let { emojiKey, customEmojis, sizeToken, glyphClass = "" }: Props = $props();

  const appearance = $derived(emojiKey ? reactionAppearance(emojiKey, customEmojis) : null);
</script>

<span aria-hidden="true" class="inline-flex shrink-0 items-center justify-center" style:--sound-emoji-size="var({sizeToken})">
  {#if appearance?.kind === "unicode"}
    <span class="text-(length:--sound-emoji-size) leading-none">{appearance.emoji}</span>
  {:else if appearance?.kind === "custom"}
    <CustomEmojiImage emoji={appearance.emoji} class="size-(--sound-emoji-size)" isDecorative />
  {:else}
    <!-- No emoji, or a deleted server emoji: the speaker stands in. -->
    <Icon name="speaker-high" class="size-(--sound-emoji-size) {glyphClass}" />
  {/if}
</span>
