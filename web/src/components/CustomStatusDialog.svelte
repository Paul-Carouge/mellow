<script lang="ts">
  import { untrack } from "svelte";

  import { strings } from "../i18n";
  import { ApiError } from "../lib/api";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import Button from "./Button.svelte";
  import { customStatusCharactersLeft } from "./custom-status";
  import ModalDialog from "./ModalDialog.svelte";
  import TextField from "./TextField.svelte";

  // The custom status dialog (DESIGN.md §5.3), opened from the user card's status menu: one field,
  // Save status, Cancel, and Clear status once there is one. Enter saves, Esc cancels. The status
  // shape stays as it is; only the sentence changes.
  interface Props {
    client: MellowClient;
    onclose: () => void;
    /** The user card's button, since the menu item that opened the dialog is gone. */
    fallbackFocus: () => void;
  }

  let { client, onclose, fallbackFocus }: Props = $props();

  // Read once: the field is the person's draft from here on.
  const savedCustomStatus = untrack(() => client.user?.custom_status ?? "");
  let draft = $state(savedCustomStatus);
  let isSaving = $state(false);
  let errorMessage = $state("");

  const charactersLeft = $derived(customStatusCharactersLeft(draft));
  const hasChanged = $derived(draft.trim() !== savedCustomStatus);

  async function save(customStatus: string | null) {
    isSaving = true;
    errorMessage = "";
    try {
      await client.updatePresence(client.user?.status ?? "online", customStatus);
      onclose();
    } catch (error) {
      errorMessage =
        error instanceof ApiError && error.code === "VALIDATION_FAILED"
          ? strings.customStatus.refused
          : describeError(error, strings.customStatus.failed);
    } finally {
      isSaving = false;
    }
  }
</script>

<ModalDialog
  title={savedCustomStatus ? strings.userCard.editCustomStatus : strings.userCard.setCustomStatus}
  submitLabel={strings.customStatus.save}
  isSubmitEnabled={hasChanged && charactersLeft >= 0}
  isBusy={isSaving}
  {errorMessage}
  {onclose}
  {fallbackFocus}
  onsubmit={() => void save(draft)}
>
  <div class="flex flex-col items-start gap-(--mw-component-text-field-helper-gap)">
    <div class="w-full">
      <TextField
        id="custom-status"
        label={strings.customStatus.field}
        bind:value={draft}
        placeholder={strings.customStatus.placeholder}
        helper={strings.customStatus.charactersLeft(charactersLeft)}
        autocomplete="off"
        enterkeyhint="done"
        readonly={isSaving}
        aria-invalid={charactersLeft < 0 || undefined}
        data-initial-focus=""
      />
    </div>
    {#if savedCustomStatus}
      <!-- Its label lines up with the field's edge; the hover wash still reaches past it. -->
      <div class="-ms-(--mw-component-button-padding-inline)">
        <Button variant="ghost" isDisabled={isSaving} onclick={() => void save(null)}>{strings.customStatus.clear}</Button>
      </div>
    {/if}
  </div>
</ModalDialog>
