<script lang="ts">
  import type { Snippet } from "svelte";
  import { SvelteSet } from "svelte/reactivity";

  import { strings } from "../i18n";
  import CustomEmojiImage from "./CustomEmojiImage.svelte";
  import type { BlockNode, InlineNode, MarkdownList } from "./markdown";
  import MessageCodeBlock from "./MessageCodeBlock.svelte";
  import { roleMentionChipColorsInTheme } from "./name-colors.svelte";
  import { roleMentionLabel } from "./role-mentions";
  import { NO_ROLE_COLOR } from "./server-settings";

  // A message body parsed by `markdown.ts` (DESIGN.md §5.4). Every node becomes an element or a
  // text node here: message text is never inserted as HTML.
  interface Props {
    blocks: readonly BlockNode[];
    /** Shown at the end of the last paragraph, as Discord places "(edited)"; on its own line otherwise. */
    trailing?: Snippet;
    /** Only emoji (`isJumboEmojiMessage`): they show large, as on Discord. */
    isJumbo?: boolean;
  }

  let { blocks, trailing, isJumbo = false }: Props = $props();

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
  /** Blocks that end in a line of text, where "(edited)" can follow the last word. */
  const isLastBlockInline = $derived(lastBlock?.kind === "paragraph" || lastBlock?.kind === "subtext");

  /** Digits of a numbered list's last number, so its markers get room and never hang into the gutter. */
  function largestNumberDigitCount(list: MarkdownList): number {
    return String((list.start ?? 0) + list.items.length - 1).length;
  }

  /** A person, `@everyone` or a role without colour (DESIGN.md §5.4): the lamp's amber chip. */
  const MENTION_CHIP_CLASS =
    "rounded-(--mw-component-mention-chip-radius) bg-(--mw-component-mention-chip-background) px-(--mw-component-mention-chip-padding-inline) font-medium text-(--mw-component-mention-chip-text) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:bg-(--mw-component-mention-chip-background-hover) hover:text-(--mw-component-mention-chip-text-hover)";
</script>

<!-- Inline nodes are written without whitespace between tags: paragraphs keep the sender's spaces. -->
{#snippet inlines(nodes: readonly InlineNode[], areLinksActive: boolean)}{#each nodes as node, index (index)}{#if node.kind === "text"}{#if isJumbo && node.text.trim() !== ""}<span
          class="text-(length:--mw-component-custom-emoji-jumbo-size) leading-none">{node.text}</span
        >{:else}{node.text}{/if}{:else if node.kind === "match"}<mark class="search-match">{node.text}</mark
      >{:else if node.kind === "customEmoji"}<CustomEmojiImage
        emoji={node.emoji}
        class={isJumbo ? "size-(--mw-component-custom-emoji-jumbo-size)" : "size-(--mw-component-custom-emoji-inline-size)"}
      />{:else if node.kind === "mention" || node.kind === "everyoneMention"}<span class={MENTION_CHIP_CLASS}>{node.text}</span
      >{:else if node.kind === "roleMention"}{#if node.role === null}{roleMentionLabel(null)}{:else if node.role.color === NO_ROLE_COLOR}<span
            class={MENTION_CHIP_CLASS}>{roleMentionLabel(node.role)}</span
          >{:else}{@const chip = roleMentionChipColorsInTheme(node.role.color)}<span
            class="role-mention"
            style:--role-mention-background={chip.background}
            style:--role-mention-text={chip.text}>{roleMentionLabel(node.role)}</span
          >{/if}{:else if node.kind === "code"}<code class="inline-code">{node.text}</code
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

<!-- Items are written without whitespace, as they keep the sender's spaces (whitespace-pre-wrap). -->
{#snippet markdownList(list: MarkdownList)}
  <svelte:element
    this={list.start === null ? "ul" : "ol"}
    start={list.start ?? undefined}
    class="markdown-list"
    style:--list-number-digits={list.start === null ? undefined : largestNumberDigitCount(list)}
  >
    {#each list.items as item, itemIndex (itemIndex)}
      <li class="break-words whitespace-pre-wrap">{@render inlines(item.children, true)}{#each item.sublists as sublist, sublistIndex (sublistIndex)}{@render markdownList(sublist)}{/each}</li>
    {/each}
  </svelte:element>
{/snippet}

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
    {:else if block.kind === "subtext"}
      <p class="type-message-subtext break-words whitespace-pre-wrap text-text-muted">{@render inlines(block.children, true)}{#if block === lastBlock && trailing}{" "}{@render trailing()}{/if}</p>
    {:else if block.kind === "quote"}
      <blockquote class="quote">{@render blockList(block.children)}</blockquote>
    {:else if block.kind === "list"}
      {@render markdownList(block)}
    {:else if block.kind === "codeBlock"}
      <!-- Keyed by its text: an edit that changes a block starts it folded again, even if blocks moved. -->
      {#key block.text}<MessageCodeBlock text={block.text} />{/key}
    {/if}
  {/each}
{/snippet}

{@render blockList(blocks)}
{#if trailing && !isLastBlockInline}
  <p>{@render trailing()}</p>
{/if}

<style>
  /* Words a search matched (DESIGN.md 6.61): the lamp's amber wash, and weight so the match never
     rests on colour alone. */
  .search-match {
    border-radius: var(--mw-component-search-match-radius);
    background: var(--mw-component-search-match-background);
    color: var(--mw-component-search-match-text);
    font-weight: var(--mw-font-weight-semibold);
    box-decoration-break: clone;
  }

  /* A role's chip (DESIGN.md §5.4): the sheet washed with the role colour, the name in that colour
     made readable on it; hovered, the two swap, as the amber chip fills. Opaque, so it reads the
     same on a highlighted row. */
  .role-mention {
    border-radius: var(--mw-component-mention-chip-radius);
    background: var(--role-mention-background);
    padding-inline: var(--mw-component-mention-chip-padding-inline);
    color: var(--role-mention-text);
    font-weight: var(--mw-font-weight-medium);
    transition:
      background-color var(--mw-motion-duration-fast) var(--mw-motion-easing-standard),
      color var(--mw-motion-duration-fast) var(--mw-motion-easing-standard);
  }

  .role-mention:hover {
    background: var(--role-mention-text);
    color: var(--role-mention-background);
  }

  .inline-code {
    border-radius: var(--mw-radius-xs);
    background: var(--mw-component-message-code-background);
    padding-inline: var(--mw-space-3xs);
    font-family: var(--mw-type-code-font-family);
    font-size: var(--mw-type-code-font-size);
  }

  .quote {
    border-inline-start: var(--mw-component-message-quote-bar-width) solid var(--mw-component-message-quote-bar-color);
    border-radius: var(--mw-radius-xs);
    padding-inline-start: var(--mw-space-sm);
  }

  .markdown-list {
    margin-inline-start: var(--mw-space-md);
    list-style: disc outside;
  }

  /* A level down reads as a level down: hollow bullets, as on Discord. */
  li > ul.markdown-list {
    list-style-type: circle;
  }

  /* Outside markers end where the text starts; each digit past the first takes one more figure's
     width, so "10." lines up with "9." and never hangs into the avatar gutter. */
  ol.markdown-list {
    margin-inline-start: calc(var(--mw-space-md) + (var(--list-number-digits) - 1) * 1ch);
    list-style-type: decimal;
  }

  ol.markdown-list > li::marker {
    font-variant-numeric: tabular-nums;
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
