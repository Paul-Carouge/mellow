<script lang="ts">
  import { strings } from "../i18n";
  import type { MellowClient } from "../lib/client.svelte";
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

  // Trimmed, as the server stores names: what the owner sees is what they type.
  const isConfirmed = $derived(typedName.trim() === guild.name);
</script>

<!-- The settings close with the server once it is deleted; this dialog goes with them. -->
<ModalDialog
  title={strings.serverSettings.deleteServerDialog.title(guild.name)}
  subtitle={strings.serverSettings.deleteServerDialog.body(guild.name)}
  submitLabel={strings.serverSettings.deleteServerDialog.submit}
  busySubmitLabel={strings.serverSettings.deleteServerDialog.deleting}
  failureSummary={strings.serverSettings.deleteServerDialog.failed}
  isSubmitDanger
  isSubmitEnabled={isConfirmed}
  {onclose}
  onsubmit={() => client.deleteServer(guild.id)}
>
  <TextField
    id="{id}-name"
    label={strings.serverSettings.deleteServerDialog.confirmLabel}
    autocomplete="off"
    spellcheck={false}
    bind:value={typedName}
  />
</ModalDialog>
