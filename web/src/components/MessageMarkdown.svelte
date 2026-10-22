<script lang="ts">
  import type { Snippet } from "svelte";
  import { SvelteSet } from "svelte/reactivity";

  import { strings } from "../i18n";
  import type { BlockNode, InlineNode } from "./markdown";

  // A message body parsed by `markdown.ts` (DESIGN.md §5.4). Every node becomes an element or a
  // text node here: message text is never inserted as HTML.
  interface Props {
    blocks: readonly BlockNode[];
    /** Shown at the end of the last paragraph, as Discord places "(edited)"; on its own line otherwise. */
    trailing?: Snippet;
  }

  let { blocks, trailing }: Props = $props();

  const revealedSpoilerIds = new SvelteSet<number>();
  let lastRevealedSpoilerId: number | null = null;

  function revealSpoiler(id: number) {
    lastRevealedSpoilerId = id;
    revealedSpoilerIds.add(id);
  }

  /** The button disappears on reveal, so focus moves to the text it hid instead of the page. */
  function focusIfJustRevealed(id: number) {
    return (element: HTMLElement) => {
      if (lastRevealedSpoilerId === id) {
        lastRevealedSpoilerId = null;
        element.focus();
      }
    };
  }

  const lastBlock = $derived(blocks.at(-1));
</script>

<!-- Inline nodes are written without whitespace between tags: paragraphs keep the sender's spaces. -->
{#snippet inlines(nodes: readonly InlineNode[], areLinksActive: boolean)}{#each nodes as node, index (index)}{#if node.kind === "text"}{node.text}{:else if node.kind === "mention"}<span
        class="rounded-(--mw-component-mention-chip-radius) bg-(--mw-component-mention-chip-background) px-(--mw-component-mention-chip-padding-inline) font-medium text-(--mw-component-mention-chip-text) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:bg-(--mw-component-mention-chip-background-hover) hover:text-(--mw-component-mention-chip-text-hover)"
        >{node.text}</span
      >{:else if node.kind === "code"}<code class="inline-code">{node.text}</code
      >{:else if node.kind === "strong"}<strong class="font-bold">{@render inlines(node.children, areLinksActive)}</strong
      >{:else if node.kind === "emphasis"}<em>{@render inlines(node.children, areLinksActive)}</em
      >{:else if node.kind === "underline"}<u>{@render inlines(node.children, areLinksActive)}</u
      >{:else if node.kind === "strikethrough"}<s>{@render inlines(node.children, areLinksActive)}</s
      >{:else if node.kind === "spoiler"}{#if revealedSpoilerIds.has(node.id)}<span
          class="spoiler-revealed"
          tabindex="-1"
          {@attach focusIfJustRevealed(node.id)}
          ><span class="sr-only">{strings.messages.spoilerRevealedPrefix}</span>{@render inlines(node.children, areLinksActive)}</span
        >{:else}<button
          type="button"
          class="spoiler-hidden focus-visible:focus-ring"
          aria-label={strings.messages.spoilerHidden}
          onclick={() => revealSpoiler(node.id)}
          ><span class="invisible" aria-hidden="true">{@render inlines(node.children, false)}</span></button
        >{/if}{:else if node.kind === "link"}{#if areLinksActive}<a
          href={node.url}
          target="_blank"
          rel="noopener noreferrer"
          title={node.isMasked ? node.url : undefined}
          class="text-text-link hover:underline">{@render inlines(node.children, false)}</a
        >{:else}<span class="text-text-link">{@render inlines(node.children, false)}</span>{/if}{/if}{/each}{/snippet}

{#snippet blockList(nodes: readonly BlockNode[])}
  {#each nodes as block, index (index)}
    {#if block.kind === "paragraph"}
      <p class="break-words whitespace-pre-wrap">{@render inlines(block.children, true)}{#if block === lastBlock && trailing}{" "}{@render trailing()}{/if}</p>
    {:else if block.kind === "heading"}
      <p
        role="heading"
        aria-level={3 + block.level}
        class={[
          "mt-xs mb-2xs break-words whitespace-pre-wrap text-text-heading first:mt-0",
          block.level === 1 && "type-message-h1",
          block.level === 2 && "type-message-h2",
          block.level === 3 && "type-message-h3",
        ]}
      >{@render inlines(block.children, true)}</p>
    {:else if block.kind === "quote"}
      <blockquote class="quote">{@render blockList(block.children)}</blockquote>
    {:else if block.kind === "list"}
      <ul class="bullet-list">
        {#each block.items as item, itemIndex (itemIndex)}
          <li class="break-words whitespace-pre-wrap">{@render inlines(item, true)}</li>
        {/each}
      </ul>
    {:else if block.kind === "codeBlock"}
      <pre class="code-block type-code"><code>{block.text}</code></pre>
    {/if}
  {/each}
{/snippet}

{@render blockList(blocks)}
{#if trailing && lastBlock?.kind !== "paragraph"}
  <p>{@render trailing()}</p>
{/if}

<style>
  .inline-code {
    border-radius: var(--mw-radius-xs);
    background: var(--mw-component-message-code-background);
    padding-inline: var(--mw-space-3xs);
    font-family: var(--mw-type-code-font-family);
    font-size: var(--mw-type-code-font-size);
  }

  .code-block {
    margin-block: var(--mw-space-2xs);
    overflow-x: auto;
    border: 1px solid var(--mw-component-message-code-border);
    border-radius: var(--mw-radius-sm);
    background: var(--mw-component-message-code-background);
    padding: var(--mw-space-xs);
    white-space: pre;
  }

  .quote {
    border-inline-start: var(--mw-component-message-quote-bar-width) solid var(--mw-component-message-quote-bar-color);
    border-radius: var(--mw-radius-xs);
    padding-inline-start: var(--mw-space-sm);
  }

  .bullet-list {
    margin-inline-start: var(--mw-space-md);
    list-style: disc outside;
  }

  .spoiler-hidden {
    border-radius: var(--mw-radius-xs);
    background: var(--mw-component-message-spoiler-background);
    cursor: pointer;
    transition: background-color var(--mw-motion-duration-fast) var(--mw-motion-easing-standard);
  }

  .spoiler-hidden:hover {
    background: var(--mw-component-message-spoiler-background-hover);
  }

  .spoiler-revealed {
    border-radius: var(--mw-radius-xs);
    background: var(--mw-component-message-spoiler-revealed-background);
    outline: none;
  }
</style>
