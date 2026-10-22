<script lang="ts">
  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { Guild } from "../lib/protocol";
  import ModalDialog from "./ModalDialog.svelte";
  import TextField from "./TextField.svelte";

  // "Delete server", the owner's last action (DESIGN.md §5.8, §8): irreversible for everyone, so
  // the server's name must be typed before the danger button wakes up. Cancel takes focus.
  interface Props {
    client: MellowClient;
    guild: Guild;
    onclose: () => void;
  }

  let { client, guild, onclose }: Props = $props();

  const id = $props.id();
  let typedName = $state("");
  let isBusy = $state(false);
  let errorMessage = $state("");

  // Trimmed, as the server stores names: what the owner sees is what they type.
  const isConfirmed = $derived(typedName.trim() === guild.name);

  async function deleteServer() {
    isBusy = true;
    errorMessage = "";
    try {
      // The settings close with the server; this dialog goes with them.
      await client.deleteServer(guild.id);
    } catch (error) {
      errorMessage = describeError(error, strings.serverSettings.deleteServerDialog.failed);
      isBusy = false;
    }
  }
</script>

<ModalDialog
  title={strings.serverSettings.deleteServerDialog.title(guild.name)}
  subtitle={strings.serverSettings.deleteServerDialog.body(guild.name)}
  {errorMessage}
  {isBusy}
  submitLabel={isBusy ? strings.serverSettings.deleteServerDialog.deleting : strings.serverSettings.deleteServerDialog.submit}
  isSubmitDanger
  isSubmitEnabled={isConfirmed}
  {onclose}
  onsubmit={() => void deleteServer()}
>
  <TextField
    id="{id}-name"
    label={strings.serverSettings.deleteServerDialog.confirmLabel}
    autocomplete="off"
    spellcheck={false}
    bind:value={typedName}
  />
</ModalDialog>
