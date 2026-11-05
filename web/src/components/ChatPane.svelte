<script lang="ts">
  import { type Snippet, onDestroy, tick } from "svelte";

  import { strings } from "../i18n";
  import { ApiError } from "../lib/api";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import {
    type Channel,
    type CustomEmoji,
    type ForwardOrigin,
    type Message,
    type NewPoll,
    Permission,
    type PublicUser,
    type Snowflake,
    customEmojiReactionKey,
    hasPermission,
    iconUrl,
  } from "../lib/protocol";
  import { visibleChannelSections } from "./channel-settings";
  import Composer from "./Composer.svelte";
  import ContextMenu, { type ContextMenuEntry } from "./ContextMenu.svelte";
  import { chatHeaderMenuActions, menuEntries } from "./context-menus";
  import { ComposerDrafts } from "./composer-drafts.svelte";
  import ConfirmMessageDialog from "./ConfirmMessageDialog.svelte";
  import { composerTextOf, withEmojiNamesAsMarkup } from "./custom-emoji";
  import DeleteMessageDialog from "./DeleteMessageDialog.svelte";
  import EmojiPicker from "./EmojiPicker.svelte";
  import { displayName, typingSegments, unixMillisecondsOfSnowflake } from "./format";
  import { forwardOriginPlace } from "./forward-messages";
  import GroupAvatar from "./GroupAvatar.svelte";
  import {
    blockedRecipientOf,
    conversationDisplayName,
    isGroupConversation,
    otherParticipants,
    participantCount,
    participantNamesOf,
  } from "./group-conversations";
  import { unreadBarLabel, unreadSummary } from "./message-rows";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import LazyComponent from "./LazyComponent.svelte";
  import type { ReactionAndPinActions } from "./message-actions";
  import { MessageEditor } from "./message-editor.svelte";
  import MessageList from "./MessageList.svelte";
  import PinnedMessagesPanel from "./PinnedMessagesPanel.svelte";
  import ReactionsDialog from "./ReactionsDialog.svelte";
  import { createPopoverToggle } from "./popover-toggle";
  import { noteEmojiUsed } from "./recent-emoji.svelte";
  import { rolesWriterMayMention, withRoleNamesAsMarkup } from "./role-mentions";
  import { triggerKlipyShare } from "../lib/klipy";

  // The chat pane (DESIGN.md §5.4, §5.5): header, message list, composer and typing band.
  interface Props {
    client: MellowClient;
    /** Below the sidebar breakpoint the header opens the navigation drawer. */
    hasNavigationDrawer: boolean;
    onopennavigation: () => void;
    /**
     * Message search (DESIGN.md 5.12): the field where the results sheet docks, else its magnifier;
     * with "Go to…" (5.10) unless that waits in the header's "More options" menu (phones).
     */
    searchControl: Snippet<[isGoToInMenu: boolean]>;
    /** "Go to…" from the header's "More options" menu. */
    onopenquickswitcher: () => void;
    isMemberListOpen: boolean;
    ontogglememberlist: () => void;
    /** The header's gear: the open channel's settings. */
    oneditchannel: (channel: Channel) => void;
    /** The header's "Add friends to the group" in a group conversation (ADR-0026). */
    onaddtogroup: () => void;
    /** A forward's footer: its original, opened in its own channel (ADR-0039). */
    onopenforwardorigin: (origin: ForwardOrigin) => void;
  }

  let {
    client,
    hasNavigationDrawer,
    onopennavigation,
    searchControl,
    onopenquickswitcher,
    isMemberListOpen,
    ontogglememberlist,
    oneditchannel,
    onaddtogroup,
    onopenforwardorigin,
  }: Props = $props();

  /** The composer grows with its text up to half of the pane (DESIGN.md §4.2). */
  const COMPOSER_MAXIMUM_SHARE_OF_PANE = 0.5;
  const TYPING_REFRESH_INTERVAL_MILLISECONDS = 1_000;

  const guild = $derived(client.selectedGuild);
  const channel = $derived(client.selectedChannel);
  const conversation = $derived(client.selectedConversation);
  const channelId = $derived(channel?.id ?? conversation?.id ?? null);
  const user = $derived(client.user);
  const conversationName = $derived(conversation ? conversationDisplayName(conversation, user?.id ?? null) : "");
  const isGroup = $derived(conversation !== null && isGroupConversation(conversation));
  // Channel overwrites included (read-only channels); only to adapt the interface (ADR-0014).
  const channelPermissions = $derived(
    client.selectedChannelId ? client.myChannelPermissions(client.selectedChannelId) : null,
  );
  // A conversation with someone you blocked stays readable but locked (ADR-0025). Being blocked
  // by them changes nothing here: the instance refuses, and the client never tells. Groups never lock.
  const blockedRecipient = $derived(
    conversation ? blockedRecipientOf(conversation, user?.id ?? null, (userId) => client.isBlocked(userId)) : null,
  );
  const canSendMessages = $derived(
    blockedRecipient === null &&
      (channelPermissions === null || hasPermission(channelPermissions, Permission.SEND_MESSAGES)),
  );
  const canManageMessages = $derived(
    channelPermissions !== null && hasPermission(channelPermissions, Permission.MANAGE_MESSAGES),
  );
  const canReadHistory = $derived(
    channelPermissions === null || hasPermission(channelPermissions, Permission.READ_MESSAGE_HISTORY),
  );
  const canEditChannel = $derived(channel !== null && visibleChannelSections(channel, channelPermissions).length > 0);
  /** The icon that failed to load shows the # glyph instead. */
  let failedIconId = $state<Snowflake | null>(null);
  const channelIcon = $derived(channel?.icon && channel.icon.id !== failedIconId ? channel.icon : null);
  const canAddReactions = $derived(
    canReadHistory && channelPermissions !== null && hasPermission(channelPermissions, Permission.ADD_REACTIONS),
  );
  const canAttachFiles = $derived(
    canSendMessages && (channelPermissions === null || hasPermission(channelPermissions, Permission.ATTACH_FILES)),
  );
  // In conversations everyone who may write there creates and votes in polls (ADR-0043).
  const canCreatePolls = $derived(
    canSendMessages && (channelPermissions === null || hasPermission(channelPermissions, Permission.SEND_POLLS)),
  );
  const canVoteInPolls = $derived(channel ? canReadHistory : canSendMessages);
  const composerLabel = $derived(
    channel
      ? strings.chat.composerInChannel(channel.name)
      : isGroup
        ? strings.chat.composerToGroup(conversationName)
        : strings.chat.composerToPerson(conversationName),
  );

  let pane: HTMLElement;
  let composer = $state<Composer | null>(null);
  let messageList = $state<{
    jumpToPresent: () => void;
    revealMessage: (messageId: Snowflake, shouldHighlight?: boolean) => Promise<void>;
  } | null>(null);
  const drafts = new ComposerDrafts();
  let replyingTo = $state<Message | null>(null);
  const messageEditor = new MessageEditor(editTextAsSent);
  let deletingMessage = $state<Message | null>(null);
  let forwardingMessage = $state<Message | null>(null);
  /** For a message's reactions, or, without one, for the composer. */
  let picker = $state<{ message: Message | null; anchor: HTMLElement } | null>(null);
  /** The GIF picker's button, while it is open (ADR-0042). */
  let gifPickerAnchor = $state<HTMLElement | null>(null);
  let pinning = $state<{ message: Message; isPinned: boolean } | null>(null);
  let removingReactionsOf = $state<Message | null>(null);
  let viewingReactionsOfId = $state<Snowflake | null>(null);
  let isCreatingPoll = $state(false);
  let viewingVotersOf = $state<{ message: Message; answerId: number } | null>(null);
  let endingPollOf = $state<Message | null>(null);
  let isPinsPanelOpen = $state(false);
  let pinsButton = $state<HTMLButtonElement | null>(null);
  const pinsPanelToggle = createPopoverToggle(() => isPinsPanelOpen);
  // Phones (the drawer layout) keep the drawer key, the name, search and members in the header,
  // and fold "Go to…", the pins, "Add friends" and the channel settings into one menu, so the
  // channel's name stays readable at 320 px.
  let moreOptionsButton = $state<HTMLButtonElement | null>(null);
  let moreOptionsMenu = $state<ContextMenu | null>(null);
  let isMoreOptionsOpen = $state(false);
  const moreOptionsToggle = createPopoverToggle(() => isMoreOptionsOpen);
  /** The pins panel opens under its own key, or under "More options" on phones. */
  const pinsPanelAnchor = $derived(hasNavigationDrawer ? moreOptionsButton : pinsButton);
  let actionError = $state("");
  /** Information rather than a failure (a search result too far back): quiet, not red. */
  let notice = $state("");
  let isDropOverlayShown = $state(false);
  // dragenter and dragleave fire for every child crossed: the overlay hides when this is back to 0.
  let fileDragDepth = 0;

  // Custom emoji are offered only in their own server's channels (ADR-0028).
  const serverEmojis = $derived(guild && channel ? client.emojisOf(guild.id) : []);
  // Who "@na" suggests: the server's members, or the conversation's people.
  const mentionCandidates = $derived(
    guild && channel
      ? (client.membersByGuild.get(guild.id) ?? []).map((member) => member.user)
      : (conversation?.recipients ?? []),
  );
  // Roles and @everyone after the people, in a server's channels only (ADR-0040).
  const canMentionEveryone = $derived(
    guild !== null &&
      channel !== null &&
      channelPermissions !== null &&
      hasPermission(channelPermissions, Permission.MENTION_EVERYONE),
  );
  const mentionableRoles = $derived(
    guild && channel ? rolesWriterMayMention(guild.roles, guild.id, canMentionEveryone) : [],
  );
  const pickerServer = $derived(guild && channel ? { name: guild.name, emojis: serverEmojis } : null);

  // Each channel keeps its own unsent text (DESIGN.md §5.5); replies and edits do not carry over.
  $effect.pre(() => {
    if (!drafts.open(channelId)) {
      return;
    }
    replyingTo = null;
    messageEditor.close();
    actionError = "";
    notice = "";
    picker = null;
    gifPickerAnchor = null;
    isPinsPanelOpen = false;
  });

  // Edits are refused where sending is.
  $effect.pre(() => {
    if (!canSendMessages) {
      messageEditor.close();
      replyingTo = null;
    }
  });

  let now = $state(Date.now());
  // Ticks only while someone is typing here, so an expired indicator does not keep a timer alive.
  const isAnyoneTyping = $derived(
    client.selectedChannelId !== null && client.typingUserIds(client.selectedChannelId, now).length > 0,
  );
  $effect(() => {
    if (!isAnyoneTyping) {
      return;
    }
    now = Date.now();
    const timer = setInterval(() => (now = Date.now()), TYPING_REFRESH_INTERVAL_MILLISECONDS);
    return () => clearInterval(timer);
  });

  function nameOfUser(userId: Snowflake): string {
    const people: PublicUser[] = [
      ...(guild ? (client.membersByGuild.get(guild.id) ?? []).map((member) => member.user) : []),
      ...(conversation?.recipients ?? []),
    ];
    const person = people.find((candidate) => candidate.id === userId);
    return person ? displayName(person) : strings.common.someone;
  }

  const typing = $derived(channelId ? typingSegments(client.typingUserIds(channelId, now).map(nameOfUser)) : []);

  // What others wrote before the visit and has not been seen yet, for the jump bar (DESIGN.md §5.4).
  // Phones show only the count, so the bar stays one line beside "Mark as read".
  const unreadBar = $derived.by((): { sentence: string; short: string } | null => {
    const marker = channelId ? client.unreadMarkers.get(channelId) : undefined;
    if (!channelId || !marker) {
      return null;
    }
    const summary = unreadSummary(
      client.messages(channelId),
      marker,
      user?.id ?? null,
      client.reachedStartChannelIds.has(channelId),
    );
    return summary.count === 0
      ? null
      : {
          sentence: unreadBarLabel(summary, unixMillisecondsOfSnowflake(marker.lastReadMessageId), new Date()),
          short: strings.chat.newMessagesShort(summary.count, summary.isPartial),
        };
  });

  function markAsRead() {
    if (channelId) {
      client.dismissUnreadMarker(channelId);
    }
    focusComposer();
  }

  /** Every path that focuses the composer comes here: a read-only channel has none to focus. */
  export function focusComposer() {
    composer?.focus();
  }

  /** A search result once its page is loaded: scrolled to, focused and lit (DESIGN.md 5.12). */
  export async function revealSearchResult(messageId: Snowflake): Promise<void> {
    notice = "";
    await tick();
    await messageList?.revealMessage(messageId, true);
  }

  /** Why a search result could not be shown, said above the composer until the next channel. */
  export function showNotice(text: string) {
    notice = text;
  }

  function maximumComposerHeight(): number {
    return pane ? pane.clientHeight * COMPOSER_MAXIMUM_SHARE_OF_PANE : Number.POSITIVE_INFINITY;
  }

  // Previews put aside with other channels' drafts are freed only when the chat goes away.
  onDestroy(() => drafts.dispose());

  function isDraggingFiles(event: DragEvent): boolean {
    return event.dataTransfer?.types.includes("Files") ?? false;
  }

  function handleDragEnter(event: DragEvent) {
    if (!isDraggingFiles(event)) {
      return;
    }
    event.preventDefault();
    fileDragDepth += 1;
    isDropOverlayShown = canAttachFiles && channelId !== null;
  }

  function handleDragOver(event: DragEvent) {
    if (!isDraggingFiles(event) || !event.dataTransfer) {
      return;
    }
    event.preventDefault();
    event.dataTransfer.dropEffect = canAttachFiles && channelId !== null ? "copy" : "none";
  }

  function handleDragLeave(event: DragEvent) {
    if (!isDraggingFiles(event)) {
      return;
    }
    fileDragDepth = Math.max(fileDragDepth - 1, 0);
    if (fileDragDepth === 0) {
      isDropOverlayShown = false;
    }
  }

  function handleDrop(event: DragEvent) {
    if (!isDraggingFiles(event)) {
      return;
    }
    event.preventDefault();
    fileDragDepth = 0;
    isDropOverlayShown = false;
    composer?.attachFiles(Array.from(event.dataTransfer?.files ?? []));
  }

  // A file dropped anywhere else would make the browser leave Mellow to show it, losing drafts.
  function refuseFileDropOutsideChat(event: DragEvent) {
    if (!isDraggingFiles(event) || event.defaultPrevented || !event.dataTransfer) {
      return;
    }
    event.preventDefault();
    event.dataTransfer.dropEffect = "none";
  }

  function editLastOwnMessage() {
    const lastOwn = channelId
      ? [...client.messages(channelId)]
          .reverse()
          // A forward or a poll cannot be edited (ADR-0039, ADR-0043): Up goes to the last message you wrote.
          .find(
            (message) =>
              message.author.id === user?.id && message.forwarded_from === undefined && message.poll === undefined,
          )
      : undefined;
    if (lastOwn) {
      messageEditor.open(lastOwn);
    }
  }

  /** As on send (Composer's `textAsSent`): typed ":name:" and a role's "@Name" become markup. */
  function editTextAsSent(text: string): string {
    const peopleNames = new Set(mentionCandidates.map((person) => person.username.toLowerCase()));
    return withRoleNamesAsMarkup(withEmojiNamesAsMarkup(text, serverEmojis), mentionableRoles, peopleNames);
  }

  async function saveEdit(message: Message) {
    const text = messageEditor.takeTextToSave();
    if (text === null) {
      return;
    }
    const trimmed = editTextAsSent(text.trim());
    // Saving an empty message asks to delete it; an unchanged one is not sent again (DESIGN.md §5.5).
    if (trimmed === "") {
      deletingMessage = message;
      return;
    }
    focusComposer();
    if (trimmed === message.content) {
      return;
    }
    actionError = "";
    try {
      // The typed text is not lost: the editor reopens with it, or keeps it for the next edit.
      await messageEditor.sendOrGiveBack(
        message,
        text,
        () => client.editMessage(message, trimmed),
        () => ({
          canReopen: message.channel_id === channelId && canSendMessages,
          shouldTakeFocus: !isTypingInAField(document.activeElement),
        }),
      );
    } catch (error) {
      showActionError(message.channel_id, describeError(error, strings.chat.editFailed));
    }
  }

  /** The reader moved on to another message: a reopened editor must not take their keystrokes. */
  function isTypingInAField(element: Element | null): boolean {
    return (element instanceof HTMLTextAreaElement || element instanceof HTMLInputElement) && element.value !== "";
  }

  /** An answer that comes back after the reader moved on is not told under another channel. */
  function showActionError(messageChannelId: Snowflake, text: string) {
    if (messageChannelId === channelId) {
      actionError = text;
    }
  }

  async function deleteMessage(message: Message) {
    actionError = "";
    try {
      await client.deleteMessage(message);
    } catch (error) {
      showActionError(message.channel_id, describeError(error, strings.chat.deleteFailed));
    }
  }

  async function copyText(text: string) {
    actionError = "";
    try {
      await navigator.clipboard.writeText(text);
    } catch (error) {
      console.warn("copying a message failed", error);
      actionError = strings.chat.copyFailed;
    }
  }

  const moreOptionsEntries = $derived.by((): ContextMenuEntry[] =>
    menuEntries(chatHeaderMenuActions({ isGroup, canEditChannel }), (action) => ({
      onselect: () => {
        switch (action) {
          case "go-to":
            onopenquickswitcher();
            break;
          case "pinned-messages":
            isPinsPanelOpen = true;
            break;
          case "add-friends-to-group":
            onaddtogroup();
            break;
          case "channel-settings":
            if (channel) {
              oneditchannel(channel);
            }
            break;
        }
      },
    })),
  );

  function openMoreOptions() {
    if (moreOptionsButton) {
      moreOptionsMenu?.open({ kind: "anchor", bounds: moreOptionsButton.getBoundingClientRect(), align: "end" }, moreOptionsButton);
    }
  }

  const headerButtonClass =
    "size-(--mw-component-chat-header-button-size) text-(--mw-component-chat-header-icon) hover:bg-state-hover hover:text-text-normal";

  async function toggleReaction(message: Message, emoji: string) {
    actionError = "";
    try {
      await client.toggleReaction(message.channel_id, message.id, emoji);
    } catch (error) {
      showActionError(message.channel_id, describeError(error, strings.reactions.failed));
    }
  }

  function pickEmoji(emoji: string | CustomEmoji) {
    const target = picker?.message ?? null;
    if (typeof emoji === "string") {
      noteEmojiUsed(emoji);
    }
    if (target) {
      closePicker(true);
      void toggleReaction(target, typeof emoji === "string" ? emoji : customEmojiReactionKey(emoji));
    } else {
      // Into the composer, at the caret it had: focus goes there rather than back to the button.
      closePicker(false);
      void composer?.insert(composerTextOf(emoji));
    }
  }

  /** Back to the button that opened the picker: its bar is still held open at this point. */
  function closePicker(shouldReturnFocus: boolean) {
    const anchor = picker?.anchor;
    picker = null;
    if (shouldReturnFocus && anchor?.isConnected) {
      anchor.focus();
    }
  }

  async function confirmPin(message: Message, isPinned: boolean) {
    actionError = "";
    try {
      await client.setPinned(message, isPinned);
    } catch (error) {
      showActionError(message.channel_id, describeError(error, isPinned ? strings.pins.pinFailed : strings.pins.unpinFailed));
    }
  }

  async function removeAllReactions(message: Message) {
    actionError = "";
    try {
      await client.removeAllReactions(message.channel_id, message.id);
    } catch (error) {
      showActionError(message.channel_id, describeError(error, strings.reactions.removeAllFailed));
    }
  }

  const reactionsBeingViewed = $derived.by(() => {
    if (!channelId || !viewingReactionsOfId) {
      return null;
    }
    const message =
      client.messages(channelId).find((candidate) => candidate.id === viewingReactionsOfId) ??
      client.pinnedMessages(channelId).find((candidate) => candidate.id === viewingReactionsOfId);
    return message ? { message, reactions: message.reactions ?? [] } : null;
  });

  async function choosePollAnswer(message: Message, answerId: number) {
    actionError = "";
    try {
      await client.choosePollAnswer(message.channel_id, message.id, answerId);
    } catch (error) {
      showActionError(message.channel_id, pollVoteErrorSentence(error, strings.polls.voteFailed));
    }
  }

  async function removeOwnPollVotes(message: Message) {
    actionError = "";
    try {
      await client.removeOwnPollVotes(message.channel_id, message.id);
    } catch (error) {
      showActionError(message.channel_id, pollVoteErrorSentence(error, strings.polls.removeVoteFailed));
    }
  }

  /** The instance refuses votes once a poll has ended, which this client's minute clock may not know yet. */
  function pollVoteErrorSentence(error: unknown, summary: string): string {
    return error instanceof ApiError && error.code === "VALIDATION_FAILED"
      ? strings.polls.hasEnded
      : describeError(error, summary);
  }

  async function endPoll(message: Message) {
    actionError = "";
    try {
      await client.endPoll(message);
    } catch (error) {
      showActionError(message.channel_id, describeError(error, strings.polls.endFailed));
    }
  }

  function sendPoll(poll: NewPoll) {
    isCreatingPoll = false;
    notice = "";
    void client.sendPoll(poll);
    messageList?.jumpToPresent();
  }

  const reactionAndPinActions: ReactionAndPinActions = $derived({
    canJoinReactions: canReadHistory,
    canAddReactions,
    canRemoveReactions: channel !== null && canManageMessages && canReadHistory,
    canManagePins: conversation !== null || canManageMessages,
    heldMessageId: picker?.message?.id ?? null,
    toggleReaction: (message, emoji) => void toggleReaction(message, emoji),
    openPicker: (message, anchor) => (picker = { message, anchor }),
    loadReactors: (message, emoji) => client.listReactingUsers(message.channel_id, message.id, emoji),
    askToPin: (message, isPinned) => (pinning = { message, isPinned }),
    viewReactions: (message) => (viewingReactionsOfId = message.id),
    askToRemoveAllReactions: (message) => (removingReactionsOf = message),
    canVoteInPolls,
    choosePollAnswer: (message, answerId) => void choosePollAnswer(message, answerId),
    removeOwnPollVotes: (message) => void removeOwnPollVotes(message),
    viewPollVoters: (message, answerId) => (viewingVotersOf = { message, answerId }),
    askToEndPoll: (message) => (endingPollOf = message),
  });

  async function startReply(message: Message) {
    replyingTo = message;
    await tick();
    focusComposer();
  }
</script>

<svelte:window ondragover={refuseFileDropOutsideChat} ondrop={refuseFileDropOutsideChat} />

<section
  bind:this={pane}
  aria-label={channel ? `#${channel.name}` : conversation ? conversationName : strings.chat.chat}
  class={[
    "relative flex h-full min-w-0 flex-1 flex-col overflow-hidden bg-surface-chat",
    // Full-bleed on phones: square corners, and messages keep more of the narrow width.
    hasNavigationDrawer ? "[--message-padding-trailing:var(--mw-space-md)]" : "rounded-(--mw-component-sheet-radius)",
  ]}
  ondragenter={handleDragEnter}
  ondragover={handleDragOver}
  ondragleave={handleDragLeave}
  ondrop={handleDrop}
>
  <!-- No line under the header: the sheet's tone and space separate it (ADR-0022 §2). -->
  <header
    class={[
      "flex h-(--mw-component-chat-header-height) shrink-0 items-center gap-xs ps-(--mw-component-chat-header-padding-inline) pe-xs",
      // Phones only: the drawer layout needs the room for the channel name; tablets keep the gaps.
      hasNavigationDrawer && "pointer-coarse:gap-0",
    ]}
  >
    {#if hasNavigationDrawer}
      <IconButton
        icon="list"
        label={strings.chat.openNavigation}
        class={headerButtonClass}
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onclick={onopennavigation}
      />
    {/if}
    {#if channel || conversation}
      <!-- On phones the name takes all the room the keys leave; elsewhere half, beside the topic. -->
      <div
        class={[
          "flex min-w-0 items-center gap-2xs text-(--mw-component-chat-header-name-text)",
          hasNavigationDrawer ? "flex-1" : "max-w-1/2",
        ]}
      >
        {#if channelIcon}
          {@const iconId = channelIcon.id}
          <img
            src={iconUrl(channelIcon, false)}
            alt=""
            draggable="false"
            class="size-(--mw-component-chat-header-icon-size) shrink-0 rounded-(--mw-component-channel-row-image-radius) object-cover"
            onerror={() => (failedIconId = iconId)}
          />
        {:else if isGroup && conversation}
          <GroupAvatar
            people={otherParticipants(conversation, user?.id ?? null)}
            sizeToken="--mw-component-member-row-avatar-size"
            ringColorToken="--mw-color-surface-chat"
          />
        {:else}
          <Icon
            name={channel ? (canSendMessages ? "hash" : "lock") : "at"}
            class="size-(--mw-component-chat-header-icon-size) text-text-muted"
          />
        {/if}
        <h1 class="min-w-0 truncate type-heading-md">{channel ? channel.name : conversationName}</h1>
        {#if channelIcon && !canSendMessages}
          <Icon name="lock" class="size-(--mw-component-channel-row-trailing-icon-size) shrink-0 text-text-muted" />
        {/if}
      </div>
    {/if}
    {#if channel?.topic && !hasNavigationDrawer}
      <span aria-hidden="true" class="mx-xs h-(--mw-component-chat-header-divider-height) w-px shrink-0 bg-border-divider"></span>
      <p class="min-w-0 flex-1 truncate type-body text-(--mw-component-chat-header-topic-text)" title={channel.topic}>
        {channel.topic}
      </p>
    {:else if isGroup && conversation && !hasNavigationDrawer}
      <!-- A group has no topic: its slot counts the people in it. -->
      <span aria-hidden="true" class="mx-xs h-(--mw-component-chat-header-divider-height) w-px shrink-0 bg-border-divider"></span>
      <p class="min-w-0 flex-1 truncate type-body text-(--mw-component-group-avatar-member-count-text)">
        {strings.groups.memberCount(participantCount(conversation, user?.id ?? null))}
      </p>
    {:else if !hasNavigationDrawer || !(channel || conversation)}
      <span class="flex-1"></span>
    {/if}
    <!-- Where Discord has it; Ctrl+K stays the quick switcher, offered again in its suggestions. -->
    <!-- "Go to" folds into "More options" only where that key exists. -->
    {@render searchControl(hasNavigationDrawer && Boolean(channel || conversation))}
    <!-- Phones fold these keys into "More options", after the member toggle. -->
    {#if channel && canEditChannel && !hasNavigationDrawer}
      <IconButton
        icon="gear-six"
        label={strings.chat.channelSettings}
        aria-haspopup="dialog"
        class={headerButtonClass}
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onclick={() => oneditchannel(channel)}
      />
    {/if}
    {#if (channel || conversation) && !hasNavigationDrawer}
      <IconButton
        bind:element={pinsButton}
        icon="push-pin"
        label={strings.pins.pinnedMessages}
        aria-haspopup="dialog"
        aria-expanded={isPinsPanelOpen}
        class="{headerButtonClass} aria-expanded:text-text-heading"
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onpointerdown={pinsPanelToggle.notePress}
        onclick={() => (isPinsPanelOpen = pinsPanelToggle.shouldOpenOnClick())}
      />
    {/if}
    {#if isGroup && !hasNavigationDrawer}
      <!-- Discord's "Add friends to DM" in the header; the members sheet has the same action. -->
      <IconButton
        icon="user-plus"
        label={strings.groups.addFriendsToGroup}
        aria-haspopup="dialog"
        class={headerButtonClass}
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onclick={onaddtogroup}
      />
    {/if}
    {#if channel || isGroup}
      <!-- The member toggle keeps its name; aria-pressed says whether the side sheet is open. -->
      <IconButton
        icon="users"
        label={strings.chat.memberList}
        aria-pressed={isMemberListOpen}
        aria-keyshortcuts="Control+U"
        class="{headerButtonClass} aria-pressed:text-text-heading"
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onclick={ontogglememberlist}
      />
    {/if}
    {#if (channel || conversation) && hasNavigationDrawer}
      <IconButton
        bind:element={moreOptionsButton}
        icon="dots-three"
        label={strings.chat.moreOptions}
        aria-haspopup="menu"
        aria-expanded={isMoreOptionsOpen}
        aria-controls="chat-more-options"
        class="{headerButtonClass} aria-expanded:text-text-heading"
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onpointerdown={moreOptionsToggle.notePress}
        onclick={() => (moreOptionsToggle.shouldOpenOnClick() ? openMoreOptions() : moreOptionsMenu?.close())}
      />
    {/if}
  </header>

  {#if hasNavigationDrawer}
    <ContextMenu
      bind:this={moreOptionsMenu}
      bind:isOpen={isMoreOptionsOpen}
      id="chat-more-options"
      label={strings.chat.moreOptions}
      entries={moreOptionsEntries}
      gapToken="space-2xs"
    />
  {/if}

  {#if unreadBar}
    <!-- Over the list, under the header, so the rows being read do not move. -->
    <div
      class="unread-bar absolute inset-x-md top-[calc(var(--mw-component-chat-header-height)_+_var(--mw-space-2xs))] z-10 flex min-h-(--mw-component-jump-bar-height) items-center justify-between gap-xs rounded-(--mw-component-jump-bar-radius) bg-(--mw-component-jump-bar-background) ps-sm pe-2xs type-body font-semibold text-(--mw-component-jump-bar-text) pointer-coarse:min-h-(--mw-size-hit-web-min)"
    >
      {#if hasNavigationDrawer}
        <span class="sr-only">{unreadBar.sentence}</span>
        <span aria-hidden="true" class="min-w-0 truncate">{unreadBar.short}</span>
      {:else}
        <span class="min-w-0">{unreadBar.sentence}</span>
      {/if}
      <button
        type="button"
        aria-keyshortcuts="Escape"
        class="h-[calc(var(--mw-component-jump-bar-height)_-_2*var(--mw-space-3xs))] shrink-0 rounded-sm px-xs font-semibold whitespace-nowrap transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:bg-(--mw-component-jump-bar-background-hover) focus-visible:[outline-color:var(--mw-component-jump-bar-text)] pointer-coarse:h-(--mw-size-hit-web-min)"
        onclick={markAsRead}>{strings.chat.markAsRead}</button
      >
    </div>
  {/if}

  {#if channelId && user}
    {#key channelId}
      <MessageList
        bind:this={messageList}
        {client}
        {channelId}
        currentUser={user}
        channelLabel={channel ? `#${channel.name}` : conversationName}
        heroIcon={channel ? "hash" : isGroup ? "users" : "at"}
        heroTitle={channel ? strings.chat.welcomeToChannel(channel.name) : conversationName}
        heroSentence={channel
          ? strings.chat.channelStart(channel.name)
          : isGroup && conversation
            ? strings.chat.groupStart(participantNamesOf(conversation, user.id))
            : strings.chat.conversationStart(conversationName)}
        topic={channel?.topic ?? ""}
        {canSendMessages}
        {canManageMessages}
        {reactionAndPinActions}
        {messageEditor}
        maximumEditorHeight={maximumComposerHeight}
        onreply={startReply}
        onforward={(message) => (forwardingMessage = message)}
        forwardOriginPlaceOf={(originChannelId) =>
          forwardOriginPlace(originChannelId, client.guilds, client.conversations, user.id)?.label ?? null}
        {onopenforwardorigin}
        onstartedit={(message) => messageEditor.open(message)}
        onsaveedit={saveEdit}
        oncanceledit={() => {
          messageEditor.close();
          focusComposer();
        }}
        ondelete={(message, skipConfirmation) =>
          skipConfirmation ? void deleteMessage(message) : (deletingMessage = message)}
        oncopy={copyText}
        onjumptopresent={focusComposer}
        onopenedconversation={() => void tick().then(focusComposer)}
      />
    {/key}

    <Composer
      bind:this={composer}
      bind:actionError
      bind:replyingTo
      {client}
      {drafts}
      {serverEmojis}
      {mentionCandidates}
      {mentionableRoles}
      {canMentionEveryone}
      currentUserId={user?.id ?? null}
      label={composerLabel}
      {canSendMessages}
      {canAttachFiles}
      {blockedRecipient}
      {hasNavigationDrawer}
      {typing}
      {notice}
      isEmojiPickerOpen={picker !== null && picker.message === null}
      canChooseGifs={client.gifSettings !== null}
      {canCreatePolls}
      isGifPickerOpen={gifPickerAnchor !== null}
      hasUnreadBar={unreadBar !== null}
      maximumHeight={maximumComposerHeight}
      onopenemojipicker={(anchor) => (picker = { message: null, anchor })}
      onopengifpicker={(anchor) => (gifPickerAnchor = anchor)}
      onopenpolldialog={() => (isCreatingPoll = true)}
      onmarkasread={markAsRead}
      oneditlastmessage={editLastOwnMessage}
      onsent={() => {
        // Writing again moves on from what the notice said (a forward sent, a result not found).
        notice = "";
        messageList?.jumpToPresent();
      }}
    />
  {:else}
    {@render emptyState()}
  {/if}

  {#if isDropOverlayShown}
    <!-- Pointer feedback only: it never takes focus, and the attach button does the same job. -->
    <div
      aria-hidden="true"
      class="pointer-events-none absolute inset-0 z-20 grid place-items-center bg-(--mw-component-drop-overlay-scrim) p-md"
    >
      <div
        class="relative flex w-full max-w-(--mw-component-drop-overlay-max-width) flex-col items-center gap-(--mw-component-drop-overlay-gap) rounded-(--mw-component-drop-overlay-radius) bg-(--mw-component-drop-overlay-background) p-(--mw-component-drop-overlay-padding) text-center text-(--mw-component-drop-overlay-text)"
      >
        <span
          class="absolute inset-(--mw-component-drop-overlay-border-inset) rounded-(--mw-component-drop-overlay-radius) border-(length:--mw-component-drop-overlay-border-width) border-dashed border-current"
        ></span>
        <Icon name="file-arrow-up" class="size-(--mw-component-drop-overlay-icon-size)" />
        <p class="type-heading-xl break-words">
          {strings.chat.dropToAttach(channel ? `#${channel.name}` : isGroup ? conversationName : `@${conversationName}`)}
        </p>
        <p class="type-body">{strings.chat.dropHint}</p>
      </div>
    </div>
  {/if}
</section>

{#snippet emptyState()}
  <div class="flex flex-1 flex-col items-center justify-center gap-xs p-xl text-center">
    <!-- Home without a conversation is the Friends space (Shell), so only servers come here. -->
    {#if guild && client.status !== "connecting"}
      <h2 class="type-heading-lg text-text-heading">{strings.chat.noVisibleChannelsTitle}</h2>
      <p class="max-w-(--mw-component-auth-card-max-width) type-body text-text-muted">
        {strings.chat.noVisibleChannelsBody(guild.name)}
      </p>
    {:else}
      <p role="status" class="type-body text-text-muted">{strings.chat.loadingWorkspace}</p>
    {/if}
  </div>
{/snippet}

{#if picker}
  <EmojiPicker
    anchor={picker.anchor}
    isCompact={hasNavigationDrawer}
    server={pickerServer}
    onpick={pickEmoji}
    onclose={closePicker}
  />
{/if}

{#if gifPickerAnchor && client.gifSettings}
  {@const anchor = gifPickerAnchor}
  {@const settings = client.gifSettings}
  <LazyComponent load={() => import("./GifPicker.svelte")} onclose={() => (gifPickerAnchor = null)}>
    {#snippet children({ default: GifPicker })}
      <GifPicker
        {anchor}
        isCompact={hasNavigationDrawer}
        {settings}
        rest={client.rest}
        onpick={(gif, query) => {
          gifPickerAnchor = null;
          composer?.sendGif(gif.messageUrl);
          triggerKlipyShare(settings, gif.slug, query);
          focusComposer();
        }}
        onclose={(shouldReturnFocus) => {
          gifPickerAnchor = null;
          if (shouldReturnFocus && anchor.isConnected) {
            anchor.focus();
          }
        }}
      />
    {/snippet}
  </LazyComponent>
{/if}

{#if isPinsPanelOpen && channelId && pinsPanelAnchor}
  <PinnedMessagesPanel
    {client}
    {channelId}
    anchor={pinsPanelAnchor}
    isCompact={hasNavigationDrawer}
    canManagePins={reactionAndPinActions.canManagePins}
    onjump={(messageId) => void messageList?.revealMessage(messageId)}
    onunpin={(message) => (pinning = { message, isPinned: false })}
    onclose={(shouldReturnFocus) => {
      isPinsPanelOpen = false;
      if (shouldReturnFocus) {
        pinsPanelAnchor?.focus();
      }
    }}
  />
{/if}

{#if pinning}
  {@const { message, isPinned } = pinning}
  <ConfirmMessageDialog
    roles={client.knownRoles}
    {message}
    title={isPinned ? strings.pins.pinTitle : strings.pins.unpinTitle}
    subtitle={isPinned
      ? strings.pins.pinSubtitle(channel ? `#${channel.name}` : conversationName)
      : strings.pins.unpinSubtitle}
    submitLabel={isPinned ? strings.pins.pinSubmit : strings.pins.unpinSubmit}
    isDanger={!isPinned}
    onclose={() => (pinning = null)}
    onconfirm={() => {
      // Read before clearing: `message` and `isPinned` follow `pinning` and would read null after.
      const [target, shouldPin] = [message, isPinned];
      pinning = null;
      void confirmPin(target, shouldPin);
    }}
    fallbackFocus={focusComposer}
  />
{/if}

{#if removingReactionsOf}
  {@const message = removingReactionsOf}
  <ConfirmMessageDialog
    roles={client.knownRoles}
    {message}
    title={strings.reactions.removeAllTitle}
    subtitle={strings.reactions.removeAllSubtitle}
    submitLabel={strings.reactions.removeAllSubmit}
    isDanger
    onclose={() => (removingReactionsOf = null)}
    onconfirm={() => {
      // Read before clearing: `message` follows `removingReactionsOf` and would read null after.
      const target = message;
      removingReactionsOf = null;
      void removeAllReactions(target);
    }}
    fallbackFocus={focusComposer}
  />
{/if}

{#if reactionsBeingViewed}
  {@const message = reactionsBeingViewed.message}
  <ReactionsDialog
    reactions={reactionsBeingViewed.reactions}
    customEmojis={client.knownCustomEmojis}
    canRemoveOthers={reactionAndPinActions.canRemoveReactions}
    loadUsers={(emoji) => client.listReactingUsers(message.channel_id, message.id, emoji)}
    onremove={(emoji, userId) => client.removeUserReaction(message.channel_id, message.id, emoji, userId)}
    onclose={() => (viewingReactionsOfId = null)}
    fallbackFocus={focusComposer}
  />
{/if}

{#if isCreatingPoll}
  <LazyComponent load={() => import("./CreatePollDialog.svelte")} onclose={() => (isCreatingPoll = false)}>
    {#snippet children({ default: CreatePollDialog })}
      <CreatePollDialog
        isConversation={conversation !== null}
        onsend={sendPoll}
        onclose={() => (isCreatingPoll = false)}
        fallbackFocus={focusComposer}
      />
    {/snippet}
  </LazyComponent>
{/if}

{#if viewingVotersOf}
  {@const { message, answerId } = viewingVotersOf}
  <LazyComponent load={() => import("./PollVotersDialog.svelte")} onclose={() => (viewingVotersOf = null)}>
    {#snippet children({ default: PollVotersDialog })}
      <PollVotersDialog
        answerText={message.poll?.answers.find((answer) => answer.id === answerId)?.text ?? ""}
        loadVoters={(after) => client.listPollVoters(message.channel_id, message.id, answerId, after)}
        onclose={() => (viewingVotersOf = null)}
        fallbackFocus={focusComposer}
      />
    {/snippet}
  </LazyComponent>
{/if}

{#if endingPollOf}
  {@const message = endingPollOf}
  <ConfirmMessageDialog
    roles={client.knownRoles}
    {message}
    title={strings.polls.endTitle}
    subtitle={strings.polls.endSubtitle}
    submitLabel={strings.polls.end}
    isDanger
    onclose={() => (endingPollOf = null)}
    onconfirm={() => {
      // Read before clearing: `message` follows `endingPollOf` and would read null after.
      const target = message;
      endingPollOf = null;
      void endPoll(target);
    }}
    fallbackFocus={focusComposer}
  />
{/if}

{#if forwardingMessage}
  {@const message = forwardingMessage}
  <LazyComponent load={() => import("./ForwardMessageDialog.svelte")} onclose={() => (forwardingMessage = null)}>
    {#snippet children({ default: ForwardMessageDialog })}
      <ForwardMessageDialog
        {client}
        {message}
        canFocusComposer={canSendMessages}
        onclose={() => (forwardingMessage = null)}
        onforwarded={(sentence) => (notice = sentence)}
        fallbackFocus={focusComposer}
      />
    {/snippet}
  </LazyComponent>
{/if}

{#if deletingMessage}
  {@const message = deletingMessage}
  <DeleteMessageDialog
    roles={client.knownRoles}
    {message}
    onclose={() => (deletingMessage = null)}
    onconfirm={() => {
      // Read before clearing: `message` follows `deletingMessage` and would read null after.
      const target = message;
      deletingMessage = null;
      void deleteMessage(target);
    }}
    fallbackFocus={focusComposer}
  />
{/if}

<style>
  .unread-bar {
    animation: unread-bar-in var(--mw-motion-duration-base) var(--mw-motion-easing-enter) both;
  }
  @keyframes unread-bar-in {
    from {
      opacity: 0;
      transform: translateY(calc(-1 * var(--mw-component-channel-switch-offset)));
    }
  }
</style>
