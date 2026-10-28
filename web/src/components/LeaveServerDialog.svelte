<script lang="ts">
  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { Snowflake } from "../lib/protocol";
  import ModalDialog from "./ModalDialog.svelte";

  // "Leave server" confirmation, the destructive pattern of DESIGN.md §8: Cancel takes focus.
  interface Props {
    client: MellowClient;
    guildId: Snowflake;
    serverName: string;
    onclose: () => void;
    fallbackFocus: () => void;
  }

  let { client, guildId, serverName, onclose, fallbackFocus }: Props = $props();

  let isBusy = $state(false);
  // Plain: read by the dialog when it closes, after this component stopped updating.
  let hasSucceeded = false;
  let errorMessage = $state("");

  async function leave() {
    isBusy = true;
    errorMessage = "";
    try {
      await client.leaveServer(guildId);
      hasSucceeded = true;
      onclose();
    } catch (error) {
      errorMessage = describeError(error, strings.leaveServer.failed);
    } finally {
      isBusy = false;
    }
  }
</script>

<ModalDialog
  shouldReturnFocusToOpener={() => !hasSucceeded}
  title={strings.leaveServer.title(serverName)}
  subtitle={strings.leaveServer.subtitle(serverName)}
  {errorMessage}
  {isBusy}
  submitLabel={isBusy ? strings.leaveServer.leaving : strings.leaveServer.submit}
  isSubmitDanger
  {onclose}
  onsubmit={leave}
  {fallbackFocus}
/>
