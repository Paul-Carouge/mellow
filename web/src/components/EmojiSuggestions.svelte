<script lang="ts">
  import { strings } from "../i18n";
  import CustomEmojiImage from "./CustomEmojiImage.svelte";
  import type { EmojiSuggestion } from "./custom-emoji";

  // The ":na" suggestions above the composer (DESIGN.md §5.5, §6.58), Discord's autocomplete: the
  // server's emoji, then built-in ones. Focus stays in the composer, which points at the active
  // option (`aria-activedescendant`); a click chooses without taking focus from it.
  interface Props {
    id: string;
    query: string;
    suggestions: readonly EmojiSuggestion[];
    activeIndex: number;
    onchoose: (suggestion: EmojiSuggestion) => void;
    onhover: (index: number) => void;
  }

  let { id, query, suggestions, activeIndex, onchoose, onhover }: Props = $props();

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
  class="absolute inset-x-0 bottom-full z-20 mx-auto flex max-h-(--mw-component-custom-emoji-suggestion-max-height) w-full max-w-(--mw-component-custom-emoji-suggestion-width) flex-col overflow-hidden rounded-(--mw-component-context-menu-radius) border-(length:--mw-component-context-menu-border-width) border-(--mw-component-context-menu-border) bg-(--mw-component-context-menu-background) shadow-(--mw-component-context-menu-shadow) sm:mx-0"
>
  <p
    aria-hidden="true"
    class="shrink-0 truncate px-(--mw-component-context-menu-item-padding-inline) pt-(--mw-component-context-menu-padding) pb-2xs type-category text-(--mw-component-emoji-picker-heading-text)"
  >
    {strings.chat.emojiSuggestionsTitle(query)}
  </p>
  <ul
    id="{id}-listbox"
    role="listbox"
    aria-label={strings.chat.emojiSuggestionsTitle(query)}
    class="min-h-0 overflow-y-auto p-(--mw-component-context-menu-padding) pt-0 [scrollbar-width:thin]"
  >
    {#each suggestions as suggestion, index (suggestion.kind === "custom" ? suggestion.emoji.id : suggestion.emoji)}
      {@const isActive = index === activeIndex}
      {@const name = suggestion.kind === "custom" ? `:${suggestion.emoji.name}:` : suggestion.name}
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
        <span aria-hidden="true" class="grid size-(--mw-component-emoji-picker-emoji-size) shrink-0 place-items-center text-(length:--mw-component-emoji-picker-emoji-size) leading-none">
          {#if suggestion.kind === "custom"}
            <CustomEmojiImage emoji={suggestion.emoji} class="size-(--mw-component-emoji-picker-emoji-size)" isDecorative />
          {:else}
            {suggestion.emoji}
          {/if}
        </span>
        <span class="min-w-0 flex-1 truncate">{name}</span>
        {#if suggestion.kind === "unicode"}
          <!-- The built-in emoji itself, for screen readers that only read the word. -->
          <span class="sr-only">{suggestion.emoji}</span>
        {/if}
      </li>
    {/each}
  </ul>
</div>
