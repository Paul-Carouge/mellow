<script lang="ts">
  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import { type Guild, MAXIMUM_SERVER_NAME_LENGTH } from "../lib/protocol";
  import Icon from "./Icon.svelte";
  import TextField from "./TextField.svelte";
  import UnsavedChangesBar from "./UnsavedChangesBar.svelte";

  // Overview (DESIGN.md §5.8): the server's name, saved through the unsaved-changes bar. A rename
  // by someone else while nothing is typed shows at once; a draft is kept until saved or reset.
  interface Props {
    client: MellowClient;
    guild: Guild;
    attention: number;
    hasUnsavedChanges: boolean;
  }

  let { client, guild, attention, hasUnsavedChanges = $bindable() }: Props = $props();

  /** Null while the field shows the stored name. */
  let draft = $state<string | null>(null);
  let isSaving = $state(false);
  let errorMessage = $state("");
  let notice = $state("");

  const id = $props.id();
  const name = $derived(draft ?? guild.name);
  const isChanged = $derived(draft !== null && draft.trim() !== guild.name);

  $effect(() => {
    hasUnsavedChanges = isChanged;
  });

  function reset() {
    draft = null;
    errorMessage = "";
  }

  async function save() {
    if (draft === null || isSaving) {
      return;
    }
    if (draft.trim() === "") {
      errorMessage = strings.serverSettings.overview.nameRequired;
      return;
    }
    isSaving = true;
    errorMessage = "";
    try {
      await client.renameServer(guild.id, draft.trim());
      draft = null;
      notice = strings.serverSettings.unsaved.saved;
    } catch (error) {
      errorMessage = describeError(error, strings.serverSettings.overview.saveFailed);
    } finally {
      isSaving = false;
    }
  }
</script>

<form
  class="flex flex-1 flex-col"
  onsubmit={(event) => {
    event.preventDefault();
    void save();
  }}
>
  <div class="max-w-(--mw-component-dialog-width)">
    <TextField
      id="{id}-name"
      label={strings.serverSettings.overview.serverName}
      helper={strings.serverSettings.overview.serverNameHelper}
      maxlength={MAXIMUM_SERVER_NAME_LENGTH}
      autocomplete="off"
      bind:value={() => name, (value) => {
        draft = value;
        notice = "";
      }}
    />
  </div>
  {#if notice && !isChanged}
    <p aria-hidden="true" class="mt-(--mw-component-settings-action-gap) flex gap-(--mw-component-settings-error-icon-gap) type-body text-(--mw-component-settings-notice-text)">
      <Icon name="check" class="size-(--mw-component-settings-error-icon-size) shrink-0" />
      <span>{notice}</span>
    </p>
  {/if}
  <p role="status" class="sr-only">{notice}</p>
  <div class="flex-1"></div>
  {#if isChanged}
    <UnsavedChangesBar {attention} {isSaving} {errorMessage} onreset={reset} onsave={() => void save()} />
  {/if}
</form>
