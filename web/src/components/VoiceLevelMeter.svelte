<script lang="ts">
  import { strings } from "../i18n";
  import { MAXIMUM_DECIBELS, MINIMUM_DECIBELS, meterFractionOfDecibels } from "../lib/voice-settings";

  // The microphone's level (docs/design/web-voice-comfort.md), Discord's model in Dusk's light:
  // with a manual sensitivity, the threshold is a mark on the meter and the part of the level
  // past it lights apricot, the colour of your speaking ring, because that part is what others
  // hear. The mark is a native range input, so arrows, Page Up/Down, Home and End move it. The
  // level redraws on every frame without transitions, so reduced motion changes nothing here.
  interface Props {
    /** After the input volume; `null` while nothing listens (an empty meter). */
    decibels: number | null;
    /** Manual sensitivity only: the mark and its slider. */
    thresholdDecibels: number | null;
    thresholdDescribedBy?: string;
    onthresholdchange: (decibels: number) => void;
  }

  let { decibels, thresholdDecibels, thresholdDescribedBy, onthresholdchange }: Props = $props();

  const level = $derived(decibels === null ? 0 : meterFractionOfDecibels(decibels));
  const threshold = $derived(thresholdDecibels === null ? 0 : meterFractionOfDecibels(thresholdDecibels));
  const quietWidth = $derived(Math.min(level, threshold));
  const heardWidth = $derived(Math.max(0, level - threshold));
  const percentOf = (fraction: number) => `${fraction * 100}%`;
</script>

<div class="relative flex h-(--mw-size-hit-web-min) items-center">
  <div
    role="meter"
    aria-label={strings.voiceSettings.level}
    aria-valuemin={MINIMUM_DECIBELS}
    aria-valuemax={MAXIMUM_DECIBELS}
    aria-valuenow={Math.round(decibels ?? MINIMUM_DECIBELS)}
    aria-valuetext={strings.voiceSettings.decibels(Math.round(decibels ?? MINIMUM_DECIBELS))}
    class="relative h-(--mw-component-voice-meter-height) w-full overflow-hidden rounded-(--mw-component-voice-meter-radius) bg-(--mw-component-voice-meter-track)"
  >
    <div class="absolute inset-y-0 start-0 bg-(--mw-component-voice-meter-level-quiet)" style:width={percentOf(quietWidth)}></div>
    <div
      class="absolute inset-y-0 bg-(--mw-component-voice-meter-level-heard)"
      style:inset-inline-start={percentOf(threshold)}
      style:width={percentOf(heardWidth)}
    ></div>
  </div>
  {#if thresholdDecibels !== null}
    <input
      type="range"
      min={MINIMUM_DECIBELS}
      max={MAXIMUM_DECIBELS}
      step="1"
      value={thresholdDecibels}
      aria-label={strings.voiceSettings.threshold}
      aria-valuetext={strings.voiceSettings.decibels(thresholdDecibels)}
      aria-describedby={thresholdDescribedBy}
      class="threshold absolute inset-0 m-0 h-full w-full cursor-ew-resize focus-visible:focus-ring"
      oninput={(event) => onthresholdchange(Number(event.currentTarget.value))}
    />
  {/if}
</div>

<style>
  /* The input is only its thumb: a mark taller than the meter, centred on it. */
  .threshold {
    appearance: none;
    background: transparent;
  }
  .threshold::-webkit-slider-runnable-track {
    height: 100%;
    background: transparent;
  }
  .threshold::-webkit-slider-thumb {
    appearance: none;
    width: var(--mw-component-voice-meter-threshold-mark-width);
    height: var(--mw-component-voice-meter-threshold-mark-height);
    margin-top: calc((var(--mw-size-hit-web-min) - var(--mw-component-voice-meter-threshold-mark-height)) / 2);
    border: 0;
    border-radius: var(--mw-radius-full);
    background: var(--mw-component-voice-meter-threshold-mark);
  }
  .threshold::-moz-range-track {
    background: transparent;
  }
  .threshold::-moz-range-thumb {
    width: var(--mw-component-voice-meter-threshold-mark-width);
    height: var(--mw-component-voice-meter-threshold-mark-height);
    border: 0;
    border-radius: var(--mw-radius-full);
    background: var(--mw-component-voice-meter-threshold-mark);
  }
</style>
