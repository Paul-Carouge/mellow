<script lang="ts">
  import { onDestroy, tick } from "svelte";

  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import { channelSections } from "../lib/channels";
  import type { PublicUser } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import { calendarDayLabel, displayName } from "./format";
  import { conversationDisplayName } from "./group-conversations";
  import Icon, { type IconName } from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import type { MessageSearch } from "./message-search.svelte";
  import {
    type SearchCaretContext,
    type SearchFilterKey,
    channelFilterValue,
    isTypingFilterValue,
    replaceInSearchQuery,
    searchCaretContext,
  } from "./search-query";
  import { type SearchSuggestion, searchSuggestions, suggestionInsertion } from "./search-suggestions";

  // The message search field (DESIGN.md 5.12): in the chat header where the results sheet docks,
  // at the top of the sheet elsewhere. A combobox owning a listbox of filter suggestions; focus
  // stays in the field and the active option is pointed at with aria-activedescendant. Enter
  // searches at once unless an option was chosen with the arrows (a value's first option is
  // active by itself, so "from:a" then Enter picks the first person).
  interface Props {
    client: MellowClient;
    search: MessageSearch;
    /** In the chat header (grows on focus) or at the top of the results sheet. */
    placement: "header" | "sheet";
    onopenquickswitcher: () => void;
  }

  let { client, search, placement, onopenquickswitcher }: Props = $props();

  const KEY_ICONS: Record<SearchFilterKey, IconName> = {
    from: "pencil-simple",
    mentions: "at",
    in: "hash",
    has: "file",
    before: "clock",
    after: "clock",
    during: "clock",
  };

  const id = $props.id();
  let input = $state<HTMLInputElement | null>(null);
  let caret = $state(0);
  let hasFocus = $state(false);
  /** Esc hid the suggestions for this text and caret; typing or moving brings them back. */
  let dismissedAt = $state<string | null>(null);
  let activeIndex = $state(-1);

  const guild = $derived(search.scopeGuildId ? (client.guilds.find((candidate) => candidate.id === search.scopeGuildId) ?? null) : null);
  const placeholder = $derived(guild ? strings.search.placeholderInServer(guild.name) : strings.search.placeholderInConversations);
  const label = $derived(guild ? strings.search.fieldLabelInServer(guild.name) : strings.search.fieldLabelInConversations);

  const people = $derived.by((): PublicUser[] => {
    if (guild) {
      return (client.membersByGuild.get(guild.id) ?? []).map((member) => member.user);
    }
    const byId = new Map<string, PublicUser>();
    if (client.user) {
      byId.set(client.user.id, client.user);
    }
    for (const conversation of client.conversations) {
      for (const recipient of conversation.recipients) {
        byId.set(recipient.id, recipient);
      }
    }
    for (const relationship of client.relationshipByUserId.values()) {
      if (relationship.kind === "friend") {
        byId.set(relationship.user.id, relationship.user);
      }
    }
    return [...byId.values()];
  });
  const channels = $derived(
    guild
      ? channelSections(guild)
          .flatMap((section) => section.channels)
          // Voice channels hold no messages.
          .filter((channel) => channel.kind === "text")
          .map((channel) => ({ id: channel.id, name: channel.name, isConversation: false, value: channelFilterValue(channel) }))
      : client.conversations.map((conversation) => ({
          id: conversation.id,
          name: conversationDisplayName(conversation, client.user?.id ?? null),
          isConversation: true,
          value: conversation.id,
        })),
  );

  const context = $derived<SearchCaretContext>(searchCaretContext(search.draft, caret));
  // Every filter is listed only in an empty field: between words, the first letters of one bring
  // it, so the list does not cover the chat after each choice.
  const suggestions = $derived(
    hasFocus &&
      dismissedAt !== `${search.draft}@${caret}` &&
      !(context.kind === "between" && search.draft.trim() !== "")
      ? searchSuggestions(context, { people, channels, today: new Date() })
      : [],
  );
  /** "Go to a channel or person" ends the list while the field is empty: the quick switcher's door. */
  const offersGoTo = $derived(hasFocus && search.draft.trim() === "" && dismissedAt !== `${search.draft}@${caret}`);
  const optionCount = $derived(suggestions.length + (offersGoTo ? 1 : 0));
  const isListShown = $derived(optionCount > 0);
  const activeOption = $derived(activeIndex < optionCount ? activeIndex : -1);
  const groupLabel = $derived.by(() => {
    if (context.kind !== "value") {
      return strings.search.groups.filters;
    }
    switch (context.key) {
      case "from":
      case "mentions":
        return strings.search.groups.people;
      case "in":
        return guild ? strings.search.groups.channels : strings.search.groups.conversations;
      case "has":
        return strings.search.groups.contents;
      case "before":
      case "after":
      case "during":
        return strings.search.groups.days;
    }
  });

  // A new list starts on its first option only for values: there Enter means "this one".
  let shownContextKey = "";
  $effect.pre(() => {
    const key = `${context.kind}:${context.kind === "value" ? context.key + context.partial : context.kind === "key" ? context.partial : ""}`;
    if (key !== shownContextKey) {
      shownContextKey = key;
      activeIndex = context.kind === "value" ? 0 : -1;
    }
  });

  $effect(() => {
    if (input) {
      search.registerField(input);
    }
  });
  onDestroy(() => search.registerField(null));

  function noteCaret() {
    caret = input?.selectionStart ?? search.draft.length;
  }

  function handleInput(event: Event & { currentTarget: HTMLInputElement }) {
    const value = event.currentTarget.value;
    caret = event.currentTarget.selectionStart ?? value.length;
    dismissedAt = null;
    search.noteTyped(value, isTypingFilterValue(value, caret));
  }

  async function choose(index: number) {
    if (index === suggestions.length && offersGoTo) {
      onopenquickswitcher();
      return;
    }
    const suggestion = suggestions[index];
    if (!suggestion || context.kind === "none") {
      return;
    }
    const { text, endsPart } = suggestionInsertion(suggestion);
    const result = replaceInSearchQuery(search.draft, context.start, context.end, text, endsPart);
    if (endsPart) {
      search.search(result.query);
    } else {
      search.noteTyped(result.query, true);
    }
    await tick();
    input?.focus();
    input?.setSelectionRange(result.caret, result.caret);
    caret = result.caret;
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.isComposing) {
      return;
    }
    const last = optionCount - 1;
    if ((event.key === "ArrowDown" || event.key === "ArrowUp") && isListShown) {
      event.preventDefault();
      activeIndex =
        event.key === "ArrowDown"
          ? activeOption >= last ? 0 : activeOption + 1
          : activeOption <= 0 ? last : activeOption - 1;
    } else if (event.key === "Enter") {
      event.preventDefault();
      if (isListShown && activeOption >= 0) {
        void choose(activeOption);
      } else {
        search.search();
        dismissedAt = `${search.draft}@${caret}`;
      }
    } else if (event.key === "Escape") {
      if (isListShown) {
        // The sheet's dialog would close on this Esc otherwise.
        event.preventDefault();
        event.stopPropagation();
        dismissedAt = `${search.draft}@${caret}`;
      } else if (search.draft !== "" && placement === "header") {
        event.preventDefault();
        search.clear();
      }
    }
  }

  function clear() {
    search.clear();
    input?.focus();
  }

  function suggestionText(suggestion: SearchSuggestion): { main: string; detail: string } {
    switch (suggestion.kind) {
      case "key":
        return {
          main: `${suggestion.key}:`,
          detail:
            suggestion.key === "in" && !guild
              ? strings.search.inConversationDescription
              : strings.search.filterDescriptions[suggestion.key],
        };
      case "person":
        return { main: displayName(suggestion.user), detail: suggestion.user.username };
      case "channel":
        return { main: suggestion.name, detail: "" };
      case "has":
        return { main: strings.search.hasValues[suggestion.value], detail: `has:${suggestion.value}` };
      case "day":
        return { main: strings.search.relativeDays[suggestion.relativeDay], detail: calendarDayLabel(suggestion.day) };
    }
  }
</script>

<div
  class={[
    "relative min-w-0",
    placement === "header"
      ? "flex-[0_1_var(--mw-component-search-field-width)] min-w-(--mw-component-search-field-min-width) transition-[flex-basis] duration-(--mw-motion-duration-base) ease-(--mw-motion-easing-standard) focus-within:flex-[0_1_var(--mw-component-search-field-width-active)]"
      : "w-full",
    placement === "header" && search.draft !== "" && "flex-[0_1_var(--mw-component-search-field-width-active)]",
  ]}
>
  <div
    class={[
      "group flex items-center gap-xs rounded-sm bg-surface-input ps-xs pe-3xs has-[input:focus-visible]:focus-ring",
      placement === "header" ? "h-(--mw-component-chat-header-button-size) pointer-coarse:h-(--mw-size-hit-web-min)" : "h-(--mw-size-hit-web-min)",
    ]}
  >
    <Icon
      name="magnifying-glass"
      class="size-(--mw-component-search-field-icon-size) text-(--mw-component-search-field-icon) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) group-hover:text-(--mw-component-search-field-icon-hover)"
    />
    <input
      bind:this={input}
      type="text"
      role="combobox"
      enterkeyhint="search"
      autocomplete="off"
      spellcheck="false"
      aria-label={label}
      aria-keyshortcuts="Control+F"
      aria-autocomplete="list"
      aria-expanded={isListShown}
      aria-controls={isListShown ? `${id}-listbox` : undefined}
      aria-activedescendant={isListShown && activeOption >= 0 ? `${id}-option-${activeOption}` : undefined}
      {placeholder}
      value={search.draft}
      class="h-full min-w-0 flex-1 bg-transparent type-body text-text-normal outline-none placeholder:text-text-muted"
      oninput={handleInput}
      onkeydown={handleKeydown}
      onkeyup={(event) => {
        if (event.key === "ArrowLeft" || event.key === "ArrowRight" || event.key === "Home" || event.key === "End") {
          noteCaret();
        }
      }}
      onclick={noteCaret}
      onfocus={() => {
        hasFocus = true;
        noteCaret();
      }}
      onblur={() => (hasFocus = false)}
    />
    {#if search.draft !== ""}
      <IconButton
        icon="x"
        label={strings.search.clear}
        class="size-[calc(var(--mw-component-chat-header-button-size)_-_2*var(--mw-space-2xs))] shrink-0 text-text-muted hover:bg-state-hover hover:text-text-normal pointer-coarse:size-(--mw-size-hit-web-min)"
        iconClass="size-(--mw-component-search-field-icon-size)"
        onclick={clear}
      />
    {:else if placement === "header" && !hasFocus}
      <kbd class="me-2xs rounded-xs border border-border-divider px-2xs font-sans type-body-sm whitespace-nowrap text-text-muted pointer-coarse:hidden">Ctrl F</kbd>
    {/if}
  </div>

  {#if isListShown}
    <div
      class={[
        "suggestions absolute top-[calc(100%_+_var(--mw-space-2xs))] z-30 flex max-h-(--mw-component-search-suggestions-max-height) flex-col overflow-hidden rounded-(--mw-component-search-suggestions-radius) border-(length:--mw-component-context-menu-border-width) border-(--mw-component-context-menu-border) bg-(--mw-component-search-suggestions-background) shadow-(--mw-component-context-menu-shadow)",
        placement === "header"
          ? "end-0 w-max min-w-full max-w-[min(var(--mw-component-search-suggestions-width),calc(100vw_-_2*var(--mw-space-xs)))] sm:w-(--mw-component-search-suggestions-width)"
          : "inset-x-0",
      ]}
    >
      <p
        aria-hidden="true"
        class="shrink-0 px-(--mw-component-context-menu-item-padding-inline) pt-(--mw-component-search-suggestions-padding) pb-2xs type-category text-(--mw-component-search-suggestion-group-text)"
      >
        {groupLabel}
      </p>
      <ul
        id="{id}-listbox"
        role="listbox"
        aria-label={groupLabel}
        class="min-h-0 overflow-y-auto p-(--mw-component-search-suggestions-padding) pt-0 [scrollbar-width:thin]"
      >
        {#each suggestions as suggestion, index (index)}
          {@const text = suggestionText(suggestion)}
          {@render option(index, text.main, text.detail, suggestion)}
        {/each}
        {#if offersGoTo}
          <li role="presentation" class="my-2xs h-px bg-border-divider"></li>
          {@render option(suggestions.length, strings.search.goTo, "Ctrl K", null)}
        {/if}
      </ul>
    </div>
  {/if}
  <p class="sr-only" aria-live="polite">{isListShown ? strings.search.suggestionsCount(optionCount) : ""}</p>
</div>

{#snippet option(index: number, main: string, detail: string, suggestion: SearchSuggestion | null)}
  {@const isActive = index === activeOption}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <li
    id="{id}-option-{index}"
    role="option"
    aria-selected={isActive}
    class={[
      "flex min-h-(--mw-component-search-suggestion-height) cursor-pointer items-center gap-xs rounded-(--mw-component-context-menu-item-radius) px-(--mw-component-context-menu-item-padding-inline) py-3xs type-body pointer-coarse:min-h-(--mw-size-hit-web-min)",
      isActive
        ? "bg-(--mw-component-context-menu-item-background-focus) text-(--mw-component-context-menu-item-text-focus)"
        : "text-(--mw-component-context-menu-item-text)",
    ]}
    {@attach (element) => {
      if (isActive) {
        element.scrollIntoView({ block: "nearest" });
      }
    }}
    onpointerdown={(event) => event.preventDefault()}
    onpointermove={() => (activeIndex = index)}
    onclick={() => void choose(index)}
  >
    {#if suggestion?.kind === "person"}
      <Avatar user={suggestion.user} sizeToken="--mw-component-search-suggestion-avatar-size" />
    {:else if suggestion?.kind === "key"}
      <Icon name={KEY_ICONS[suggestion.key]} class="size-(--mw-component-search-suggestion-icon-size) text-text-muted" />
    {:else if suggestion?.kind === "channel"}
      <Icon name={suggestion.isConversation ? "at" : "hash"} class="size-(--mw-component-search-suggestion-icon-size) text-text-muted" />
    {/if}
    <!-- A filter's name stays whole and its description takes the rest; a value's name comes first. -->
    <span class={["truncate", suggestion?.kind === "key" ? "shrink-0 font-semibold" : "min-w-0 flex-1"]}>{main}</span>
    {#if detail}
      <span
        class={[
          "truncate type-body-sm",
          suggestion?.kind === "key" ? "min-w-0 flex-1 text-end" : "max-w-1/2 shrink-0",
          isActive ? "text-(--mw-component-context-menu-item-text-focus)" : "text-(--mw-component-search-suggestion-detail-text)",
          suggestion === null && "rounded-xs border border-border-divider px-2xs pointer-coarse:hidden",
        ]}>{detail}</span
      >
    {/if}
  </li>
{/snippet}

<style>
  .suggestions {
    animation: mw-drop-in var(--mw-motion-duration-fast) var(--mw-motion-easing-enter) both;
  }
</style>
