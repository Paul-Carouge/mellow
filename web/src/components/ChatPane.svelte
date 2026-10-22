<script lang="ts">
  import { onDestroy, tick } from "svelte";

  import { ApiError } from "../lib/api";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import {
    type Message,
    PREVIEWABLE_IMAGE_TYPES,
    Permission,
    type PublicUser,
    type Snowflake,
    compareSnowflakes,
    hasPermission,
  } from "../lib/protocol";
  import AttachmentTray, { type AttachmentTrayItem } from "./AttachmentTray.svelte";
  import { autosize } from "./autosize";
  import Button from "./Button.svelte";
  import DeleteMessageDialog from "./DeleteMessageDialog.svelte";
  import { displayName, humanFileSize, timeOfDayLabel, typingLabel, unixMillisecondsOfSnowflake } from "./format";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import MessageList from "./MessageList.svelte";

  // The chat pane (DESIGN.md §5.4, §5.5): header, message list, composer and typing band.
  interface Props {
    client: MellowClient;
    /** Below the sidebar breakpoint the header opens the navigation drawer. */
    hasNavigationDrawer: boolean;
    onopennavigation: () => void;
    onopenquickswitcher: () => void;
    isMemberListOpen: boolean;
    ontogglememberlist: () => void;
    onaddserver: () => void;
  }

  let {
    client,
    hasNavigationDrawer,
    onopennavigation,
    onopenquickswitcher,
    isMemberListOpen,
    ontogglememberlist,
    onaddserver,
  }: Props = $props();

  /** The composer grows with its text up to half of the pane (DESIGN.md §4.2). */
  const COMPOSER_MAXIMUM_SHARE_OF_PANE = 0.5;
  const TYPING_REFRESH_INTERVAL_MILLISECONDS = 1_000;
  /** The instance links at most this many files to one message (ADR-0020). */
  const MAXIMUM_ATTACHMENTS_PER_MESSAGE = 10;
  const TOO_MANY_ATTACHMENTS_SENTENCE = `You can attach up to ${MAXIMUM_ATTACHMENTS_PER_MESSAGE} files to one message.`;
  const UPLOADS_RUNNING_SENTENCE = "Your files are still uploading. Send again once they're done.";

  const guild = $derived(client.selectedGuild);
  const channel = $derived(client.selectedChannel);
  const conversation = $derived(client.selectedConversation);
  const channelId = $derived(channel?.id ?? conversation?.id ?? null);
  const user = $derived(client.user);
  const conversationName = $derived(
    conversation
      ? conversation.recipients
          .filter((candidate) => candidate.id !== user?.id)
          .map(displayName)
          .join(", ")
      : "",
  );
  // Channel overwrites included (read-only channels); only to adapt the interface (ADR-0014).
  const channelPermissions = $derived(
    client.selectedChannelId ? client.myChannelPermissions(client.selectedChannelId) : null,
  );
  const canSendMessages = $derived(
    channelPermissions === null || hasPermission(channelPermissions, Permission.SEND_MESSAGES),
  );
  const canManageMessages = $derived(
    channelPermissions !== null && hasPermission(channelPermissions, Permission.MANAGE_MESSAGES),
  );
  const canAttachFiles = $derived(
    canSendMessages && (channelPermissions === null || hasPermission(channelPermissions, Permission.ATTACH_FILES)),
  );
  const composerLabel = $derived(channel ? `Message #${channel.name}` : `Message @${conversationName}`);

  let pane: HTMLElement;
  let composer = $state<HTMLTextAreaElement | null>(null);
  let messageList = $state<{ jumpToPresent: () => void } | null>(null);
  const draftsByChannel = new Map<Snowflake, string>();
  let draft = $state("");
  let replyingTo = $state<Message | null>(null);
  let editingMessageId = $state<Snowflake | null>(null);
  let deletingMessage = $state<Message | null>(null);
  let actionError = $state("");
  let fileInput = $state<HTMLInputElement | null>(null);
  // Each channel keeps its own tray, like its draft: its files were uploaded to that channel.
  const trayItemsByChannel = new Map<Snowflake, AttachmentTrayItem[]>();
  let trayItems = $state<AttachmentTrayItem[]>([]);
  let nextTrayItemKey = 0;
  let uploadAnnouncement = $state("");
  let isDropOverlayShown = $state(false);
  // dragenter and dragleave fire for every child crossed: the overlay hides when this is back to 0.
  let fileDragDepth = 0;

  const isUploading = $derived(trayItems.some((item) => item.status === "uploading"));
  const uploadedAttachments = $derived(
    trayItems.flatMap((item) => (item.status === "uploaded" && item.attachment ? [item.attachment] : [])),
  );
  const canSend = $derived((draft.trim() !== "" || uploadedAttachments.length > 0) && !isUploading);

  // Each channel keeps its own unsent text (DESIGN.md §5.5); replies and edits do not carry over.
  let shownChannelId: Snowflake | null = null;
  $effect.pre(() => {
    if (channelId === shownChannelId) {
      return;
    }
    if (shownChannelId) {
      draftsByChannel.set(shownChannelId, draft);
      trayItemsByChannel.set(shownChannelId, trayItems);
    }
    shownChannelId = channelId;
    draft = (channelId && draftsByChannel.get(channelId)) || "";
    trayItems = (channelId && trayItemsByChannel.get(channelId)) || [];
    replyingTo = null;
    editingMessageId = null;
    actionError = "";
  });

  // Edits are refused where sending is.
  $effect.pre(() => {
    if (!canSendMessages) {
      editingMessageId = null;
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
    return person ? displayName(person) : "Someone";
  }

  const typingText = $derived(channelId ? typingLabel(client.typingUserIds(channelId, now).map(nameOfUser)) : "");

  // What others wrote since the last visit, for the jump bar (DESIGN.md §5.4).
  const unreadSinceMarker = $derived.by(() => {
    const marker = channelId ? client.unreadMarkers.get(channelId) : undefined;
    return channelId && marker
      ? client
          .messages(channelId)
          .filter((message) => compareSnowflakes(message.id, marker) > 0 && message.author.id !== user?.id)
      : [];
  });
  const unreadBarText = $derived.by(() => {
    const first = unreadSinceMarker[0];
    if (!first) {
      return "";
    }
    const count = unreadSinceMarker.length;
    return `${count} new ${count === 1 ? "message" : "messages"} since ${timeOfDayLabel(unixMillisecondsOfSnowflake(first.id))}`;
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

  function maximumComposerHeight(): number {
    return pane ? pane.clientHeight * COMPOSER_MAXIMUM_SHARE_OF_PANE : Number.POSITIVE_INFINITY;
  }

  function send() {
    if (isUploading) {
      actionError = UPLOADS_RUNNING_SENTENCE;
      return;
    }
    if (!canSend) {
      return;
    }
    void client.sendMessage(draft.trim(), replyingTo?.id ?? null, $state.snapshot(uploadedAttachments));
    draft = "";
    replyingTo = null;
    removeAllTrayItems();
    if (actionError === UPLOADS_RUNNING_SENTENCE || actionError === TOO_MANY_ATTACHMENTS_SENTENCE) {
      actionError = "";
    }
    messageList?.jumpToPresent();
  }

  /** Each file uploads at once to the open channel and waits in the tray for Send (DESIGN.md §5.5). */
  function attachFiles(files: readonly File[]) {
    if (!canAttachFiles || files.length === 0) {
      return;
    }
    const room = Math.max(MAXIMUM_ATTACHMENTS_PER_MESSAGE - trayItems.length, 0);
    if (files.length > room) {
      actionError = TOO_MANY_ATTACHMENTS_SENTENCE;
    }
    for (const file of files.slice(0, room)) {
      const key = nextTrayItemKey++;
      trayItems.push({
        key,
        file,
        previewUrl: PREVIEWABLE_IMAGE_TYPES.includes(file.type) ? URL.createObjectURL(file) : null,
        status: "uploading",
        attachment: null,
        error: "",
      });
      // The reactive copy: updates to it reach the tray, even after a channel switch put it aside.
      const item = trayItems.find((candidate) => candidate.key === key);
      if (item) {
        void upload(item);
      }
    }
  }

  async function upload(item: AttachmentTrayItem) {
    try {
      item.attachment = await client.uploadAttachment(item.file);
      item.status = "uploaded";
      announceUpload(item, `${item.file.name} uploaded.`);
    } catch (error) {
      item.status = "failed";
      item.error = uploadErrorSentence(item.file, error);
      announceUpload(item, item.error);
    }
  }

  /** Only for files still in the open channel's tray: removed ones and other channels' stay quiet. */
  function announceUpload(item: AttachmentTrayItem, sentence: string) {
    if (trayItems.includes(item)) {
      uploadAnnouncement = sentence;
    }
  }

  function uploadErrorSentence(file: File, error: unknown): string {
    // The instance does not tell its limit, so the sentence names the size and the instance (DESIGN.md §8).
    if (error instanceof ApiError && error.code === "FILE_TOO_LARGE") {
      return `${file.name} is ${humanFileSize(file.size)}, more than ${window.location.host} accepts.`;
    }
    return describeError(error, `Couldn't upload ${file.name}`);
  }

  function removeTrayItem(key: number) {
    const item = trayItems.find((candidate) => candidate.key === key);
    if (item?.previewUrl) {
      URL.revokeObjectURL(item.previewUrl);
    }
    trayItems = trayItems.filter((candidate) => candidate.key !== key);
    // The Remove button is gone with its item.
    focusComposer();
  }

  function revokePreviews(items: readonly AttachmentTrayItem[]) {
    for (const item of items) {
      if (item.previewUrl) {
        URL.revokeObjectURL(item.previewUrl);
      }
    }
  }

  function removeAllTrayItems() {
    revokePreviews(trayItems);
    trayItems = [];
  }

  onDestroy(() => {
    revokePreviews(trayItems);
    for (const items of trayItemsByChannel.values()) {
      revokePreviews(items);
    }
  });

  function handleComposerPaste(event: ClipboardEvent) {
    const files = Array.from(event.clipboardData?.files ?? []);
    if (files.length === 0 || !canAttachFiles) {
      return;
    }
    event.preventDefault();
    attachFiles(files);
  }

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
    attachFiles(Array.from(event.dataTransfer?.files ?? []));
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
      ? [...client.messages(channelId)].reverse().find((message) => message.author.id === user?.id)
      : undefined;
    if (lastOwn) {
      editingMessageId = lastOwn.id;
    }
  }

  function handleComposerKeydown(event: KeyboardEvent) {
    if (event.isComposing) {
      return;
    }
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      send();
    } else if (event.key === "Escape" && replyingTo) {
      event.preventDefault();
      replyingTo = null;
    } else if (event.key === "Escape" && unreadSinceMarker.length > 0) {
      event.preventDefault();
      markAsRead();
    } else if (event.key === "ArrowUp" && draft === "" && canSendMessages) {
      event.preventDefault();
      editLastOwnMessage();
    }
  }

  async function saveEdit(message: Message, text: string) {
    editingMessageId = null;
    const trimmed = text.trim();
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
      await client.editMessage(message.id, trimmed);
    } catch (error) {
      actionError = describeError(error, "Couldn't edit the message");
    }
  }

  async function deleteMessage(message: Message) {
    actionError = "";
    try {
      await client.deleteMessage(message.id);
    } catch (error) {
      actionError = describeError(error, "Couldn't delete the message");
    }
  }

  async function copyText(text: string) {
    actionError = "";
    try {
      await navigator.clipboard.writeText(text);
    } catch (error) {
      console.warn("copying a message failed", error);
      actionError = "Couldn't copy the text. Select it and copy it by hand.";
    }
  }

  const headerButtonClass =
    "size-(--mw-component-chat-header-button-size) text-(--mw-component-chat-header-icon) hover:bg-state-hover hover:text-text-normal";
  // The composer's buttons sit centred on its base row, which is as tall as the user card.
  const composerButtonClass =
    "my-[calc((var(--mw-size-dock-height)_-_var(--mw-component-composer-attach-button-size))/2)] size-(--mw-component-composer-attach-button-size) text-(--mw-component-composer-icon) hover:bg-state-hover hover:text-(--mw-component-composer-icon-hover)";

  async function startReply(message: Message) {
    replyingTo = message;
    await tick();
    focusComposer();
  }
</script>

<svelte:window ondragover={refuseFileDropOutsideChat} ondrop={refuseFileDropOutsideChat} />

<section
  bind:this={pane}
  aria-label={channel ? `#${channel.name}` : conversation ? conversationName : "Chat"}
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
    class="flex h-(--mw-component-chat-header-height) shrink-0 items-center gap-xs ps-(--mw-component-chat-header-padding-inline) pe-xs"
  >
    {#if hasNavigationDrawer}
      <IconButton
        icon="list"
        label="Open servers and channels"
        class={headerButtonClass}
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onclick={onopennavigation}
      />
    {/if}
    {#if channel || conversation}
      <div class="flex max-w-1/2 min-w-0 shrink-0 items-center gap-2xs text-(--mw-component-chat-header-name-text)">
        <Icon
          name={channel ? (canSendMessages ? "hash" : "lock") : "at"}
          class="size-(--mw-component-chat-header-icon-size) text-text-muted"
        />
        <h1 class="min-w-0 truncate type-heading-md">{channel ? channel.name : conversationName}</h1>
      </div>
    {/if}
    {#if channel?.topic && !hasNavigationDrawer}
      <span aria-hidden="true" class="mx-xs h-(--mw-component-chat-header-divider-height) w-px shrink-0 bg-border-divider"></span>
      <p class="min-w-0 flex-1 truncate type-body text-(--mw-component-chat-header-topic-text)" title={channel.topic}>
        {channel.topic}
      </p>
    {:else}
      <span class="flex-1"></span>
    {/if}
    {#if !hasNavigationDrawer}
      <!-- The quick switcher, visible: Mellow's fastest way around (ADR-0022 §1). -->
      <button
        type="button"
        aria-label="Go to a channel, server or person"
        aria-keyshortcuts="Control+K"
        class="flex h-(--mw-component-chat-header-button-size) shrink-0 items-center gap-xs rounded-sm bg-surface-input ps-sm pe-2xs type-body text-text-secondary transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:text-text-heading pointer-coarse:h-(--mw-size-hit-web-min)"
        onclick={onopenquickswitcher}
      >
        Go to…
        <kbd class="rounded-xs border border-border-divider px-2xs font-sans type-body-sm text-text-muted">Ctrl K</kbd>
      </button>
    {/if}
    {#if channel}
      <!-- The member toggle keeps its name; aria-pressed says whether the side sheet is open. -->
      <IconButton
        icon="users"
        label="Member list"
        aria-pressed={isMemberListOpen}
        aria-keyshortcuts="Control+U"
        class="{headerButtonClass} aria-pressed:text-text-heading"
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onclick={ontogglememberlist}
      />
    {/if}
  </header>

  {#if unreadBarText}
    <!-- Over the list, under the header, so the rows being read do not move. -->
    <div
      class="unread-bar absolute inset-x-md top-[calc(var(--mw-component-chat-header-height)_+_var(--mw-space-2xs))] z-10 flex h-(--mw-component-jump-bar-height) items-center justify-between gap-xs rounded-(--mw-component-jump-bar-radius) bg-(--mw-component-jump-bar-background) ps-sm pe-2xs type-body font-semibold text-(--mw-component-jump-bar-text) pointer-coarse:h-(--mw-size-hit-web-min)"
    >
      <span class="min-w-0 truncate">{unreadBarText}</span>
      <button
        type="button"
        aria-keyshortcuts="Escape"
        class="h-[calc(var(--mw-component-jump-bar-height)_-_2*var(--mw-space-3xs))] shrink-0 rounded-sm px-xs font-semibold whitespace-nowrap transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:bg-(--mw-component-jump-bar-background-hover) focus-visible:[outline-color:var(--mw-component-jump-bar-text)] pointer-coarse:h-(--mw-size-hit-web-min)"
        onclick={markAsRead}>Mark as read</button
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
        heroIcon={channel ? "hash" : "at"}
        heroTitle={channel ? `Welcome to #${channel.name}` : conversationName}
        heroSentence={channel
          ? `This is the start of the #${channel.name} channel.`
          : `This is the beginning of your direct message history with ${conversationName}.`}
        topic={channel?.topic ?? ""}
        {canSendMessages}
        {canManageMessages}
        {editingMessageId}
        maximumEditorHeight={maximumComposerHeight}
        onreply={startReply}
        onstartedit={(message) => (editingMessageId = message.id)}
        onsaveedit={saveEdit}
        oncanceledit={() => {
          editingMessageId = null;
          focusComposer();
        }}
        ondelete={(message, skipConfirmation) =>
          skipConfirmation ? void deleteMessage(message) : (deletingMessage = message)}
        oncopy={copyText}
        onjumptopresent={focusComposer}
      />
    {/key}

    <div role="alert" class="contents">
      {#if actionError}
        <p class="px-(--mw-component-composer-margin-inline) py-(--mw-component-composer-error-padding-block) type-body text-(--mw-component-composer-error-text)">
          {actionError}
        </p>
      {/if}
    </div>

    <!-- The composer zone (ADR-0022 §4): the typing row above, then the composer, whose base row is
         as tall as the user card and as far from the bottom; it grows upward. -->
    <div class="shrink-0 px-(--mw-component-composer-margin-inline) pb-(--mw-size-dock-margin)">
      <p
        aria-hidden="true"
        class="flex h-(--mw-component-composer-typing-height) min-w-0 items-center gap-2xs ps-xs type-typing text-(--mw-component-composer-typing-text)"
      >
        {#if typingText}
          <span class="typing-dots inline-flex shrink-0 gap-(--mw-component-composer-typing-dot-gap)">
            <i></i><i></i><i></i>
          </span>
          <span class="truncate">{typingText}</span>
        {/if}
      </p>
      <div
        class="overflow-hidden rounded-(--mw-component-composer-radius) bg-(--mw-component-composer-background) has-[textarea:focus-visible]:focus-ring"
      >
        {#if replyingTo && canSendMessages}
          <div
            class="reply-bar flex min-h-(--mw-component-composer-reply-bar-min-height) items-center gap-xs bg-(--mw-component-composer-reply-bar-background) ps-(--mw-component-composer-reply-bar-padding-inline) pe-2xs type-body text-(--mw-component-composer-reply-bar-text)"
          >
            <p class="min-w-0 flex-1 truncate">
              Replying to <span class="font-semibold text-(--mw-component-composer-reply-bar-author-text)"
                >{displayName(replyingTo.author)}</span
              >
            </p>
            <IconButton
              icon="x-circle"
              label="Cancel reply"
              class="size-(--mw-component-composer-reply-bar-button-size) text-(--mw-component-composer-reply-bar-icon) hover:bg-(--mw-component-composer-reply-bar-button-background-hover) hover:text-(--mw-component-composer-icon-hover)"
              iconClass="size-(--mw-component-composer-reply-bar-icon-size)"
              onclick={() => {
                replyingTo = null;
                focusComposer();
              }}
            />
          </div>
        {/if}
        {#if canSendMessages && trayItems.length > 0}
          <AttachmentTray items={trayItems} onremove={removeTrayItem} />
        {/if}
        <div class="flex items-end gap-2xs px-xs">
          {#if canSendMessages}
            {#if canAttachFiles}
              <IconButton
                icon="plus-circle"
                label="Attach files"
                class={composerButtonClass}
                iconClass="size-(--mw-component-composer-icon-size)"
                onclick={() => fileInput?.click()}
              />
              <!-- Opened by the attach button only; the button is the control people reach. -->
              <input
                bind:this={fileInput}
                type="file"
                multiple
                hidden
                tabindex="-1"
                aria-hidden="true"
                onchange={(event) => {
                  attachFiles(Array.from(event.currentTarget.files ?? []));
                  // Choosing the same file again must fire change again.
                  event.currentTarget.value = "";
                }}
              />
            {/if}
            <textarea
              bind:this={composer}
              bind:value={draft}
              rows="1"
              aria-label={composerLabel}
              aria-describedby={replyingTo ? "composer-reply-hint" : undefined}
              placeholder={composerLabel}
              class="min-h-(--mw-size-dock-height) min-w-0 flex-1 resize-none bg-transparent px-2xs py-[calc((var(--mw-size-dock-height)_-_var(--mw-type-message-font-size)_*_var(--mw-type-message-line-height))/2)] type-message text-(--mw-component-composer-text) outline-none placeholder:text-(--mw-component-composer-placeholder)"
              onkeydown={handleComposerKeydown}
              onpaste={handleComposerPaste}
              oninput={() => client.noteTyping()}
              {@attach autosize(() => draft, maximumComposerHeight)}
            ></textarea>
            {#if replyingTo}
              <span id="composer-reply-hint" class="sr-only">
                Replying to {displayName(replyingTo.author)}. Escape cancels the reply.
              </span>
            {/if}
            <!-- Phones have no Enter-to-send habit: the send button shows in the phone layout and
                 under a coarse pointer (DESIGN.md §4.3). -->
            <span class={hasNavigationDrawer ? "contents" : "hidden pointer-coarse:contents"}>
              <IconButton
                icon="paper-plane-right"
                label="Send message"
                aria-disabled={!canSend}
                class="{composerButtonClass} aria-disabled:opacity-(--mw-component-button-disabled-opacity)"
                iconClass="size-(--mw-component-composer-icon-size)"
                onclick={send}
              />
            </span>
          {:else}
            <p class="flex min-h-(--mw-size-dock-height) items-center px-xs type-message text-(--mw-component-composer-locked-text)">
              You do not have permission to send messages in this channel.
            </p>
          {/if}
        </div>
      </div>
    </div>
    <p class="sr-only" aria-live="polite">{uploadAnnouncement}</p>
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
        <p class="type-heading-xl break-words">Drop to attach to {channel ? `#${channel.name}` : `@${conversationName}`}</p>
        <p class="type-body">You can add a message before sending.</p>
      </div>
    </div>
  {/if}
</section>

{#snippet emptyState()}
  <div class="flex flex-1 flex-col items-center justify-center gap-xs p-xl text-center">
    {#if client.status === "connecting"}
      <p role="status" class="type-body text-text-muted">Loading your servers and conversations…</p>
    {:else if guild}
      <h2 class="type-heading-lg text-text-heading">No channels you can see</h2>
      <p class="max-w-(--mw-component-auth-card-max-width) type-body text-text-muted">
        Channels you can read in {guild.name} will show here.
      </p>
    {:else if client.guilds.length === 0}
      <h2 class="type-heading-lg text-text-heading">No servers yet</h2>
      <p class="max-w-(--mw-component-auth-card-max-width) type-body text-text-muted">
        Create a server for your group, or join one with an invite code.
      </p>
      <Button variant="primary" onclick={onaddserver}>Add a server</Button>
    {:else}
      <h2 class="type-heading-lg text-text-heading">No conversations yet</h2>
      <p class="max-w-(--mw-component-auth-card-max-width) type-body text-text-muted">
        Start one with someone from your servers: open their profile in the member list and choose Send message.
      </p>
    {/if}
  </div>
{/snippet}

{#if deletingMessage}
  {@const message = deletingMessage}
  <DeleteMessageDialog
    {message}
    onclose={() => (deletingMessage = null)}
    onconfirm={() => {
      deletingMessage = null;
      void deleteMessage(message);
    }}
    fallbackFocus={focusComposer}
  />
{/if}

<style>
  /* Loops only while someone types; under reduced motion the dots hold still (ADR-0022 §3). */
  .typing-dots i {
    width: var(--mw-component-composer-typing-dot-size);
    height: var(--mw-component-composer-typing-dot-size);
    border-radius: var(--mw-radius-full);
    background: currentColor;
    animation: mw-typing-dot var(--mw-component-composer-typing-dot-duration) ease-in-out infinite;
  }
  .typing-dots i:nth-child(2) {
    animation-delay: var(--mw-component-composer-typing-dot-stagger);
  }
  .typing-dots i:nth-child(3) {
    animation-delay: calc(2 * var(--mw-component-composer-typing-dot-stagger));
  }
  .reply-bar {
    animation: mw-fade-in var(--mw-motion-duration-fast) var(--mw-motion-easing-standard) both;
  }
  .unread-bar {
    animation: unread-bar-in var(--mw-motion-duration-base) var(--mw-motion-easing-enter) both;
  }
  @keyframes unread-bar-in {
    from {
      opacity: 0;
      transform: translateY(calc(-1 * var(--mw-component-channel-switch-offset)));
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .typing-dots i {
      animation: none;
      opacity: 0.8;
    }
  }
</style>
