// The sound player's rules (DESIGN.md 5.4, "Sound player"): the waveform's bars, how keys move the
// playhead, the speeds. No DOM here; `AudioPlayer.svelte` draws them and `audio-waveforms.svelte.ts`
// reads a sound's shape.

/** Arrow keys move the playhead this far, as most players do. */
export const SEEK_STEP_SECONDS = 5;
/** Page Up and Page Down move it a tenth of the sound. */
const SEEK_PAGE_FRACTION = 0.1;
/** Before the sound's length is known, an arrow moves it a twentieth. */
const SEEK_STEP_FRACTION_WITHOUT_DURATION = 0.05;

/** The speeds the speed key cycles through: voice notes at 1.5× and 2×, music at 1×. */
export const PLAYBACK_SPEEDS: readonly number[] = [1, 1.5, 2];

/**
 * The waveform is read only for files up to this size, once played: the file is downloaded again to
 * be decoded, so a long recording would cost its whole size a second time.
 */
export const MAXIMUM_WAVEFORM_SOURCE_BYTES = 10 * 1024 * 1024;
/**
 * Nor for sounds longer than this, whatever their size: 10 MiB of silent Opus or FLAC last hours,
 * and decoding them would hold hundreds of megabytes and the main thread.
 */
export const MAXIMUM_WAVEFORM_SOURCE_SECONDS = 15 * 60;
/** Decoding at this rate is enough for bars a 48th of the sound wide, and keeps decoding cheap. */
export const WAVEFORM_DECODE_SAMPLE_RATE = 8_000;

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(maximum, Math.max(minimum, value));
}

/** Peaks scaled so the loudest bar fills the box: a quiet voice note still shows its words. */
export function normalizedPeaks(peaks: readonly number[]): number[] {
  const loudest = Math.max(0, ...peaks);
  return loudest > 0 ? peaks.map((peak) => clamp(peak / loudest, 0, 1)) : peaks.map(() => 0);
}

/** The flat line drawn while the sound's shape is unknown. */
export function placeholderPeaks(barCount: number, level: number): number[] {
  return Array.from({ length: Math.max(0, barCount) }, () => level);
}

/** How many bars are behind the playhead: a bar counts once the playhead passed its middle. */
export function playedBarCount(fraction: number, barCount: number): number {
  return clamp(Math.round(clamp(fraction, 0, 1) * barCount), 0, barCount);
}

/** Where the sound is, 0 to 1; 0 while its length is unknown. */
export function playbackFraction(currentSeconds: number, durationSeconds: number | null): number {
  if (durationSeconds === null || !Number.isFinite(durationSeconds) || durationSeconds <= 0) {
    return 0;
  }
  return clamp(currentSeconds / durationSeconds, 0, 1);
}

/**
 * Where a key on the waveform moves the playhead, as a fraction of the sound, or null for a key it
 * does not handle. Left and Down go back, Right and Up forward (the slider's keys everywhere).
 */
export function seekFractionAfterKey(key: string, fraction: number, durationSeconds: number | null): number | null {
  const hasDuration = durationSeconds !== null && Number.isFinite(durationSeconds) && durationSeconds > 0;
  const step = hasDuration ? SEEK_STEP_SECONDS / durationSeconds : SEEK_STEP_FRACTION_WITHOUT_DURATION;
  switch (key) {
    case "ArrowRight":
    case "ArrowUp":
      return clamp(fraction + step, 0, 1);
    case "ArrowLeft":
    case "ArrowDown":
      return clamp(fraction - step, 0, 1);
    case "PageUp":
      return clamp(fraction + SEEK_PAGE_FRACTION, 0, 1);
    case "PageDown":
      return clamp(fraction - SEEK_PAGE_FRACTION, 0, 1);
    case "Home":
      return 0;
    case "End":
      return 1;
    default:
      return null;
  }
}

/** The speed after `speed` in the cycle; back to 1× after the last or from any other speed. */
export function nextPlaybackSpeed(speed: number): number {
  const index = PLAYBACK_SPEEDS.indexOf(speed);
  return PLAYBACK_SPEEDS[index + 1] ?? PLAYBACK_SPEEDS[0] ?? 1;
}

/** "1,5" in French, "1.5" in English: the speed key shows it with "×". */
export function playbackSpeedNumber(speed: number, locale: string): string {
  return speed.toLocaleString(locale, { maximumFractionDigits: 2 });
}
