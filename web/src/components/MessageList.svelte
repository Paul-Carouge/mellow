<script lang="ts">
  import { onMount, untrack } from "svelte";

  import type { MellowClient } from "../lib/client.svelte";
  import Button from "./Button.svelte";
  import type { Message, PublicUser, Snowflake } from "../lib/protocol";
  import { attachmentCountLabel, displayName } from "./format";
  import Icon, { type IconName } from "./Icon.svelte";
  import MessageItem from "./MessageItem.svelte";
  import { buildMessageRows } from "./message-rows";
  import { durationTokenInMilliseconds, lengthTokenInPixels } from "./tokens";

  // The message list (DESIGN.md §5.4): history start or skeletons above the oldest message, the
  // rows, and "Jump to present" while reading older messages. It sticks to the latest message
  // and loads older pages near the top without moving the rows being read.
  interface Props {
    client: MellowClient;
    channelId: Snowflake;
    currentUser: PublicUser;
    /** "#general" or "Ana": names the list and the history start. */
    channelLabel: string;
    heroIcon: IconName;
    heroTitle: string;
    heroSentence: string;
    topic: string;
    canSendMessages: boolean;
    canManageMessages: boolean;
    editingMessageId: Snowflake | null;
    maximumEditorHeight: () => number;
    onreply: (message: Message) => void;
    onstartedit: (message: Message) => void;
    onsaveedit: (message: Message, text: string) => void;
    oncanceledit: () => void;
    ondelete: (message: Message, skipConfirmation: boolean) => void;
    oncopy: (text: string) => void;
    onjumptopresent: () => void;
  }

  let {
    client,
    channelId,
    currentUser,
    channelLabel,
    heroIcon,
    heroTitle,
    heroSentence,
    topic,
    canSendMessages,
    canManageMessages,
    editingMessageId,
    maximumEditorHeight,
    onreply,
    onstartedit,
    onsaveedit,
    oncanceledit,
    ondelete,
    oncopy,
    onjumptopresent,
  }: Props = $props();

  const INITIAL_SKELETON_COUNT = 5;
  const OLDER_HISTORY_SKELETON_COUNT = 3;
  /** Varied on purpose, so placeholders read as messages rather than a grid (as on desktop). */
  const SKELETON_SHAPES = [
    { name: 0.75, lines: [0.72, 0.46] },
    { name: 0.5, lines: [0.58] },
    { name: 0.9, lines: [0.86, 0.78, 0.34] },
    { name: 0.6, lines: [0.4] },
    { name: 0.8, lines: [0.66, 0.52] },
  ];

  const messages = $derived(client.messages(channelId));
  const rows = $derived(
    buildMessageRows(messages, client.pendingMessagesIn(channelId), currentUser.id, Date.now()),
  );
  const hasReachedStart = $derived(client.reachedStartChannelIds.has(channelId));
  const isInitialLoad = $derived(messages.length === 0 && !hasReachedStart);
  // Set when a history request failed: shown with "Try again" instead of skeletons forever.
  const hasHistoryFailed = $derived(client.failedHistoryChannelIds.has(channelId));

  let viewport: HTMLElement;
  let list: HTMLElement;
  // Not reactive: written by scrolling, read around DOM updates.
  let isPinnedToLatest = true;
  let isViewingOlderMessages = $state(false);
  let rovingKey = $state<string | null>(null);
  let announcement = $state("");
  let isSkeletonDelayElapsed = $state(false);

  onMount(() => {
    const timer = setTimeout(() => (isSkeletonDelayElapsed = true), durationTokenInMilliseconds("motion-delay-skeleton"));
    // Window resizes, a growing composer and images that finish loading change the sizes: stay
    // on the latest message.
    // shortcut: rows above the reader still move when their images load; reserve each image's box
    // once the protocol carries pixel sizes (ADR-0020).
    const observer = new ResizeObserver(() => {
      if (isPinnedToLatest) {
        viewport.scrollTop = viewport.scrollHeight;
      }
    });
    observer.observe(viewport);
    observer.observe(list);
    return () => {
      clearTimeout(timer);
      observer.disconnect();
    };
  });

  function loadOlderWhenNearTop() {
    if (viewport.scrollTop < lengthTokenInPixels("component-message-history-load-distance")) {
      void client.loadOlderMessages();
    }
  }

  function handleScroll() {
    isPinnedToLatest = viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight <= 1;
    isViewingOlderMessages = !isPinnedToLatest;
    loadOlderWhenNearTop();
  }

  // Before the DOM changes: remember where the first visible row sits, to put it back after.
  let anchor: { key: string; offset: number } | null = null;
  $effect.pre(() => {
    void rows;
    void client.isLoadingOlder;
    if (!viewport || isPinnedToLatest) {
      anchor = null;
      return;
    }
    const top = viewport.getBoundingClientRect().top;
    const firstVisible = [...viewport.querySelectorAll<HTMLElement>("[data-row-key]")].find(
      (element) => element.getBoundingClientRect().bottom > top,
    );
    anchor = firstVisible?.dataset.rowKey
      ? { key: firstVisible.dataset.rowKey, offset: firstVisible.getBoundingClientRect().top - top }
      : null;
  });

  let lastAnnouncedKey: string | null = null;
  $effect(() => {
    void rows;
    void client.isLoadingOlder;
    if (isPinnedToLatest) {
      viewport.scrollTop = viewport.scrollHeight;
    } else if (anchor) {
      const element = viewport.querySelector<HTMLElement>(`[data-row-key="${CSS.escape(anchor.key)}"]`);
      if (element) {
        viewport.scrollTop += element.getBoundingClientRect().top - viewport.getBoundingClientRect().top - anchor.offset;
      }
    }
    // A short first page leaves nothing to scroll: keep loading until the list fills or starts.
    if (rows.length > 0) {
      untrack(loadOlderWhenNearTop);
    }
    announceLatestMessage();
  });

  /** New messages from others are read out politely; the first page and own messages are not. */
  function announceLatestMessage() {
    const latest = rows.at(-1);
    if (latest?.kind !== "sent") {
      return;
    }
    if (lastAnnouncedKey !== null && latest.key !== lastAnnouncedKey && latest.message.author.id !== currentUser.id) {
      announcement = `${displayName(latest.message.author)}: ${messageSummary(latest.message)}`;
    }
    lastAnnouncedKey = latest.key;
  }

  export function jumpToPresent() {
    isPinnedToLatest = true;
    isViewingOlderMessages = false;
    viewport.scrollTop = viewport.scrollHeight;
  }

  /** The text, then "2 attachments" when files came with it (DESIGN.md §7.3). */
  function messageSummary(message: Message): string {
    return [message.content, attachmentCountLabel(message.attachments?.length ?? 0)].filter(Boolean).join(" ");
  }

  function replyPreviewOf(message: Message): string {
    if (message.reply_to_id === null) {
      return "";
    }
    const original = messages.find((candidate) => candidate.id === message.reply_to_id);
    return original
      ? `@${displayName(original.author)} ${messageSummary(original)}`
      : "Original message was deleted or is not loaded";
  }

  // One Tab stop for the whole list; arrows move between messages, as on Discord (DESIGN.md §7.2).
  function moveFocus(event: KeyboardEvent) {
    const target = event.target as HTMLElement;
    if (!target.matches("[data-row-key]")) {
      return;
    }
    const items = [...viewport.querySelectorAll<HTMLElement>("[data-row-key]")];
    const index = items.indexOf(target);
    const next =
      event.key === "ArrowUp"
        ? items[index - 1]
        : event.key === "ArrowDown"
          ? items[index + 1]
          : event.key === "Home"
            ? items[0]
            : event.key === "End"
              ? items.at(-1)
              : undefined;
    if (next) {
      event.preventDefault();
      next.focus();
    }
  }
</script>

{#snippet skeletons(count: number)}
  <div role="status" aria-label="Loading messages">
    {#each SKELETON_SHAPES.slice(0, count) as shape, index (index)}
      <div
        aria-hidden="true"
        class="relative mt-(--mw-component-message-group-gap) pr-(--mw-component-message-padding-trailing) pl-(--mw-component-message-gutter) py-(--mw-component-message-padding-block)"
      >
        <span
          class="absolute left-(--mw-component-message-avatar-inset) size-(--mw-component-message-avatar-size) rounded-full bg-(--mw-component-message-skeleton-fill)"
        ></span>
        <span class="flex h-[calc(var(--mw-type-message-font-size)*var(--mw-type-message-line-height))] items-center">
          <span
            class="h-(--mw-component-message-skeleton-bar-height) rounded-full bg-(--mw-component-message-skeleton-fill)"
            style:width="calc(var(--mw-component-message-skeleton-name-max-width) * {shape.name})"
          ></span>
        </span>
        {#each shape.lines as line, lineIndex (lineIndex)}
          <span class="flex h-[calc(var(--mw-type-message-font-size)*var(--mw-type-message-line-height))] items-center">
            <span
              class="h-(--mw-component-message-skeleton-bar-height) rounded-full bg-(--mw-component-message-skeleton-fill)"
              style:width="{line * 100}%"
            ></span>
          </span>
        {/each}
      </div>
    {/each}
  </div>
{/snippet}

<div class="relative min-h-0 flex-1">
  <div
    bind:this={viewport}
    class="flex h-full flex-col overflow-y-auto [overflow-anchor:none]"
    onscroll={handleScroll}
  >
    <!-- Pushes a short history to the bottom, as Discord fills a channel from the composer up. -->
    <div class="flex-1"></div>
    {#if hasReachedStart}
      <section
        aria-label="Start of {channelLabel}"
        class="flex flex-col gap-(--mw-component-message-channel-start-gap) pr-(--mw-component-message-padding-trailing) pl-(--mw-component-message-avatar-inset) py-(--mw-component-message-channel-start-padding-block)"
      >
        <span
          class="grid size-(--mw-component-message-channel-start-icon-circle-size) place-items-center rounded-full bg-(--mw-component-message-channel-start-icon-background) text-(--mw-component-message-channel-start-icon)"
        >
          <Icon name={heroIcon} class="size-(--mw-component-message-channel-start-icon-size)" />
        </span>
        <h2 class="type-display break-words text-(--mw-component-message-channel-start-title-text)">{heroTitle}</h2>
        <p class="type-message text-(--mw-component-message-channel-start-description-text)">{heroSentence}</p>
        {#if topic}
          <p class="type-message text-(--mw-component-message-channel-start-description-text)">{topic}</p>
        {/if}
      </section>
    {:else if hasHistoryFailed}
      <div
        role="alert"
        class="flex flex-col items-start gap-sm pr-(--mw-component-message-padding-trailing) pl-(--mw-component-message-avatar-inset) py-(--mw-component-message-channel-start-padding-block)"
      >
        <p class="type-message text-text-muted">
          {isInitialLoad ? `Couldn't load messages in ${channelLabel}.` : "Couldn't load older messages."}
        </p>
        <Button onclick={() => client.retryLoadingHistory()}>Try again</Button>
      </div>
    {:else if isSkeletonDelayElapsed && (isInitialLoad || client.isLoadingOlder)}
      {@render skeletons(isInitialLoad ? INITIAL_SKELETON_COUNT : OLDER_HISTORY_SKELETON_COUNT)}
    {/if}

    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <ol bind:this={list} aria-label="Messages in {channelLabel}" class="pb-(--mw-space-lg)" onkeydown={moveFocus}>
      {#each rows as row (row.key)}
        <MessageItem
          {row}
          {currentUser}
          replyPreview={row.kind === "sent" ? replyPreviewOf(row.message) : ""}
          {canSendMessages}
          {canManageMessages}
          isEditing={row.kind === "sent" && row.message.id === editingMessageId}
          isRovingTarget={row.key === (rovingKey ?? rows.at(-1)?.key)}
          {maximumEditorHeight}
          {onreply}
          {onstartedit}
          {onsaveedit}
          {oncanceledit}
          {ondelete}
          {oncopy}
          onretry={(nonce) => void client.retryMessage(nonce)}
          ondiscard={(nonce) => client.discardMessage(nonce)}
          onfocusrow={() => (rovingKey = row.key)}
        />
      {/each}
    </ol>
  </div>

  {#if isViewingOlderMessages}
    <!-- Over the list rather than above it: taking height would move the rows being read. -->
    <button
      type="button"
      class="absolute inset-x-(--mw-component-composer-margin-inline) bottom-0 flex h-(--mw-component-jump-bar-height) items-center gap-xs rounded-t-(--mw-component-jump-bar-radius) bg-(--mw-component-jump-bar-background) px-md text-(--mw-component-jump-bar-text) hover:bg-(--mw-component-jump-bar-background-hover) focus-visible:[outline:var(--mw-size-focus-ring-width)_solid_var(--mw-component-jump-bar-text)] focus-visible:[outline-offset:calc(-1*var(--mw-size-focus-ring-width))] pointer-coarse:h-(--mw-size-hit-web-min)"
      onclick={() => {
        jumpToPresent();
        onjumptopresent();
      }}
    >
      <span class="min-w-0 flex-1 truncate text-start type-body">You're viewing older messages</span>
      <span class="type-button">Jump to present</span>
    </button>
  {/if}

  <p class="sr-only" aria-live="polite">{announcement}</p>
</div>
