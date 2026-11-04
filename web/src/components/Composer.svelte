<script lang="ts">
  import { tick } from "svelte";

  import { strings } from "../i18n";
  import { en } from "../i18n/en";
  import { ApiError } from "../lib/api";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import {
    type CustomEmoji,
    type Message,
    type PublicUser,
    type Role,
    type Snowflake,
    MAXIMUM_MESSAGE_LENGTH,
  } from "../lib/protocol";
  import AttachmentTray, { type AttachmentTrayItem } from "./AttachmentTray.svelte";
  import { autosize } from "./autosize";
  import Avatar from "./Avatar.svelte";
  import Button from "./Button.svelte";
  import type { ComposerDrafts } from "./composer-drafts.svelte";
  import {
    type EmojiSuggestion,
    type EmojiSuggestionQuery,
    composerTextOf,
    emojiSuggestionQuery,
    replaceInDraft,
    suggestEmoji,
    withEmojiNamesAsMarkup,
  } from "./custom-emoji";
  import ComposerSuggestions from "./ComposerSuggestions.svelte";
  import CustomEmojiImage from "./CustomEmojiImage.svelte";
  import { displayName, humanFileSize } from "./format";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import { EVERYONE_MENTION_WORD, type MentionSuggestionQuery, mentionSuggestionQuery, suggestMentions } from "./mentions";
  import { type RoleMentionSuggestion, suggestRoleMentions, withRoleNamesAsMarkup } from "./role-mentions";
  import { NO_ROLE_COLOR, roleColorHex } from "./server-settings";
  import { MESSAGE_LENGTH_COUNTER_THRESHOLD, messageCharactersLeft } from "./message-length";
  import { createPopoverToggle } from "./popover-toggle";
  import { noteEmojiUsed } from "./recent-emoji.svelte";
  import {
    type NamedSlashCommand,
    type SlashCommandQuery,
    type TextEdit,
    applySlashCommand,
    differingRange,
    formattingMarksForShortcut,
    namedSlashCommands,
    slashCommandQuery,
    suggestSlashCommands,
    toggleInlineMarks,
  } from "./slash-commands";

  // The chat's composer zone (DESIGN.md §5.5, ADR-0022 §4): the notices above it, the typing row,
  // the field with its reply bar and tray, or the sentence that says why it is locked.
  interface Props {
    client: MellowClient;
    /** The open channel's text and tray; ChatPane switches them with the channel. */
    drafts: ComposerDrafts;
    /** Typed as ":name:" and sent as markup (ADR-0028); empty outside a server channel. */
    serverEmojis: readonly CustomEmoji[];
    /** Who "@na" suggests; the writer is left out. */
    mentionCandidates: readonly PublicUser[];
    /** The roles "@na" suggests after people (`rolesWriterMayMention`); empty outside a server channel. */
    mentionableRoles: readonly Role[];
    /** The writer holds `MENTION_EVERYONE` in this channel: "@na" offers @everyone (ADR-0040). */
    canMentionEveryone: boolean;
    currentUserId: Snowflake | null;
    label: string;
    canSendMessages: boolean;
    canAttachFiles: boolean;
    /** The other person of a conversation locked by a block (ADR-0025). */
    blockedRecipient: PublicUser | null;
    /** The phone layout always shows the send button. */
    hasNavigationDrawer: boolean;
    typing: readonly { text: string; isName: boolean }[];
    /** Information rather than a failure (a search result too far back): quiet, not red. */
    notice: string;
    /** Shared with the chat's own actions: the last failure, said above the composer. */
    actionError: string;
    replyingTo: Message | null;
    isEmojiPickerOpen: boolean;
    /** Escape marks the channel as read while the unread bar shows. */
    hasUnreadBar: boolean;
    maximumHeight: () => number;
    onopenemojipicker: (anchor: HTMLElement) => void;
    onmarkasread: () => void;
    oneditlastmessage: () => void;
    onsent: () => void;
  }

  let {
    client,
    drafts,
    serverEmojis,
    mentionCandidates,
    mentionableRoles,
    canMentionEveryone,
    currentUserId,
    label,
    canSendMessages,
    canAttachFiles,
    blockedRecipient,
    hasNavigationDrawer,
    typing,
    notice,
    actionError = $bindable(),
    replyingTo = $bindable(),
    isEmojiPickerOpen,
    hasUnreadBar,
    maximumHeight,
    onopenemojipicker,
    onmarkasread,
    oneditlastmessage,
    onsent,
  }: Props = $props();

  /** The instance links at most this many files to one message (ADR-0020). */
  const MAXIMUM_ATTACHMENTS_PER_MESSAGE = 10;
  const TOO_MANY_ATTACHMENTS_SENTENCE = strings.chat.tooManyAttachments(MAXIMUM_ATTACHMENTS_PER_MESSAGE);
  const UPLOADS_RUNNING_SENTENCE = strings.chat.uploadsRunning;
  // The composer's buttons sit centred on its base row, which is as tall as the user card.
  const BUTTON_CLASS =
    "my-[calc((var(--mw-size-dock-height)_-_var(--mw-component-composer-attach-button-size))/2)] size-(--mw-component-composer-attach-button-size) text-(--mw-component-composer-icon) hover:bg-state-hover hover:text-(--mw-component-composer-icon-hover)";

  let field = $state<HTMLTextAreaElement | null>(null);
  let emojiButton = $state<HTMLButtonElement | null>(null);
  const emojiPickerToggle = createPopoverToggle(() => isEmojiPickerOpen);
  let fileInput = $state<HTMLInputElement | null>(null);
  let uploadAnnouncement = $state("");
  let isUnblocking = $state(false);

  const isUploading = $derived(drafts.trayItems.some((item) => item.status === "uploading"));
  const uploadedAttachments = $derived(
    drafts.trayItems.flatMap((item) => (item.status === "uploaded" && item.attachment ? [item.attachment] : [])),
  );
  // Usernames: a role named like someone stays that person's mention.
  const peopleNames = $derived(new Set(mentionCandidates.map((person) => person.username.toLowerCase())));
  /** The text as sent: ":name:" and a picked role's "@Name" become their markup (ADR-0028, ADR-0040). */
  function textAsSent(text: string): string {
    return withRoleNamesAsMarkup(withEmojiNamesAsMarkup(text, serverEmojis), mentionableRoles, peopleNames);
  }
  // Counted as sent: a typed ":name:" or "@Role" goes out as its longer markup.
  const charactersLeft = $derived(messageCharactersLeft(textAsSent(drafts.text)));

  // The ":na", "@na" and "/" suggestions (DESIGN.md §5.5): follow the caret; Esc hides them until
  // the word changes. A word is one or the other: ":", "@" and "/" each open their own.
  type SuggestionQuery =
    | (EmojiSuggestionQuery & { kind: "emoji" })
    | (MentionSuggestionQuery & { kind: "mention" })
    | (SlashCommandQuery & { kind: "command" });
  type ComposerSuggestion =
    | EmojiSuggestion
    | { kind: "person"; person: PublicUser }
    | RoleMentionSuggestion
    | { kind: "command"; named: NamedSlashCommand };
  // English names find the commands in any language: "/bold" works in French too.
  const NAMED_SLASH_COMMANDS = namedSlashCommands(strings.chat.slashCommands, en.chat.slashCommands);
  let suggestionQuery = $state<SuggestionQuery | null>(null);
  let dismissedSuggestionStart = $state<number | null>(null);
  let activeSuggestionIndex = $state(0);
  const suggestions: readonly ComposerSuggestion[] = $derived.by(() => {
    if (!suggestionQuery || suggestionQuery.start === dismissedSuggestionStart) {
      return [];
    }
    if (suggestionQuery.kind === "command") {
      return suggestSlashCommands(suggestionQuery.query, NAMED_SLASH_COMMANDS).map(
        (named): ComposerSuggestion => ({ kind: "command", named }),
      );
    }
    return suggestionQuery.kind === "emoji"
      ? suggestEmoji(suggestionQuery.query, serverEmojis)
      : [
          ...suggestMentions(suggestionQuery.query, mentionCandidates, currentUserId ?? "").map(
            (person): ComposerSuggestion => ({ kind: "person", person }),
          ),
          // After the people, as on Discord (ADR-0040).
          ...suggestRoleMentions(suggestionQuery.query, mentionableRoles, canMentionEveryone),
        ];
  });
  // Within the list as it is now: a gateway update can shorten it under the arrows.
  const activeSuggestion = $derived(Math.min(activeSuggestionIndex, Math.max(suggestions.length - 1, 0)));
  const suggestionsId = $props.id();
  const isTooLong = $derived(charactersLeft < 0);
  // The instance would refuse it: say why at the first try, until the text fits again.
  let hasTriedSendingTooLong = $state(false);
  const canSend = $derived((drafts.text.trim() !== "" || uploadedAttachments.length > 0) && !isUploading && !isTooLong);

  /** A read-only channel has no field to focus. */
  export function focus() {
    field?.focus();
  }

  /** Writes `text` and a space at the caret, as the emoji picker does. */
  export function insert(text: string): Promise<void> {
    return insertAt(text, null);
  }

  /** Each file uploads at once to the open channel and waits in the tray for Send (DESIGN.md §5.5). */
  export function attachFiles(files: readonly File[]) {
    if (!canAttachFiles || files.length === 0) {
      return;
    }
    const room = Math.max(MAXIMUM_ATTACHMENTS_PER_MESSAGE - drafts.trayItems.length, 0);
    if (files.length > room) {
      actionError = TOO_MANY_ATTACHMENTS_SENTENCE;
    }
    for (const file of files.slice(0, room)) {
      void upload(drafts.attach(file));
    }
  }

  async function unblockRecipient(recipient: PublicUser) {
    isUnblocking = true;
    actionError = "";
    try {
      await client.removeRelationship(recipient.id);
      await tick();
      focus();
    } catch (error) {
      actionError = describeError(error, strings.friends.unblockFailed(displayName(recipient)));
    } finally {
      isUnblocking = false;
    }
  }

  function send() {
    if (isUploading) {
      actionError = UPLOADS_RUNNING_SENTENCE;
      return;
    }
    if (isTooLong) {
      hasTriedSendingTooLong = true;
      return;
    }
    if (!canSend) {
      return;
    }
    hasTriedSendingTooLong = false;
    void client.sendMessage(
      textAsSent(drafts.text.trim()),
      replyingTo?.id ?? null,
      $state.snapshot(uploadedAttachments),
    );
    suggestionQuery = null;
    drafts.text = "";
    replyingTo = null;
    drafts.clearTray();
    if (actionError === UPLOADS_RUNNING_SENTENCE || actionError === TOO_MANY_ATTACHMENTS_SENTENCE) {
      actionError = "";
    }
    onsent();
  }

  async function upload(item: AttachmentTrayItem) {
    try {
      item.attachment = await client.uploadAttachment(item.file);
      item.status = "uploaded";
      announceUpload(item, strings.chat.uploaded(item.file.name));
    } catch (error) {
      item.status = "failed";
      item.error = uploadErrorSentence(item.file, error);
      announceUpload(item, item.error);
    }
  }

  /** Only for files still in the open channel's tray: removed ones and other channels' stay quiet. */
  function announceUpload(item: AttachmentTrayItem, sentence: string) {
    if (drafts.trayItems.includes(item)) {
      uploadAnnouncement = sentence;
    }
  }

  function uploadErrorSentence(file: File, error: unknown): string {
    // The instance does not tell its limit, so the sentence names the size and the instance (DESIGN.md §8).
    if (error instanceof ApiError && error.code === "FILE_TOO_LARGE") {
      return strings.chat.fileTooLarge(file.name, humanFileSize(file.size), window.location.host);
    }
    return describeError(error, strings.chat.uploadFailed(file.name));
  }

  function removeTrayItem(key: number) {
    drafts.removeTrayItem(key);
    // The Remove button is gone with its item.
    focus();
  }

  function handlePaste(event: ClipboardEvent) {
    const files = Array.from(event.clipboardData?.files ?? []);
    if (files.length === 0 || !canAttachFiles) {
      return;
    }
    event.preventDefault();
    attachFiles(files);
  }

  /** Reads the word before the caret again; called on typing, clicks and caret moves. */
  function refreshSuggestions() {
    const caret = field?.selectionStart ?? null;
    const isCaretCollapsed = caret !== null && caret === field?.selectionEnd;
    // The field's own value: the draft may not have followed this input event yet.
    const text = field?.value ?? drafts.text;
    const emojiQuery = isCaretCollapsed ? emojiSuggestionQuery(text, caret) : null;
    const mentionQuery = isCaretCollapsed && !emojiQuery ? mentionSuggestionQuery(text, caret) : null;
    const commandQuery = isCaretCollapsed && !emojiQuery && !mentionQuery ? slashCommandQuery(text, caret) : null;
    const query: SuggestionQuery | null = emojiQuery
      ? { ...emojiQuery, kind: "emoji" }
      : mentionQuery
        ? { ...mentionQuery, kind: "mention" }
        : commandQuery
          ? { ...commandQuery, kind: "command" }
          : null;
    if (
      query?.start !== suggestionQuery?.start ||
      query?.query !== suggestionQuery?.query ||
      query?.kind !== suggestionQuery?.kind
    ) {
      activeSuggestionIndex = 0;
    }
    if (query?.start !== dismissedSuggestionStart) {
      dismissedSuggestionStart = null;
    }
    suggestionQuery = query;
  }

  /** Writes `text` and a space at the caret, or in place of the word being completed. */
  async function insertAt(text: string, range: { start: number; end: number } | null) {
    const start = range?.start ?? field?.selectionStart ?? drafts.text.length;
    const end = range?.end ?? field?.selectionEnd ?? drafts.text.length;
    const result = replaceInDraft(drafts.text, start, end, text);
    drafts.text = result.text;
    suggestionQuery = null;
    await tick();
    field?.focus();
    field?.setSelectionRange(result.caret, result.caret);
  }

  /**
   * Writes `edit` into the field as typing would, so Ctrl+Z gives the text back: assigning the
   * value would wipe the browser's undo history. `execCommand` is deprecated but still the only
   * way to edit a text field undoably; where it does nothing, the text is set directly.
   */
  async function writeEdit(edit: TextEdit) {
    suggestionQuery = null;
    const currentField = field;
    if (!currentField) {
      drafts.text = edit.text;
      return;
    }
    const change = differingRange(currentField.value, edit.text);
    currentField.focus();
    currentField.setSelectionRange(change.start, change.end);
    if (change.inserted === "") {
      document.execCommand("delete");
    } else {
      document.execCommand("insertText", false, change.inserted);
    }
    if (currentField.value !== edit.text) {
      drafts.text = edit.text;
      await tick();
    }
    currentField.setSelectionRange(edit.selectionStart, edit.selectionEnd);
    // The input event read the caret before it was placed.
    suggestionQuery = null;
  }

  function chooseSuggestion(suggestion: ComposerSuggestion) {
    if (suggestion.kind === "command") {
      if (suggestionQuery && field) {
        void writeEdit(applySlashCommand(field.value, suggestionQuery, suggestion.named.command, strings.chat.linkTextPlaceholder));
      }
      return;
    }
    if (suggestion.kind === "person") {
      // The username: it always mentions, where a display name may be shared or hold spaces.
      void insertAt(`@${suggestion.person.username}`, suggestionQuery);
      return;
    }
    if (suggestion.kind === "everyone") {
      void insertAt(`@${EVERYONE_MENTION_WORD}`, suggestionQuery);
      return;
    }
    if (suggestion.kind === "role") {
      // Shown as "@Name" while writing, sent as `<@&id>` (`textAsSent`).
      void insertAt(`@${suggestion.role.name}`, suggestionQuery);
      return;
    }
    if (suggestion.kind === "unicode") {
      noteEmojiUsed(suggestion.emoji);
    }
    void insertAt(composerTextOf(suggestion.emoji), suggestionQuery);
  }

  function suggestionKey(suggestion: ComposerSuggestion): string {
    switch (suggestion.kind) {
      case "person":
        return `person-${suggestion.person.id}`;
      case "everyone":
        return "everyone";
      case "role":
        return `role-${suggestion.role.id}`;
      case "custom":
        return `custom-${suggestion.emoji.id}`;
      case "unicode":
        return suggestion.emoji;
      case "command":
        return `command-${suggestion.named.command.id}`;
    }
  }

  /** Arrows, Enter and Tab work the suggestions while they show; true when the key was theirs. */
  function handleSuggestionKeydown(event: KeyboardEvent): boolean {
    if (suggestions.length === 0) {
      return false;
    }
    const last = suggestions.length - 1;
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      activeSuggestionIndex =
        event.key === "ArrowDown"
          ? activeSuggestion >= last ? 0 : activeSuggestion + 1
          : activeSuggestion <= 0 ? last : activeSuggestion - 1;
    } else if ((event.key === "Enter" && !event.shiftKey) || (event.key === "Tab" && !event.shiftKey)) {
      const chosen = suggestions[activeSuggestion];
      if (chosen) {
        chooseSuggestion(chosen);
      }
    } else if (event.key === "Escape") {
      dismissedSuggestionStart = suggestionQuery?.start ?? null;
    } else {
      return false;
    }
    event.preventDefault();
    return true;
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.isComposing || handleSuggestionKeydown(event)) {
      return;
    }
    if (event.key.toLowerCase() === "e" && (event.ctrlKey || event.metaKey) && !event.altKey && emojiButton) {
      // Discord's shortcut for the emoji picker (DESIGN.md §5.5).
      event.preventDefault();
      onopenemojipicker(emojiButton);
      return;
    }
    const marks = formattingMarksForShortcut(event);
    // Ctrl+U without a selection stays the member list's (Shell), as on Discord.
    if (marks && field && !(marks === "__" && field.selectionStart === field.selectionEnd)) {
      event.preventDefault();
      void writeEdit(toggleInlineMarks(field.value, field.selectionStart, field.selectionEnd, marks));
      return;
    }
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      send();
    } else if (event.key === "Escape" && replyingTo) {
      event.preventDefault();
      replyingTo = null;
    } else if (event.key === "Escape" && hasUnreadBar) {
      event.preventDefault();
      onmarkasread();
    } else if (event.key === "ArrowUp" && drafts.text === "" && canSendMessages) {
      event.preventDefault();
      oneditlastmessage();
    }
  }
</script>

{#snippet suggestionRow(suggestion: ComposerSuggestion)}
  {#if suggestion.kind === "command"}
    <span aria-hidden="true" class="grid size-(--mw-component-composer-suggestions-avatar-size) shrink-0 place-items-center">
      <Icon name={suggestion.named.command.icon} class="size-(--mw-component-context-menu-item-icon-size)" />
    </span>
    <span class="min-w-0 truncate">{suggestion.named.name}</span>
    <span class="min-w-0 flex-1 truncate text-text-secondary">{suggestion.named.description}</span>
    <!-- What the command writes, as inline code shows in messages: the menu teaches the Markdown. -->
    <code class="min-w-0 truncate rounded-xs bg-(--mw-component-message-code-background) px-3xs type-code text-text-normal">{suggestion.named.command.markup}</code>
  {:else if suggestion.kind === "person"}
    <Avatar user={suggestion.person} sizeToken="--mw-component-composer-suggestions-avatar-size" />
    <span class="min-w-0 truncate">{displayName(suggestion.person)}</span>
    <span class="min-w-0 flex-1 truncate text-text-secondary">@{suggestion.person.username}</span>
  {:else if suggestion.kind === "everyone"}
    <span aria-hidden="true" class="grid size-(--mw-component-composer-suggestions-avatar-size) shrink-0 place-items-center">
      <Icon name="users" class="size-(--mw-component-context-menu-item-icon-size)" />
    </span>
    <span class="min-w-0 truncate">@{EVERYONE_MENTION_WORD}</span>
    <span class="min-w-0 flex-1 truncate text-text-secondary" title={strings.chat.everyoneMentionHint}>{strings.chat.everyoneMentionHint}</span>
  {:else if suggestion.kind === "role"}
    <span aria-hidden="true" class="grid size-(--mw-component-composer-suggestions-avatar-size) shrink-0 place-items-center">
      <span
        class={[
          "size-(--mw-component-roles-row-dot-size) rounded-full",
          suggestion.role.color === NO_ROLE_COLOR && "bg-(--mw-component-roles-neutral-color)",
        ]}
        style:background-color={suggestion.role.color === NO_ROLE_COLOR ? undefined : roleColorHex(suggestion.role.color)}
      ></span>
    </span>
    <span class="min-w-0 truncate">@{suggestion.role.name}</span>
    <span class="min-w-0 flex-1 truncate text-text-secondary">{strings.chat.roleMentionHint}</span>
  {:else}
    <span
      aria-hidden="true"
      class="grid size-(--mw-component-emoji-picker-emoji-size) shrink-0 place-items-center text-(length:--mw-component-emoji-picker-emoji-size) leading-none"
    >
      {#if suggestion.kind === "custom"}
        <CustomEmojiImage emoji={suggestion.emoji} class="size-(--mw-component-emoji-picker-emoji-size)" isDecorative />
      {:else}
        {suggestion.emoji}
      {/if}
    </span>
    <span class="min-w-0 flex-1 truncate">{suggestion.kind === "custom" ? `:${suggestion.emoji.name}:` : suggestion.name}</span>
    {#if suggestion.kind === "unicode"}
      <!-- The built-in emoji itself, for screen readers that only read the word. -->
      <span class="sr-only">{suggestion.emoji}</span>
    {/if}
  {/if}
{/snippet}

<div role="status" class="contents">
  {#if notice}
    <p class="px-(--mw-component-composer-margin-inline) py-(--mw-component-composer-error-padding-block) type-body text-text-secondary">
      {notice}
    </p>
  {/if}
</div>
<div role="alert" class="contents">
  {#if actionError}
    <p class="px-(--mw-component-composer-margin-inline) py-(--mw-component-composer-error-padding-block) type-body text-(--mw-component-composer-error-text)">
      {actionError}
    </p>
  {/if}
  {#if isTooLong && hasTriedSendingTooLong}
    <p class="px-(--mw-component-composer-margin-inline) py-(--mw-component-composer-error-padding-block) type-body text-(--mw-component-composer-error-text)">
      {strings.chat.messageTooLong(MAXIMUM_MESSAGE_LENGTH)}
    </p>
  {/if}
</div>

<!-- The composer zone (ADR-0022 §4): the typing row above, then the composer, whose base row is
     as tall as the user card and as far from the bottom; it grows upward. -->
<!-- Clicks go through its empty parts and the typing row to the "viewing older messages" bar,
     which rests on the composer from under them (MessageList). -->
<div
  class="pointer-events-none relative shrink-0 px-(--mw-component-composer-margin-inline) pb-(--mw-size-dock-margin) [&>*:not([data-typing-row])]:pointer-events-auto"
>
  {#if canSendMessages && suggestionQuery && suggestions.length > 0}
    <div class="absolute inset-x-(--mw-component-composer-margin-inline) top-0">
      <ComposerSuggestions
        id={suggestionsId}
        title={suggestionQuery.kind === "emoji"
          ? strings.chat.emojiSuggestionsTitle(suggestionQuery.query)
          : suggestionQuery.kind === "command"
            ? strings.chat.slashCommandSuggestionsTitle(suggestionQuery.query)
            : strings.chat.mentionSuggestionsTitle(suggestionQuery.query)}
        isWide={suggestionQuery.kind === "command"}
        {suggestions}
        keyOf={suggestionKey}
        activeIndex={activeSuggestion}
        row={suggestionRow}
        onchoose={chooseSuggestion}
        onhover={(index) => (activeSuggestionIndex = index)}
      />
    </div>
  {/if}
  <p class="sr-only" aria-live="polite">
    {suggestions.length === 0
      ? ""
      : suggestionQuery?.kind === "mention"
        ? strings.chat.mentionSuggestionsCount(suggestions.length)
        : suggestionQuery?.kind === "command"
          ? strings.chat.slashCommandSuggestionsCount(suggestions.length)
          : strings.chat.emojiSuggestionsCount(suggestions.length)}
  </p>
  <p
    aria-hidden="true"
    data-typing-row
    class="flex h-(--mw-component-composer-typing-height) min-w-0 items-center gap-2xs ps-xs type-typing text-(--mw-component-composer-typing-text)"
  >
    {#if typing.length > 0}
      <span class="typing-dots inline-flex shrink-0 gap-(--mw-component-composer-typing-dot-gap)">
        <i></i><i></i><i></i>
      </span>
      <span class="truncate"
        >{#each typing as segment, index (index)}{#if segment.isName}<b class="font-bold">{segment.text}</b
            >{:else}{segment.text}{/if}{/each}</span
      >
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
          {strings.chat.replyingTo} <span class="font-semibold text-(--mw-component-composer-reply-bar-author-text)"
            >{displayName(replyingTo.author)}</span
          >
        </p>
        <IconButton
          icon="x-circle"
          label={strings.chat.cancelReply}
          class="size-(--mw-component-composer-reply-bar-button-size) text-(--mw-component-composer-reply-bar-icon) hover:bg-(--mw-component-composer-reply-bar-button-background-hover) hover:text-(--mw-component-composer-icon-hover)"
          iconClass="size-(--mw-component-composer-reply-bar-icon-size)"
          onclick={() => {
            replyingTo = null;
            focus();
          }}
        />
      </div>
    {/if}
    {#if canSendMessages && drafts.trayItems.length > 0}
      <AttachmentTray items={drafts.trayItems} onremove={removeTrayItem} />
    {/if}
    <div class="flex items-end gap-2xs px-xs">
      {#if canSendMessages}
        {#if canAttachFiles}
          <IconButton
            icon="plus-circle"
            label={strings.chat.attachFiles}
            class={BUTTON_CLASS}
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
        <!-- The emoji suggestions are owned as a textbox's list, without role="combobox" (nor
             aria-expanded, which needs it): ARIA's combobox is a single-line field, and screen
             readers would stop announcing a multi-line text box where Shift+Enter adds lines. -->
        <textarea
          bind:this={field}
          bind:value={drafts.text}
          rows="1"
          aria-label={label}
          aria-describedby={[
            replyingTo && "composer-reply-hint",
            charactersLeft <= MESSAGE_LENGTH_COUNTER_THRESHOLD && "composer-characters-left",
          ]
            .filter(Boolean)
            .join(" ") || undefined}
          placeholder={label}
          class="min-h-(--mw-size-dock-height) min-w-0 flex-1 resize-none bg-transparent px-2xs py-[calc((var(--mw-size-dock-height)_-_var(--mw-type-message-font-size)_*_var(--mw-type-message-line-height))/2)] type-message text-(--mw-component-composer-text) outline-none placeholder:text-(--mw-component-composer-placeholder)"
          aria-autocomplete="list"
          aria-controls={suggestions.length > 0 ? `${suggestionsId}-listbox` : undefined}
          aria-activedescendant={suggestions.length > 0 ? `${suggestionsId}-option-${activeSuggestion}` : undefined}
          onkeydown={handleKeydown}
          onkeyup={(event) => {
            if (event.key.startsWith("Arrow") || event.key === "Home" || event.key === "End") {
              refreshSuggestions();
            }
          }}
          onclick={refreshSuggestions}
          onblur={() => (suggestionQuery = null)}
          onpaste={handlePaste}
          oninput={() => {
            client.noteTyping();
            refreshSuggestions();
          }}
          {@attach autosize(() => drafts.text, maximumHeight)}
        ></textarea>
        {#if charactersLeft <= MESSAGE_LENGTH_COUNTER_THRESHOLD}
          <!-- Discord's counter: only near the limit, the number alone on screen. -->
          <span
            id="composer-characters-left"
            class={[
              "flex min-h-(--mw-size-dock-height) shrink-0 items-center px-2xs type-message-subtext tabular-nums",
              isTooLong ? "text-(--mw-component-composer-error-text)" : "text-(--mw-component-composer-placeholder)",
            ]}
          >
            <span aria-hidden="true">{charactersLeft}</span>
            <span class="sr-only">{strings.chat.messageCharactersLeft(charactersLeft)}</span>
          </span>
        {/if}
        {#if replyingTo}
          <span id="composer-reply-hint" class="sr-only">
            {strings.chat.replyHint(displayName(replyingTo.author))}
          </span>
        {/if}
        <IconButton
          bind:element={emojiButton}
          icon="smiley"
          label={strings.chat.chooseEmoji}
          aria-haspopup="dialog"
          aria-expanded={isEmojiPickerOpen}
          aria-keyshortcuts="Control+E"
          class={BUTTON_CLASS}
          iconClass="size-(--mw-component-composer-icon-size)"
          onpointerdown={emojiPickerToggle.notePress}
          onclick={(event) => {
            if (emojiPickerToggle.shouldOpenOnClick()) {
              onopenemojipicker(event.currentTarget);
            }
          }}
        />
        <!-- Phones have no Enter-to-send habit: the send button shows in the phone layout and
             under a coarse pointer (DESIGN.md §4.3). -->
        <span class={hasNavigationDrawer ? "contents" : "hidden pointer-coarse:contents"}>
          <IconButton
            icon="paper-plane-right"
            label={strings.chat.sendMessage}
            aria-disabled={!canSend}
            class="{BUTTON_CLASS} aria-disabled:opacity-(--mw-component-button-disabled-opacity)"
            iconClass="size-(--mw-component-composer-icon-size)"
            onclick={send}
          />
        </span>
      {:else}
        {#if blockedRecipient}
          {@const recipient = blockedRecipient}
          <div class="flex min-h-(--mw-size-dock-height) flex-1 flex-wrap items-center justify-between gap-x-sm gap-y-2xs px-xs py-2xs">
            <p class="type-message text-(--mw-component-composer-locked-text)">
              {strings.chat.youBlocked(displayName(recipient))}
            </p>
            <Button isDisabled={isUnblocking} onclick={() => void unblockRecipient(recipient)}
              >{strings.friends.unblock}</Button
            >
          </div>
        {:else}
          <p class="flex min-h-(--mw-size-dock-height) items-center px-xs type-message text-(--mw-component-composer-locked-text)">
            {strings.chat.cannotSend}
          </p>
        {/if}
      {/if}
    </div>
  </div>
</div>
<p class="sr-only" aria-live="polite">{uploadAnnouncement}</p>

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
  @media (prefers-reduced-motion: reduce) {
    .typing-dots i {
      animation: none;
      opacity: var(--mw-component-composer-typing-dot-opacity-still);
    }
  }
</style>
