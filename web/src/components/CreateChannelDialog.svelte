<script lang="ts">
  import { untrack } from "svelte";

  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import { type Channel, type ChannelKind, MAXIMUM_CHANNEL_NAME_LENGTH, MAXIMUM_CHANNEL_TOPIC_LENGTH } from "../lib/protocol";
  import type { VoiceUserLimitChoice } from "./channel-settings";
  import ChoiceOption from "./ChoiceOption.svelte";
  import ModalDialog from "./ModalDialog.svelte";
  import TextField from "./TextField.svelte";
  import UserLimitField from "./UserLimitField.svelte";

  // Discord's "Create channel" (DESIGN.md §5.3, §8): a text channel, a voice channel or a
  // category, its name kept as typed, an optional topic for a text channel and an optional user
  // limit for a voice channel. From a category's "+" the channel goes in it, so Category is not
  // offered: a category cannot hold another.
  interface Props {
    client: MellowClient;
    /** From a category's "+": the channel goes in it, and the subtitle says so. */
    category?: Channel | null;
    /** "Create category" in the server menu opens on Category. */
    initialKind?: ChannelKind;
    onclose: () => void;
    fallbackFocus: () => void;
  }

  let { client, category = null, initialKind = "text", onclose, fallbackFocus }: Props = $props();

  const id = $props.id();
  let kind = $state<ChannelKind>(untrack(() => (category ? "text" : initialKind)));
  let name = $state("");
  let topic = $state("");
  let userLimit = $state<VoiceUserLimitChoice>("unlimited");
  let isBusy = $state(false);
  // Plain: read by the dialog when it closes, after this component stopped updating.
  let hasSucceeded = false;
  let errorMessage = $state("");

  const isCategory = $derived(kind === "category");
  const isVoice = $derived(kind === "voice");

  async function submit() {
    const kindToCreate = kind;
    isBusy = true;
    errorMessage = "";
    try {
      await client.createChannel(
        kindToCreate,
        name.trim(),
        kindToCreate === "text" ? topic.trim() : "",
        kindToCreate === "category" ? null : (category?.id ?? null),
        kindToCreate === "voice" && userLimit !== "unlimited" ? userLimit : null,
      );
      hasSucceeded = true;
      onclose();
    } catch (error) {
      errorMessage = describeError(error, kindToCreate === "category" ? strings.createChannel.failedCategory : strings.createChannel.failed);
    } finally {
      isBusy = false;
    }
  }
</script>

<ModalDialog
  shouldReturnFocusToOpener={() => !hasSucceeded}
  title={isCategory ? strings.createChannel.titleCategory : strings.createChannel.title}
  subtitle={category ? strings.createChannel.inCategory(category.name) : ""}
  {errorMessage}
  {isBusy}
  submitLabel={isBusy ? strings.createChannel.creating : isCategory ? strings.createChannel.submitCategory : strings.createChannel.submit}
  isSubmitEnabled={!isBusy && name.trim() !== ""}
  {onclose}
  onsubmit={submit}
  {fallbackFocus}
>
  <fieldset class="flex flex-col gap-(--mw-component-choice-list-gap)" disabled={isBusy}>
    <legend class="mb-(--mw-component-text-field-label-gap) type-category text-(--mw-component-text-field-label-text)">
      {strings.createChannel.channelType}
    </legend>
    <ChoiceOption
      name="{id}-kind"
      value="text"
      bind:group={kind}
      icon="hash"
      label={strings.createChannel.textChannel}
      description={strings.createChannel.textChannelDescription}
    />
    <ChoiceOption
      name="{id}-kind"
      value="voice"
      bind:group={kind}
      icon="speaker-high"
      label={strings.voice.voiceChannel}
      description={strings.voice.voiceChannelDescription}
    />
    {#if !category}
      <ChoiceOption
        name="{id}-kind"
        value="category"
        bind:group={kind}
        icon="list"
        label={strings.createChannel.category}
        description={strings.createChannel.categoryDescription}
      />
    {/if}
  </fieldset>
  <TextField
    id="{id}-name"
    label={isCategory ? strings.createChannel.categoryName : strings.createChannel.channelName}
    leadingIcon={isCategory ? undefined : isVoice ? "speaker-high" : "hash"}
    maxlength={MAXIMUM_CHANNEL_NAME_LENGTH}
    autocomplete="off"
    readonly={isBusy}
    data-initial-focus=""
    bind:value={name}
  />
  {#if isVoice}
    <UserLimitField value={userLimit} onchange={(choice) => (userLimit = choice)} isDisabled={isBusy} />
  {:else if !isCategory}
    <TextField
      id="{id}-topic"
      label={strings.createChannel.topic}
      helper={strings.createChannel.topicHelper}
      maxlength={MAXIMUM_CHANNEL_TOPIC_LENGTH}
      autocomplete="off"
      readonly={isBusy}
      bind:value={topic}
    />
  {/if}
</ModalDialog>
