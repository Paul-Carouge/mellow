<script lang="ts">
  import { tick } from "svelte";

  import { type MellowClient, describeError } from "../lib/client.svelte";
  import { type Message, Permission, type PublicUser, type Snowflake, hasPermission } from "../lib/protocol";
  import { autosize } from "./autosize";
  import Button from "./Button.svelte";
  import DeleteMessageDialog from "./DeleteMessageDialog.svelte";
  import { displayName, typingLabel } from "./format";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import MessageList from "./MessageList.svelte";

  // The chat pane (DESIGN.md §5.4, §5.5): header, message list, composer and typing band.
  interface Props {
    client: MellowClient;
    /** Below the sidebar breakpoint the header opens the navigation drawer. */
    hasNavigationDrawer: boolean;
    onopennavigation: () => void;
    isMemberListOpen: boolean;
    ontogglememberlist: () => void;
    onaddserver: () => void;
  }

  let { client, hasNavigationDrawer, onopennavigation, isMemberListOpen, ontogglememberlist, onaddserver }: Props =
    $props();

  /** The composer grows with its text up to half of the pane (DESIGN.md §4.2). */
  const COMPOSER_MAXIMUM_SHARE_OF_PANE = 0.5;
  const TYPING_REFRESH_INTERVAL_MILLISECONDS = 1_000;

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
  const canSendMessages = $derived(guild ? hasPermission(guild.my_permissions, Permission.SEND_MESSAGES) : true);
  const canManageMessages = $derived(guild ? hasPermission(guild.my_permissions, Permission.MANAGE_MESSAGES) : false);
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

  // Each channel keeps its own unsent text (DESIGN.md §5.5); replies and edits do not carry over.
  let shownChannelId: Snowflake | null = null;
  $effect.pre(() => {
    if (channelId === shownChannelId) {
      return;
    }
    if (shownChannelId) {
      draftsByChannel.set(shownChannelId, draft);
    }
    shownChannelId = channelId;
    draft = (channelId && draftsByChannel.get(channelId)) || "";
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
  $effect(() => {
    if (client.typingUntil.size === 0) {
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

  /** Every path that focuses the composer comes here: a read-only channel has none to focus. */
  export function focusComposer() {
    composer?.focus();
  }

  function maximumComposerHeight(): number {
    return pane ? pane.clientHeight * COMPOSER_MAXIMUM_SHARE_OF_PANE : Number.POSITIVE_INFINITY;
  }

  function send() {
    const text = draft.trim();
    if (!text) {
      return;
    }
    void client.sendMessage(text, replyingTo?.id ?? null);
    draft = "";
    replyingTo = null;
    messageList?.jumpToPresent();
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

  async function startReply(message: Message) {
    replyingTo = message;
    await tick();
    focusComposer();
  }
</script>

<section
  bind:this={pane}
  aria-label={channel ? `#${channel.name}` : conversation ? conversationName : "Chat"}
  class="flex h-full min-w-0 flex-1 flex-col bg-surface-chat"
>
  <header
    class="relative z-10 flex h-(--mw-component-chat-header-height) shrink-0 items-center gap-xs bg-(--mw-component-chat-header-background) px-(--mw-component-chat-header-padding-inline) shadow-(--mw-component-chat-header-shadow)"
  >
    {#if hasNavigationDrawer}
      <IconButton
        icon="list"
        label="Open servers and channels"
        class="-ms-xs size-(--mw-component-chat-header-button-size) text-(--mw-component-chat-header-icon) hover:bg-state-hover"
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onclick={onopennavigation}
      />
    {/if}
    {#if channel}
      <Icon
        name={canSendMessages ? "hash" : "lock"}
        class="size-(--mw-component-chat-header-icon-size) text-(--mw-component-chat-header-icon)"
      />
      <h1 class="shrink-0 truncate type-heading-md text-(--mw-component-chat-header-name-text) max-w-1/2">{channel.name}</h1>
      {#if channel.topic}
        <span aria-hidden="true" class="h-(--mw-component-chat-header-divider-height) w-px shrink-0 bg-border-divider"></span>
        <p class="min-w-0 flex-1 truncate type-body-sm text-(--mw-component-chat-header-topic-text)" title={channel.topic}>
          {channel.topic}
        </p>
      {:else}
        <span class="flex-1"></span>
      {/if}
      <IconButton
        icon="users"
        label={isMemberListOpen ? "Hide member list" : "Show member list"}
        aria-pressed={isMemberListOpen}
        class={[
          "size-(--mw-component-chat-header-button-size) hover:bg-state-hover",
          isMemberListOpen ? "text-text-heading" : "text-(--mw-component-chat-header-icon)",
        ].join(" ")}
        iconClass="size-(--mw-component-chat-header-icon-size)"
        onclick={ontogglememberlist}
      />
    {:else if conversation}
      <Icon name="at" class="size-(--mw-component-chat-header-icon-size) text-(--mw-component-chat-header-icon)" />
      <h1 class="min-w-0 flex-1 truncate type-heading-md text-(--mw-component-chat-header-name-text)">
        {conversationName}
      </h1>
    {/if}
  </header>

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

    <div class="shrink-0 px-(--mw-component-composer-margin-inline)">
      {#if replyingTo && canSendMessages}
        <div
          class="flex min-h-(--mw-component-composer-reply-bar-min-height) items-center rounded-t-(--mw-component-composer-radius) bg-(--mw-component-composer-reply-bar-background) ps-(--mw-component-composer-reply-bar-padding-inline) pe-2xs type-body text-(--mw-component-composer-reply-bar-text)"
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
      <div
        class={[
          "flex min-h-(--mw-component-composer-min-height) items-center gap-xs rounded-b-(--mw-component-composer-radius) bg-(--mw-component-composer-background) px-(--mw-component-composer-padding-inline) focus-within:focus-ring-inset",
          !(replyingTo && canSendMessages) && "rounded-t-(--mw-component-composer-radius)",
        ]}
      >
        {#if canSendMessages}
          <textarea
            bind:this={composer}
            bind:value={draft}
            rows="1"
            aria-label={composerLabel}
            aria-describedby={replyingTo ? "composer-reply-hint" : undefined}
            placeholder={composerLabel}
            class="min-w-0 flex-1 resize-none bg-transparent py-sm type-message text-(--mw-component-composer-text) outline-none placeholder:text-(--mw-component-composer-placeholder)"
            onkeydown={handleComposerKeydown}
            oninput={() => client.noteTyping()}
            {@attach autosize(() => draft, maximumComposerHeight)}
          ></textarea>
          {#if replyingTo}
            <span id="composer-reply-hint" class="sr-only">
              Replying to {displayName(replyingTo.author)}. Escape cancels the reply.
            </span>
          {/if}
          <!-- Phones have no Enter-to-send habit: the send button shows under a coarse pointer (DESIGN.md §4.3). -->
          <span class="hidden pointer-coarse:contents">
            <IconButton
              icon="paper-plane-right"
              label="Send message"
              class="text-(--mw-component-composer-icon) hover:text-(--mw-component-composer-icon-hover)"
              iconClass="size-(--mw-component-composer-icon-size)"
              onclick={send}
            />
          </span>
        {:else}
          <p class="py-sm type-message text-(--mw-component-composer-locked-text)">
            You do not have permission to send messages in this channel.
          </p>
        {/if}
      </div>
    </div>
    <p
      aria-hidden="true"
      class="h-(--mw-component-composer-typing-height) shrink-0 truncate px-(--mw-component-composer-margin-inline) type-typing text-(--mw-component-composer-typing-text)"
    >
      {typingText}
    </p>
  {:else}
    {@render emptyState()}
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
        Start one with someone from your servers: open their profile in the member list and choose Message.
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
