<script lang="ts">
  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { Channel } from "../lib/protocol";
  import ModalDialog from "./ModalDialog.svelte";
  import TextField from "./TextField.svelte";

  // Discord's "Create channel" dialog, text channels only for now (DESIGN.md §5.3, §8).
  interface Props {
    client: MellowClient;
    /** From a category's "+": the channel goes in it, and the subtitle says so. */
    category?: Channel | null;
    onclose: () => void;
    fallbackFocus: () => void;
  }

  let { client, category = null, onclose, fallbackFocus }: Props = $props();

  let channelName = $state("");
  let isBusy = $state(false);
  // Plain: read by the dialog when it closes, after this component stopped updating.
  let hasSucceeded = false;
  let errorMessage = $state("");

  async function submit() {
    isBusy = true;
    errorMessage = "";
    try {
      await client.createChannel(channelName.trim(), category?.id ?? null);
      hasSucceeded = true;
      onclose();
    } catch (error) {
      errorMessage = describeError(error, strings.createChannel.failed);
    } finally {
      isBusy = false;
    }
  }
</script>

<ModalDialog
  shouldReturnFocusToOpener={() => !hasSucceeded}
  title={strings.createChannel.title}
  subtitle={category ? strings.createChannel.inCategory(category.name) : ""}
  {errorMessage}
  {isBusy}
  submitLabel={isBusy ? strings.createChannel.creating : strings.createChannel.submit}
  isSubmitEnabled={!isBusy && channelName.trim() !== ""}
  {onclose}
  onsubmit={submit}
  {fallbackFocus}
>
  <TextField
    id="create-channel-name"
    label={strings.createChannel.channelName}
    placeholder={strings.createChannel.placeholder}
    helper={strings.createChannel.helper}
    leadingIcon="hash"
    autocomplete="off"
    readonly={isBusy}
    data-initial-focus=""
    bind:value={channelName}
  />
</ModalDialog>
