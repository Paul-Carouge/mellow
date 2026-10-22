<script lang="ts">
  import type { Message } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import { displayName } from "./format";

  // The message a dialog acts on, as a framed preview (DESIGN.md §6.35): author and plain text.
  interface Props {
    message: Message;
  }

  let { message }: Props = $props();
</script>

<div
  class="flex gap-(--mw-component-dialog-preview-padding) rounded-(--mw-component-dialog-preview-radius) border-(length:--mw-component-dialog-preview-border-width) border-(--mw-component-dialog-preview-border) p-(--mw-component-dialog-preview-padding)"
>
  <Avatar user={message.author} sizeToken="--mw-component-dialog-preview-avatar-size" />
  <div class="min-w-0 flex-1">
    <p class="truncate type-message-author text-(--mw-component-message-author-text)">
      {displayName(message.author)}
    </p>
    <p
      class="max-h-(--mw-component-dialog-preview-body-max-height) overflow-hidden type-message break-words whitespace-pre-wrap text-(--mw-component-message-body-text)"
    >
      {message.content}
    </p>
  </div>
</div>
