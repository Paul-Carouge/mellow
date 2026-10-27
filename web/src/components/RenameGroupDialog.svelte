<script lang="ts">
  import { untrack } from "svelte";

  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import { type DirectMessageChannel, MAXIMUM_GROUP_CONVERSATION_NAME_LENGTH } from "../lib/protocol";
  import { participantNamesOf } from "./group-conversations";
  import ModalDialog from "./ModalDialog.svelte";
  import TextField from "./TextField.svelte";

  // "Rename group" (DESIGN.md §5.6, ADR-0026): anyone in it renames it; an empty name brings back
  // the people's names, which the field shows as its placeholder.
  interface Props {
    client: MellowClient;
    group: DirectMessageChannel;
    onclose: () => void;
    /** After the instance agreed: the result's sentence, for a live region. */
    ondone: (announcement: string) => void;
  }

  let { client, group, onclose, ondone }: Props = $props();

  const id = $props.id();
  const currentUserId = $derived(client.user?.id ?? null);
  // The name as it was when the dialog opened; someone else renaming meanwhile does not erase typing.
  let name = $state(untrack(() => group.name ?? ""));
  let isBusy = $state(false);
  let errorMessage = $state("");

  async function save() {
    const trimmed = name.trim();
    if (trimmed === (group.name ?? "").trim()) {
      onclose();
      return;
    }
    isBusy = true;
    errorMessage = "";
    try {
      await client.renameGroupConversation(group.id, trimmed);
      ondone(trimmed ? strings.groups.renamed(trimmed) : strings.groups.nameCleared);
      onclose();
    } catch (error) {
      errorMessage = describeError(error, strings.groups.renameFailed);
    } finally {
      isBusy = false;
    }
  }
</script>

<ModalDialog
  title={strings.groups.rename}
  {errorMessage}
  {isBusy}
  submitLabel={isBusy ? strings.common.saving : strings.groups.saveName}
  {onclose}
  onsubmit={() => void save()}
>
  <TextField
    id="{id}-name"
    label={strings.groups.nameLabel}
    bind:value={name}
    helper={strings.groups.groupNameHelper}
    placeholder={participantNamesOf(group, currentUserId)}
    maxlength={MAXIMUM_GROUP_CONVERSATION_NAME_LENGTH}
    autocomplete="off"
    data-initial-focus=""
  />
</ModalDialog>
