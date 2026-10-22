<script lang="ts">
  import { onMount } from "svelte";

  import { strings } from "../i18n";
  import type { Message, PublicUser } from "../lib/protocol";
  import { autosize } from "./autosize";
  import Avatar from "./Avatar.svelte";
  import {
    displayName,
    fullTimestampLabel,
    messageTimestampLabel,
    splitMessageText,
    timeOfDayLabel,
  } from "./format";
  import Icon from "./Icon.svelte";
  import IconButton from "./IconButton.svelte";
  import { type MentionablePeople, mentionsUser, splitMentions } from "./mentions";
  import MessageAttachments from "./MessageAttachments.svelte";
  import type { MessageListRow } from "./message-rows";
  import { durationTokenInMilliseconds } from "./tokens";

  // One message row (DESIGN.md §5.4): group start with avatar, author and time, or a continuation
  // aligned on the gutter; the hover bar with its actions; the inline editor; outbox states.
  interface Props {
    row: MessageListRow;
    currentUser: PublicUser;
    /** The original's author and text, or no author and a sentence when it is not here; null when not a reply. */
    replyReference: { author: PublicUser | null; text: string } | null;
    /** The author's role colour, already readable on the sheet; null keeps the heading colour. */
    authorColor: string | null;
    /** Who "@name" can point at here: the server's members or the conversation's people. */
    mentionablePeople: MentionablePeople;
    canSendMessages: boolean;
    canManageMessages: boolean;
    isEditing: boolean;
    isRovingTarget: boolean;
    maximumEditorHeight: () => number;
    onreply: (message: Message) => void;
    onstartedit: (message: Message) => void;
    onsaveedit: (message: Message, text: string) => void;
    oncanceledit: () => void;
    /** `skipConfirmation` when Shift was held, as on Discord. */
    ondelete: (message: Message, skipConfirmation: boolean) => void;
    oncopy: (text: string) => void;
    onretry: (nonce: string) => void;
    ondiscard: (nonce: string) => void;
    /** Focus entered the row: it becomes the list's single Tab stop. */
    onfocusrow: () => void;
  }

  let {
    row,
    currentUser,
    replyReference,
    authorColor,
    mentionablePeople,
    canSendMessages,
    canManageMessages,
    isEditing,
    isRovingTarget,
    maximumEditorHeight,
    onreply,
    onstartedit,
    onsaveedit,
    oncanceledit,
    ondelete,
    oncopy,
    onretry,
    ondiscard,
    onfocusrow,
  }: Props = $props();

  const message = $derived(row.kind === "sent" ? row.message : null);
  const author = $derived(message ? message.author : currentUser);
  const content = $derived(row.kind === "sent" ? row.message.content : row.pending.content);
  const attachments = $derived(row.kind === "sent" ? (row.message.attachments ?? []) : row.pending.attachments);
  const sentAt = $derived(row.kind === "sent" ? row.sentAt : null);
  const isOwn = $derived(author.id === currentUser.id);
  const isSending = $derived(row.kind === "pending" && row.pending.status === "sending");
  const isFailed = $derived(row.kind === "pending" && row.pending.status === "failed");
  const isMentioningCurrentUser = $derived(mentionsUser(content, mentionablePeople, currentUser.id));

  // Taken from the message when the editor opens; later updates to the message keep the typing.
  let draft = $state("");
  let wasEditing = false;
  $effect.pre(() => {
    if (isEditing && !wasEditing && message) {
      draft = message.content;
    }
    wasEditing = isEditing;
  });

  // "Sending…" appears only after a delay, so fast sends do not flicker (DESIGN.md §5.4).
  let isSendingLabelShown = $state(false);
  onMount(() => {
    const timer = setTimeout(() => (isSendingLabelShown = true), durationTokenInMilliseconds("motion-delay-pending-label"));
    return () => clearTimeout(timer);
  });

  function handleEditorKeydown(event: KeyboardEvent & { currentTarget: HTMLTextAreaElement }) {
    if (!message) {
      return;
    }
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      oncanceledit();
    } else if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      onsaveedit(message, draft);
    }
  }

  function focusEditorAtEnd(textarea: HTMLTextAreaElement) {
    textarea.focus();
    textarea.setSelectionRange(textarea.value.length, textarea.value.length);
  }
</script>

{#snippet body()}
  <p
    class={[
      "type-message break-words whitespace-pre-wrap",
      isSending ? "text-(--mw-component-message-pending-text)" : "text-(--mw-component-message-body-text)",
    ]}
  >
    {#if !row.startsAuthorGroup}<span class="sr-only">{strings.messages.authorSaid(displayName(author), "")}</span>{/if}{#each splitMessageText(content) as segment, index (index)}{#if segment.kind === "link"}<a
          href={segment.url}
          target="_blank"
          rel="noopener noreferrer"
          class="text-text-link hover:underline">{segment.text}</a
        >{:else}{#each splitMentions(segment.text, mentionablePeople) as part, partIndex (partIndex)}{#if part.kind === "mention"}<span
                class="rounded-(--mw-component-mention-chip-radius) bg-(--mw-component-mention-chip-background) px-(--mw-component-mention-chip-padding-inline) font-medium text-(--mw-component-mention-chip-text) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) hover:bg-(--mw-component-mention-chip-background-hover) hover:text-(--mw-component-mention-chip-text-hover)"
                >{part.text}</span
              >{:else}{part.text}{/if}{/each}{/if}{/each}{#if message?.edited_at_unix_milliseconds}
      <span
        class="type-message-timestamp text-(--mw-component-message-edited-text)"
        title={strings.messages.editedAt(fullTimestampLabel(message.edited_at_unix_milliseconds))}
        >{strings.messages.edited}</span
      >{/if}
  </p>
{/snippet}

<!-- Messages take focus (one roving Tab stop, arrows between them) so their actions are reachable
     by keyboard, as on Discord (DESIGN.md §7.2). -->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<li
  data-row-key={row.key}
  tabindex={isRovingTarget ? 0 : -1}
  onfocusin={onfocusrow}
  class={[
    "group relative pr-(--message-padding-trailing,var(--mw-component-message-padding-trailing)) pl-(--mw-component-message-gutter) py-(--mw-component-message-padding-block) transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) focus-visible:focus-ring-inset",
    row.startsAuthorGroup && "mt-(--mw-component-message-group-gap)",
    // A message that mentions you: tint and a leading bar, so colour is never the only sign.
    isMentioningCurrentUser
      ? "bg-(--mw-component-mention-background) shadow-[inset_var(--mw-component-mention-bar-width)_0_0_0_var(--mw-component-mention-bar-color)] hover:bg-(--mw-component-mention-background-hover)"
      : isEditing
        ? "bg-(--mw-component-message-background-hover)"
        : "hover:bg-(--mw-component-message-background-hover) focus-within:bg-(--mw-component-message-background-hover)",
  ]}
>
  {#if isMentioningCurrentUser}
    <span class="sr-only">{strings.messages.mentionsYou}</span>
  {/if}
  {#if replyReference}
    <!-- The spine runs from the author's avatar up to the original (DESIGN.md §5.4). -->
    <p class="reply-reference relative flex h-(--mw-component-message-reply-height) min-w-0 items-center gap-2xs type-body text-text-secondary">
      {#if replyReference.author}
        <Avatar user={replyReference.author} sizeToken="--mw-component-message-reply-avatar-size" />
        <span class="shrink-0 font-medium text-text-heading">{displayName(replyReference.author)}</span>
      {:else}
        <Icon name="arrow-bend-up-left" class="size-(--mw-component-message-reply-avatar-size)" />
      {/if}
      <span class="truncate">{replyReference.text}</span>
    </p>
  {/if}

  {#if row.startsAuthorGroup}
    <span
      class={[
        "absolute left-(--mw-component-message-avatar-inset)",
        replyReference
          ? "top-[calc(var(--mw-component-message-padding-block)_+_var(--mw-space-2xs)_+_var(--mw-component-message-reply-height))]"
          : "top-[calc(var(--mw-component-message-padding-block)_+_var(--mw-space-3xs))]",
      ]}
    >
      <Avatar user={author} sizeToken="--mw-component-message-avatar-size" />
    </span>
    <h3 class="flex flex-wrap items-baseline gap-x-xs">
      <span class="type-message-author text-(--mw-component-message-author-text)" style:color={authorColor}
        >{displayName(author)}</span
      >
      {#if sentAt !== null}
        <time
          datetime={new Date(sentAt).toISOString()}
          title={fullTimestampLabel(sentAt)}
          class="type-message-timestamp text-(--mw-component-message-timestamp-text)"
          >{messageTimestampLabel(sentAt, new Date())}</time
        >
      {/if}
    </h3>
  {:else if sentAt !== null}
    <time
      datetime={new Date(sentAt).toISOString()}
      title={fullTimestampLabel(sentAt)}
      aria-hidden="true"
      class="invisible absolute left-0 w-(--mw-component-message-gutter) text-center type-message-timestamp text-(--mw-component-message-timestamp-text) group-focus-within:visible group-hover:visible"
      >{timeOfDayLabel(sentAt)}</time
    >
  {/if}

  {#if isEditing && message}
    <div class="flex flex-col gap-(--mw-component-message-editor-hint-gap) py-(--mw-space-3xs)">
      <textarea
        bind:value={draft}
        rows="1"
        aria-label={strings.messages.editMessage}
        aria-describedby="edit-hint-{message.id}"
        class="w-full resize-none rounded-(--mw-component-message-editor-radius) bg-(--mw-component-message-editor-background) px-(--mw-component-message-editor-padding-inline) py-(--mw-component-message-editor-padding-block) type-message text-(--mw-component-message-editor-text) focus-visible:focus-ring-inset"
        onkeydown={handleEditorKeydown}
        {@attach autosize(() => draft, maximumEditorHeight)}
        {@attach focusEditorAtEnd}
      ></textarea>
      <p id="edit-hint-{message.id}" class="type-message-subtext text-(--mw-component-message-editor-hint-text)">
        {strings.messages.escTo} <button
          type="button"
          class="text-(--mw-component-message-editor-hint-link) hover:underline"
          aria-label={strings.messages.cancelEditing}
          onclick={oncanceledit}>{strings.messages.cancelLowercase}</button
        >
        {strings.messages.enterTo}
        <button
          type="button"
          class="text-(--mw-component-message-editor-hint-link) hover:underline"
          aria-label={strings.messages.saveChanges}
          onclick={() => onsaveedit(message, draft)}>{strings.messages.saveLowercase}</button
        >
      </p>
    </div>
  {:else}
    {@render body()}
    {#if attachments.length > 0}
      <MessageAttachments {attachments} isPending={row.kind === "pending"} />
    {/if}
  {/if}

  {#if isSending && isSendingLabelShown}
    <p class="type-message-subtext text-(--mw-component-message-pending-text)">{strings.messages.sending}</p>
  {/if}
  {#if isFailed && row.kind === "pending"}
    <p class="flex flex-wrap items-center gap-x-xs type-message-subtext text-(--mw-component-message-failed-status-text)">
      <Icon name="warning-circle" class="size-(--mw-component-message-status-icon-size)" />
      {strings.messages.notSent}
      <button
        type="button"
        class="min-h-(--mw-size-hit-desktop-min) text-(--mw-component-message-editor-hint-link) hover:underline pointer-coarse:min-h-(--mw-size-hit-web-min)"
        aria-label={strings.messages.retryLabel}
        onclick={() => onretry(row.pending.nonce)}>{strings.messages.retry}</button
      >
      <button
        type="button"
        class="min-h-(--mw-size-hit-desktop-min) text-(--mw-component-message-editor-hint-link) hover:underline pointer-coarse:min-h-(--mw-size-hit-web-min)"
        aria-label={strings.messages.deleteUnsentLabel}
        onclick={() => ondiscard(row.pending.nonce)}>{strings.common.delete}</button
      >
    </p>
  {/if}

  {#if message && !isEditing}
    {@const buttonClass =
      "size-(--mw-component-action-bar-button-size) text-(--mw-component-action-bar-icon) hover:bg-(--mw-component-action-bar-button-background-hover) hover:text-(--mw-component-action-bar-icon-hover) focus-visible:focus-ring-inset"}
    {@const iconClass = "size-(--mw-component-action-bar-icon-size)"}
    <!-- Hidden until the row is hovered or holds focus; hidden controls leave the Tab order. -->
    <div
      role="toolbar"
      aria-label={strings.messages.actions}
      class="invisible absolute top-(--mw-component-action-bar-offset-top) right-(--mw-component-action-bar-offset-trailing) z-10 flex rounded-(--mw-component-action-bar-radius) border border-(--mw-component-action-bar-border) bg-(--mw-component-action-bar-background) shadow-(--mw-component-action-bar-shadow) group-focus-within:visible group-hover:visible"
    >
      {#if isOwn && canSendMessages}
        <IconButton icon="pencil-simple" label={strings.common.edit} class={buttonClass} {iconClass} onclick={() => onstartedit(message)} />
      {/if}
      {#if canSendMessages}
        <IconButton icon="arrow-bend-up-left" label={strings.messages.reply} class={buttonClass} {iconClass} onclick={() => onreply(message)} />
      {/if}
      <IconButton icon="copy" label={strings.messages.copyText} class={buttonClass} {iconClass} onclick={() => oncopy(message.content)} />
      {#if isOwn || canManageMessages}
        <IconButton
          icon="trash"
          label={strings.common.delete}
          class={[buttonClass, "hover:text-(--mw-component-action-bar-icon-danger-hover)"].join(" ")}
          {iconClass}
          onclick={(event) => ondelete(message, event.shiftKey)}
        />
      {/if}
    </div>
  {/if}
</li>

<style>
  .reply-reference::before {
    content: "";
    position: absolute;
    /* From the centre of the author's avatar in the gutter, up and over to the original. */
    left: calc(
      var(--mw-component-message-avatar-inset) + var(--mw-component-message-avatar-size) / 2 -
        var(--mw-component-message-gutter) - var(--mw-component-message-reply-spine-width) / 2
    );
    top: calc(var(--mw-component-message-reply-height) / 2);
    width: calc(
      var(--mw-component-message-gutter) - var(--mw-component-message-avatar-inset) -
        var(--mw-component-message-avatar-size) / 2 - var(--mw-space-xs) + var(--mw-component-message-reply-spine-width)
    );
    height: calc(
      var(--mw-component-message-reply-height) / 2 + var(--mw-space-2xs) + var(--mw-component-message-reply-spine-width) / 2
    );
    border-top: var(--mw-component-message-reply-spine-width) solid var(--mw-component-message-reply-spine-color);
    border-left: var(--mw-component-message-reply-spine-width) solid var(--mw-component-message-reply-spine-color);
    border-top-left-radius: var(--mw-component-message-reply-spine-radius);
  }
</style>
