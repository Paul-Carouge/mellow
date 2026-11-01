<script lang="ts">
  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { Guild, SoundboardSound } from "../lib/protocol";
  import { soundNameProblem } from "../lib/soundboard";
  import ModalDialog from "./ModalDialog.svelte";
  import SoundDetailsFields from "./SoundDetailsFields.svelte";

  // "Edit" on a sound (docs/design/web-soundboard.md): its name and emoji, both sent together as
  // the instance replaces both. The samples never change: a new cut is a new sound.
  interface Props {
    client: MellowClient;
    guild: Guild;
    sound: SoundboardSound;
    isCompact: boolean;
    onclose: () => void;
    onsaved: (sentence: string) => void;
  }

  let { client, guild, sound, isCompact, onclose, onsaved }: Props = $props();

  const id = $props.id();
  // svelte-ignore state_referenced_locally
  let name = $state(sound.name);
  // svelte-ignore state_referenced_locally
  let emojiKey = $state<string | null>(sound.emoji ?? null);
  let nameError = $state("");
  let errorMessage = $state("");
  let isSaving = $state(false);

  async function save() {
    const problem = soundNameProblem(name);
    nameError =
      problem === "length" ? strings.serverSettings.sounds.nameLength : problem === "control-character" ? strings.serverSettings.sounds.nameControl : "";
    if (problem) {
      document.getElementById(`${id}-name`)?.focus();
      return;
    }
    if (name.trim() === sound.name && emojiKey === (sound.emoji ?? null)) {
      onclose();
      return;
    }
    isSaving = true;
    errorMessage = "";
    try {
      await client.updateSound(guild.id, sound.id, name.trim(), emojiKey);
      onsaved(strings.serverSettings.sounds.saved(name.trim()));
    } catch (error) {
      errorMessage = describeError(error, strings.serverSettings.sounds.saveFailed);
    } finally {
      isSaving = false;
    }
  }
</script>

<ModalDialog
  title={strings.serverSettings.sounds.editTitle(sound.name)}
  {errorMessage}
  isBusy={isSaving}
  submitLabel={isSaving ? strings.common.saving : strings.serverSettings.sounds.save}
  {onclose}
  onsubmit={() => void save()}
>
  <SoundDetailsFields
    {id}
    {guild}
    customEmojis={client.knownCustomEmojis}
    {isCompact}
    bind:name
    bind:emojiKey
    {nameError}
    isDisabled={isSaving}
  />
</ModalDialog>
