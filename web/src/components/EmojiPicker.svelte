<script lang="ts">
  import { onMount, tick } from "svelte";

  import { strings } from "../i18n";
  import { EMOJI_CATALOGUE, EMOJI_CATEGORY_IDS, searchEmoji } from "../lib/emoji-catalogue";
  import { displayEmoji, reactionKey } from "../lib/reactions";
  import Icon from "./Icon.svelte";
  import { recentEmoji } from "./recent-emoji.svelte";

  // The emoji picker (DESIGN.md §6.52): search first, then the recently used and the built-in set
  // by category. One Tab stop for the emoji; arrows move between them, Enter picks, Esc closes.
  // A light-dismiss popover beside its button, or a sheet at the bottom of a phone's screen.
  interface Props {
    anchor: HTMLElement;
    isCompact: boolean;
    onpick: (emoji: string) => void;
    /** Closed by Esc, a click outside, or after a pick; the caller puts focus back. */
    onclose: () => void;
  }

  let { anchor, isCompact, onpick, onclose }: Props = $props();

  const id = $props.id();
  let panel: HTMLElement;
  let search = $state<HTMLInputElement | null>(null);
  let query = $state("");
  let position = $state({ top: 0, left: 0 });
  let activeIndex = $state(0);

  const sections = $derived.by(() => {
    if (query.trim() !== "") {
      return [{ id: "results", title: strings.emojiPicker.results, emoji: searchEmoji(query).map((entry) => entry.emoji) }];
    }
    const recent = recentEmoji().map((emoji) => displayEmoji(reactionKey(emoji)));
    return [
      ...(recent.length > 0 ? [{ id: "recent", title: strings.emojiPicker.recent, emoji: recent }] : []),
      ...EMOJI_CATEGORY_IDS.map((category) => ({
        id: category,
        title: strings.emojiPicker.categories[category],
        emoji: EMOJI_CATALOGUE.filter((entry) => entry.category === category).map((entry) => entry.emoji),
      })),
    ];
  });
  const emojiCount = $derived(sections.reduce((total, section) => total + section.emoji.length, 0));

  onMount(() => {
    panel.showPopover();
    place();
    search?.focus();
  });

  /** Beside the button, inside the window: below it when there is room, else above. */
  function place() {
    if (isCompact) {
      return;
    }
    const anchorBounds = anchor.getBoundingClientRect();
    const panelBounds = panel.getBoundingClientRect();
    const fitsBelow = anchorBounds.bottom + panelBounds.height <= window.innerHeight;
    const top = fitsBelow ? anchorBounds.bottom : Math.max(anchorBounds.top - panelBounds.height, 0);
    const left = Math.min(Math.max(anchorBounds.right - panelBounds.width, 0), window.innerWidth - panelBounds.width);
    position = { top, left: Math.max(left, 0) };
  }

  function cells(): HTMLButtonElement[] {
    return [...panel.querySelectorAll<HTMLButtonElement>("[data-emoji-cell]")];
  }

  /** How many cells sit on one row: the ones level with the first. */
  function columnCount(all: readonly HTMLButtonElement[]): number {
    const firstTop = all[0]?.offsetTop;
    const sameRow = all.findIndex((cell) => cell.offsetTop !== firstTop);
    return sameRow === -1 ? all.length : sameRow;
  }

  async function focusCell(index: number) {
    activeIndex = index;
    await tick();
    const cell = cells()[index];
    cell?.focus();
    cell?.scrollIntoView({ block: "nearest" });
  }

  function handleGridKeydown(event: KeyboardEvent) {
    const all = cells();
    const current = all.indexOf(document.activeElement as HTMLButtonElement);
    if (current === -1) {
      return;
    }
    const columns = columnCount(all);
    const next =
      event.key === "ArrowRight"
        ? current + 1
        : event.key === "ArrowLeft"
          ? current - 1
          : event.key === "ArrowDown"
            ? current + columns
            : event.key === "ArrowUp"
              ? current - columns
              : event.key === "Home"
                ? 0
                : event.key === "End"
                  ? all.length - 1
                  : null;
    if (next === null) {
      return;
    }
    event.preventDefault();
    if (next < 0) {
      search?.focus();
    } else {
      void focusCell(Math.min(next, all.length - 1));
    }
  }

  function handleSearchKeydown(event: KeyboardEvent) {
    if (event.key === "ArrowDown" && emojiCount > 0) {
      event.preventDefault();
      void focusCell(0);
    } else if (event.key === "Enter" && emojiCount > 0) {
      event.preventDefault();
      cells()[0]?.click();
    }
  }
</script>

<div
  bind:this={panel}
  popover="auto"
  role="dialog"
  aria-label={strings.emojiPicker.label}
  class={[
    "fixed m-0 flex-col gap-(--mw-component-emoji-picker-gap) border-(length:--mw-component-emoji-picker-border-width) border-(--mw-component-emoji-picker-border) bg-(--mw-component-emoji-picker-background) p-(--mw-component-emoji-picker-padding) shadow-(--mw-component-emoji-picker-shadow) [&:popover-open]:flex",
    isCompact
      ? "inset-x-0 top-auto bottom-0 h-(--mw-component-emoji-picker-height) max-h-[70dvh] w-full max-w-none rounded-t-(--mw-component-emoji-picker-radius) [&:popover-open]:animate-[mw-rise_var(--mw-motion-duration-base)_var(--mw-motion-easing-enter)_both]"
      : "inset-auto h-(--mw-component-emoji-picker-height) max-h-[calc(100dvh-2*var(--mw-space-xs))] w-(--mw-component-emoji-picker-width) max-w-[calc(100vw-2*var(--mw-space-xs))] rounded-(--mw-component-emoji-picker-radius) [&:popover-open]:animate-[mw-drop-in_var(--mw-motion-duration-fast)_var(--mw-motion-easing-enter)_both]",
  ]}
  style:top={isCompact ? undefined : `${position.top}px`}
  style:left={isCompact ? undefined : `${position.left}px`}
  ontoggle={(event) => {
    if (event.newState === "closed") {
      onclose();
    }
  }}
>
  <label
    class="flex h-(--mw-component-emoji-picker-search-height) shrink-0 items-center gap-xs rounded-sm bg-(--mw-component-emoji-picker-search-background) px-xs has-[input:focus-visible]:focus-ring pointer-coarse:h-(--mw-size-hit-web-min)"
  >
    <span class="sr-only">{strings.emojiPicker.search}</span>
    <input
      bind:this={search}
      bind:value={query}
      type="search"
      autocomplete="off"
      spellcheck="false"
      placeholder={strings.emojiPicker.placeholder}
      aria-describedby="{id}-help"
      aria-controls="{id}-emoji"
      class="min-w-0 flex-1 bg-transparent type-body text-(--mw-component-emoji-picker-search-text) outline-none placeholder:text-(--mw-component-emoji-picker-search-placeholder) [&::-webkit-search-cancel-button]:appearance-none"
      oninput={() => (activeIndex = 0)}
      onkeydown={handleSearchKeydown}
    />
    <Icon name="magnifying-glass" class="size-(--mw-component-context-menu-item-icon-size) text-(--mw-component-emoji-picker-search-placeholder)" />
  </label>
  <p id="{id}-help" class="sr-only">{strings.emojiPicker.help}</p>

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div id="{id}-emoji" class="min-h-0 flex-1 overflow-y-auto [scrollbar-width:thin]" onkeydown={handleGridKeydown}>
    {#if emojiCount === 0}
      <p role="status" class="p-xs type-body text-(--mw-component-emoji-picker-empty-text)">
        {strings.emojiPicker.noResults(query.trim())}
      </p>
    {/if}
    {#each sections as section, sectionIndex (section.id)}
      {@const offset = sections.slice(0, sectionIndex).reduce((total, previous) => total + previous.emoji.length, 0)}
      {#if section.emoji.length > 0}
        <section aria-labelledby="{id}-{section.id}">
          <h3
            id="{id}-{section.id}"
            class="sticky top-0 z-1 bg-(--mw-component-emoji-picker-background) px-2xs py-2xs type-category text-(--mw-component-emoji-picker-heading-text)"
          >
            {#if section.id === "recent"}<Icon name="clock" class="me-2xs size-(--mw-component-reaction-pinned-icon-size) align-[-0.125em]" />{/if}{section.title}
          </h3>
          <div class="grid grid-cols-[repeat(auto-fill,minmax(var(--mw-component-emoji-picker-cell-size),1fr))] pointer-coarse:grid-cols-[repeat(auto-fill,minmax(var(--mw-size-hit-web-min),1fr))]">
            {#each section.emoji as emoji, index (emoji)}
              <button
                type="button"
                data-emoji-cell
                tabindex={offset + index === activeIndex ? 0 : -1}
                class="grid size-(--mw-component-emoji-picker-cell-size) place-items-center rounded-(--mw-component-emoji-picker-cell-radius) text-(length:--mw-component-emoji-picker-emoji-size) leading-none transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:bg-(--mw-component-emoji-picker-cell-background-hover) focus-visible:bg-(--mw-component-emoji-picker-cell-background-hover) focus-visible:focus-ring-inset pointer-coarse:size-(--mw-size-hit-web-min)"
                onfocus={() => (activeIndex = offset + index)}
                onclick={() => onpick(emoji)}>{emoji}</button
              >
            {/each}
          </div>
        </section>
      {/if}
    {/each}
  </div>
</div>
