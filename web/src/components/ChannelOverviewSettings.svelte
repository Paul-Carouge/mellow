<script lang="ts">
  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import { type Channel, MAXIMUM_CHANNEL_NAME_LENGTH, MAXIMUM_CHANNEL_TOPIC_LENGTH } from "../lib/protocol";
  import { channelNameAsSaved } from "./channel-settings";
  import Icon from "./Icon.svelte";
  import IconEditor from "./IconEditor.svelte";
  import TextField from "./TextField.svelte";
  import UnsavedChangesBar from "./UnsavedChangesBar.svelte";

  // Channel Overview (DESIGN.md §5.8): the icon, saved at once, then the name and the topic, saved
  // together through the unsaved-changes bar. A change by someone else while nothing is typed shows
  // at once; a draft is kept until saved or reset.
  interface Props {
    client: MellowClient;
    channel: Channel;
    attention: number;
    /** True while a field differs from the stored channel; false again when this page goes away. */
    onunsavedchange: (hasChanges: boolean) => void;
  }

  let { client, channel, attention, onunsavedchange }: Props = $props();

  interface Fields {
    name: string;
    topic: string;
  }

  /** Null while the fields show the stored channel. */
  let draft = $state<Fields | null>(null);
  /** Saved, and waiting for the gateway to bring it: not an unsaved change any more. */
  let saved = $state<Fields | null>(null);
  let isSaving = $state(false);
  let errorMessage = $state("");
  let notice = $state("");

  const id = $props.id();
  const shown = $derived(draft ?? { name: channel.name, topic: channel.topic });

  /** The instance lowercases names, dashes their spaces and trims both fields. */
  function isStored(fields: Fields): boolean {
    return channelNameAsSaved(fields.name) === channel.name && fields.topic.trim() === channel.topic;
  }

  const isChanged = $derived(
    draft !== null && !isStored(draft) && !(saved !== null && saved.name === draft.name && saved.topic === draft.topic),
  );

  $effect(() => {
    onunsavedchange(isChanged);
    return () => onunsavedchange(false);
  });

  // The stored channel caught up with the fields (our save, echoed by the gateway).
  $effect(() => {
    if (draft !== null && isStored(draft)) {
      draft = null;
      saved = null;
    }
  });

  function edit(fields: Fields) {
    draft = fields;
    notice = "";
  }

  function reset() {
    draft = null;
    saved = null;
    errorMessage = "";
  }

  async function save() {
    if (draft === null || isSaving) {
      return;
    }
    const name = channelNameAsSaved(draft.name);
    if (name === "") {
      errorMessage = strings.channelSettings.overview.nameRequired;
      return;
    }
    const topic = draft.topic.trim();
    isSaving = true;
    errorMessage = "";
    const fields = draft;
    try {
      await client.updateChannel(channel.id, {
        ...(name !== channel.name ? { name } : {}),
        ...(topic !== channel.topic ? { topic } : {}),
      });
      // The fields keep the saved text until the gateway brings it.
      saved = fields;
      notice = strings.serverSettings.unsaved.saved;
    } catch (error) {
      errorMessage = describeError(error, strings.channelSettings.overview.saveFailed);
    } finally {
      isSaving = false;
    }
  }
</script>

<IconEditor
  kind="channel"
  icon={channel.icon}
  name={channel.name}
  onupload={(file, crop) => client.setChannelIcon(channel.id, file, crop)}
  onremove={() => client.removeChannelIcon(channel.id)}
/>

<form
  class="mt-(--mw-component-settings-section-gap) flex flex-1 flex-col"
  onsubmit={(event) => {
    event.preventDefault();
    void save();
  }}
>
  <div class="flex max-w-(--mw-component-dialog-width) flex-col gap-(--mw-component-settings-section-gap)">
    <TextField
      id="{id}-name"
      label={strings.channelSettings.overview.channelName}
      helper={strings.channelSettings.overview.channelNameHelper}
      leadingIcon="hash"
      maxlength={MAXIMUM_CHANNEL_NAME_LENGTH}
      autocomplete="off"
      bind:value={() => shown.name, (name) => edit({ ...shown, name })}
    />
    <TextField
      id="{id}-topic"
      label={strings.channelSettings.overview.topic}
      helper={strings.channelSettings.overview.topicHelper}
      maxlength={MAXIMUM_CHANNEL_TOPIC_LENGTH}
      autocomplete="off"
      bind:value={() => shown.topic, (topic) => edit({ ...shown, topic })}
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
