<script lang="ts">
  import { onMount, untrack } from "svelte";

  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import Button from "./Button.svelte";
  import type { Message, PublicUser, Snowflake } from "../lib/protocol";
  import { attachmentCountLabel, displayName, timeOfDayLabel } from "./format";
  import Icon, { type IconName } from "./Icon.svelte";
  import type { ReactionAndPinActions } from "./message-actions";
  import MessageItem from "./MessageItem.svelte";
  import { mentionablePeople } from "./mentions";
  import { type MessageListRow, buildMessageRows, isUnreadStartLoaded, reuseUnchangedRows } from "./message-rows";
  import { nameColorsByUserId } from "./name-colors.svelte";
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
    reactionAndPinActions: ReactionAndPinActions;
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
    reactionAndPinActions,
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
  // Role colours on author names, readable on the sheet in the current theme (DESIGN.md §2.3).
  const authorColors = $derived(
    nameColorsByUserId(
      client.selectedGuild,
      client.selectedGuild ? client.membersByGuild.get(client.selectedGuild.id) : undefined,
      "color-surface-chat",
    ),
  );
  // Each new message re-sorts the conversations into new objects; their recipients stay the same
  // array, so reading it alone keeps every row from parsing its text again.
  const conversationRecipients = $derived(client.selectedConversation?.recipients ?? []);
  const mentionable = $derived(
    mentionablePeople([
      currentUser,
      ...(client.selectedGuild ? (client.membersByGuild.get(client.selectedGuild.id) ?? []).map((member) => member.user) : []),
      ...conversationRecipients,
    ]),
  );
  const unreadMarker = $derived(client.unreadMarkers.get(channelId) ?? null);
  // The divider waits until the first new message is loaded, so it never jumps as pages arrive.
  const dividerAfterMessageId = $derived(
    unreadMarker && isUnreadStartLoaded(messages, unreadMarker, client.reachedStartChannelIds.has(channelId))
      ? unreadMarker.lastReadMessageId
      : null,
  );
  // Not reactive: the rows of the last build, so unchanged rows keep their identity.
  let previousRowsByKey = new Map<string, MessageListRow>();
  const rows = $derived.by(() => {
    const built = reuseUnchangedRows(
      previousRowsByKey,
      buildMessageRows(messages, client.pendingMessagesIn(channelId), currentUser.id, Date.now(), dividerAfterMessageId),
    );
    previousRowsByKey = new Map(built.map((row) => [row.key, row]));
    return built;
  });
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
  // The focused row may be gone (deleted, or a sent row replacing its pending one): the list
  // must keep its one Tab stop.
  const rovingTargetKey = $derived(rows.some((row) => row.key === rovingKey) ? rovingKey : (rows.at(-1)?.key ?? null));
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
    noteUnreadDividerSeen();
    releaseUnreadMarkerAtLatest();
  }

  // The jump bar goes once the divider has been seen and the reader scrolled down to the latest
  // message (DESIGN.md §5.4). Scrolls the list makes itself (opening, new messages) do not count.
  let hasReaderScrolled = false;
  let hasSeenUnreadDivider = false;

  function noteReaderScrolling() {
    hasReaderScrolled = true;
  }

  function noteUnreadDividerSeen() {
    // Runs on every scroll event: skip the query of the whole list when it cannot change anything.
    if (hasSeenUnreadDivider || dividerAfterMessageId === null) {
      return;
    }
    const divider = viewport.querySelector<HTMLElement>("[data-unread-divider]");
    if (!divider) {
      return;
    }
    const view = viewport.getBoundingClientRect();
    const line = divider.getBoundingClientRect();
    hasSeenUnreadDivider ||= line.top >= view.top && line.bottom <= view.bottom;
  }

  function releaseUnreadMarkerAtLatest() {
    if (hasReaderScrolled && hasSeenUnreadDivider && isPinnedToLatest && unreadMarker) {
      client.dismissUnreadMarker(channelId);
    }
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
  let hasShownUnreadDivider = false;
  $effect(() => {
    void rows;
    void client.isLoadingOlder;
    if (isPinnedToLatest) {
      viewport.scrollTop = viewport.scrollHeight;
      showUnreadDividerOnce();
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

  /**
   * Opening a channel with unread messages shows the first of them (DESIGN.md §5.4): when the
   * "New" divider is above the view, the list scrolls it under the jump bar instead of the bottom.
   */
  function showUnreadDividerOnce() {
    if (hasShownUnreadDivider || dividerAfterMessageId === null) {
      return;
    }
    const divider = viewport.querySelector<HTMLElement>("[data-unread-divider]");
    if (!divider) {
      return;
    }
    hasShownUnreadDivider = true;
    const distanceAboveView = viewport.getBoundingClientRect().top - divider.getBoundingClientRect().top;
    if (distanceAboveView > 0) {
      const clearanceUnderJumpBar = lengthTokenInPixels("component-jump-bar-height") + lengthTokenInPixels("space-md");
      viewport.scrollTop -= distanceAboveView + clearanceUnderJumpBar;
      isPinnedToLatest = false;
      isViewingOlderMessages = true;
    }
    noteUnreadDividerSeen();
  }

  /** New messages from others are read out politely; the first page and own messages are not. */
  function announceLatestMessage() {
    const latest = rows.at(-1);
    if (latest?.kind !== "sent") {
      return;
    }
    if (lastAnnouncedKey !== null && latest.key !== lastAnnouncedKey && latest.message.author.id !== currentUser.id) {
      announcement = strings.messages.authorSaid(displayName(latest.message.author), messageSummary(latest.message));
    }
    lastAnnouncedKey = latest.key;
  }

  /** A pinned message chosen in the pins panel: scrolled into the middle and focused. */
  export function revealMessage(messageId: Snowflake): void {
    const element = viewport.querySelector<HTMLElement>(`[data-row-key="${CSS.escape(messageId)}"]`);
    if (!element) {
      return;
    }
    noteReaderScrolling();
    rovingKey = messageId;
    element.scrollIntoView({ block: "center" });
    element.focus({ preventScroll: true });
    handleScroll();
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

  function replyReferenceOf(message: Message): { author: PublicUser | null; text: string } | null {
    if (message.reply_to_id === null) {
      return null;
    }
    const original = messages.find((candidate) => candidate.id === message.reply_to_id);
    return original
      ? { author: original.author, text: messageSummary(original) }
      : { author: null, text: strings.messages.originalUnavailable };
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
  <div role="status" aria-label={strings.messages.loading}>
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
  <!-- The input listeners only note that the reader scrolls; they handle nothing themselves. -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    bind:this={viewport}
    class="flex h-full flex-col overflow-y-auto [overflow-anchor:none]"
    onscroll={handleScroll}
    onwheel={noteReaderScrolling}
    ontouchmove={noteReaderScrolling}
    onpointerdown={noteReaderScrolling}
    onkeydown={noteReaderScrolling}
  >
    <!-- Pushes a short history to the bottom, as Discord fills a channel from the composer up. -->
    <div class="flex-1"></div>
    {#if hasReachedStart}
      <section
        aria-label={strings.messages.startOf(channelLabel)}
        class="flex flex-col gap-(--mw-component-message-channel-start-gap) px-md pt-md pb-xs"
      >
        <span
          class="grid size-(--mw-component-message-channel-start-icon-circle-size) place-items-center rounded-full bg-(--mw-component-message-channel-start-icon-background) text-(--mw-component-message-channel-start-icon)"
        >
          <Icon name={heroIcon} class="size-(--mw-component-message-channel-start-icon-size)" />
        </span>
        <h2 class="mt-xs type-display break-words text-(--mw-component-message-channel-start-title-text)">{heroTitle}</h2>
        <p class="type-body-lg text-(--mw-component-message-channel-start-description-text)">{heroSentence}</p>
        {#if topic}
          <p class="type-body-lg text-(--mw-component-message-channel-start-description-text)">{topic}</p>
        {/if}
      </section>
    {:else if hasHistoryFailed}
      <div
        role="alert"
        class="flex flex-col items-start gap-sm pr-(--mw-component-message-padding-trailing) pl-(--mw-component-message-avatar-inset) py-(--mw-component-message-channel-start-padding-block)"
      >
        <p class="type-message text-text-muted">
          {isInitialLoad ? strings.messages.loadFailed(channelLabel) : strings.messages.loadOlderFailed}
        </p>
        <Button onclick={() => client.retryLoadingHistory()}>{strings.common.tryAgain}</Button>
      </div>
    {:else if isSkeletonDelayElapsed && (isInitialLoad || client.isLoadingOlder)}
      {@render skeletons(isInitialLoad ? INITIAL_SKELETON_COUNT : OLDER_HISTORY_SKELETON_COUNT)}
    {/if}

    <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
    <ol bind:this={list} aria-label={strings.messages.messagesIn(channelLabel)} class="pb-xs" onkeydown={moveFocus}>
      {#each rows as row (row.key)}
        {#if row.kind === "sent" && row.isFirstUnread}
          <!-- An apricot line with its "New" pill: new messages are "now" (DESIGN.md §2.3, §5.4). -->
          <li
            data-unread-divider
            role="separator"
            aria-label={strings.messages.newSince(timeOfDayLabel(row.sentAt))}
            class="relative mx-md mt-(--mw-component-message-group-gap) h-0 border-t-(length:--mw-component-unread-divider-line-width) border-(--mw-component-unread-divider-line)"
          >
            <span
              aria-hidden="true"
              class="absolute end-0 top-[calc(var(--mw-component-unread-divider-pill-height)/-2)] grid h-(--mw-component-unread-divider-pill-height) place-items-center rounded-(--mw-component-unread-divider-pill-radius) bg-(--mw-component-unread-divider-pill-background) px-xs type-badge text-(--mw-component-unread-divider-pill-text)"
              >{strings.messages.newPill}</span
            >
          </li>
        {/if}
        <MessageItem
          {row}
          {currentUser}
          mentionablePeople={mentionable}
          authorColor={authorColors.get(row.kind === "sent" ? row.message.author.id : currentUser.id) ?? null}
          replyReference={row.kind === "sent" ? replyReferenceOf(row.message) : null}
          {canSendMessages}
          {canManageMessages}
          {reactionAndPinActions}
          isEditing={row.kind === "sent" && row.message.id === editingMessageId}
          isRovingTarget={row.key === rovingTargetKey}
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
        noteReaderScrolling();
        jumpToPresent();
        releaseUnreadMarkerAtLatest();
        onjumptopresent();
      }}
    >
      <span class="min-w-0 flex-1 truncate text-start type-body">{strings.messages.viewingOlder}</span>
      <span class="type-button">{strings.messages.jumpToPresent}</span>
    </button>
  {/if}

  <p class="sr-only" aria-live="polite">{announcement}</p>
</div>
