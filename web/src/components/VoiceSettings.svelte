<script lang="ts">
  import { onMount } from "svelte";

  import { strings } from "../i18n";
  import type { MicrophoneProblem } from "../lib/voice";
  import { CAN_CHOOSE_OUTPUT_DEVICE, CAN_SUPPRESS_NOISE_IN_PAGE, listAudioDevices, watchAudioDevices } from "../lib/voice-browser";
  import type { VoiceSession } from "../lib/voice-session.svelte";
  import {
    type AudioDevice,
    type DeviceChoices,
    EQUALIZER_BANDS,
    MAXIMUM_EQUALIZER_DECIBELS,
    MAXIMUM_PUSH_TO_TALK_RELEASE_DELAY_MILLISECONDS,
    MAXIMUM_VOLUME_PERCENT,
    MINIMUM_EQUALIZER_DECIBELS,
    MINIMUM_VOLUME_PERCENT,
    type NoiseSuppressionMode,
    VOICE_ENHANCEMENT_PRESETS,
    type VoiceSettings as Settings,
    deviceChoices,
    withEqualizerBand,
    withVoiceEnhancementPreset,
  } from "../lib/voice-settings";
  import Button from "./Button.svelte";
  import ChoiceOption from "./ChoiceOption.svelte";
  import Icon from "./Icon.svelte";
  import PushToTalkKeyRecorder from "./PushToTalkKeyRecorder.svelte";
  import RangeField from "./RangeField.svelte";
  import Switch from "./Switch.svelte";
  import VoiceLevelMeter from "./VoiceLevelMeter.svelte";

  // User settings → Voice (docs/design/web-voice-comfort.md): Discord's Voice settings in Mellow's
  // order and words. Every change applies at once, during a call too, and stays in this browser.
  // The meter reads the call's microphone while connected; otherwise "Test the microphone" opens
  // one from that click, and leaving the page frees it. Voice processing (ADR-0031,
  // docs/design/web-voice-enhancement.md) sits just above the test, so "S'écouter" answers it.
  interface Props {
    voice: VoiceSession;
    /** The gallery shows Safari's case in any browser. */
    canChooseOutputDevice?: boolean;
    /** The gallery shows a browser without AudioWorklet in any browser. */
    canSuppressNoiseInPage?: boolean;
  }

  let { voice, canChooseOutputDevice = CAN_CHOOSE_OUTPUT_DEVICE, canSuppressNoiseInPage = CAN_SUPPRESS_NOISE_IN_PAGE }: Props = $props();

  const NOISE_SUPPRESSION_CHOICES: readonly NoiseSuppressionMode[] = ["advanced", "browser", "off"];

  const id = $props.id();
  const RELEASE_DELAY_STEP_MILLISECONDS = 20;
  const VOLUME_STEP_PERCENT = 1;

  const settings = $derived(voice.settings);
  let devices = $state.raw<readonly AudioDevice[]>([]);
  let decibels = $state<number | null>(null);
  let allowProblem = $state<MicrophoneProblem | null>(null);

  const inputs = $derived(deviceChoices(devices, "audioinput", strings.voiceSettings.unnamedMicrophone));
  const outputs = $derived(deviceChoices(devices, "audiooutput", strings.voiceSettings.unnamedOutput));
  const areDevicesHidden = $derived(inputs.areLabelsHidden);
  const isManualSensitivity = $derived(settings.inputMode === "voice-activity" && !settings.isSensitivityAutomatic);
  /** Known before any microphone opens where the browser lacks AudioWorklet; after a failed load otherwise. */
  const noiseSuppressorProblem = $derived(
    settings.noiseSuppression === "advanced" ? (voice.noiseSuppressorProblem ?? (canSuppressNoiseInPage ? null : "unsupported")) : null,
  );

  function update(change: Partial<Settings>) {
    voice.updateSettings({ ...voice.settings, ...change });
  }

  /** The radio rows bind a string; this passes it through as the setting. */
  const inputMode = {
    get value(): string {
      return voice.settings.inputMode;
    },
    set value(mode: string) {
      update({ inputMode: mode === "push-to-talk" ? "push-to-talk" : "voice-activity" });
    },
  };

  const noiseSuppression = {
    get value(): string {
      return voice.settings.noiseSuppression;
    },
    set value(mode: string) {
      update({ noiseSuppression: NOISE_SUPPRESSION_CHOICES.find((choice) => choice === mode) ?? "advanced" });
    },
  };

  async function refreshDevices() {
    try {
      devices = await listAudioDevices();
    } catch (error) {
      console.warn("couldn't list the audio devices", error);
    }
  }

  async function allowMicrophone() {
    allowProblem = await voice.allowMicrophone();
    await refreshDevices();
  }

  async function toggleMicrophoneTest() {
    if (voice.isMicrophoneTestRunning) {
      voice.stopMicrophoneTest();
      return;
    }
    await voice.startMicrophoneTest();
    // Allowed just now: the browser names the devices from here on.
    await refreshDevices();
  }

  onMount(() => {
    voice.preloadNoiseSuppressor();
    void refreshDevices();
    const stopWatching = watchAudioDevices(() => void refreshDevices());
    let frame = requestAnimationFrame(function readLevel() {
      decibels = voice.microphoneDecibels();
      frame = requestAnimationFrame(readLevel);
    });
    return () => {
      stopWatching();
      cancelAnimationFrame(frame);
      voice.stopMicrophoneTest();
    };
  });
</script>

{#snippet deviceField(
  key: string,
  label: string,
  choices: DeviceChoices,
  selectedId: string | null,
  missingSentence: string,
  onchange: (deviceId: string | null) => void,
)}
  {@const isUnplugged = selectedId !== null && !choices.devices.some((device) => device.deviceId === selectedId)}
  <div class="flex flex-col gap-(--mw-component-text-field-label-gap)">
    <label for="{id}-{key}" class="type-category text-(--mw-component-text-field-label-text)">{label}</label>
    <div class="relative">
      <select
        id="{id}-{key}"
        aria-describedby={isUnplugged ? `${id}-${key}-missing` : undefined}
        class="h-(--mw-component-text-field-height) w-full cursor-pointer appearance-none truncate rounded-(--mw-component-text-field-radius) border-(length:--mw-component-text-field-border-width) border-(--mw-component-text-field-border) bg-(--mw-component-text-field-background) ps-(--mw-component-text-field-padding-inline) pe-[calc(2*var(--mw-component-text-field-padding-inline)_+_var(--mw-component-text-field-icon-size))] type-body-lg text-(--mw-component-text-field-text) focus-visible:focus-ring"
        onchange={(event) => onchange(event.currentTarget.value || null)}
      >
        <option value="" selected={selectedId === null}>{strings.voiceSettings.defaultDevice(choices.defaultDeviceName)}</option>
        {#each choices.devices as device (device.deviceId)}
          <option value={device.deviceId} selected={device.deviceId === selectedId}>{device.label}</option>
        {/each}
        {#if isUnplugged && selectedId !== null}
          <option value={selectedId} selected>{strings.voiceSettings.unpluggedDevice}</option>
        {/if}
      </select>
      <Icon
        name="caret-down"
        class="pointer-events-none absolute end-(--mw-component-text-field-padding-inline) top-1/2 size-(--mw-component-text-field-icon-size) -translate-y-1/2 text-(--mw-component-invite-select-icon)"
      />
    </div>
    {#if isUnplugged}
      <p id="{id}-{key}-missing" class="flex gap-2xs type-body text-(--mw-component-text-field-helper-text)">
        <Icon name="warning-circle" class="mt-3xs size-(--mw-component-voice-panel-signal-icon-size) shrink-0" />
        <span>{missingSentence}</span>
      </p>
    {/if}
  </div>
{/snippet}

{#snippet switchRow(key: string, label: string, description: string, isOn: boolean, onchange: (isOn: boolean) => void)}
  <div class="flex items-center gap-md border-b border-border-divider py-sm">
    <div class="min-w-0 flex-1">
      <p id="{id}-{key}" class="type-channel-name text-text-heading">{label}</p>
      <p id="{id}-{key}-description" class="type-body text-text-muted">{description}</p>
    </div>
    <Switch {isOn} labelledBy="{id}-{key}" describedBy="{id}-{key}-description" {onchange} />
  </div>
{/snippet}

{#snippet sectionTitle(title: string)}
  <h3 class="mt-(--mw-component-settings-section-gap) mb-(--mw-component-settings-section-label-gap) type-category text-(--mw-component-settings-section-label-text)">
    {title}
  </h3>
{/snippet}

<h2 class="type-heading-lg text-(--mw-component-settings-page-title-text)">{strings.settings.voice}</h2>
<p class="mt-(--mw-component-settings-page-title-gap) type-body-lg text-(--mw-component-settings-body-text)">
  {strings.voiceSettings.intro}
</p>

{#if areDevicesHidden}
  <div class="mt-(--mw-component-settings-section-gap) flex flex-col items-start gap-xs">
    <p class="type-body-lg text-(--mw-component-settings-body-text)">{strings.voiceSettings.devicesHidden}</p>
    <Button variant="primary" onclick={() => void allowMicrophone()}>{strings.voiceSettings.allowMicrophone}</Button>
    {#if allowProblem}
      <p role="alert" class="flex gap-2xs type-body text-(--mw-component-settings-error-text)">
        <Icon name="microphone-slash" class="mt-3xs size-(--mw-component-voice-panel-signal-icon-size) shrink-0" />
        <span>{strings.voice.microphoneProblems[allowProblem]}</span>
      </p>
    {/if}
  </div>
{/if}

<div
  class="mt-(--mw-component-settings-section-gap) grid grid-cols-[repeat(auto-fit,minmax(min(100%,var(--mw-component-settings-voice-column-min-width)),1fr))] gap-x-lg gap-y-md"
>
  <div class="flex min-w-0 flex-col gap-sm">
    {#if !areDevicesHidden}
      {@render deviceField(
        "input",
        strings.voiceSettings.inputDevice,
        inputs,
        settings.inputDeviceId,
        strings.voiceSettings.inputDeviceMissing,
        (deviceId) => update({ inputDeviceId: deviceId }),
      )}
    {/if}
    <RangeField
      label={strings.voiceSettings.inputVolume}
      value={settings.inputVolumePercent}
      min={MINIMUM_VOLUME_PERCENT}
      max={MAXIMUM_VOLUME_PERCENT}
      step={VOLUME_STEP_PERCENT}
      valueText={strings.voiceSettings.percent}
      oninput={(value) => update({ inputVolumePercent: value })}
    />
  </div>
  <div class="flex min-w-0 flex-col gap-sm">
    {#if !canChooseOutputDevice}
      <div class="flex flex-col gap-(--mw-component-text-field-label-gap)">
        <p class="type-category text-(--mw-component-text-field-label-text)">{strings.voiceSettings.outputDevice}</p>
        <p class="type-body text-(--mw-component-text-field-helper-text)">{strings.voiceSettings.outputUnsupported}</p>
      </div>
    {:else if !areDevicesHidden}
      {@render deviceField(
        "output",
        strings.voiceSettings.outputDevice,
        outputs,
        settings.outputDeviceId,
        strings.voiceSettings.outputDeviceMissing,
        (deviceId) => update({ outputDeviceId: deviceId }),
      )}
    {/if}
    <RangeField
      label={strings.voiceSettings.outputVolume}
      value={settings.outputVolumePercent}
      min={MINIMUM_VOLUME_PERCENT}
      max={MAXIMUM_VOLUME_PERCENT}
      step={VOLUME_STEP_PERCENT}
      valueText={strings.voiceSettings.percent}
      oninput={(value) => update({ outputVolumePercent: value })}
    />
  </div>
</div>

<fieldset class="mt-(--mw-component-settings-section-gap) flex flex-col gap-(--mw-component-choice-list-gap)">
  <legend class="mb-(--mw-component-settings-section-label-gap) type-category text-(--mw-component-settings-section-label-text)">
    {strings.voiceSettings.inputMode}
  </legend>
  <ChoiceOption
    name="{id}-mode"
    value="voice-activity"
    bind:group={inputMode.value}
    label={strings.voiceSettings.voiceActivity}
    description={strings.voiceSettings.voiceActivityDescription}
  />
  <ChoiceOption
    name="{id}-mode"
    value="push-to-talk"
    bind:group={inputMode.value}
    label={strings.voiceSettings.pushToTalk}
    description={strings.voiceSettings.pushToTalkDescription}
  />
</fieldset>

{#if settings.inputMode === "push-to-talk"}
  <div class="mt-md flex flex-col gap-md">
    <PushToTalkKeyRecorder pushToTalkKey={settings.pushToTalkKey} onchange={(pushToTalkKey) => update({ pushToTalkKey })} />
    <RangeField
      label={strings.voiceSettings.releaseDelay}
      value={settings.pushToTalkReleaseDelayMilliseconds}
      min={0}
      max={MAXIMUM_PUSH_TO_TALK_RELEASE_DELAY_MILLISECONDS}
      step={RELEASE_DELAY_STEP_MILLISECONDS}
      valueText={strings.voiceSettings.milliseconds}
      describedBy="{id}-release-description"
      oninput={(value) => update({ pushToTalkReleaseDelayMilliseconds: value })}
    />
    <p id="{id}-release-description" class="-mt-sm type-body text-(--mw-component-text-field-helper-text)">
      {strings.voiceSettings.releaseDelayDescription}
    </p>
  </div>
  {@render switchRow(
    "push-to-talk-sounds",
    strings.voiceSettings.pushToTalkSounds,
    strings.voiceSettings.pushToTalkSoundsDescription,
    settings.arePushToTalkSoundsOn,
    (isOn) => update({ arePushToTalkSoundsOn: isOn }),
  )}
{:else}
  {@render sectionTitle(strings.voiceSettings.sensitivity)}
  {@render switchRow(
    "automatic",
    strings.voiceSettings.automaticSensitivity,
    strings.voiceSettings.automaticSensitivityDescription,
    settings.isSensitivityAutomatic,
    (isOn) => update({ isSensitivityAutomatic: isOn }),
  )}
{/if}

{@render sectionTitle(strings.voiceSettings.voiceProcessing)}
<fieldset class="flex flex-col gap-(--mw-component-choice-list-gap)">
  <legend class="mb-(--mw-component-settings-section-label-gap) type-channel-name text-text-heading">
    {strings.voiceSettings.noiseSuppression}
  </legend>
  {#each NOISE_SUPPRESSION_CHOICES as mode (mode)}
    <ChoiceOption
      name="{id}-noise"
      value={mode}
      bind:group={noiseSuppression.value}
      label={strings.voiceSettings.noiseSuppressionModes[mode].label}
      description={strings.voiceSettings.noiseSuppressionModes[mode].description}
    />
  {/each}
</fieldset>
{#if noiseSuppressorProblem}
  <p role="status" class="mt-xs flex gap-2xs type-body text-(--mw-component-text-field-helper-text)">
    <Icon name="warning-circle" class="mt-3xs size-(--mw-component-voice-panel-signal-icon-size) shrink-0" />
    <span>{strings.voiceSettings.noiseSuppressorProblems[noiseSuppressorProblem]}</span>
  </p>
{/if}

<div class="mt-sm">
  {@render switchRow(
    "enhancement",
    strings.voiceSettings.voiceEnhancement,
    strings.voiceSettings.voiceEnhancementDescription,
    settings.isVoiceEnhancementOn,
    (isOn) => update({ isVoiceEnhancementOn: isOn }),
  )}
</div>
{#if settings.isVoiceEnhancementOn}
  {@const preset = settings.voiceEnhancementPreset}
  <div class="mt-md flex flex-col gap-md">
    <div class="flex flex-col gap-(--mw-component-text-field-label-gap)">
      <p class="flex items-baseline justify-between gap-xs">
        <span id="{id}-style" class="type-category text-(--mw-component-text-field-label-text)">{strings.voiceSettings.voiceEnhancementStyle}</span>
        {#if preset === "custom"}
          <span class="type-body-sm text-(--mw-component-slider-value-text)">{strings.voiceSettings.voiceEnhancementPresets.custom.label}</span>
        {/if}
      </p>
      <!-- Native radios: arrows move between styles, the chosen one is read with the group. -->
      <div
        role="radiogroup"
        aria-labelledby="{id}-style"
        aria-describedby="{id}-style-description"
        class="flex w-full flex-wrap gap-3xs rounded-(--mw-component-voice-enhancement-preset-track-radius) bg-(--mw-component-voice-enhancement-preset-track-background) p-(--mw-component-voice-enhancement-preset-track-padding) sm:w-max"
      >
        {#each VOICE_ENHANCEMENT_PRESETS as option (option)}
          {@const isChosen = preset === option}
          <label
            class={[
              "flex h-(--mw-component-voice-enhancement-preset-height) flex-1 cursor-pointer items-center justify-center rounded-(--mw-component-voice-enhancement-preset-radius) px-(--mw-component-voice-enhancement-preset-padding-inline) type-body transition-colors duration-(--mw-motion-duration-fast) ease-(--mw-motion-easing-standard) has-[input:focus-visible]:focus-ring",
              isChosen
                ? "bg-(--mw-component-voice-enhancement-preset-background-checked) font-semibold text-(--mw-component-voice-enhancement-preset-text-checked)"
                : "text-(--mw-component-voice-enhancement-preset-text) hover:text-(--mw-component-voice-enhancement-preset-text-hover)",
            ]}
          >
            <input
              type="radio"
              name="{id}-style"
              value={option}
              checked={isChosen}
              class="sr-only"
              onchange={() => voice.updateSettings(withVoiceEnhancementPreset(voice.settings, option))}
            />
            {strings.voiceSettings.voiceEnhancementPresets[option].label}
          </label>
        {/each}
      </div>
      <p id="{id}-style-description" class="type-body text-(--mw-component-text-field-helper-text)">
        {strings.voiceSettings.voiceEnhancementPresets[preset].description}
      </p>
    </div>
    <!-- A tick at 0 dB on each slider (Chromium draws it): the bands cut on the left of it, boost on the right. -->
    <datalist id="{id}-flat"><option value="0"></option></datalist>
    {#each EQUALIZER_BANDS as band (band)}
      <RangeField
        inputAttributes={{ list: `${id}-flat` }}
        label={strings.voiceSettings.equalizerBands[band]}
        value={settings.equalizerDecibels[band]}
        min={MINIMUM_EQUALIZER_DECIBELS}
        max={MAXIMUM_EQUALIZER_DECIBELS}
        valueText={strings.voiceSettings.equalizerDecibels}
        oninput={(value) => voice.updateSettings(withEqualizerBand(voice.settings, band, value))}
      />
    {/each}
  </div>
{/if}

{@render sectionTitle(strings.voiceSettings.microphoneTest)}
<p id="{id}-meter-description" class="type-body text-(--mw-component-text-field-helper-text)">
  {voice.isMicrophoneLive ? strings.voiceSettings.liveDescription : strings.voiceSettings.testDescription}
  {#if isManualSensitivity}{" "}{strings.voiceSettings.thresholdDescription}{/if}
</p>
{#if isManualSensitivity}
  <p class="mt-xs flex items-baseline justify-between gap-xs">
    <span class="type-category text-(--mw-component-text-field-label-text)">{strings.voiceSettings.threshold}</span>
    <span aria-hidden="true" class="type-body-sm tabular-nums text-(--mw-component-slider-value-text)">
      {strings.voiceSettings.decibels(settings.sensitivityThresholdDecibels)}
    </span>
  </p>
{/if}
<div class="flex flex-wrap items-center gap-x-md">
  <div class="min-w-0 flex-[1_1_var(--mw-component-settings-voice-column-min-width)]">
    <VoiceLevelMeter
      {decibels}
      thresholdDecibels={isManualSensitivity ? settings.sensitivityThresholdDecibels : null}
      thresholdDescribedBy="{id}-meter-description"
      onthresholdchange={(value) => update({ sensitivityThresholdDecibels: value })}
    />
  </div>
  {#if !voice.isMicrophoneLive}
    <Button variant="secondary" onclick={() => void toggleMicrophoneTest()}>
      {voice.isMicrophoneTestRunning ? strings.voiceSettings.stopTest : strings.voiceSettings.startTest}
    </Button>
  {/if}
</div>
{#if voice.microphoneTestProblem}
  <p role="alert" class="mt-xs flex gap-2xs type-body text-(--mw-component-settings-error-text)">
    <Icon name="microphone-slash" class="mt-3xs size-(--mw-component-voice-panel-signal-icon-size) shrink-0" />
    <span>{strings.voice.microphoneProblems[voice.microphoneTestProblem]}</span>
  </p>
{/if}
<div class="mt-xs">
  {@render switchRow(
    "hear-yourself",
    strings.voiceSettings.hearYourself,
    strings.voiceSettings.hearYourselfDescription,
    voice.isMonitoring,
    (isOn) => voice.setMonitoring(isOn),
  )}
</div>
