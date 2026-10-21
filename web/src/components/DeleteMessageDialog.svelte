<script lang="ts">
  import type { Message } from "../lib/protocol";
  import Avatar from "./Avatar.svelte";
  import { displayName } from "./format";
  import ModalDialog from "./ModalDialog.svelte";

  // "Delete this message?" (DESIGN.md §5.4, §8): the message as a preview, Cancel focused first.
  interface Props {
    message: Message;
    onconfirm: () => void;
    onclose: () => void;
    fallbackFocus: () => void;
  }

  let { message, onconfirm, onclose, fallbackFocus }: Props = $props();
</script>

<ModalDialog
  title="Delete this message?"
  subtitle="It will be removed for everyone."
  submitLabel="Delete message"
  isSubmitDanger
  {onclose}
  onsubmit={onconfirm}
  {fallbackFocus}
>
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
  <p class="type-body-sm text-(--mw-component-dialog-hint-text)">Tip: hold Shift while deleting to skip this next time.</p>
</ModalDialog>
