<script lang="ts">
  import { untrack } from "svelte";

  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
  import { type Channel, MAXIMUM_CHANNEL_NAME_LENGTH } from "../lib/protocol";
  import ModalDialog from "./ModalDialog.svelte";
  import TextField from "./TextField.svelte";

  // "Rename category" (DESIGN.md §5.3): categories have no settings page, so their menu renames
  // them in one field. The name is kept as typed; the gateway brings it to everyone.
  interface Props {
    client: MellowClient;
    category: Channel;
    onclose: () => void;
    /** After the instance agreed: the result's sentence, for a live region. */
    ondone: (announcement: string) => void;
  }

  let { client, category, onclose, ondone }: Props = $props();

  const id = $props.id();
  // The name as it was when the dialog opened; someone else renaming meanwhile does not erase typing.
  let name = $state(untrack(() => category.name));

  async function rename() {
    const trimmed = name.trim();
    if (trimmed === category.name) {
      onclose();
      return;
    }
    await client.updateChannel(category.id, { name: trimmed });
    ondone(strings.channelRemoval.categoryRenamed(trimmed));
    onclose();
  }
</script>

<ModalDialog
  title={strings.channelRemoval.renameCategory}
  submitLabel={strings.channelRemoval.renameCategory}
  busySubmitLabel={strings.channelRemoval.renaming}
  failureSummary={strings.channelRemoval.renameFailed}
  isSubmitEnabled={name.trim() !== ""}
  {onclose}
  onsubmit={rename}
>
  {#snippet children(isBusy)}
    <TextField
      id="{id}-name"
      label={strings.createChannel.categoryName}
      maxlength={MAXIMUM_CHANNEL_NAME_LENGTH}
      autocomplete="off"
      readonly={isBusy}
      data-initial-focus=""
      bind:value={name}
    />
  {/snippet}
</ModalDialog>
