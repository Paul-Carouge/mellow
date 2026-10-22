<script lang="ts">
  import { strings } from "../i18n";
  import type { Message } from "../lib/protocol";
  import MessagePreview from "./MessagePreview.svelte";
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
  title={strings.deleteMessage.title}
  subtitle={strings.deleteMessage.subtitle}
  submitLabel={strings.deleteMessage.submit}
  isSubmitDanger
  {onclose}
  onsubmit={onconfirm}
  {fallbackFocus}
>
  <MessagePreview {message} />
  <p class="type-body-sm text-(--mw-component-dialog-hint-text)">{strings.deleteMessage.tip}</p>
</ModalDialog>
