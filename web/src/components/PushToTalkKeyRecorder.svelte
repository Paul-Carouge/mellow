<script lang="ts">
  import { strings } from "../i18n";
  import { type PushToTalkKey, pushToTalkKeyOf } from "../lib/voice-settings";
  import Button from "./Button.svelte";
  import Icon from "./Icon.svelte";

  // The push-to-talk key (docs/design/web-voice-comfort.md): shown as a key cap, recorded from the
  // next key pressed on the button (Esc cancels, Tab leaves), cleared with its own button. There
  // is no default key: any letter would stop typing while in voice, so the reader picks one. The
  // shell's push-to-talk listener ignores keys pressed here (`data-push-to-talk-recorder`).
  interface Props {
    pushToTalkKey: PushToTalkKey | null;
    onchange: (pushToTalkKey: PushToTalkKey | null) => void;
  }

  let { pushToTalkKey, onchange }: Props = $props();

  const id = $props.id();
  let isRecording = $state(false);
  let announcement = $state("");
  /** The key just recorded: its keyup must not click the button again (Space activates on keyup). */
  let recordedCode: string | null = null;
  let recordButton = $state<HTMLElement | null>(null);

  const isCharacterKey = $derived(pushToTalkKey !== null && ([...pushToTalkKey.label].length === 1 || pushToTalkKey.code === "Space"));

  function startRecording() {
    isRecording = true;
    announcement = strings.voiceSettings.recording;
  }

  function record(event: KeyboardEvent) {
    if (!isRecording || event.code === "Tab") {
      return;
    }
    event.preventDefault();
    // Esc would otherwise close the settings.
    event.stopPropagation();
    isRecording = false;
    if (event.code === "Escape") {
      announcement = "";
      return;
    }
    const key = pushToTalkKeyOf(event, strings.voiceSettings.namedKeys);
    if (key) {
      recordedCode = key.code;
      onchange(key);
      announcement = strings.voiceSettings.keyRecorded(key.label);
    }
  }

  function clear() {
    onchange(null);
    announcement = strings.voiceSettings.keyCleared;
    // The Clear button goes away: focus stays on the key's own button.
    recordButton?.querySelector("button")?.focus();
  }
</script>

<div class="flex flex-col gap-(--mw-component-text-field-label-gap)">
  <p id="{id}-label" class="type-category text-(--mw-component-text-field-label-text)">{strings.voiceSettings.shortcut}</p>
  <div class="flex flex-wrap items-center gap-xs" data-push-to-talk-recorder>
    <kbd
      id="{id}-key"
      class={[
        "inline-flex h-(--mw-component-key-chip-height) min-w-(--mw-component-key-chip-min-width) items-center justify-center rounded-(--mw-component-key-chip-radius) border bg-(--mw-component-key-chip-background) px-(--mw-component-key-chip-padding-inline) font-sans type-button",
        isRecording
          ? "border-(--mw-component-key-chip-border-recording) text-(--mw-component-key-chip-empty-text)"
          : "border-(--mw-component-key-chip-border)",
        !isRecording && (pushToTalkKey ? "text-(--mw-component-key-chip-text)" : "text-(--mw-component-key-chip-empty-text)"),
      ]}
    >
      {isRecording ? strings.voiceSettings.recording : (pushToTalkKey?.label ?? strings.voiceSettings.noKey)}
    </kbd>
    <span bind:this={recordButton} class="contents">
      <Button
        variant="secondary"
        aria-describedby="{id}-label {id}-key"
        aria-pressed={isRecording}
        onclick={startRecording}
        onkeydown={record}
        onkeyup={(event: KeyboardEvent) => {
          if (event.code === recordedCode) {
            event.preventDefault();
            recordedCode = null;
          }
        }}
        onblur={() => (isRecording = false)}
      >
        {pushToTalkKey ? strings.voiceSettings.changeKey : strings.voiceSettings.recordKey}
      </Button>
    </span>
    {#if pushToTalkKey}
      <Button variant="ghost" onclick={clear}>{strings.voiceSettings.clearKey}</Button>
    {/if}
  </div>
  {#if !pushToTalkKey}
    <p class="flex gap-2xs type-body text-(--mw-component-settings-error-text)">
      <Icon name="warning-circle" class="mt-3xs size-(--mw-component-voice-panel-signal-icon-size) shrink-0" />
      <span>{strings.voiceSettings.noKeyWarning}</span>
    </p>
  {/if}
  <p class="type-body text-(--mw-component-text-field-helper-text)">
    {strings.voiceSettings.focusNote}{#if isCharacterKey}{" "}{strings.voiceSettings.characterKeyNote}{/if}
  </p>
  <p class="sr-only" aria-live="polite">{announcement}</p>
</div>
