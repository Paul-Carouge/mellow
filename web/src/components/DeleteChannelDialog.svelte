<script lang="ts">
  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { Channel } from "../lib/protocol";
  import ModalDialog from "./ModalDialog.svelte";
  import TextField from "./TextField.svelte";

  // "Delete channel" and "Delete category" (DESIGN.md §5.3, §8). A channel takes every message
  // with it, for everyone: its name must be typed first. A category loses nothing but its heading
  // (its channels stay, uncategorised), so Cancel and Delete are enough. Cancel takes focus.
  interface Props {
    client: MellowClient;
    channel: Channel;
    onclose: () => void;
    /** After the instance agreed: the result's sentence, for a live region. */
    ondone?: (announcement: string) => void;
    fallbackFocus?: () => void;
  }

  let { client, channel, onclose, ondone, fallbackFocus }: Props = $props();

  const id = $props.id();
  const isCategory = $derived(channel.kind === "category");
  let typedName = $state("");
  let isBusy = $state(false);
  let errorMessage = $state("");
  // Plain: read by the dialog when it closes, after this component stopped updating.
  let hasSucceeded = false;

  // Trimmed, as the instance stores names: what the reader sees is what they type.
  const isConfirmed = $derived(isCategory || typedName.trim() === channel.name);

  async function deleteChannel() {
    const { guild_id: guildId, id: channelId, name } = channel;
    const wasCategory = isCategory;
    if (guildId === null) {
      return;
    }
    isBusy = true;
    errorMessage = "";
    try {
      await client.deleteChannel(guildId, channelId);
      hasSucceeded = true;
      ondone?.(wasCategory ? strings.channelRemoval.categoryDeleted(name) : strings.channelRemoval.channelDeleted(name));
      onclose();
    } catch (error) {
      errorMessage = describeError(
        error,
        wasCategory ? strings.channelRemoval.categoryDeleteFailed : strings.channelRemoval.channelDeleteFailed,
      );
    } finally {
      isBusy = false;
    }
  }
</script>

<ModalDialog
  shouldReturnFocusToOpener={() => !hasSucceeded}
  title={isCategory ? strings.channelRemoval.categoryTitle(channel.name) : strings.channelRemoval.channelTitle(channel.name)}
  subtitle={isCategory ? strings.channelRemoval.categoryBody : strings.channelRemoval.channelBody}
  {errorMessage}
  {isBusy}
  submitLabel={isBusy
    ? strings.channelRemoval.deleting
    : isCategory
      ? strings.channelRemoval.deleteCategory
      : strings.channelRemoval.deleteChannel}
  isSubmitDanger
  isSubmitEnabled={isConfirmed}
  {onclose}
  onsubmit={() => void deleteChannel()}
  {fallbackFocus}
>
  {#if !isCategory}
    <TextField
      id="{id}-name"
      label={strings.channelRemoval.confirmLabel(channel.name)}
      autocomplete="off"
      spellcheck={false}
      readonly={isBusy}
      bind:value={typedName}
    />
  {/if}
</ModalDialog>
