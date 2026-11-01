<script lang="ts">
  import { tick } from "svelte";

  import { strings } from "../i18n";
  import { type CustomEmoji, type Guild, MAXIMUM_SOUND_NAME_LENGTH, type Snowflake, customEmojiReactionKey } from "../lib/protocol";
  import { reactionKey } from "../lib/reactions";
  import Button from "./Button.svelte";
  import { reactionAppearance } from "./custom-emoji";
  import EmojiPicker from "./EmojiPicker.svelte";
  import SoundEmoji from "./SoundEmoji.svelte";
  import TextField from "./TextField.svelte";

  // A sound's name and emoji (docs/design/web-soundboard.md), shared by "Add a sound" and "Edit".
  // The emoji comes from the picker the composer uses, the server's own emoji included; it is
  // optional, and "Remove the emoji" puts the speaker glyph back. The picker lives inside the
  // dialog, so the modal dialog does not make it inert.
  interface Props {
    id: string;
    guild: Guild;
    customEmojis: ReadonlyMap<Snowflake, CustomEmoji>;
    isCompact: boolean;
    name: string;
    /** A reaction key, or `null` for none. */
    emojiKey: string | null;
    /** Why the name was refused, shown under it; empty while it is fine. */
    nameError: string;
    isDisabled?: boolean;
  }

  let {
    id,
    guild,
    customEmojis,
    isCompact,
    name = $bindable(),
    emojiKey = $bindable(),
    nameError,
    isDisabled = false,
  }: Props = $props();

  let emojiButton = $state<HTMLButtonElement | null>(null);
  let isPickerOpen = $state(false);
  const emojiLabel = $derived(emojiKey ? reactionAppearance(emojiKey, customEmojis).label : "");

  function pick(emoji: string | CustomEmoji) {
    emojiKey = typeof emoji === "string" ? reactionKey(emoji) : customEmojiReactionKey(emoji);
    isPickerOpen = false;
    emojiButton?.focus();
  }

  async function removeEmoji() {
    emojiKey = null;
    await tick();
    emojiButton?.focus();
  }
</script>

<div class="flex flex-col gap-(--mw-component-text-field-label-gap)">
  <TextField
    id="{id}-name"
    label={strings.serverSettings.sounds.name}
    bind:value={name}
    helper={strings.serverSettings.sounds.nameHelper}
    maxlength={MAXIMUM_SOUND_NAME_LENGTH}
    autocomplete="off"
    required
    disabled={isDisabled}
    aria-invalid={nameError ? "true" : undefined}
    aria-errormessage={nameError ? `${id}-name-error` : undefined}
  />
  {#if nameError}
    <p id="{id}-name-error" class="type-body text-(--mw-component-settings-error-text)">{nameError}</p>
  {/if}
</div>

<div class="flex flex-col gap-(--mw-component-text-field-label-gap)">
  <p id="{id}-emoji-label" class="type-category text-(--mw-component-text-field-label-text)">{strings.serverSettings.sounds.emoji}</p>
  <div class="flex flex-wrap items-center gap-xs">
    <button
      bind:this={emojiButton}
      type="button"
      aria-haspopup="dialog"
      aria-expanded={isPickerOpen}
      aria-label={emojiKey ? strings.serverSettings.sounds.changeEmoji(emojiLabel) : undefined}
      aria-describedby="{id}-emoji-helper"
      disabled={isDisabled}
      class="inline-flex h-(--mw-component-text-field-height) items-center gap-xs rounded-(--mw-component-text-field-radius) border-(length:--mw-component-text-field-border-width) border-(--mw-component-text-field-border) bg-(--mw-component-text-field-background) px-(--mw-component-text-field-padding-inline) type-body text-(--mw-component-text-field-text) hover:border-(--mw-component-custom-emoji-settings-name-border-hover) focus-visible:focus-ring disabled:opacity-(--mw-component-button-disabled-opacity)"
      onclick={() => (isPickerOpen = true)}
    >
      <SoundEmoji
        {emojiKey}
        {customEmojis}
        sizeToken="--mw-component-emoji-picker-emoji-size"
        glyphClass="text-(--mw-component-text-field-icon)"
      />
      <span>{emojiKey ? strings.serverSettings.sounds.change : strings.serverSettings.sounds.chooseEmoji}</span>
    </button>
    {#if emojiKey}
      <Button variant="ghost" isDisabled={isDisabled} onclick={() => void removeEmoji()}>{strings.serverSettings.sounds.removeEmoji}</Button>
    {/if}
  </div>
  <p id="{id}-emoji-helper" class="type-body text-(--mw-component-text-field-helper-text)">{strings.serverSettings.sounds.emojiHelper}</p>
</div>

{#if isPickerOpen && emojiButton}
  <EmojiPicker
    anchor={emojiButton}
    {isCompact}
    server={{ name: guild.name, emojis: guild.emojis ?? [] }}
    onpick={pick}
    onclose={(shouldReturnFocus) => {
      isPickerOpen = false;
      if (shouldReturnFocus) {
        emojiButton?.focus();
      }
    }}
  />
{/if}
