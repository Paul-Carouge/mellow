<script lang="ts">
  import { strings } from "../i18n";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import { type Guild, MAXIMUM_SOUNDS_PER_GUILD, type SoundboardSound } from "../lib/protocol";
  import { soundboardGain } from "../lib/soundboard";
  import AddSoundDialog from "./AddSoundDialog.svelte";
  import Button from "./Button.svelte";
  import EditSoundDialog from "./EditSoundDialog.svelte";
  import IconButton from "./IconButton.svelte";
  import ModalDialog from "./ModalDialog.svelte";
  import { focusAfterRemoval } from "./server-settings";
  import SoundEmoji from "./SoundEmoji.svelte";
  import { previewSound } from "./soundboard-audio";

  // Server settings → Soundboard (docs/design/web-soundboard.md, ADR-0032), beside Emoji and with
  // the same permission. One sound at a time goes through "Add a sound" (trim, volume, name,
  // emoji); each row previews, edits and deletes. The count sits on the list it limits; the list
  // follows the gateway (`GUILD_SOUNDS_UPDATE`), so there is nothing to wait for but the upload.
  interface Props {
    client: MellowClient;
    guild: Guild;
    isCompact: boolean;
  }

  let { client, guild, isCompact }: Props = $props();

  const id = $props.id();
  const sounds = $derived(guild.sounds ?? []);
  const isFull = $derived(sounds.length >= MAXIMUM_SOUNDS_PER_GUILD);
  const gain = $derived(soundboardGain(client.voice.settings.outputVolumePercent, client.voice.settings.soundboardVolumePercent));

  let fileInput = $state<HTMLInputElement | null>(null);
  let addingFile = $state<File | null>(null);
  let editing = $state<SoundboardSound | null>(null);
  let deleting = $state<SoundboardSound | null>(null);
  let isDeleting = $state(false);
  let deleteError = $state("");
  let focusAfterDeleteId: string | null = null;
  let notice = $state("");

  function rowId(sound: SoundboardSound): string {
    return `${id}-sound-${sound.id}`;
  }

  async function confirmDelete() {
    const sound = deleting;
    if (!sound || isDeleting) {
      return;
    }
    isDeleting = true;
    deleteError = "";
    const shownIds = sounds.map(rowId);
    try {
      await client.deleteSound(guild.id, sound.id);
      focusAfterDeleteId = focusAfterRemoval(shownIds, rowId(sound));
      deleting = null;
      notice = strings.serverSettings.sounds.deleted(sound.name);
    } catch (error) {
      deleteError = describeError(error, strings.serverSettings.sounds.deleteFailed);
    } finally {
      isDeleting = false;
    }
  }
</script>

<div class="flex flex-col gap-(--mw-component-server-settings-toolbar-gap)">
  <div class="flex flex-col gap-(--mw-component-server-settings-text-gap)">
    <p class="type-body-lg text-(--mw-component-settings-body-text)">{strings.serverSettings.sounds.intro}</p>
    <p id="{id}-rules" class="type-body-sm text-(--mw-component-server-settings-secondary-text)">{strings.serverSettings.sounds.rules}</p>
  </div>

  <div class="flex flex-col items-start gap-(--mw-component-server-settings-text-gap)">
    <Button
      id="{id}-upload"
      variant="primary"
      isDisabled={isFull}
      aria-describedby={isFull ? `${id}-full` : `${id}-rules`}
      onclick={() => {
        if (!isFull) {
          fileInput?.click();
        }
      }}
    >
      {strings.serverSettings.sounds.upload}
    </Button>
    {#if isFull}
      <p id="{id}-full" class="type-body-sm text-(--mw-component-server-settings-secondary-text)">{strings.serverSettings.sounds.fullSentence}</p>
    {/if}
    <!-- Opened by the button only; the button is the control people reach. -->
    <input
      bind:this={fileInput}
      type="file"
      accept="audio/*"
      hidden
      tabindex="-1"
      aria-hidden="true"
      onchange={(event) => {
        addingFile = event.currentTarget.files?.[0] ?? null;
        // Choosing the same file again must fire change again.
        event.currentTarget.value = "";
      }}
    />
  </div>

  {#if sounds.length === 0}
    <p class="type-body-lg text-(--mw-component-settings-body-text)">{strings.serverSettings.sounds.empty}</p>
  {:else}
    <section aria-labelledby="{id}-list">
      <h3
        class="flex items-baseline justify-between gap-sm border-b border-(--mw-component-server-settings-row-divider) pb-2xs type-category text-(--mw-component-settings-nav-section-text)"
      >
        <span id="{id}-list">{strings.serverSettings.sounds.listLabel}</span>
        <span class={["tabular-nums", isFull && "text-(--mw-component-custom-emoji-settings-count-full-text)"]}>
          <span aria-hidden="true">{strings.serverSettings.sounds.count(sounds.length, MAXIMUM_SOUNDS_PER_GUILD)}</span>
          <span class="sr-only">{strings.serverSettings.sounds.countLabel(sounds.length, MAXIMUM_SOUNDS_PER_GUILD)}</span>
          {#if isFull}· {strings.serverSettings.sounds.full}{/if}
        </span>
      </h3>
      <ul aria-labelledby="{id}-list">
        {#each sounds as sound (sound.id)}
          <li
            class="flex min-h-(--mw-component-server-settings-row-min-height) items-center gap-(--mw-component-server-settings-row-gap) border-b border-(--mw-component-server-settings-row-divider) py-(--mw-component-server-settings-row-padding-block)"
          >
            <span
              class="grid size-(--mw-component-custom-emoji-settings-picture-size) shrink-0 place-items-center rounded-(--mw-component-custom-emoji-settings-picture-radius) bg-(--mw-component-custom-emoji-settings-picture-background)"
            >
              <SoundEmoji
                emojiKey={sound.emoji}
                customEmojis={client.knownCustomEmojis}
                sizeToken="--mw-component-emoji-picker-emoji-size"
                glyphClass="text-(--mw-component-server-settings-secondary-text)"
              />
            </span>
            <div class="flex min-w-0 flex-1 flex-col gap-(--mw-component-server-settings-text-gap)">
              <p class="truncate type-body-lg text-text-heading">{sound.name}</p>
              <p class="type-body-sm tabular-nums text-(--mw-component-server-settings-secondary-text)">
                {strings.soundboard.duration(sound.duration_milliseconds)}{#if guild.my_join_sound_id === sound.id}<span>, {strings.soundboard.joinSound.toLocaleLowerCase()}</span>{/if}
              </p>
            </div>
            <IconButton
              id={rowId(sound)}
              icon="headphones"
              label={strings.soundboard.previewLabel(sound.name)}
              class="size-(--mw-size-hit-web-min) text-(--mw-component-server-settings-secondary-text) hover:bg-state-hover hover:text-text-normal"
              iconClass="size-(--mw-component-context-menu-item-icon-size)"
              onclick={() => void previewSound(sound.id, gain)}
            />
            <IconButton
              icon="pencil-simple"
              label={strings.serverSettings.sounds.editLabel(sound.name)}
              aria-haspopup="dialog"
              class="size-(--mw-size-hit-web-min) text-(--mw-component-server-settings-secondary-text) hover:bg-state-hover hover:text-text-normal"
              iconClass="size-(--mw-component-context-menu-item-icon-size)"
              onclick={() => (editing = sound)}
            />
            <IconButton
              icon="trash"
              label={strings.serverSettings.sounds.deleteLabel(sound.name)}
              aria-haspopup="dialog"
              class="size-(--mw-size-hit-web-min) text-(--mw-component-server-settings-secondary-text) hover:bg-state-hover hover:text-text-danger"
              iconClass="size-(--mw-component-context-menu-item-icon-size)"
              onclick={() => {
                deleteError = "";
                focusAfterDeleteId = null;
                deleting = sound;
              }}
            />
          </li>
        {/each}
      </ul>
    </section>
  {/if}
  <p role="status" class="sr-only">{notice}</p>
</div>

{#if addingFile}
  <AddSoundDialog
    {client}
    {guild}
    file={addingFile}
    {isCompact}
    onclose={() => (addingFile = null)}
    onadded={(sentence) => {
      addingFile = null;
      notice = sentence;
    }}
  />
{:else if editing}
  <EditSoundDialog
    {client}
    {guild}
    sound={editing}
    {isCompact}
    onclose={() => (editing = null)}
    onsaved={(sentence) => {
      editing = null;
      notice = sentence;
    }}
  />
{:else if deleting}
  {@const sound = deleting}
  <ModalDialog
    title={strings.serverSettings.sounds.deleteTitle(sound.name)}
    subtitle={strings.serverSettings.sounds.deleteBody}
    errorMessage={deleteError}
    isBusy={isDeleting}
    submitLabel={isDeleting ? strings.serverSettings.sounds.deleting : strings.serverSettings.sounds.deleteSubmit}
    isSubmitDanger
    onclose={() => {
      deleting = null;
      deleteError = "";
    }}
    onsubmit={() => void confirmDelete()}
    fallbackFocus={() => document.getElementById(focusAfterDeleteId ?? `${id}-upload`)?.focus()}
  />
{/if}
