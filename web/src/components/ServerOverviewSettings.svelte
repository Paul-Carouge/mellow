<script lang="ts">
  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import { type Guild, MAXIMUM_SERVER_NAME_LENGTH, iconUrl } from "../lib/protocol";
  import Icon from "./Icon.svelte";
  import IconEditor from "./IconEditor.svelte";
  import ServerPreview from "./ServerPreview.svelte";
  import TextField from "./TextField.svelte";
  import UnsavedChangesBar from "./UnsavedChangesBar.svelte";

  // Overview (DESIGN.md §5.8): the server's icon, saved at once, then its name, saved through the
  // unsaved-changes bar. A rename by someone else while nothing is typed shows at once; a draft is
  // kept until saved or reset.
  interface Props {
    client: MellowClient;
    guild: Guild;
    attention: number;
    /** True while the name differs from the stored one; false again when this page goes away. */
    onunsavedchange: (hasChanges: boolean) => void;
  }

  let { client, guild, attention, onunsavedchange }: Props = $props();

  /** Null while the field shows the stored name. */
  let draft = $state<string | null>(null);
  let isSaving = $state(false);
  let errorMessage = $state("");
  let notice = $state("");
  /** Saved, and waiting for the gateway to bring it: not an unsaved change any more. */
  let savedName = $state<string | null>(null);

  const id = $props.id();
  const name = $derived(draft ?? guild.name);
  const isChanged = $derived(draft !== null && draft.trim() !== guild.name && draft !== savedName);
  /** The stored icon is already the cropped square, so it fills the preview's circle as it is. */
  const previewIcon = $derived(
    guild.icon ? { url: iconUrl(guild.icon, false), placement: { left: 0, top: 0, width: 100 } } : null,
  );

  $effect(() => {
    onunsavedchange(isChanged);
    return () => onunsavedchange(false);
  });

  // The stored name caught up with the field (our save, echoed by the gateway).
  $effect(() => {
    if (draft !== null && draft === guild.name) {
      draft = null;
      savedName = null;
    }
  });

  function reset() {
    draft = null;
    savedName = null;
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
    const name = draft.trim();
    try {
      await client.renameServer(guild.id, name);
      // The field keeps the saved name until the gateway brings it.
      draft = name;
      savedName = name;
      notice = strings.serverSettings.unsaved.saved;
    } catch (error) {
      errorMessage = describeError(error, strings.serverSettings.overview.saveFailed);
    } finally {
      isSaving = false;
    }
  }
</script>

<IconEditor
  kind="server"
  icon={guild.icon}
  name={guild.name}
  onupload={(file, crop) => client.setServerIcon(guild.id, file, crop)}
  onremove={() => client.removeServerIcon(guild.id)}
/>

<form
  class="mt-(--mw-component-settings-section-gap) flex flex-1 flex-col"
  onsubmit={(event) => {
    event.preventDefault();
    void save();
  }}
>
  <!-- Side by side while both fit, as in "Create a server"; the preview wraps under on a phone. -->
  <div class="flex flex-wrap items-start gap-(--mw-component-dialog-section-gap)">
    <div class="max-w-(--mw-component-dialog-width) min-w-0 flex-[999_1_var(--mw-component-server-preview-width)]">
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
    <div class="flex min-w-0 flex-[1_1_var(--mw-component-server-preview-width)] flex-col gap-(--mw-component-text-field-label-gap)">
      <p class="type-category text-(--mw-component-text-field-label-text)">{strings.addServer.preview}</p>
      <ServerPreview {name} icon={previewIcon} categories={[]} />
    </div>
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
