<script lang="ts">
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
      errorMessage = describeError(error, "Couldn't create the channel");
    } finally {
      isBusy = false;
    }
  }
</script>

<ModalDialog
  shouldReturnFocusToOpener={() => !hasSucceeded}
  title="Create channel"
  subtitle={category ? `in ${category.name}` : ""}
  {errorMessage}
  {isBusy}
  submitLabel={isBusy ? "Creating…" : "Create channel"}
  isSubmitEnabled={!isBusy && channelName.trim() !== ""}
  {onclose}
  onsubmit={submit}
  {fallbackFocus}
>
  <TextField
    id="create-channel-name"
    label="Channel name"
    placeholder="new-channel"
    helper="Names are lowercase, with dashes instead of spaces."
    leadingIcon="hash"
    autocomplete="off"
    readonly={isBusy}
    data-initial-focus=""
    bind:value={channelName}
  />
</ModalDialog>
