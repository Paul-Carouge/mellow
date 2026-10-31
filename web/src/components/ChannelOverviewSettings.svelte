<script lang="ts">
  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import { type Channel, MAXIMUM_CHANNEL_NAME_LENGTH, MAXIMUM_CHANNEL_TOPIC_LENGTH } from "../lib/protocol";
  import { type ChannelOverviewDrafts, channelNameAsSaved, channelOverviewChanges, userLimitChoiceOf } from "./channel-settings";
  import Icon from "./Icon.svelte";
  import IconEditor from "./IconEditor.svelte";
  import TextField from "./TextField.svelte";
  import UnsavedChangesBar from "./UnsavedChangesBar.svelte";
  import UserLimitField from "./UserLimitField.svelte";

  // Channel Overview (DESIGN.md §5.8): the icon, saved at once, then the name and the topic (a voice
  // channel: its user limit), saved together through the unsaved-changes bar. A change by someone else to a field not typed in shows
  // at once; a typed field is kept until saved or reset, and only typed fields are sent.
  interface Props {
    client: MellowClient;
    channel: Channel;
    attention: number;
    /** True while a field differs from the stored channel; false again when this page goes away. */
    onunsavedchange: (hasChanges: boolean) => void;
  }

  let { client, channel, attention, onunsavedchange }: Props = $props();

  /** Each field typed and not yet stored, apart: a change by someone else to the other one shows. */
  let drafts = $state<ChannelOverviewDrafts>({ name: null, topic: null, userLimit: null });
  /** Saved, and waiting for the gateway to bring it: not an unsaved change any more. */
  let saved = $state<ChannelOverviewDrafts | null>(null);
  let isSaving = $state(false);
  let errorMessage = $state("");
  let notice = $state("");

  const id = $props.id();
  const shown = $derived({
    name: drafts.name ?? channel.name,
    topic: drafts.topic ?? channel.topic,
    userLimit: drafts.userLimit ?? userLimitChoiceOf(channel),
  });
  const changes = $derived(channelOverviewChanges(channel, drafts));

  const isChanged = $derived(
    Object.keys(changes).length > 0 &&
      !(saved !== null && saved.name === drafts.name && saved.topic === drafts.topic && saved.userLimit === drafts.userLimit),
  );

  $effect(() => {
    onunsavedchange(isChanged);
    return () => onunsavedchange(false);
  });

  // The stored channel caught up with a field (our save, echoed by the gateway): it shows the
  // stored value again. The instance trims both fields and keeps names as typed otherwise.
  $effect(() => {
    if (drafts.name !== null && channelNameAsSaved(drafts.name) === channel.name) {
      drafts.name = null;
    }
    if (drafts.topic !== null && drafts.topic.trim() === channel.topic) {
      drafts.topic = null;
    }
    if (drafts.userLimit != null && drafts.userLimit === userLimitChoiceOf(channel)) {
      drafts.userLimit = null;
    }
    if (drafts.name === null && drafts.topic === null && drafts.userLimit == null) {
      saved = null;
    }
  });

  function edit(field: "name" | "topic", value: string) {
    drafts[field] = value;
    notice = "";
  }

  function reset() {
    drafts = { name: null, topic: null, userLimit: null };
    saved = null;
    errorMessage = "";
  }

  async function save() {
    if (!isChanged || isSaving) {
      return;
    }
    if (changes.name === "") {
      errorMessage = strings.channelSettings.overview.nameRequired;
      return;
    }
    isSaving = true;
    errorMessage = "";
    const sent = { ...drafts };
    try {
      await client.updateChannel(channel.id, changes);
      // The fields keep the saved text until the gateway brings it.
      saved = sent;
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
      leadingIcon={channel.kind === "voice" ? "speaker-high" : "hash"}
      maxlength={MAXIMUM_CHANNEL_NAME_LENGTH}
      autocomplete="off"
      bind:value={() => shown.name, (name) => edit("name", name)}
    />
    {#if channel.kind === "voice"}
      <UserLimitField
        value={shown.userLimit}
        onchange={(choice) => {
          drafts.userLimit = choice;
          notice = "";
        }}
      />
    {:else}
      <TextField
        id="{id}-topic"
        label={strings.channelSettings.overview.topic}
        helper={strings.channelSettings.overview.topicHelper}
        maxlength={MAXIMUM_CHANNEL_TOPIC_LENGTH}
        autocomplete="off"
        bind:value={() => shown.topic, (topic) => edit("topic", topic)}
      />
    {/if}
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
