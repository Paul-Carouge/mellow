<script lang="ts">
  import { onMount, tick } from "svelte";

  import { strings } from "../i18n";
  import { EMOJI_CATALOGUE, EMOJI_CATEGORY_IDS, type EmojiCategoryId, searchEmoji } from "../lib/emoji-catalogue";
  import { prefersReducedMotion } from "../lib/motion.svelte";
  import { displayEmoji, reactionKey } from "../lib/reactions";
  import { nextEmojiCell, sectionAtScrollTop } from "./emoji-grid";
  import Icon, { type IconName } from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import { recentEmoji } from "./recent-emoji.svelte";

  // The emoji picker (DESIGN.md §6.52): search first, then a bar of categories, then the recently
  // used and the built-in set by category. One Tab stop each for the bar and the emoji; arrows move
  // within them, Page Up and Page Down jump between categories, Enter picks, Esc closes.
  // A light-dismiss popover beside its button, or a sheet at the bottom of a phone's screen.
  interface Props {
    anchor: HTMLElement;
    isCompact: boolean;
    onpick: (emoji: string) => void;
    /**
     * Closed by Esc or a click outside; the caller puts focus back only if it was in the picker
     * (or nowhere): a click elsewhere keeps the focus it gave.
     */
    onclose: (shouldReturnFocus: boolean) => void;
  }

  let { anchor, isCompact, onpick, onclose }: Props = $props();

  interface Section {
    id: "results" | "recent" | EmojiCategoryId;
    title: string;
    emoji: readonly string[];
  }

  const CATEGORY_ICONS: Record<"recent" | EmojiCategoryId, IconName> = {
    recent: "clock",
    people: "smiley",
    nature: "paw-print",
    food: "bowl-food",
    activities: "soccer-ball",
    travel: "airplane-tilt",
    objects: "lightbulb",
    symbols: "heart",
    flags: "flag",
  };

  // Built once: the catalogue never changes while the picker is open.
  const CATEGORY_SECTIONS: readonly Section[] = EMOJI_CATEGORY_IDS.map((category) => ({
    id: category,
    title: strings.emojiPicker.categories[category],
    emoji: EMOJI_CATALOGUE.filter((entry) => entry.category === category).map((entry) => entry.emoji),
  }));

  const id = $props.id();
  let panel: HTMLElement;
  let list: HTMLElement;
  let search = $state<HTMLInputElement | null>(null);
  let query = $state("");
  let position = $state({ top: 0, left: 0 });
  let activeIndex = $state(0);
  let currentSectionIndex = $state(0);

  const isBrowsing = $derived(query.trim() === "");
  // Never an empty section: the keys and the bar count on it.
  const sections = $derived.by((): Section[] => {
    if (!isBrowsing) {
      const results = searchEmoji(query).map((entry) => entry.emoji);
      return results.length > 0 ? [{ id: "results", title: strings.emojiPicker.results, emoji: results }] : [];
    }
    const recent = recentEmoji().map((emoji) => displayEmoji(reactionKey(emoji)));
    return [...(recent.length > 0 ? [{ id: "recent" as const, title: strings.emojiPicker.recent, emoji: recent }] : []), ...CATEGORY_SECTIONS];
  });
  const sectionLengths = $derived(sections.map((section) => section.emoji.length));
  const sectionStarts = $derived(sectionLengths.map((_, index) => sectionLengths.slice(0, index).reduce((total, length) => total + length, 0)));
  const emojiCount = $derived(sectionLengths.reduce((total, length) => total + length, 0));

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
    return [...list.querySelectorAll<HTMLButtonElement>("[data-emoji-cell]")];
  }

  /** How many cells a row of the grid holds, from its laid-out columns (every section has the same). */
  function columnCount(): number {
    const grid = list.querySelector("[data-emoji-grid]");
    if (!grid) {
      return 1;
    }
    return Math.max(getComputedStyle(grid).gridTemplateColumns.split(" ").filter(Boolean).length, 1);
  }

  async function focusCell(index: number) {
    activeIndex = index;
    await tick();
    const cell = cells()[index];
    if (!cell) {
      return;
    }
    cell.focus({ preventScroll: true });
    cell.scrollIntoView({ block: "nearest" });
    // The section's heading sticks to the top of the list: keep the cell out from under it.
    const heading = cell.closest("section")?.querySelector("h3");
    const hiddenBy = heading ? heading.getBoundingClientRect().bottom - cell.getBoundingClientRect().top : 0;
    if (hiddenBy > 0) {
      list.scrollTop -= hiddenBy;
    }
  }

  function handleGridKeydown(event: KeyboardEvent) {
    const current = cells().indexOf(document.activeElement as HTMLButtonElement);
    if (current === -1) {
      return;
    }
    const move = nextEmojiCell(sectionLengths, columnCount(), current, {
      key: event.key,
      isControlHeld: event.ctrlKey || event.metaKey,
    });
    if (move === null) {
      return;
    }
    event.preventDefault();
    if (move === "search") {
      search?.focus();
    } else if (event.key === "PageDown" || event.key === "PageUp") {
      // A new category: its heading goes to the top, as a click in the bar does.
      void focusCell(move).then(() => {
        const section = cells()[move]?.closest("section");
        if (section) {
          list.scrollTop = section.offsetTop;
        }
      });
    } else {
      void focusCell(move);
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

  /** The bar follows the list: the section whose heading has reached the top is the current one. */
  function followScroll() {
    const tops = [...list.querySelectorAll<HTMLElement>("section")].map((section) => section.offsetTop);
    const isAtEnd = list.scrollTop + list.clientHeight >= list.scrollHeight - 1;
    showAsCurrent(sectionAtScrollTop(tops, list.scrollTop, isAtEnd));
  }

  function showAsCurrent(sectionIndex: number) {
    if (sectionIndex === currentSectionIndex) {
      return;
    }
    currentSectionIndex = sectionIndex;
    // On a phone the bar scrolls sideways: keep the current category in it.
    panel.querySelectorAll("[data-category]")[sectionIndex]?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }

  /** A category from the bar: the list scrolls to it, and the emoji's Tab stop moves to its first. */
  function showSection(index: number) {
    const section = list.querySelectorAll<HTMLElement>("section")[index];
    if (!section) {
      return;
    }
    list.scrollTo({ top: section.offsetTop, behavior: prefersReducedMotion.current ? "instant" : "smooth" });
    currentSectionIndex = index;
    activeIndex = sectionStarts[index] ?? 0;
  }

  function handleCategoryKeydown(event: KeyboardEvent) {
    const buttons = [...panel.querySelectorAll<HTMLButtonElement>("[data-category]")];
    const current = buttons.indexOf(document.activeElement as HTMLButtonElement);
    const last = buttons.length - 1;
    const next =
      event.key === "ArrowRight"
        ? current === last ? 0 : current + 1
        : event.key === "ArrowLeft"
          ? current <= 0 ? last : current - 1
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? last
              : null;
    if (next === null || current === -1) {
      return;
    }
    event.preventDefault();
    buttons[next]?.focus();
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
      ? "inset-x-0 top-auto bottom-0 h-[calc(var(--mw-component-emoji-picker-height)+env(safe-area-inset-bottom))] max-h-[70dvh] pb-[calc(var(--mw-component-emoji-picker-padding)+env(safe-area-inset-bottom))] w-full max-w-none rounded-t-(--mw-component-emoji-picker-radius) [&:popover-open]:animate-[mw-rise_var(--mw-motion-duration-base)_var(--mw-motion-easing-enter)_both]"
      : "inset-auto h-(--mw-component-emoji-picker-height) max-h-[calc(100dvh-2*var(--mw-space-xs))] w-(--mw-component-emoji-picker-width) max-w-[calc(100vw-2*var(--mw-space-xs))] rounded-(--mw-component-emoji-picker-radius) [&:popover-open]:animate-[mw-drop-in_var(--mw-motion-duration-fast)_var(--mw-motion-easing-enter)_both]",
  ]}
  style:top={isCompact ? undefined : `${position.top}px`}
  style:left={isCompact ? undefined : `${position.left}px`}
  ontoggle={(event) => {
    if (event.newState === "closed") {
      onclose(panel.contains(document.activeElement) || document.activeElement === document.body);
    }
  }}
>
  <label
    class="flex h-(--mw-component-emoji-picker-search-height) shrink-0 items-center gap-xs rounded-sm bg-(--mw-component-emoji-picker-search-background) px-xs has-[input:focus-visible]:focus-ring pointer-coarse:h-(--mw-size-hit-web-min)"
  >
    <Icon name="magnifying-glass" class="size-(--mw-component-context-menu-item-icon-size) text-(--mw-component-emoji-picker-search-placeholder)" />
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
      oninput={() => {
        activeIndex = 0;
        currentSectionIndex = 0;
        list.scrollTop = 0;
      }}
      onkeydown={handleSearchKeydown}
    />
  </label>
  <p id="{id}-help" class="sr-only">{strings.emojiPicker.help}</p>

  {#if isBrowsing}
    <!-- Shortcuts into the list; the list itself stays the one way to reach every emoji. Focus
         lives on the bar's buttons (one Tab stop, arrows between them), never on the bar. -->
    <!-- svelte-ignore a11y_interactive_supports_focus -->
    <div
      role="toolbar"
      aria-label={strings.emojiPicker.categoryBar}
      aria-controls="{id}-emoji"
      class="flex shrink-0 justify-between overflow-x-auto [scrollbar-width:none]"
      onkeydown={handleCategoryKeydown}
    >
      {#each sections as section, sectionIndex (section.id)}
        {@const isCurrent = sectionIndex === currentSectionIndex}
        <IconButton
          icon={section.id === "results" ? "magnifying-glass" : CATEGORY_ICONS[section.id]}
          label={section.title}
          data-category
          aria-current={isCurrent ? "true" : undefined}
          tabindex={isCurrent ? 0 : -1}
          class={[
            "size-(--mw-component-emoji-picker-category-size) rounded-(--mw-component-emoji-picker-category-radius)",
            isCurrent
              ? "bg-(--mw-component-emoji-picker-category-background-current) text-(--mw-component-emoji-picker-category-icon-current)"
              : "text-(--mw-component-emoji-picker-category-icon) hover:text-(--mw-component-emoji-picker-category-icon-hover)",
          ].join(" ")}
          iconClass="size-(--mw-component-emoji-picker-category-icon-size)"
          onclick={() => showSection(sectionIndex)}
        />
      {/each}
    </div>
  {/if}

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    bind:this={list}
    id="{id}-emoji"
    class="relative min-h-0 flex-1 overflow-y-auto [scrollbar-width:thin]"
    onkeydown={handleGridKeydown}
    onscroll={followScroll}
  >
    {#if emojiCount === 0}
      <p role="status" class="p-xs type-body text-(--mw-component-emoji-picker-empty-text)">
        {strings.emojiPicker.noResults(query.trim())}
      </p>
    {/if}
    {#each sections as section, sectionIndex (section.id)}
      {@const offset = sectionStarts[sectionIndex] ?? 0}
      <section aria-labelledby="{id}-{section.id}">
        <h3
          id="{id}-{section.id}"
          class="sticky top-0 z-1 bg-(--mw-component-emoji-picker-background) px-2xs py-2xs type-category text-(--mw-component-emoji-picker-heading-text)"
        >
          {section.title}
        </h3>
        <div
          data-emoji-grid
          class="grid grid-cols-[repeat(auto-fill,minmax(var(--mw-component-emoji-picker-cell-size),1fr))] pointer-coarse:grid-cols-[repeat(auto-fill,minmax(var(--mw-size-hit-web-min),1fr))]"
        >
          {#each section.emoji as emoji, index (emoji)}
            <button
              type="button"
              data-emoji-cell
              tabindex={offset + index === activeIndex ? 0 : -1}
              class="grid size-(--mw-component-emoji-picker-cell-size) place-items-center rounded-(--mw-component-emoji-picker-cell-radius) text-(length:--mw-component-emoji-picker-emoji-size) leading-none transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:bg-(--mw-component-emoji-picker-cell-background-hover) focus-visible:bg-(--mw-component-emoji-picker-cell-background-hover) focus-visible:focus-ring-inset pointer-coarse:size-(--mw-size-hit-web-min) pointer-coarse:text-(length:--mw-component-emoji-picker-emoji-size-coarse)"
              onfocus={() => (activeIndex = offset + index)}
              onclick={() => onpick(emoji)}>{emoji}</button
            >
          {/each}
        </div>
      </section>
    {/each}
  </div>
</div>
