<script lang="ts">
  import { onDestroy, onMount, tick } from "svelte";

  import { formattingLocale, strings } from "../i18n";
  import { ApiError } from "../lib/api";
  import { type MellowClient, describeError } from "../lib/client.svelte";
  import type { Guild } from "../lib/protocol";
  import {
    MAXIMUM_SOUND_SOURCE_BYTES,
    type SoundSelection,
    encodeWav,
    formatSoundSeconds,
    initialSelection,
    soundNameFromFileName,
    soundNameProblem,
    soundboardGain,
    waveformPeaks,
    withSelectionCentredAt,
    withSelectionEnd,
    withSelectionStart,
  } from "../lib/soundboard";
  import Button from "./Button.svelte";
  import { humanFileSize } from "./format";
  import ModalDialog from "./ModalDialog.svelte";
  import RangeField from "./RangeField.svelte";
  import SoundDetailsFields from "./SoundDetailsFields.svelte";
  import { type PlayingSound, decodeSoundFile, previewSamples, renderSound } from "./soundboard-audio";

  // "Add a sound" (docs/design/web-soundboard.md, ADR-0032): Discord's trimmer in Dusk's words.
  // The file is decoded here, never sent as it is: the waveform shows all of it, the lit bars are
  // the part kept (at most 5.2 s), moved by a click or a drag on the waveform and set to the
  // hundredth by the Start and End sliders. Volume, name and emoji follow; "Preview" plays exactly
  // what will be uploaded, through your own output. The instance gets mono 48 kHz 16-bit PCM.
  interface Props {
    client: MellowClient;
    guild: Guild;
    file: File;
    isCompact: boolean;
    onclose: () => void;
    /** After the sound was added: the page announces it. */
    onadded: (sentence: string) => void;
  }

  let { client, guild, file, isCompact, onclose, onadded }: Props = $props();

  /** Bars across the waveform: enough to see a word's shape, few enough to draw cheaply. */
  const WAVEFORM_BARS = 96;
  /** In the waveform's own units (its box is 100 tall): the loudest bar, and the quietest still drawn. */
  const LOUDEST_BAR_HEIGHT = 92;
  const QUIETEST_BAR_HEIGHT = 2;
  /** The sliders move by a hundredth of a second. */
  const SECONDS_STEP = 0.01;
  const DEFAULT_VOLUME_PERCENT = 100;
  const MAXIMUM_UPLOAD_VOLUME_PERCENT = 100;
  const decimalSeparator = (1.5).toLocaleString(formattingLocale).charAt(1);

  const id = $props.id();
  type Reading = { status: "reading" } | { status: "unreadable" } | { status: "too-large" } | { status: "ready"; buffer: AudioBuffer };
  // svelte-ignore state_referenced_locally
  let reading = $state.raw<Reading>(file.size > MAXIMUM_SOUND_SOURCE_BYTES ? { status: "too-large" } : { status: "reading" });
  let peaks = $state.raw<number[]>([]);
  let selection = $state.raw<SoundSelection>({ startSeconds: 0, endSeconds: 0 });
  let volumePercent = $state(DEFAULT_VOLUME_PERCENT);
  // svelte-ignore state_referenced_locally
  let name = $state(soundNameFromFileName(file.name));
  let emojiKey = $state<string | null>(null);
  let nameError = $state("");
  let errorMessage = $state("");
  let isAdding = $state(false);
  let playing = $state.raw<PlayingSound | null>(null);
  let waveform = $state<SVGSVGElement | null>(null);

  const duration = $derived(reading.status === "ready" ? reading.buffer.duration : 0);
  const keptMilliseconds = $derived(Math.round((selection.endSeconds - selection.startSeconds) * 1000));
  const playbackGain = $derived(soundboardGain(client.voice.settings.outputVolumePercent, client.voice.settings.soundboardVolumePercent));
  const formatSeconds = (seconds: number) => formatSoundSeconds(seconds, decimalSeparator);

  onMount(() => {
    if (reading.status !== "reading") {
      return;
    }
    decodeSoundFile(file).then(
      (buffer) => {
        const channels = Array.from({ length: buffer.numberOfChannels }, (_, index) => buffer.getChannelData(index));
        peaks = waveformPeaks(channels, WAVEFORM_BARS);
        selection = initialSelection(buffer.duration);
        reading = { status: "ready", buffer };
        // Focus waited on Cancel while the file was read: the trimmer is the first thing to do.
        void tick().then(() => waveform?.closest("section")?.querySelector<HTMLInputElement>("input[type=range]")?.focus());
      },
      (error: unknown) => {
        console.warn("couldn't decode a sound file", error);
        reading = { status: "unreadable" };
      },
    );
  });

  onDestroy(() => playing?.stop());

  function isBarKept(index: number): boolean {
    const barStart = (index / WAVEFORM_BARS) * duration;
    const barEnd = ((index + 1) / WAVEFORM_BARS) * duration;
    return barEnd > selection.startSeconds && barStart < selection.endSeconds;
  }

  /** The time under the pointer, from the waveform's own box (its direction included). */
  function secondsAt(event: PointerEvent): number {
    if (!waveform) {
      return 0;
    }
    const bounds = waveform.getBoundingClientRect();
    const isRightToLeft = getComputedStyle(waveform).direction === "rtl";
    const fraction = (event.clientX - bounds.left) / bounds.width;
    return (isRightToLeft ? 1 - fraction : fraction) * duration;
  }

  function moveSelection(event: PointerEvent) {
    selection = withSelectionCentredAt(selection, secondsAt(event), duration);
  }

  function stopPreview() {
    playing?.stop();
    playing = null;
  }

  async function render(): Promise<Float32Array<ArrayBuffer> | null> {
    return reading.status === "ready" ? renderSound(reading.buffer, selection, volumePercent) : null;
  }

  async function togglePreview() {
    if (playing) {
      stopPreview();
      return;
    }
    const samples = await render();
    const started = samples ? await previewSamples(samples, playbackGain) : null;
    playing = started;
    void started?.ended.then(() => {
      if (playing === started) {
        playing = null;
      }
    });
  }

  function uploadFailure(error: unknown): string {
    const sentences = strings.serverSettings.sounds;
    if (error instanceof ApiError) {
      switch (error.code) {
        case "LIMIT_REACHED":
          return sentences.limitReached;
        case "RATE_LIMITED":
          return sentences.tooMany;
        case "VALIDATION_FAILED":
          return sentences.refused;
        default:
          break;
      }
    }
    return describeError(error, sentences.addFailed);
  }

  async function add() {
    const problem = soundNameProblem(name);
    nameError =
      problem === "length" ? strings.serverSettings.sounds.nameLength : problem === "control-character" ? strings.serverSettings.sounds.nameControl : "";
    if (problem || isAdding || reading.status !== "ready") {
      if (problem) {
        document.getElementById(`${id}-name`)?.focus();
      }
      return;
    }
    isAdding = true;
    errorMessage = "";
    stopPreview();
    try {
      const samples = await render();
      if (samples === null) {
        return;
      }
      const created = await client.createSound(guild.id, name.trim(), emojiKey, new Blob([encodeWav(samples)], { type: "audio/wav" }));
      onadded(strings.serverSettings.sounds.added(created.name));
    } catch (error) {
      errorMessage = uploadFailure(error);
    } finally {
      isAdding = false;
    }
  }
</script>

<ModalDialog
  title={strings.serverSettings.sounds.addTitle}
  subtitle={file.name}
  {errorMessage}
  isBusy={isAdding}
  submitLabel={reading.status === "ready" ? (isAdding ? strings.serverSettings.sounds.adding : strings.serverSettings.sounds.submit) : ""}
  {onclose}
  onsubmit={() => void add()}
>
  {#if reading.status === "reading"}
    <p role="status" class="type-body text-(--mw-component-text-field-helper-text)">{strings.serverSettings.sounds.reading(file.name)}</p>
  {:else if reading.status === "unreadable" || reading.status === "too-large"}
    <p role="alert" class="type-body text-(--mw-component-settings-error-text)">
      {reading.status === "unreadable" ? strings.serverSettings.sounds.unreadable(file.name) : strings.serverSettings.sounds.tooLargeFile(humanFileSize(file.size))}
    </p>
  {:else}
    <section aria-labelledby="{id}-trim" class="flex flex-col gap-xs">
      <div class="flex items-baseline justify-between gap-xs">
        <h3 id="{id}-trim" class="type-category text-(--mw-component-text-field-label-text)">{strings.serverSettings.sounds.trim}</h3>
        <p class="type-body-sm tabular-nums text-(--mw-component-slider-value-text)" aria-live="polite">
          {strings.serverSettings.sounds.kept(strings.soundboard.duration(keptMilliseconds))}
        </p>
      </div>
      <!-- A pointer target only: the sliders below are the keyboard's way to the same selection. -->
      <svg
        bind:this={waveform}
        role="img"
        aria-label={strings.serverSettings.sounds.waveform(file.name)}
        viewBox="0 0 {WAVEFORM_BARS} 100"
        preserveAspectRatio="none"
        class="h-[calc(2*var(--mw-size-hit-web-min))] w-full cursor-ew-resize touch-none rounded-(--mw-component-text-field-radius) bg-(--mw-component-voice-meter-track) rtl:-scale-x-100"
        onpointerdown={(event) => {
          event.currentTarget.setPointerCapture(event.pointerId);
          moveSelection(event);
        }}
        onpointermove={(event) => {
          if (event.currentTarget.hasPointerCapture(event.pointerId)) {
            moveSelection(event);
          }
        }}
      >
        {#each peaks as peak, index (index)}
          {@const height = Math.max(peak * LOUDEST_BAR_HEIGHT, QUIETEST_BAR_HEIGHT)}
          <rect
            x={index + 0.15}
            y={50 - height / 2}
            width="0.7"
            {height}
            class={isBarKept(index) ? "fill-(--mw-component-voice-meter-level-heard)" : "fill-(--mw-component-voice-meter-level-quiet)"}
          />
        {/each}
      </svg>
      <p id="{id}-trim-help" class="type-body text-(--mw-component-text-field-helper-text)">{strings.serverSettings.sounds.trimHelp}</p>
      <div class="grid grid-cols-2 gap-x-md gap-y-xs">
        <RangeField
          label={strings.serverSettings.sounds.start}
          value={selection.startSeconds}
          min={0}
          max={duration}
          step={SECONDS_STEP}
          valueText={formatSeconds}
          describedBy="{id}-trim-help"
          oninput={(seconds) => (selection = withSelectionStart(selection, seconds, duration))}
        />
        <RangeField
          label={strings.serverSettings.sounds.end}
          value={selection.endSeconds}
          min={0}
          max={duration}
          step={SECONDS_STEP}
          valueText={formatSeconds}
          describedBy="{id}-trim-help"
          oninput={(seconds) => (selection = withSelectionEnd(selection, seconds, duration))}
        />
      </div>
      <div class="flex flex-wrap items-end gap-md">
        <div class="min-w-0 flex-1">
          <RangeField
            label={strings.serverSettings.sounds.volume}
            value={volumePercent}
            min={0}
            max={MAXIMUM_UPLOAD_VOLUME_PERCENT}
            valueText={strings.voiceSettings.percent}
            oninput={(value) => (volumePercent = value)}
          />
        </div>
        <Button onclick={() => void togglePreview()}>
          {playing ? strings.serverSettings.sounds.stopPreview : strings.serverSettings.sounds.previewSelection}
        </Button>
      </div>
    </section>
    <SoundDetailsFields
      {id}
      {guild}
      customEmojis={client.knownCustomEmojis}
      {isCompact}
      bind:name
      bind:emojiKey
      {nameError}
      isDisabled={isAdding}
    />
  {/if}
</ModalDialog>
