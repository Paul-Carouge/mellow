<script lang="ts">
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import ModalDialog from "./ModalDialog.svelte";

  // "Leave server" confirmation, the destructive pattern of DESIGN.md §8: Cancel takes focus.
  interface Props {
    client: MellowClient;
    serverName: string;
    onclose: () => void;
    fallbackFocus: () => void;
  }

  let { client, serverName, onclose, fallbackFocus }: Props = $props();

  let isBusy = $state(false);
  let errorMessage = $state("");

  async function leave() {
    isBusy = true;
    errorMessage = "";
    try {
      await client.leaveServer();
      onclose();
    } catch (error) {
      errorMessage = describeError(error, "Couldn't leave the server");
    } finally {
      isBusy = false;
    }
  }
</script>

<ModalDialog
  title="Leave '{serverName}'"
  subtitle="Are you sure you want to leave {serverName}? You won't be able to rejoin unless you are re-invited."
  {errorMessage}
  {isBusy}
  submitLabel={isBusy ? "Leaving…" : "Leave server"}
  isSubmitDanger
  {onclose}
  onsubmit={leave}
  {fallbackFocus}
/>
