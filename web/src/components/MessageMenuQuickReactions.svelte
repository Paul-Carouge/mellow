<script lang="ts">
  import { strings } from "../i18n";
  import type { ReactionSummary } from "../lib/protocol";
  import { reactionKey } from "../lib/reactions";
  import { noteEmojiUsed, quickReactionEmoji } from "./recent-emoji.svelte";

  // The first row of the message menu's sheet on phones (DESIGN.md §5.4): the quick reactions the
  // hover bar shows with a mouse, as large keys drawn like the message's own reaction chips, so
  // the reader sees what they are about to add; one they already gave is checked and takes it back.
  interface Props {
    reactions: readonly ReactionSummary[];
    onreact: (emoji: string) => void;
  }

  let { reactions, onreact }: Props = $props();
</script>

<div role="group" aria-label={strings.reactions.addReaction} class="flex gap-xs px-(--mw-component-context-menu-item-padding-inline) py-2xs">
  {#each quickReactionEmoji() as emoji (emoji)}
    {@const isGiven = reactions.some((reaction) => reaction.me && reaction.emoji === reactionKey(emoji))}
    <button
      type="button"
      role="menuitemcheckbox"
      aria-checked={isGiven}
      aria-label={strings.reactions.reactWith(emoji)}
      class={[
        "inline-flex size-(--mw-size-mobile-row-height) items-center justify-center rounded-(--mw-component-reaction-radius) border-(length:--mw-component-reaction-border-width) text-(length:--mw-component-emoji-picker-emoji-size-coarse) leading-none focus-visible:focus-ring",
        isGiven
          ? "border-(--mw-component-reaction-border-me) bg-(--mw-component-reaction-background-me)"
          : "border-(--mw-component-reaction-border) bg-(--mw-component-reaction-background) active:bg-(--mw-component-reaction-background-hover)",
      ]}
      onclick={() => {
        noteEmojiUsed(emoji);
        onreact(emoji);
      }}>{emoji}</button
    >
  {/each}
</div>
