<script lang="ts">
  import { onMount, tick, untrack } from "svelte";
  import { SvelteSet } from "svelte/reactivity";

  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import Button from "./Button.svelte";
  import { type MenuLine, type MessageMenuAction, menuEntries } from "./context-menus";
  import ContextMenu from "./ContextMenu.svelte";
  import type { ForwardOrigin, Message, PublicUser, Snowflake } from "../lib/protocol";
  import { dayDividerLabel, displayName, timeOfDayLabel } from "./format";
  import Icon, { type IconName } from "./Icon.svelte";
  import type { ReactionAndPinActions } from "./message-actions";
  import type { MessageEditor } from "./message-editor.svelte";
  import type { MenuOrigin } from "./menu-placement";
  import MessageItem from "./MessageItem.svelte";
  import { memoizedMentionablePeople } from "./mentions";
  import {
    type BlockedRun,
    type MessageListRow,
    arrivalsSinceLeavingPresent,
    buildMessageRows,
    collapseBlockedRuns,
    isUnreadStartLoaded,
    reuseUnchangedRows,
  } from "./message-rows";
  import { nameColorsByUserId } from "./name-colors.svelte";
  import PersonContextMenu from "./PersonContextMenu.svelte";
  import { createReplyReferenceLookup, messageSummary } from "./reply-references";
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
    /** The inline editor, open on at most one message. */
    messageEditor: MessageEditor;
    maximumEditorHeight: () => number;
    onreply: (message: Message) => void;
    onforward: (message: Message) => void;
    forwardOriginPlaceOf: (channelId: Snowflake) => string | null;
    onopenforwardorigin: (origin: ForwardOrigin) => void;
    onstartedit: (message: Message) => void;
    onsaveedit: (message: Message) => void;
    oncanceledit: () => void;
    ondelete: (message: Message, skipConfirmation: boolean) => void;
    oncopy: (text: string) => void;
    onjumptopresent: () => void;
    /** After "Message" in an author's menu opened their conversation. */
    onopenedconversation: () => void;
    /** Below the drawer breakpoint: the jump bar keeps only its action, on one line. */
    isCompact: boolean;
    /** The chat's top unread bar while it shows: it covers the top of the list. */
    topUnreadBar: HTMLElement | null;
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
    messageEditor,
    maximumEditorHeight,
    onreply,
    onforward,
    forwardOriginPlaceOf,
    onopenforwardorigin,
    onstartedit,
    onsaveedit,
    oncanceledit,
    ondelete,
    oncopy,
    onjumptopresent,
    onopenedconversation,
    isCompact,
    topUnreadBar,
  }: Props = $props();

  const INITIAL_SKELETON_COUNT = 5;
  const OLDER_HISTORY_SKELETON_COUNT = 3;
  const NO_ROLE_IDS: readonly Snowflake[] = [];
  /** Varied on purpose, so placeholders read as messages rather than a grid (as on desktop). */
  const SKELETON_SHAPES = [
    { name: 0.75, lines: [0.72, 0.46] },
    { name: 0.5, lines: [0.58] },
    { name: 0.9, lines: [0.86, 0.78, 0.34] },
    { name: 0.6, lines: [0.4] },
    { name: 0.8, lines: [0.66, 0.52] },
  ];

  const messages = $derived(client.messages(channelId));
  const messagesById = $derived(new Map(messages.map((message) => [message.id, message])));
  const replyReferenceOf = createReplyReferenceLookup();
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
  // A role change rebuilds the member list around the same users: the same map keeps every row
  // from parsing its text again.
  const mentionablePeopleOf = memoizedMentionablePeople();
  const mentionable = $derived(
    mentionablePeopleOf([
      currentUser,
      ...(client.selectedGuild ? (client.membersByGuild.get(client.selectedGuild.id) ?? []).map((member) => member.user) : []),
      ...conversationRecipients,
    ]),
  );
  // Primitives and the guild's own array: a guild update that changes neither keeps every row's
  // "mentions me" as it was (ADR-0040).
  const readerGuildId = $derived(client.selectedGuild?.id ?? null);
  const readerRoleIds = $derived(client.selectedGuild?.my_role_ids ?? NO_ROLE_IDS);
  const mentionReader = $derived({ userId: currentUser.id, guildId: readerGuildId, roleIds: readerRoleIds });
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
      buildMessageRows(
        messages,
        client.pendingMessagesIn(channelId),
        currentUser.id,
        Date.now(),
        dividerAfterMessageId,
        client.reachedStartChannelIds.has(channelId),
      ),
    );
    previousRowsByKey = new Map(built.map((row) => [row.key, row]));
    return built;
  });
  /** Messages from someone you blocked that you chose to show, for this visit only: by message,
   * so a run extended by an older page stays shown. */
  const revealedBlockedMessageIds = new SvelteSet<Snowflake>();
  const entries = $derived(
    collapseBlockedRuns(rows, (userId) => client.isBlocked(userId), revealedBlockedMessageIds),
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
  /** The newest message when the reader left the present: "· 3 new" counts what came after it. */
  let latestMessageIdWhenLeavingPresent = $state<Snowflake | null>(null);
  const arrivalsWhileAway = $derived(
    latestMessageIdWhenLeavingPresent === null
      ? 0
      : arrivalsSinceLeavingPresent(messages, latestMessageIdWhenLeavingPresent, currentUser.id),
  );
  let rovingKey = $state<string | null>(null);
  // The focused row may be gone (deleted, or a sent row replacing its pending one): the list
  // must keep its one Tab stop.
  const rovingTargetKey = $derived(
    entries.some((entry) => entry.key === rovingKey) ? rovingKey : (entries.at(-1)?.key ?? null),
  );
  let announcement = $state("");

  // One menu for every message (DESIGN.md §6.33): its More button, a right-click, a long press or
  // Shift+F10 on the message; and one for the people who wrote them.
  let messageMenu = $state<ContextMenu | null>(null);
  let personMenu = $state<PersonContextMenu | null>(null);
  let menuTarget = $state<{ message: Message; lines: MenuLine<MessageMenuAction>[]; invoker: HTMLElement } | null>(null);
  // A message deleted while its menu is open takes the menu with it: its actions would act on nothing.
  $effect(() => {
    if (menuTarget && !messages.some((message) => message.id === menuTarget?.message.id)) {
      messageMenu?.close();
    }
  });
  const messageMenuEntries = $derived.by(() => {
    if (!menuTarget) {
      return [];
    }
    const { message, lines, invoker } = menuTarget;
    const actions = reactionAndPinActions;
    return menuEntries(lines, (action) => {
      switch (action) {
        case "add-reaction":
          return { onselect: () => actions.openPicker(message, invoker) };
        case "reply":
          return { onselect: () => onreply(message) };
        case "forward":
          return { onselect: () => onforward(message) };
        case "edit":
          return { onselect: () => onstartedit(message) };
        case "copy-text":
          return { onselect: () => oncopy(message.content) };
        case "pin":
          return { onselect: () => actions.askToPin(message, true) };
        case "unpin":
          return { onselect: () => actions.askToPin(message, false) };
        case "view-reactions":
          return { onselect: () => actions.viewReactions(message) };
        case "remove-all-reactions":
          return { onselect: () => actions.askToRemoveAllReactions(message) };
        case "end-poll":
          return { onselect: () => actions.askToEndPoll(message) };
        case "delete":
          return { onselect: () => ondelete(message, false) };
      }
    });
  });

  function openMessageMenu(message: Message, lines: MenuLine<MessageMenuAction>[], origin: MenuOrigin, invoker: HTMLElement) {
    menuTarget = { message, lines, invoker };
    messageMenu?.open(origin, invoker);
  }

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

  // `MessageCodeBlock` also calls this through a synthetic `scroll` right after it unfolds, so the
  // list re-reads its position before its ResizeObserver re-pins the bottom: keep it synchronous.
  function handleScroll() {
    const wasPinnedToLatest = isPinnedToLatest;
    isPinnedToLatest = viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight <= 1;
    if (isPinnedToLatest && !wasPinnedToLatest) {
      client.noteReaderAtLatest(channelId);
    }
    isViewingOlderMessages = !isPinnedToLatest;
    if (isPinnedToLatest) {
      latestMessageIdWhenLeavingPresent = null;
    } else {
      latestMessageIdWhenLeavingPresent ??= messages.at(-1)?.id ?? null;
    }
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
  // A folded run's key follows its first message, which an older page can change: it is found
  // again by its last message (`data-anchor-key`), which a prepended page never changes.
  let anchor: { key: string; anchorKey: string; offset: number } | null = null;
  $effect.pre(() => {
    void entries;
    void client.isLoadingOlder;
    if (!viewport || isPinnedToLatest) {
      anchor = null;
      return;
    }
    const top = viewport.getBoundingClientRect().top;
    // Runs before every update while reading older messages: no copy of every loaded row.
    let firstVisible: HTMLElement | undefined;
    for (const element of viewport.querySelectorAll<HTMLElement>("[data-row-key]")) {
      if (element.getBoundingClientRect().bottom > top) {
        firstVisible = element;
        break;
      }
    }
    const key = firstVisible?.dataset.rowKey;
    anchor =
      firstVisible && key
        ? {
            key,
            anchorKey: firstVisible.dataset.anchorKey ?? key,
            offset: firstVisible.getBoundingClientRect().top - top,
          }
        : null;
  });

  let lastAnnouncedKey: string | null = null;
  let hasShownUnreadDivider = false;
  $effect(() => {
    void entries;
    void client.isLoadingOlder;
    if (isPinnedToLatest) {
      viewport.scrollTop = viewport.scrollHeight;
      showUnreadDividerOnce();
    } else if (anchor) {
      const element =
        viewport.querySelector<HTMLElement>(`[data-row-key="${CSS.escape(anchor.key)}"]`) ??
        viewport.querySelector<HTMLElement>(`[data-anchor-key="${CSS.escape(anchor.anchorKey)}"]`);
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
      // The top unread bar wraps on phones and grows to 44 px under touch: clear what it really
      // covers, from its top offset under the header. offsetHeight ignores its entry transform.
      const coveredHeight = topUnreadBar
        ? lengthTokenInPixels("space-2xs") + topUnreadBar.offsetHeight
        : lengthTokenInPixels("component-jump-bar-height");
      const clearanceUnderJumpBar = coveredHeight + lengthTokenInPixels("space-md");
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
    // Someone you blocked is never read out (ADR-0025).
    if (
      lastAnnouncedKey !== null &&
      latest.key !== lastAnnouncedKey &&
      latest.message.author.id !== currentUser.id &&
      !client.isBlocked(latest.message.author.id)
    ) {
      announcement = strings.messages.authorSaid(displayName(latest.message.author), messageSummary(latest.message, client.knownRoles));
    }
    lastAnnouncedKey = latest.key;
  }

  /**
   * A pinned message chosen in the pins panel, or a search result: scrolled into the middle and
   * focused; `shouldHighlight` lights it briefly so the eye finds it (DESIGN.md 5.12).
   */
  export async function revealMessage(messageId: Snowflake, shouldHighlight = false): Promise<void> {
    // A message folded with someone you blocked shows its run first, as Show would.
    const run = entries.find((entry) => entry.kind === "blocked-run" && entry.messageIds.includes(messageId));
    if (run?.kind === "blocked-run" && !run.isRevealed) {
      toggleBlockedRun(run);
      await tick();
    }
    const element = viewport.querySelector<HTMLElement>(`[data-row-key="${CSS.escape(messageId)}"]`);
    if (!element) {
      return;
    }
    noteReaderScrolling();
    rovingKey = messageId;
    element.scrollIntoView({ block: "center" });
    element.focus({ preventScroll: true });
    handleScroll();
    if (shouldHighlight) {
      highlightBriefly(element);
    }
  }

  /** The row takes the match wash, which fades (or, under reduced motion, holds and then goes). */
  function highlightBriefly(element: HTMLElement) {
    delete element.dataset.jumpHighlight;
    // Restarts the fade when the same message is opened twice in a row.
    void element.offsetWidth;
    element.dataset.jumpHighlight = "";
    setTimeout(() => delete element.dataset.jumpHighlight, durationTokenInMilliseconds("motion-delay-jump-highlight"));
  }

  export function jumpToPresent() {
    client.noteReaderAtLatest(channelId);
    isPinnedToLatest = true;
    isViewingOlderMessages = false;
    latestMessageIdWhenLeavingPresent = null;
    viewport.scrollTop = viewport.scrollHeight;
  }

  function toggleBlockedRun(run: BlockedRun) {
    for (const messageId of run.messageIds) {
      if (run.isRevealed) {
        revealedBlockedMessageIds.delete(messageId);
      } else {
        revealedBlockedMessageIds.add(messageId);
      }
    }
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
      {#each entries as row (row.key)}
        {@const isFirstUnread = row.kind !== "pending" && row.isFirstUnread}
        {#if row.dayStart !== null}
          {@const day = dayDividerLabel(row.dayStart)}
          <!-- A day divider: the date between two rules (DESIGN.md §5.4). Where new messages start
               the same day, it is the unread divider too, as on Discord: one apricot line, one date,
               the "New" pill at its end. -->
          <li
            data-unread-divider={isFirstUnread ? "" : undefined}
            role="separator"
            aria-label={isFirstUnread
              ? strings.messages.dayAndNewSince(day, timeOfDayLabel(row.sentAt))
              : day}
            class="mx-md mt-(--mw-component-message-group-gap) flex items-center gap-xs"
          >
            <span
              class={[
                "h-(--mw-component-unread-divider-line-width) flex-1",
                isFirstUnread ? "bg-(--mw-component-unread-divider-line)" : "bg-border-divider",
              ]}
            ></span>
            <span class="type-message-timestamp text-text-muted">{day}</span>
            <span
              class={[
                "h-(--mw-component-unread-divider-line-width) flex-1",
                isFirstUnread ? "bg-(--mw-component-unread-divider-line)" : "bg-border-divider",
              ]}
            ></span>
            {#if isFirstUnread}
              <span
                aria-hidden="true"
                class="-ms-xs grid h-(--mw-component-unread-divider-pill-height) place-items-center rounded-(--mw-component-unread-divider-pill-radius) bg-(--mw-component-unread-divider-pill-background) px-xs type-badge text-(--mw-component-unread-divider-pill-text)"
                >{strings.messages.newPill}</span
              >
            {/if}
          </li>
        {:else if isFirstUnread}
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
        {#if row.kind === "blocked-run"}
          {@const run = row}
          <!-- Messages from someone you blocked, folded (ADR-0025): one row per run in the message
               column, with Show and Hide; it holds the list's one Tab stop like a message, where
               Enter or Space toggles it, so its button stays out of the Tab order. -->
          <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
          <li
            data-row-key={run.key}
            data-anchor-key={run.anchorKey}
            tabindex={run.key === rovingTargetKey ? 0 : -1}
            class="relative mt-(--mw-component-message-group-gap) flex flex-wrap items-center gap-x-xs pr-(--mw-component-message-padding-trailing) pl-(--mw-component-message-gutter) py-(--mw-component-message-padding-block) type-body text-(--mw-component-message-blocked-text) focus-visible:focus-ring-inset"
            onfocusin={() => (rovingKey = run.key)}
            onkeydown={(event) => {
              if (event.target === event.currentTarget && (event.key === "Enter" || event.key === " ")) {
                event.preventDefault();
                toggleBlockedRun(run);
              }
            }}
          >
            <span
              class="absolute left-(--mw-component-message-avatar-inset) grid w-(--mw-component-message-avatar-size) place-items-center"
            >
              <Icon name="prohibit" class="size-(--mw-component-message-blocked-icon-size)" />
            </span>
            <span>{strings.messages.blockedRun(run.messageIds.length)}</span>
            <button
              type="button"
              tabindex="-1"
              aria-expanded={run.isRevealed}
              class="rounded-xs font-semibold text-(--mw-component-message-blocked-action-text) hover:underline pointer-coarse:min-h-(--mw-size-hit-web-min)"
              onclick={() => toggleBlockedRun(run)}
              >{run.isRevealed ? strings.messages.hideBlocked : strings.messages.showBlocked}</button
            >
          </li>
        {:else}
          <MessageItem
            {row}
            {currentUser}
            mentionablePeople={mentionable}
            customEmojis={client.knownCustomEmojis}
            roles={client.knownRoles}
            {mentionReader}
            doesServerFindMentions={client.doesServerFindMentions}
            areGifsShown={client.gifSettings !== null}
            authorColor={authorColors.get(row.kind === "sent" ? row.message.author.id : currentUser.id) ?? null}
            replyReference={row.kind === "sent"
              ? replyReferenceOf(row.message, messagesById, (userId) => client.isBlocked(userId), client.knownRoles)
              : null}
            {canSendMessages}
            {canManageMessages}
            {reactionAndPinActions}
            isEditing={row.kind === "sent" && row.message.id === messageEditor.messageId}
            {messageEditor}
            isRovingTarget={row.key === rovingTargetKey}
            isMenuOpen={row.kind === "sent" && menuTarget?.message.id === row.message.id}
            onopenmenu={openMessageMenu}
            onopenauthormenu={(author, origin, invoker) => personMenu?.openFor(author, origin, invoker) ?? false}
            {maximumEditorHeight}
            {onreply}
            {onforward}
            {forwardOriginPlaceOf}
            {onopenforwardorigin}
            {onstartedit}
            {onsaveedit}
            {oncanceledit}
            {ondelete}
            {oncopy}
            onretry={(nonce) => void client.retryMessage(nonce)}
            ondiscard={(nonce) => client.discardMessage(nonce)}
            onfocusrow={() => (rovingKey = row.key)}
          />
        {/if}
      {/each}
    </ol>
  </div>

  {#if isViewingOlderMessages}
    <!-- Over the list rather than above it: taking height would move the rows being read. It
         rests on the composer: down over the typing row, its foot tucked under the composer's
         rounded top, which the composer zone paints over it. -->
    <button
      type="button"
      class="absolute inset-x-(--mw-component-composer-margin-inline) bottom-[calc(-1*(var(--mw-component-composer-typing-height)+var(--mw-component-composer-radius)))] flex min-h-[calc(var(--mw-component-jump-bar-height)+var(--mw-component-composer-radius))] items-center gap-xs rounded-t-(--mw-component-jump-bar-radius) bg-(--mw-component-jump-bar-background) px-md pt-2xs pb-[calc(var(--mw-space-2xs)+var(--mw-component-composer-radius))] text-(--mw-component-jump-bar-text) hover:bg-(--mw-component-jump-bar-background-hover) focus-visible:[outline:var(--mw-size-focus-ring-width)_solid_var(--mw-component-jump-bar-text)] focus-visible:[outline-offset:calc(-1*var(--mw-size-focus-ring-width))] pointer-coarse:min-h-[calc(var(--mw-size-hit-web-min)+var(--mw-component-composer-radius))]"
      onclick={() => {
        noteReaderScrolling();
        jumpToPresent();
        releaseUnreadMarkerAtLatest();
        onjumptopresent();
      }}
    >
      <!-- On phones the sentence is only read out: the bar keeps its action on one line instead of
           growing over the last message (design pass L2). -->
      <span class={isCompact ? "sr-only" : "min-w-0 flex-1 text-start type-body"}>{strings.messages.viewingOlder}</span>
      <span class={["flex items-center gap-2xs type-button", isCompact ? "mx-auto min-w-0 text-center" : "shrink-0"]}>
        {#if isCompact}
          <Icon name="caret-down" class="size-(--mw-component-jump-bar-icon-size) shrink-0" />
        {/if}
        {arrivalsWhileAway > 0
          ? strings.messages.jumpToPresentWithArrivals(arrivalsWhileAway)
          : strings.messages.jumpToPresent}
      </span>
    </button>
  {/if}

  <p class="sr-only" aria-live="polite">{announcement}</p>
</div>

<ContextMenu
  bind:this={messageMenu}
  label={strings.messages.actions}
  entries={messageMenuEntries}
  fallbackFocus={() => list?.querySelector<HTMLElement>('[tabindex="0"]')?.focus()}
  onclose={() => (menuTarget = null)}
/>
<PersonContextMenu bind:this={personMenu} {client} {onopenedconversation} />
