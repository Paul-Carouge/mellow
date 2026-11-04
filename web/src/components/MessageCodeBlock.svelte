<script lang="ts">
  import { flushSync, onMount, tick } from "svelte";

  import { strings } from "../i18n";
  import {
    codeBlockLineCount,
    FOLDED_CODE_BLOCK_VISIBLE_LINES,
    firstCodeBlockLines,
    isCodeBlockFoldable,
  } from "./code-blocks";
  import Icon from "./Icon.svelte";
  import { durationTokenInMilliseconds } from "./tokens";

  // A fenced code block in a message body (DESIGN.md §5.4, v0.48): "Copier le code" at its
  // trailing top, and past 20 lines a fold that shows the first 12 with "Afficher tout (42 lignes)".
  // The open state lives here: the block keeps its place in the message's keyed list, so gateway
  // updates that parse the message again keep it.
  interface Props {
    text: string;
  }

  let { text }: Props = $props();

  const id = $props.id();
  const lineCount = $derived(codeBlockLineCount(text));
  const isFoldable = $derived(isCodeBlockFoldable(lineCount));
  let isOpen = $state(false);
  const isFolded = $derived(isFoldable && !isOpen);
  // Only the shown lines are in the page: a 400-line paste costs nothing until it is opened.
  const shownText = $derived(isFolded ? firstCodeBlockLines(text, FOLDED_CODE_BLOCK_VISIBLE_LINES) : text);

  let block: HTMLDivElement;
  let copyState = $state<"idle" | "copied" | "failed">("idle");
  let announcement = $state("");
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;

  onMount(() => () => clearTimeout(copiedTimer));

  async function copyCode() {
    let nextState: "copied" | "failed";
    try {
      await navigator.clipboard.writeText(text);
      nextState = "copied";
    } catch (error) {
      console.warn("copying a code block failed", error);
      nextState = "failed";
    }
    copyState = nextState;
    // Emptied first, so a second copy is announced again.
    announcement = "";
    await tick();
    announcement = nextState === "copied" ? strings.messages.codeCopiedAnnouncement : strings.messages.copyCodeFailed;
    clearTimeout(copiedTimer);
    copiedTimer = setTimeout(() => (copyState = "idle"), durationTokenInMilliseconds("component-message-code-copied-label-duration"));
  }

  function scrollContainerOf(element: HTMLElement): HTMLElement | null {
    for (let parent = element.parentElement; parent; parent = parent.parentElement) {
      const overflowY = getComputedStyle(parent).overflowY;
      if (overflowY === "auto" || overflowY === "scroll") {
        return parent;
      }
    }
    return null;
  }

  /**
   * Opening keeps the block's first line where it is, so reading goes on below line 12; folding
   * keeps the strip under the pointer, so the block never leaves the view from under the reader.
   */
  function toggleFold(event: MouseEvent & { currentTarget: HTMLButtonElement }) {
    const reference = isOpen ? event.currentTarget : block;
    const scroller = scrollContainerOf(block);
    const topBefore = reference.getBoundingClientRect().top;
    isOpen = !isOpen;
    flushSync();
    if (!scroller) {
      return;
    }
    scroller.scrollTop += reference.getBoundingClientRect().top - topBefore;
    // Lets the message list read its position now, before its ResizeObserver would pin a list
    // that was at the latest message back to the bottom and move the block from under the reader.
    scroller.dispatchEvent(new Event("scroll"));
  }
</script>

<div bind:this={block} class="code-block group/code">
  <pre id="{id}-code" class={["code-scroll type-code", isFolded && "is-folded"]}><code>{shownText}</code></pre>
  <button
    type="button"
    title={strings.messages.copyCode}
    class={[
      "copy-button focus-visible:focus-ring",
      "opacity-0 group-hover/code:opacity-100 focus-visible:opacity-100 pointer-coarse:opacity-100",
      copyState !== "idle" && "opacity-100",
    ]}
    onclick={() => void copyCode()}
  >
    {#if copyState === "copied"}
      <Icon name="check" class="size-(--mw-component-message-code-copy-icon-size)" />
      <span class="type-body-sm font-semibold">{strings.messages.codeCopied}</span>
    {:else if copyState === "failed"}
      <Icon name="warning-circle" class="size-(--mw-component-message-code-copy-icon-size)" />
      <span class="type-body-sm font-semibold">{strings.messages.copyCodeFailedShort}</span>
    {:else}
      <Icon name="copy" class="size-(--mw-component-message-code-copy-icon-size)" />
      <span class="sr-only">{strings.messages.copyCode}</span>
    {/if}
  </button>
  {#if isFoldable}
    <button
      type="button"
      aria-expanded={isOpen}
      aria-controls="{id}-code"
      class="fold-button type-body-sm font-semibold focus-visible:focus-ring-inset"
      onclick={toggleFold}
    >
      <Icon name={isOpen ? "caret-up" : "caret-down"} class="size-(--mw-component-message-code-copy-icon-size)" />
      <span class="tabular-nums">{isOpen ? strings.messages.foldCode : strings.messages.showAllCode(lineCount)}</span>
    </button>
  {/if}
  <span class="sr-only" aria-live="polite">{announcement}</span>
</div>

<style>
  .code-block {
    position: relative;
    margin-block: var(--mw-space-2xs);
    border: 1px solid var(--mw-component-message-code-border);
    border-radius: var(--mw-radius-sm);
    background: var(--mw-component-message-code-background);
  }

  /* Long lines scroll inside the block, never widen the message; the trailing padding keeps the
     copy key off the end of a line that fits. */
  .code-scroll {
    overflow-x: auto;
    padding: var(--mw-space-xs);
    padding-inline-end: calc(var(--mw-component-message-code-copy-button-size) + 2 * var(--mw-component-message-code-copy-button-inset));
    white-space: pre;
  }

  /* As wide as its longest line, so the fade below covers every column it scrolls to. */
  .code-scroll code {
    display: inline-block;
    min-width: 100%;
  }

  /* The last two shown lines fade into the well: the code goes on. A mask works on both themes
     and on a highlighted row, and leaves the horizontal scrollbar untouched. Only its alpha counts. */
  .is-folded code {
    mask-image: linear-gradient(
      to bottom,
      black calc(100% - var(--mw-component-message-code-fade-height)),
      transparent
    );
  }

  .copy-button {
    position: absolute;
    inset-block-start: var(--mw-component-message-code-copy-button-inset);
    inset-inline-end: var(--mw-component-message-code-copy-button-inset);
    display: inline-flex;
    gap: var(--mw-space-3xs);
    justify-content: center;
    align-items: center;
    border: 1px solid var(--mw-component-message-code-copy-button-border);
    border-radius: var(--mw-radius-xs);
    background: var(--mw-component-message-code-copy-button-background);
    min-width: var(--mw-component-message-code-copy-button-size);
    height: var(--mw-component-message-code-copy-button-size);
    padding-inline: var(--mw-space-3xs);
    color: var(--mw-component-message-code-control-text);
    transition:
      opacity var(--mw-motion-duration-fast) var(--mw-motion-easing-standard),
      color var(--mw-motion-duration-fast) var(--mw-motion-easing-standard);
  }

  .copy-button:hover {
    color: var(--mw-component-message-code-control-text-hover);
  }

  /* The strip across the block's foot: the whole width is the target. */
  .fold-button {
    display: flex;
    gap: var(--mw-space-2xs);
    align-items: center;
    border-block-start: 1px solid var(--mw-component-message-code-border);
    border-end-start-radius: var(--mw-radius-sm);
    border-end-end-radius: var(--mw-radius-sm);
    width: 100%;
    height: var(--mw-component-message-code-fold-height);
    padding-inline: var(--mw-space-xs);
    color: var(--mw-component-message-code-control-text);
    transition:
      background-color var(--mw-motion-duration-fast) var(--mw-motion-easing-standard),
      color var(--mw-motion-duration-fast) var(--mw-motion-easing-standard);
  }

  .fold-button:hover {
    background: var(--mw-component-message-code-control-background-hover);
    color: var(--mw-component-message-code-control-text-hover);
  }

  @media (pointer: coarse) {
    .code-scroll {
      min-height: calc(var(--mw-size-hit-web-min) + 2 * var(--mw-component-message-code-copy-button-inset));
      padding-inline-end: calc(var(--mw-size-hit-web-min) + 2 * var(--mw-component-message-code-copy-button-inset));
    }

    .copy-button {
      min-width: var(--mw-size-hit-web-min);
      height: var(--mw-size-hit-web-min);
    }

    .fold-button {
      height: var(--mw-size-hit-web-min);
    }
  }
</style>
