<script lang="ts">
  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
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

  // Plain: read by the dialog when it closes, after this component stopped updating.
  let hasSucceeded = false;

  async function leave() {
    await client.leaveServer(guildId);
    hasSucceeded = true;
    onclose();
  }
</script>

<ModalDialog
  shouldReturnFocusToOpener={() => !hasSucceeded}
  title={strings.leaveServer.title(serverName)}
  subtitle={strings.leaveServer.subtitle(serverName)}
  submitLabel={strings.leaveServer.submit}
  busySubmitLabel={strings.leaveServer.leaving}
  failureSummary={strings.leaveServer.failed}
  isSubmitDanger
  {onclose}
  onsubmit={leave}
  {fallbackFocus}
/>
