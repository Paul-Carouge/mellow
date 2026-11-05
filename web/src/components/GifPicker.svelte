<script lang="ts">
  import { onDestroy, onMount, tick } from "svelte";

  import { formattingLocale, strings } from "../i18n";
  import { ApiError, type RestClient } from "../lib/api";
  import { describeError } from "../lib/client.svelte";
  import {
    type KlipyFailure,
    type KlipyGif,
    type KlipyItem,
    KLIPY_PAGE_SIZE,
    KlipyError,
    fetchKlipyPage,
    klipyItemsUrl,
    klipyListUrl,
    klipyLocaleOf,
  } from "../lib/klipy";
  import { isPictureAnimationWanted } from "../lib/motion.svelte";
  import { type GifSettings, MAXIMUM_GIF_FAVOURITES } from "../lib/protocol";
  import {
    GIF_SEARCH_DEBOUNCE_MILLISECONDS,
    createLatestRequestGuard,
    debounce,
    favouritesPage,
    gifItemKey,
    gifsInSlugOrder,
    nextGifTile,
    tileAspectRatio,
    withFavouriteSet,
  } from "./gif-picker";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";

  // The GIF picker (DESIGN.md §6.80, docs/design/web-gif-picker.md, ADR-0042): Klipy's search
  // field, the Favoris and Tendances tabs, then a contact sheet of GIFs in justified rows, in
  // Klipy's order. The emoji picker's shell: a light-dismiss popover beside its button, or a sheet
  // at the bottom of a phone's screen. One Tab stop for the tiles (arrows between them), then the
  // active tile's star; Enter sends, Esc closes.
  interface Props {
    anchor: HTMLElement;
    isCompact: boolean;
    settings: GifSettings;
    rest: RestClient;
    /** `query` is the search that found it, empty from the tabs (Klipy's share trigger wants it). */
    onpick: (gif: KlipyGif, query: string) => void;
    /** As the emoji picker's: focus goes back to the button only if it was in the picker. */
    onclose: (shouldReturnFocus: boolean) => void;
  }

  let { anchor, isCompact, settings, rest, onpick, onclose }: Props = $props();

  type Tab = "favourites" | "trending";
  type Status = { kind: "loading" } | { kind: "shown" } | { kind: "failed"; failure: KlipyFailure | "favourites" };

  const id = $props.id();
  const LOCALE = klipyLocaleOf(formattingLocale);
  const PLACEHOLDER_ASPECT_RATIOS = [1.3, 0.8, 1, 1.6, 1, 0.75, 1.2, 1.4, 0.9];

  let panel: HTMLElement;
  let list: HTMLElement;
  let search = $state<HTMLInputElement | null>(null);
  let position = $state({ top: 0, left: 0 });

  let query = $state("");
  /** The search Klipy was asked for, after typing paused. */
  let searchedQuery = $state("");
  let tab = $state<Tab>("trending");
  const isSearching = $derived(searchedQuery !== "");

  let items = $state.raw<readonly KlipyItem[]>([]);
  let status = $state<Status>({ kind: "loading" });
  let hasNext = $state(false);
  let nextPage = 1;
  /**
   * Favourites are paged here, over the list as it was when their first page loaded (stars
   * changed meanwhile reorder the live one), counting the slugs already asked of Klipy.
   */
  let frozenFavouriteSlugs: readonly string[] = [];
  let favouritesAskedCount = 0;
  let isLoadingMore = $state(false);
  let moreFailure = $state<KlipyFailure | null>(null);
  let activeIndex = $state(0);

  /** Null until the instance answered, and when it could not. */
  let favouriteSlugs = $state.raw<readonly string[] | null>(null);
  /** False until the instance's list arrived: stars changed before that do not make it whole. */
  let areFavouritesKnown = false;
  const starredSlugs = $derived(new Set(favouriteSlugs ?? []));
  let favouriteMessage = $state("");
  let favouriteAnnouncement = $state("");

  const requests = createLatestRequestGuard();
  let inFlight: AbortController | null = null;
  const searchAfterPause = debounce(() => {
    searchedQuery = query.trim();
    void loadFirstPage();
  }, GIF_SEARCH_DEBOUNCE_MILLISECONDS);

  /** Each item's place among the focusable tiles (ads are not); -1 for an ad. */
  const tileIndexes = $derived.by(() => {
    let next = 0;
    return items.map((item) => (item.kind === "gif" ? next++ : -1));
  });
  const gifCount = $derived(tileIndexes.filter((tileIndex) => tileIndex >= 0).length);
  const listTitle = $derived(
    isSearching ? strings.gifs.results(searchedQuery) : tab === "favourites" ? strings.gifs.favourites : strings.gifs.trending,
  );

  onMount(() => {
    panel.showPopover();
    place();
    search?.focus();
    void openOnFavouritesOrTrending();
  });

  onDestroy(() => {
    searchAfterPause.cancel();
    inFlight?.abort();
  });

  /** Favourites first when there are some, as on Discord; trending otherwise. */
  async function openOnFavouritesOrTrending() {
    // A tab clicked or a search typed meanwhile starts its own request and wins.
    const opening = requests.start();
    await loadFavouriteSlugs();
    if (!requests.isLatest(opening)) {
      return;
    }
    tab = favouriteSlugs !== null && favouriteSlugs.length > 0 ? "favourites" : "trending";
    await loadFirstPage();
  }

  async function loadFavouriteSlugs() {
    try {
      favouriteSlugs = await rest.listGifFavourites();
      areFavouritesKnown = true;
    } catch (error) {
      console.warn("couldn't load GIF favourites", error);
      favouriteSlugs = null;
    }
  }

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

  function failureOf(error: unknown): KlipyFailure {
    return error instanceof KlipyError ? error.failure : "failed";
  }

  /** The next page of whatever the picker shows; only the latest request may change it. */
  async function fetchNextItems(signal: AbortSignal): Promise<{ items: KlipyItem[]; hasNext: boolean }> {
    if (isSearching || tab === "trending") {
      const page = await fetchKlipyPage(klipyListUrl(settings, searchedQuery, nextPage, LOCALE), signal);
      nextPage += 1;
      return { items: [...page.items], hasNext: page.hasNext };
    }
    const next = favouritesPage(frozenFavouriteSlugs, favouritesAskedCount, KLIPY_PAGE_SIZE);
    if (next.slugs.length === 0) {
      return { items: [], hasNext: false };
    }
    const page = await fetchKlipyPage(klipyItemsUrl(settings, next.slugs), signal);
    favouritesAskedCount = next.askedCount;
    return { items: gifsInSlugOrder(page.items, next.slugs), hasNext: next.hasNext };
  }

  async function loadFirstPage() {
    const request = requests.start();
    inFlight?.abort();
    const controller = new AbortController();
    inFlight = controller;
    nextPage = 1;
    favouritesAskedCount = 0;
    items = [];
    hasNext = false;
    moreFailure = null;
    isLoadingMore = false;
    activeIndex = 0;
    list.scrollTop = 0;
    if (!isSearching && tab === "favourites" && !areFavouritesKnown) {
      await loadFavouriteSlugs();
      if (!requests.isLatest(request)) {
        return;
      }
      if (favouriteSlugs === null) {
        status = { kind: "failed", failure: "favourites" };
        return;
      }
    }
    frozenFavouriteSlugs = favouriteSlugs ?? [];
    status = { kind: "loading" };
    try {
      const next = await fetchNextItems(controller.signal);
      if (requests.isLatest(request)) {
        items = next.items;
        hasNext = next.hasNext;
        status = { kind: "shown" };
      }
    } catch (error) {
      if (requests.isLatest(request) && !controller.signal.aborted) {
        status = { kind: "failed", failure: failureOf(error) };
      }
    }
  }

  async function loadMore() {
    if (isLoadingMore || !hasNext || status.kind !== "shown") {
      return;
    }
    const request = requests.start();
    const controller = new AbortController();
    inFlight = controller;
    isLoadingMore = true;
    moreFailure = null;
    try {
      const next = await fetchNextItems(controller.signal);
      if (requests.isLatest(request)) {
        items = [...items, ...next.items];
        hasNext = next.hasNext;
      }
    } catch (error) {
      if (requests.isLatest(request) && !controller.signal.aborted) {
        moreFailure = failureOf(error);
      }
    } finally {
      if (requests.isLatest(request)) {
        isLoadingMore = false;
      }
    }
  }

  function showTab(next: Tab) {
    // While the picker opens, even the tab already shown is a choice that must stick.
    if (next === tab && !isSearching && status.kind !== "loading") {
      return;
    }
    tab = next;
    void loadFirstPage();
  }

  function handleQueryInput() {
    if (query.trim() === "") {
      // Back to the tabs at once: nothing to wait for.
      searchAfterPause.cancel();
      if (isSearching) {
        searchedQuery = "";
        void loadFirstPage();
      }
      return;
    }
    if (query.trim() === searchedQuery) {
      // Back to the words already searched: nothing new to ask.
      searchAfterPause.cancel();
    } else {
      searchAfterPause.call();
    }
  }

  /** Optimistic: the star changes at once and comes back if the instance refuses. */
  async function toggleFavourite(gif: KlipyGif) {
    const shouldStar = !starredSlugs.has(gif.slug);
    favouriteMessage = "";
    if (shouldStar && (favouriteSlugs?.length ?? 0) >= MAXIMUM_GIF_FAVOURITES) {
      favouriteMessage = strings.gifs.tooManyFavourites(MAXIMUM_GIF_FAVOURITES);
      return;
    }
    favouriteSlugs = withFavouriteSet(favouriteSlugs ?? [], gif.slug, shouldStar);
    try {
      await (shouldStar ? rest.addGifFavourite(gif.slug) : rest.removeGifFavourite(gif.slug));
      favouriteAnnouncement = shouldStar ? strings.gifs.favouriteAdded : strings.gifs.favouriteRemoved;
    } catch (error) {
      // Only this GIF goes back: other stars changed meanwhile stay as they are.
      favouriteSlugs = withFavouriteSet(favouriteSlugs ?? [], gif.slug, !shouldStar);
      favouriteMessage = favouriteFailureSentence(error);
    }
  }

  function favouriteFailureSentence(error: unknown): string {
    if (error instanceof ApiError && (error.status === 409 || error.code === "LIMIT_REACHED")) {
      return strings.gifs.tooManyFavourites(MAXIMUM_GIF_FAVOURITES);
    }
    if (error instanceof ApiError && error.code === "RATE_LIMITED") {
      return strings.gifs.favouritesBusy;
    }
    return describeError(error, strings.gifs.favouriteFailed);
  }

  function failureSentence(failure: KlipyFailure | "favourites"): string {
    switch (failure) {
      case "offline":
        return strings.gifs.offline;
      case "rate-limited":
        return strings.gifs.rateLimited;
      case "refused":
        return strings.gifs.refused;
      case "favourites":
        return strings.gifs.favouritesFailed;
      case "failed":
        return strings.gifs.failed;
      default: {
        const unknown: never = failure;
        throw new Error(`unhandled GIF failure ${String(unknown)}`);
      }
    }
  }

  function tiles(): HTMLButtonElement[] {
    return [...list.querySelectorAll<HTMLButtonElement>("[data-gif-tile]")];
  }

  async function focusTile(index: number) {
    activeIndex = index;
    await tick();
    const tile = tiles()[index];
    tile?.focus({ preventScroll: true });
    tile?.scrollIntoView({ block: "nearest" });
  }

  function handleListKeydown(event: KeyboardEvent) {
    const all = tiles();
    const current = all.indexOf(document.activeElement as HTMLButtonElement);
    if (current === -1) {
      return;
    }
    const move = nextGifTile(
      all.map((tile) => tile.getBoundingClientRect()),
      current,
      event.key,
    );
    if (move === null) {
      return;
    }
    event.preventDefault();
    if (move === "search") {
      search?.focus();
    } else {
      void focusTile(move);
    }
  }

  function handleSearchKeydown(event: KeyboardEvent) {
    if (event.key === "ArrowDown" && tiles().length > 0) {
      event.preventDefault();
      void focusTile(0);
    } else if (event.key === "Enter") {
      // Enter searches now rather than after the pause; it never sends a GIF by itself.
      event.preventDefault();
      if (query.trim() !== "") {
        searchAfterPause.flush();
      }
    }
  }

  function handleTabKeydown(event: KeyboardEvent) {
    if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") {
      return;
    }
    event.preventDefault();
    const next: Tab = tab === "favourites" ? "trending" : "favourites";
    showTab(next);
    void tick().then(() => panel.querySelector<HTMLButtonElement>(`[data-tab="${next}"]`)?.focus());
  }

  /** "Plus de GIF" asks by itself once it scrolls into view; it stays a button for keyboards. */
  function loadWhenSeen(button: HTMLElement) {
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          void loadMore();
        }
      },
      { root: list, rootMargin: "0px 0px 50% 0px" },
    );
    observer.observe(button);
    return () => observer.disconnect();
  }

  function retryAfterReconnecting() {
    if (status.kind === "failed" && status.failure === "offline") {
      void loadFirstPage();
    } else if (moreFailure === "offline") {
      moreFailure = null;
      void loadMore();
    }
  }
</script>

<svelte:window ononline={retryAfterReconnecting} />

{#snippet tileMedia(gif: KlipyGif)}
  {@const isAnimated = isPictureAnimationWanted(true)}
  {#if isAnimated && gif.tile.isVideo}
    <video
      src={gif.tile.url}
      width={gif.tile.width}
      height={gif.tile.height}
      muted
      loop
      autoplay
      playsinline
      disablepictureinpicture
      preload="auto"
      aria-hidden="true"
      class="pointer-events-none size-full object-cover"
    ></video>
  {:else if isAnimated || gif.still}
    {@const picture = isAnimated ? gif.tile : (gif.still ?? gif.tile)}
    <img
      src={picture.url}
      width={picture.width}
      height={picture.height}
      alt=""
      loading="lazy"
      decoding="async"
      referrerpolicy="no-referrer"
      draggable="false"
      class="pointer-events-none size-full object-cover"
    />
  {/if}
{/snippet}

<div
  bind:this={panel}
  popover="auto"
  role="dialog"
  aria-label={strings.gifs.label}
  class={[
    "fixed m-0 flex-col gap-(--mw-component-gif-picker-gap) border-(length:--mw-component-gif-picker-border-width) border-(--mw-component-gif-picker-border) bg-(--mw-component-gif-picker-background) p-(--mw-component-gif-picker-padding) shadow-(--mw-component-gif-picker-shadow) [&:popover-open]:flex",
    isCompact
      ? "inset-x-0 top-auto bottom-0 h-[calc(var(--mw-component-gif-picker-height)+env(safe-area-inset-bottom))] max-h-[70dvh] w-full max-w-none rounded-t-(--mw-component-gif-picker-radius) pb-[calc(var(--mw-component-gif-picker-padding)+env(safe-area-inset-bottom))] [&:popover-open]:animate-[mw-rise_var(--mw-motion-duration-base)_var(--mw-motion-easing-enter)_both]"
      : "inset-auto h-(--mw-component-gif-picker-height) max-h-[calc(100dvh-2*var(--mw-space-xs))] w-(--mw-component-gif-picker-width) max-w-[calc(100vw-2*var(--mw-space-xs))] rounded-(--mw-component-gif-picker-radius) [&:popover-open]:animate-[mw-drop-in_var(--mw-motion-duration-fast)_var(--mw-motion-easing-enter)_both]",
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
    <span class="sr-only">{strings.gifs.search}</span>
    <input
      bind:this={search}
      bind:value={query}
      type="search"
      autocomplete="off"
      spellcheck="false"
      enterkeyhint="search"
      placeholder={strings.gifs.placeholder}
      aria-describedby="{id}-help"
      aria-controls="{id}-gifs"
      class="min-w-0 flex-1 bg-transparent type-body text-(--mw-component-emoji-picker-search-text) outline-none placeholder:text-(--mw-component-emoji-picker-search-placeholder) [&::-webkit-search-cancel-button]:appearance-none"
      oninput={handleQueryInput}
      onkeydown={handleSearchKeydown}
    />
  </label>
  <p id="{id}-help" class="sr-only">{strings.gifs.help}</p>

  <!-- A search replaces the tabs; the field already says what the results are for. -->
  {#if !isSearching}
    <!-- One Tab stop; Left and Right switch, as tabs do. -->
    <div
      role="tablist"
      tabindex="-1"
      aria-label={strings.gifs.lists}
      class="flex shrink-0 gap-(--mw-component-tabs-gap) border-b border-(--mw-component-tabs-divider) px-2xs"
      onkeydown={handleTabKeydown}
    >
      {#each [{ value: "favourites", label: strings.gifs.favourites }, { value: "trending", label: strings.gifs.trending }] as const as option (option.value)}
        {@const isSelected = tab === option.value}
        <button
          type="button"
          role="tab"
          data-tab={option.value}
          aria-selected={isSelected}
          aria-controls="{id}-gifs"
          tabindex={isSelected ? 0 : -1}
          class={[
            "-mb-px inline-flex h-(--mw-component-emoji-picker-search-height) items-center gap-2xs border-b-(length:--mw-component-tabs-indicator-height) type-body font-semibold transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) focus-visible:focus-ring pointer-coarse:h-(--mw-size-hit-web-min)",
            isSelected
              ? "border-(--mw-component-tabs-indicator) text-(--mw-component-tabs-text-selected)"
              : "border-transparent text-(--mw-component-tabs-text) hover:text-(--mw-component-tabs-text-hover)",
          ]}
          onclick={() => showTab(option.value)}
        >
          {#if option.value === "favourites"}
            <Icon name="star" class="size-(--mw-component-gif-star-icon-size)" />
          {/if}
          {option.label}
        </button>
      {/each}
    </div>
  {/if}

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    bind:this={list}
    id="{id}-gifs"
    role={isSearching ? "region" : "tabpanel"}
    aria-label={listTitle}
    aria-busy={status.kind === "loading" || isLoadingMore}
    class="min-h-0 flex-1 overflow-y-auto [scrollbar-width:thin]"
    onkeydown={handleListKeydown}
  >
    {#if status.kind === "loading"}
      <p class="sr-only" role="status">{strings.gifs.loading}</p>
      <div aria-hidden="true" class="flex flex-wrap gap-(--mw-component-gif-tile-gap)">
        {#each PLACEHOLDER_ASPECT_RATIOS as ratio, index (index)}
          <div
            class="h-(--mw-component-gif-row-height) rounded-(--mw-component-gif-tile-radius) bg-(--mw-component-gif-tile-placeholder)"
            style:flex="{ratio} 1 calc(var(--mw-component-gif-row-height) * {ratio})"
          ></div>
        {/each}
      </div>
    {:else if status.kind === "failed"}
      {@const failure = status.failure}
      <div role="alert" class="flex flex-col items-start gap-xs p-xs">
        <p class="type-body text-(--mw-component-gif-status-text)">{failureSentence(failure)}</p>
        {#if failure !== "offline" && failure !== "refused"}
          <Button onclick={() => void loadFirstPage()}>{strings.gifs.retry}</Button>
        {/if}
      </div>
    {:else if items.length === 0}
      <p role="status" class="p-xs type-body text-(--mw-component-gif-status-text)">
        {isSearching ? strings.gifs.noResults(searchedQuery) : tab === "favourites" ? strings.gifs.noFavourites : strings.gifs.nothingTrending}
      </p>
    {:else}
      <!-- Justified rows: each tile takes the row's height and a width from its shape, so the
           sheet reads in Klipy's order, row by row; the spacer keeps the last row from stretching. -->
      <div class="flex flex-wrap gap-(--mw-component-gif-tile-gap)">
        {#each items as item, index (gifItemKey(item, index))}
          {@const ratio = tileAspectRatio(item)}
          {#if item.kind === "gif"}
            {@const title = item.title || strings.gifs.untitled}
            {@const isStarred = starredSlugs.has(item.slug)}
            {@const tileIndex = tileIndexes[index] ?? -1}
            <div
              class="group/tile relative h-(--mw-component-gif-row-height) overflow-hidden rounded-(--mw-component-gif-tile-radius) bg-(--mw-component-gif-tile-placeholder) bg-cover bg-center"
              style:flex="{ratio} 1 calc(var(--mw-component-gif-row-height) * {ratio})"
              style:background-image={item.blurPreview ? `url("${item.blurPreview}")` : undefined}
            >
              <button
                type="button"
                data-gif-tile
                tabindex={tileIndex === activeIndex ? 0 : -1}
                aria-label={title}
                title={title}
                class="block size-full [--mw-color-focus:var(--mw-component-gif-tile-focus-ring)] focus-visible:focus-ring-inset"
                onfocus={() => (activeIndex = tileIndex)}
                onclick={() => onpick(item, searchedQuery)}
              >
                {@render tileMedia(item)}
              </button>
              <button
                type="button"
                aria-label={isStarred ? strings.gifs.removeFavourite(title) : strings.gifs.addFavourite(title)}
                title={isStarred ? strings.gifs.removeFavourite(title) : strings.gifs.addFavourite(title)}
                tabindex={tileIndex === activeIndex ? 0 : -1}
                class={[
                  "absolute end-(--mw-component-gif-star-inset) top-(--mw-component-gif-star-inset) grid size-(--mw-component-gif-star-button-size) place-items-center rounded-full bg-(--mw-component-gif-star-background) transition-opacity duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) [--mw-color-focus:var(--mw-component-gif-tile-focus-ring)] focus-visible:focus-ring pointer-coarse:size-(--mw-size-hit-web-min)",
                  isStarred
                    ? "text-(--mw-component-gif-star-icon-on)"
                    : "text-(--mw-component-gif-star-icon) opacity-0 group-focus-within/tile:opacity-100 group-hover/tile:opacity-100 pointer-coarse:opacity-100",
                ]}
                onclick={() => void toggleFavourite(item)}
              >
                <Icon name={isStarred ? "star" : "star-outline"} class="size-(--mw-component-gif-star-icon-size)" />
              </button>
            </div>
          {:else}
            <!-- Klipy's ads are HTML for a web view, never rendered here: the tile keeps its place. -->
            <div
              role="img"
              aria-label={strings.gifs.sponsoredLabel}
              class="grid h-(--mw-component-gif-row-height) place-items-center rounded-(--mw-component-gif-tile-radius) bg-(--mw-component-gif-tile-placeholder) type-body-sm text-(--mw-component-gif-ad-text)"
              style:flex="{ratio} 1 calc(var(--mw-component-gif-row-height) * {ratio})"
            >
              <span aria-hidden="true">{strings.gifs.sponsored}</span>
            </div>
          {/if}
        {/each}
        <span aria-hidden="true" class="h-0 grow-[1000]"></span>
      </div>
      {#if moreFailure}
        <div role="alert" class="flex flex-wrap items-center gap-xs p-xs">
          <p class="type-body text-(--mw-component-gif-status-text)">{failureSentence(moreFailure)}</p>
          {#if moreFailure !== "offline" && moreFailure !== "refused"}
            <button
              type="button"
              class="inline-flex min-h-(--mw-size-hit-desktop-min) items-center rounded-sm px-xs type-body font-semibold text-text-link hover:underline focus-visible:focus-ring pointer-coarse:min-h-(--mw-size-hit-web-min)"
              onclick={() => void loadMore()}>{strings.gifs.retry}</button
            >
          {/if}
        </div>
      {:else if hasNext}
        <button
          type="button"
          class="mt-(--mw-component-gif-tile-gap) flex min-h-(--mw-size-hit-desktop-min) w-full items-center justify-center rounded-sm type-body text-(--mw-component-gif-status-text) hover:bg-state-hover focus-visible:focus-ring pointer-coarse:min-h-(--mw-size-hit-web-min)"
          disabled={isLoadingMore}
          onclick={() => void loadMore()}
          {@attach loadWhenSeen}>{isLoadingMore ? strings.gifs.loading : strings.gifs.loadMore}</button
        >
      {/if}
      <p class="sr-only" aria-live="polite">{strings.gifs.count(gifCount)}</p>
    {/if}
  </div>

  <div class="flex shrink-0 items-center gap-xs px-2xs">
    <p role="status" class="min-w-0 flex-1 type-body-sm text-(--mw-component-gif-status-text)">{favouriteMessage}</p>
    <p class="shrink-0 type-body-sm text-(--mw-component-gif-credit-text)">{strings.gifs.poweredBy}</p>
  </div>
  <p class="sr-only" aria-live="polite">{favouriteAnnouncement}</p>
</div>
