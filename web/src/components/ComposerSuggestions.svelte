<script lang="ts" generics="Suggestion">
  import type { Snippet } from "svelte";

  // The ":na", "@na" and "/" suggestions above the composer (DESIGN.md §5.5, §6.58), Discord's
  // autocomplete. Focus stays in the composer, which points at the active option
  // (`aria-activedescendant`); a click chooses without taking focus from it.
  interface Props {
    id: string;
    title: string;
    /** The "/" menu: its rows also show a description and the Markdown they write. */
    isWide?: boolean;
    suggestions: readonly Suggestion[];
    keyOf: (suggestion: Suggestion) => string;
    activeIndex: number;
    row: Snippet<[Suggestion]>;
    onchoose: (suggestion: Suggestion) => void;
    onhover: (index: number) => void;
  }

  let { id, title, isWide = false, suggestions, keyOf, activeIndex, row, onchoose, onhover }: Props = $props();

  /** Keeps the active option in view as the arrows move it. */
  function scrollIntoViewWhenActive(isActive: boolean) {
    return (element: HTMLElement) => {
      if (isActive) {
        element.scrollIntoView({ block: "nearest" });
      }
    };
  }
</script>

<div
  class={[
    "absolute inset-x-0 bottom-full z-20 mx-auto flex max-h-(--mw-component-custom-emoji-suggestion-max-height) w-full flex-col overflow-hidden rounded-(--mw-component-context-menu-radius) border-(length:--mw-component-context-menu-border-width) border-(--mw-component-context-menu-border) bg-(--mw-component-context-menu-background) shadow-(--mw-component-context-menu-shadow) sm:mx-0",
    isWide ? "max-w-(--mw-component-composer-suggestions-command-list-width)" : "max-w-(--mw-component-custom-emoji-suggestion-width)",
  ]}
>
  <p
    aria-hidden="true"
    class="shrink-0 truncate px-(--mw-component-context-menu-item-padding-inline) pt-(--mw-component-context-menu-padding) pb-2xs type-category text-(--mw-component-emoji-picker-heading-text)"
  >
    {title}
  </p>
  <ul
    id="{id}-listbox"
    role="listbox"
    aria-label={title}
    class="min-h-0 overflow-y-auto p-(--mw-component-context-menu-padding) pt-0 [scrollbar-width:thin]"
  >
    {#each suggestions as suggestion, index (keyOf(suggestion))}
      {@const isActive = index === activeIndex}
      <!-- svelte-ignore a11y_click_events_have_key_events -->
      <li
        id="{id}-option-{index}"
        role="option"
        aria-selected={isActive}
        class={[
          "flex h-(--mw-component-context-menu-item-height) cursor-pointer items-center gap-xs rounded-(--mw-component-context-menu-item-radius) px-(--mw-component-context-menu-item-padding-inline) type-body pointer-coarse:h-(--mw-size-hit-web-min)",
          isActive
            ? "bg-(--mw-component-context-menu-item-background-focus) text-(--mw-component-context-menu-item-text-focus)"
            : "text-(--mw-component-context-menu-item-text)",
        ]}
        {@attach scrollIntoViewWhenActive(isActive)}
        onpointerdown={(event) => event.preventDefault()}
        onpointermove={() => onhover(index)}
        onclick={() => onchoose(suggestion)}
      >
        {@render row(suggestion)}
      </li>
    {/each}
  </ul>
</div>
