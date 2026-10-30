<script lang="ts">
  import { prefersReducedMotion } from "../lib/motion.svelte";
  import { type CustomEmoji, emojiUrl } from "../lib/protocol";

  // A server emoji's picture (DESIGN.md §6.58): `:name:` as its text alternative and tooltip, the
  // still frame under reduced motion, and the name as text if the picture cannot be loaded.
  // Hidden windows cost nothing: browsers neither paint nor advance images they do not show.
  interface Props {
    emoji: CustomEmoji;
    /** As written in the message; the emoji's own name otherwise. */
    name?: string;
    /** Size utilities from a component's tokens. */
    class: string;
    /** Inside a control that already names it (a reaction chip, a picker cell). */
    isDecorative?: boolean;
  }

  let { emoji, name = emoji.name, class: className, isDecorative = false }: Props = $props();

  const source = $derived(emojiUrl(emoji, !prefersReducedMotion.current));
  let failedSource = $state<string | null>(null);
  const label = $derived(`:${name}:`);
</script>

{#if failedSource === source}
  <span title={isDecorative ? undefined : label}>{label}</span>
{:else}
  <img
    src={source}
    alt={isDecorative ? "" : label}
    title={isDecorative ? undefined : label}
    loading="lazy"
    draggable="false"
    class={["inline-block shrink-0 object-contain align-bottom", className]}
    onerror={() => (failedSource = source)}
  />
{/if}
