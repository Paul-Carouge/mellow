<script lang="ts">
  import { onMount } from "svelte";
  import { SvelteSet } from "svelte/reactivity";

  import { formattingLocale, strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import {
    MAXIMUM_REACHABLE_SEARCH_RESULTS,
    MAXIMUM_SEARCH_QUERY_LENGTH,
    MAXIMUM_SEARCH_QUERY_PARTS,
    type MessageSearchHit,
    type MessageSearchSort,
    type Snowflake,
  } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import Button from "./Button.svelte";
  import { calendarDayLabel, displayName, fullTimestampLabel, messageTimestampLabel, unixMillisecondsOfSnowflake } from "./format";
  import { conversationDisplayName } from "./group-conversations";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import { parseMessageMarkdown } from "./markdown";
  import { mentionablePeople } from "./mentions";
  import MessageAttachments from "./MessageAttachments.svelte";
  import MessageMarkdown from "./MessageMarkdown.svelte";
  import type { MessageSearch } from "./message-search.svelte";
  import MessageSearchField from "./MessageSearchField.svelte";
  import { withSearchHighlights } from "./search-highlights";
  import {
    type SearchFilter,
    type SearchQueryPart,
    hasSearchedWords,
    searchQueryParts,
    searchedWordsOf,
    withoutSearchQueryPart,
  } from "./search-query";
  import { isNarrowedBy, queryNarrowedToAuthor, queryNarrowedToChannel, searchResultRows } from "./search-results";
  import { durationTokenInMilliseconds } from "./tokens";

  // The search results side sheet (DESIGN.md 5.12): docked beside the conversation like the member
  // list, over it below the docking width (full window on phones, with its own field). The count
  // titles it; then the sort, the filters applied, the facets that narrow, and the results as slips
  // of the conversation sheet, labelled by channel, with the matched words lit.
  interface Props {
    client: MellowClient;
    search: MessageSearch;
    isDocked: boolean;
    isFullWindow?: boolean;
    /** The result being opened: its Jump reads "Opening…". */
    openingMessageId: Snowflake | null;
    onopenresult: (hit: MessageSearchHit) => void;
    onclose: () => void;
    onopenquickswitcher: () => void;
  }

  let {
    client,
    search,
    isDocked,
    isFullWindow = false,
    openingMessageId,
    onopenresult,
    onclose,
    onopenquickswitcher,
  }: Props = $props();

  const SKELETON_RESULT_COUNT = 3;
  const MAXIMUM_FACETS_SHOWN = 5;

  const id = $props.id();
  let list = $state<HTMLElement | null>(null);
  let sentinel = $state<HTMLElement | null>(null);
  let isSkeletonDelayElapsed = $state(false);
  /** Results by someone you blocked that you chose to show, while the sheet stays (ADR-0025). */
  const revealedBlockedIds = new SvelteSet<Snowflake>();

  const guild = $derived(search.scopeGuildId ? (client.guilds.find((candidate) => candidate.id === search.scopeGuildId) ?? null) : null);
  const status = $derived(search.status);
  const rows = $derived(searchResultRows(search.hits));
  const parts = $derived(searchQueryParts(search.query));
  const appliedFilters = $derived(parts.filter((part): part is SearchQueryPart & { filter: SearchFilter } => part.filter !== null));
  const showsSort = $derived(hasSearchedWords(search.query));
  const channelFacets = $derived(
    !isNarrowedBy(search.query, "in") && search.channelFacets.length > 1 ? search.channelFacets.slice(0, MAXIMUM_FACETS_SHOWN) : [],
  );
  const authorFacets = $derived(
    !isNarrowedBy(search.query, "from") && search.authorFacets.length > 1 ? search.authorFacets.slice(0, MAXIMUM_FACETS_SHOWN) : [],
  );
  const mentionable = $derived(
    mentionablePeople([
      ...(client.user ? [client.user] : []),
      ...(guild ? (client.membersByGuild.get(guild.id) ?? []).map((member) => member.user) : []),
      ...client.conversations.flatMap((conversation) => conversation.recipients),
    ]),
  );
  const announcement = $derived(
    status.kind === "loaded"
      ? search.hits.length === 0
        ? strings.search.noResultsTitle
        : strings.search.resultsAnnouncement(search.totalCount, search.isTotalLowerBound, searchedWordsOf(search.query))
      : "",
  );
  const title = $derived(
    status.kind === "loaded" && search.hits.length > 0
      ? strings.search.resultCount(search.totalCount, search.isTotalLowerBound)
      : strings.search.title,
  );

  onMount(() => {
    const timer = setTimeout(() => (isSkeletonDelayElapsed = true), durationTokenInMilliseconds("motion-delay-skeleton"));
    return () => clearTimeout(timer);
  });

  // The next page loads as the end of the list comes into view (and at once when it already is).
  $effect(() => {
    if (!sentinel || !list) {
      return;
    }
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          search.loadMore();
        }
      },
      { root: list },
    );
    observer.observe(sentinel);
    return () => observer.disconnect();
  });

  /** A new search starts at the top of the list. */
  let shownQuery = "";
  $effect(() => {
    const key = `${search.query}|${search.effectiveSort}`;
    if (key !== shownQuery) {
      shownQuery = key;
      list?.scrollTo({ top: 0 });
    }
  });

  function placeOf(channelId: Snowflake): { name: string; isConversation: boolean; filterName: string | null } {
    const channel = guild?.channels.find((candidate) => candidate.id === channelId);
    if (channel) {
      return { name: `#${channel.name}`, isConversation: false, filterName: channel.name };
    }
    const conversation = client.conversations.find((candidate) => candidate.id === channelId);
    if (conversation) {
      return { name: conversationDisplayName(conversation, client.user?.id ?? null), isConversation: true, filterName: null };
    }
    return { name: strings.search.unknownChannel, isConversation: !guild, filterName: null };
  }

  function filterChipLabel(filter: SearchFilter): string {
    const value = filter.value;
    switch (filter.key) {
      case "from":
      case "mentions": {
        const username = value.replace(/^@+/u, "").toLowerCase();
        const person =
          search.authorFacets.find((facet) => facet.author.username.toLowerCase() === username)?.author ??
          (guild ? (client.membersByGuild.get(guild.id) ?? []).map((member) => member.user) : client.conversations.flatMap((c) => c.recipients)).find(
            (candidate) => candidate.username.toLowerCase() === username,
          );
        return strings.search.filterChip[filter.key](person ? displayName(person) : username);
      }
      case "in": {
        const reference = value.replace(/^#+/u, "");
        const channel = guild?.channels.find((candidate) => candidate.id === reference || candidate.name.toLowerCase() === reference.toLowerCase());
        const conversation = client.conversations.find((candidate) => candidate.id === reference);
        const name = channel
          ? `#${channel.name}`
          : conversation
            ? conversationDisplayName(conversation, client.user?.id ?? null)
            : `#${reference}`;
        return strings.search.filterChip.in(name);
      }
      case "has":
        return strings.search.hasChip[value.toLowerCase() as keyof typeof strings.search.hasChip] ?? value;
      case "before":
      case "after":
      case "during":
        return strings.search.filterChip[filter.key](calendarDayLabel(value));
    }
  }

  function removeFilter(part: SearchQueryPart) {
    const query = withoutSearchQueryPart(search.query, part);
    if (query === "") {
      search.clear();
      search.isPanelOpen = true;
      search.focusField();
    } else {
      search.search(query);
    }
  }

  /** A click on the slip itself opens it; its links, spoilers and files keep their own clicks. */
  function openFromSlip(event: MouseEvent, hit: MessageSearchHit) {
    const target = event.target as HTMLElement;
    if (target.closest("a, button, [role=button], video, audio") || window.getSelection()?.toString()) {
      return;
    }
    onopenresult(hit);
  }

  const sortOptions: { value: MessageSearchSort; label: string }[] = [
    { value: "relevance", label: strings.search.relevance },
    { value: "newest", label: strings.search.newest },
  ];
  const chipClass =
    "inline-flex min-h-(--mw-component-search-facet-height) max-w-full items-center gap-2xs rounded-full ps-xs type-body-sm transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) pointer-coarse:min-h-(--mw-size-hit-web-min)";
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<aside
  aria-label={strings.search.results}
  class={[
    "flex h-full shrink-0 flex-col overflow-hidden bg-(--mw-component-side-sheet-background)",
    isFullWindow ? "w-full safe-area-padding" : "w-(--mw-component-search-panel-width) max-w-full rounded-(--mw-component-side-sheet-radius)",
    isDocked && "side-sheet-docked",
  ]}
  onkeydown={(event) => {
    if (isDocked && event.key === "Escape" && !event.defaultPrevented) {
      event.preventDefault();
      onclose();
    }
  }}
>
  <div class="flex h-(--mw-component-chat-header-height) shrink-0 items-center gap-2xs ps-md pe-2xs text-text-heading">
    <h2 id="{id}-title" class="min-w-0 flex-1 truncate type-heading-md tabular-nums">{title}</h2>
    <IconButton
      icon="x"
      label={strings.common.close}
      class="size-(--mw-component-user-panel-button-size) text-(--mw-component-user-panel-button-icon) hover:bg-state-hover hover:text-text-normal"
      iconClass="size-(--mw-component-user-panel-button-icon-size)"
      onclick={onclose}
    />
  </div>

  {#if !isDocked}
    <div class="shrink-0 px-sm pb-xs">
      <MessageSearchField {client} {search} placement="sheet" {onopenquickswitcher} />
    </div>
  {/if}

  <p class="sr-only" aria-live="polite">{announcement}</p>

  <div bind:this={list} class="min-h-0 flex-1 overflow-y-auto px-sm pb-md [scrollbar-width:thin]" aria-busy={status.kind === "loading"}>
    {#if status.kind === "idle"}
      <div class="flex flex-col gap-sm pt-xs">
        <p class="type-heading-md text-text-heading">
          {guild ? strings.search.idleTitleInServer(guild.name) : strings.search.idleTitleInConversations}
        </p>
        <p class="type-body text-text-secondary">{strings.search.idleBody}</p>
        <dl class="flex flex-col gap-xs">
          {#each strings.search.tips as tip (tip.example)}
            <div class="flex flex-col gap-3xs">
              <dt><code class="rounded-xs bg-(--mw-component-message-code-background) px-2xs font-mono type-body-sm text-text-heading">{tip.example}</code></dt>
              <dd class="type-body-sm text-text-muted">{tip.meaning}</dd>
            </div>
          {/each}
        </dl>
      </div>
    {:else if status.kind === "too-large"}
      <p role="alert" class="pt-xs type-body text-text-secondary">
        {status.problem === "too-long" ? strings.search.tooLong(MAXIMUM_SEARCH_QUERY_LENGTH) : strings.search.tooManyParts(MAXIMUM_SEARCH_QUERY_PARTS)}
      </p>
    {:else if status.kind === "rate-limited" || status.kind === "failed"}
      <div role="alert" class="flex flex-col items-start gap-sm pt-xs">
        {#if status.kind === "rate-limited"}
          <p class="type-heading-md text-text-heading">{strings.search.rateLimitedTitle}</p>
          <p class="type-body text-text-secondary">{strings.search.rateLimitedBody}</p>
        {:else}
          <p class="type-body text-text-secondary">{describeError(status.error, strings.search.failed)}</p>
        {/if}
        <Button onclick={() => search.retry()}>{strings.common.tryAgain}</Button>
      </div>
    {:else if status.kind === "loading"}
      <div role="status" aria-label={strings.search.searching} class="flex flex-col gap-(--mw-component-search-result-gap) pt-xs">
        {#if isSkeletonDelayElapsed}
          {#each Array.from({ length: SKELETON_RESULT_COUNT }, (_, index) => index) as index (index)}
            {@render skeletonSlip()}
          {/each}
        {/if}
      </div>
    {:else if search.hits.length === 0}
      <div class="flex flex-col gap-xs pt-xs">
        <p class="type-heading-md text-text-heading">{strings.search.noResultsTitle}</p>
        <p class="type-body text-text-secondary">{strings.search.noResultsBody}</p>
        {#if appliedFilters.length > 0}
          {@render filters()}
        {/if}
      </div>
    {:else}
      {#if showsSort}
        <div role="radiogroup" aria-label={strings.search.sortLabel} class="flex w-max gap-3xs rounded-sm bg-(--mw-component-search-facet-background) p-3xs">
          {#each sortOptions as option (option.value)}
            {@const isChosen = search.effectiveSort === option.value}
            <label
              class={[
                "flex h-(--mw-component-search-sort-height) cursor-pointer items-center rounded-xs px-xs type-body-sm font-semibold transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) has-[input:focus-visible]:focus-ring pointer-coarse:h-(--mw-size-hit-web-min)",
                isChosen
                  ? "bg-(--mw-component-search-sort-selected-background) text-(--mw-component-search-sort-selected-text)"
                  : "text-(--mw-component-search-sort-text) hover:text-text-heading",
              ]}
            >
              <input
                type="radio"
                name="{id}-sort"
                value={option.value}
                checked={isChosen}
                class="sr-only"
                onchange={() => search.setSort(option.value)}
              />
              {option.label}
            </label>
          {/each}
        </div>
      {/if}

      {#if appliedFilters.length > 0}
        {@render filters()}
      {/if}

      {#if channelFacets.length > 0}
        {@render facetGroup(
          guild ? strings.search.narrowByChannel : strings.search.narrowByConversation,
          channelFacets.map((facet) => {
            const place = placeOf(facet.channel_id);
            return {
              key: facet.channel_id,
              name: place.name,
              count: facet.count,
              onchoose: () => search.search(queryNarrowedToChannel(search.query, { id: facet.channel_id, name: place.filterName })),
            };
          }),
        )}
      {/if}
      {#if authorFacets.length > 0}
        {@render facetGroup(
          strings.search.narrowByPerson,
          authorFacets.map((facet) => ({
            key: facet.author.id,
            name: displayName(facet.author),
            count: facet.count,
            onchoose: () => search.search(queryNarrowedToAuthor(search.query, facet.author)),
          })),
        )}
      {/if}

      <ol aria-labelledby="{id}-title" class="mt-sm flex flex-col">
        {#each rows as row (row.hit.message.id)}
          {@const message = row.hit.message}
          {@const place = placeOf(message.channel_id)}
          <li class="flex flex-col">
            {#if row.startsChannelRun}
              <p class="mt-xs mb-2xs flex min-w-0 items-center gap-2xs ps-2xs type-body-sm font-semibold text-(--mw-component-search-channel-label-text) first:mt-0">
                <Icon name={place.isConversation ? "at" : "hash"} class="size-(--mw-component-search-field-icon-size) shrink-0" />
                <span class="truncate">{place.isConversation ? place.name : place.name.slice(1)}</span>
              </p>
            {/if}
            {@render slip(row.hit, place.name)}
          </li>
        {/each}
      </ol>

      {#if search.isLoadingMore}
        <div role="status" aria-label={strings.search.loadingMore} class="mt-(--mw-component-search-result-gap)">
          {@render skeletonSlip()}
        </div>
      {:else if search.loadMoreError !== null}
        <div role="alert" class="mt-sm flex flex-col items-start gap-xs">
          <p class="type-body text-text-secondary">{describeError(search.loadMoreError, strings.search.loadMoreFailed)}</p>
          <Button onclick={() => search.retry()}>{strings.common.tryAgain}</Button>
        </div>
      {:else if search.hasReachedLimit}
        <p class="mt-sm type-body-sm text-text-muted">{strings.search.limitReached(MAXIMUM_REACHABLE_SEARCH_RESULTS)}</p>
      {/if}
      {#if search.canLoadMore}
        <div bind:this={sentinel} aria-hidden="true" class="h-px"></div>
      {/if}
    {/if}
  </div>
</aside>

{#snippet filters()}
  <ul aria-label={strings.search.appliedFilters} class="mt-sm flex flex-wrap gap-2xs">
    {#each appliedFilters as part (part.start)}
      {@const label = filterChipLabel(part.filter)}
      <li class="{chipClass} bg-(--mw-component-search-filter-background) text-(--mw-component-search-filter-text)">
        <span class="min-w-0 truncate">{label}</span>
        <button
          type="button"
          aria-label={strings.search.removeFilter(label)}
          title={strings.search.removeFilter(label)}
          class="grid size-(--mw-component-search-facet-height) shrink-0 place-items-center rounded-full hover:bg-state-hover pointer-coarse:size-(--mw-size-hit-web-min)"
          onclick={() => removeFilter(part)}
        >
          <Icon name="x" class="size-(--mw-component-search-field-icon-size)" />
        </button>
      </li>
    {/each}
  </ul>
{/snippet}

{#snippet facetGroup(label: string, facets: { key: string; name: string; count: number; onchoose: () => void }[])}
  <div role="group" aria-label={label} class="mt-sm flex flex-col gap-2xs">
    <p aria-hidden="true" class="type-body-sm text-text-muted">{label}</p>
    <div class="flex flex-wrap gap-2xs">
      {#each facets as facet (facet.key)}
        <button
          type="button"
          aria-label={strings.search.narrowTo(facet.name, facet.count, false)}
          class="{chipClass} pe-xs bg-(--mw-component-search-facet-background) text-(--mw-component-search-facet-text) hover:bg-(--mw-component-search-facet-background-hover)"
          onclick={facet.onchoose}
        >
          <span class="min-w-0 truncate">{facet.name}</span>
          <span class="tabular-nums text-(--mw-component-search-facet-count-text)">{facet.count.toLocaleString(formattingLocale)}</span>
        </button>
      {/each}
    </div>
  </div>
{/snippet}

{#snippet slip(hit: MessageSearchHit, placeName: string)}
  {@const message = hit.message}
  {@const sentAt = unixMillisecondsOfSnowflake(message.id)}
  {@const isOpening = openingMessageId === message.id}
  {@const isFolded = client.isBlocked(message.author.id) && !revealedBlockedIds.has(message.id)}
  {#if isFolded}
    <div
      class="flex flex-wrap items-center gap-x-xs rounded-(--mw-component-search-result-radius) bg-(--mw-component-search-result-background) px-(--mw-component-search-result-padding) py-xs type-body text-(--mw-component-message-blocked-text)"
    >
      <Icon name="prohibit" class="size-(--mw-component-message-blocked-icon-size)" />
      <span>{strings.messages.blockedRun(1)}</span>
      <button
        type="button"
        class="rounded-xs font-semibold text-(--mw-component-message-blocked-action-text) hover:underline pointer-coarse:min-h-(--mw-size-hit-web-min)"
        onclick={() => revealedBlockedIds.add(message.id)}>{strings.messages.showBlocked}</button
      >
    </div>
  {:else}
    {@const blocks = withSearchHighlights(parseMessageMarkdown(message.content, mentionable, client.knownCustomEmojis), message.content, hit.highlights)}
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_noninteractive_element_interactions -->
    <article
      aria-label={strings.messages.authorSaid(displayName(message.author), "")}
      aria-busy={isOpening}
      class="group/slip relative mt-(--mw-component-search-result-gap) flex cursor-pointer gap-sm rounded-(--mw-component-search-result-radius) bg-(--mw-component-search-result-background) p-(--mw-component-search-result-padding) first:mt-0"
      onclick={(event) => openFromSlip(event, hit)}
    >
      <Avatar user={message.author} sizeToken="--mw-component-search-result-avatar-size" />
      <div class="flex min-w-0 flex-1 flex-col gap-3xs">
        <p class="flex flex-wrap items-baseline gap-x-xs pe-[calc(var(--mw-component-search-sort-height)_+_var(--mw-space-xl))]">
          <span class="type-message-author text-(--mw-component-message-author-text)">{displayName(message.author)}</span>
          <time
            datetime={new Date(sentAt).toISOString()}
            title={fullTimestampLabel(sentAt)}
            class="type-message-timestamp text-(--mw-component-message-timestamp-text)">{messageTimestampLabel(sentAt, new Date())}</time
          >
          <span class="sr-only">{placeName}</span>
        </p>
        {#if message.content}
          <div class="type-message text-(--mw-component-message-body-text)">
            <MessageMarkdown {blocks} />
          </div>
        {/if}
        {#if (message.attachments?.length ?? 0) > 0}
          <MessageAttachments attachments={message.attachments ?? []} isPending={false} />
        {/if}
      </div>
      <!-- The keyboard's way in; the slip itself answers a click. Shown on hover and focus with a
           mouse, always under a coarse pointer. -->
      <button
        type="button"
        aria-label={isOpening ? strings.search.opening : strings.search.jumpTo(displayName(message.author))}
        aria-disabled={isOpening || undefined}
        class={[
          "absolute end-xs top-xs flex h-(--mw-component-search-sort-height) items-center rounded-sm bg-(--mw-component-button-secondary-background) px-xs type-body-sm font-semibold text-(--mw-component-button-secondary-text) transition-[opacity,background-color] duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:bg-(--mw-component-button-secondary-background-hover) focus-visible:opacity-100 pointer-coarse:h-(--mw-size-hit-web-min) pointer-coarse:opacity-100",
          isOpening ? "opacity-100" : "opacity-0 group-hover/slip:opacity-100",
        ]}
        onclick={() => {
          if (!isOpening) {
            onopenresult(hit);
          }
        }}>{isOpening ? strings.search.opening : strings.search.jump}</button
      >
    </article>
  {/if}
{/snippet}

{#snippet skeletonSlip()}
  <div aria-hidden="true" class="flex gap-sm rounded-(--mw-component-search-result-radius) bg-(--mw-component-search-result-background) p-(--mw-component-search-result-padding)">
    <span class="size-(--mw-component-search-result-avatar-size) shrink-0 rounded-full bg-(--mw-component-message-skeleton-fill)"></span>
    <span class="flex flex-1 flex-col gap-xs pt-2xs">
      <span class="h-(--mw-component-message-skeleton-bar-height) w-2/5 rounded-full bg-(--mw-component-message-skeleton-fill)"></span>
      <span class="h-(--mw-component-message-skeleton-bar-height) w-4/5 rounded-full bg-(--mw-component-message-skeleton-fill)"></span>
      <span class="h-(--mw-component-message-skeleton-bar-height) w-3/5 rounded-full bg-(--mw-component-message-skeleton-fill)"></span>
    </span>
  </div>
{/snippet}

<style>
  .side-sheet-docked {
    animation: mw-slide-from-end var(--mw-motion-duration-slow) var(--mw-motion-easing-enter) both;
  }
</style>
