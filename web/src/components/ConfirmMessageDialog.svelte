<script lang="ts">
  import type { Message, Role, Snowflake } from "../lib/protocol";
  import MessagePreview from "./MessagePreview.svelte";
  import ModalDialog from "./ModalDialog.svelte";

  // A confirmation about one message, with its preview: pin, unpin, remove all reactions.
  interface Props {
    message: Message;
    roles: ReadonlyMap<Snowflake, Role>;
    title: string;
    subtitle: string;
    submitLabel: string;
    isDanger?: boolean;
    onconfirm: () => void;
    onclose: () => void;
    fallbackFocus: () => void;
  }

  let { message, roles, title, subtitle, submitLabel, isDanger = false, onconfirm, onclose, fallbackFocus }: Props =
    $props();
</script>

<ModalDialog {title} {subtitle} {submitLabel} isSubmitDanger={isDanger} {onclose} onsubmit={onconfirm} {fallbackFocus}>
  <MessagePreview {message} {roles} />
</ModalDialog>
