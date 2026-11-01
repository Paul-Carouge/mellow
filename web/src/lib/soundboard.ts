// Pure rules of the soundboard (ADR-0032, docs/design/web-soundboard.md): the one WAV format the
// instance accepts, the part of a file a person keeps, sound names, when the soundboard can play,
// and which incoming sounds this client plays.

import {
  MAXIMUM_SOUND_NAME_LENGTH,
  MAXIMUM_SOUND_SAMPLES,
  MINIMUM_MILLISECONDS_BETWEEN_SOUNDS,
  MINIMUM_SOUND_NAME_LENGTH,
  Permission,
  type PermissionBits,
  SOUND_SAMPLE_RATE,
  type Snowflake,
  type SoundboardSound,
  hasPermission,
} from "./protocol";

// --- The WAV file --------------------------------------------------------------------------------

const WAV_HEADER_BYTES = 44;
const FMT_CHUNK_BYTES = 16;
const PCM_FORMAT = 1;
const CHANNEL_COUNT = 1;
const BYTES_PER_SAMPLE = 2;
const BITS_PER_SAMPLE = 16;

/**
 * The canonical file the instance accepts, exactly: a 44-byte RIFF/WAVE header (PCM, mono,
 * 48 kHz, 16-bit) and the samples, little-endian, nothing after. Samples outside -1 to 1 are
 * clamped; `NaN` becomes silence. Throws for an empty or too long sound: the trimmer prevents both.
 */
export function encodeWav(samples: Float32Array): Uint8Array<ArrayBuffer> {
  if (samples.length < 1 || samples.length > MAXIMUM_SOUND_SAMPLES) {
    throw new RangeError(`a sound holds 1 to ${MAXIMUM_SOUND_SAMPLES} samples, not ${samples.length}`);
  }
  const dataBytes = samples.length * BYTES_PER_SAMPLE;
  const bytes = new Uint8Array(WAV_HEADER_BYTES + dataBytes);
  const view = new DataView(bytes.buffer);
  const writeText = (offset: number, text: string) => {
    for (let index = 0; index < text.length; index += 1) {
      bytes[offset + index] = text.charCodeAt(index);
    }
  };
  writeText(0, "RIFF");
  view.setUint32(4, bytes.length - 8, true);
  writeText(8, "WAVE");
  writeText(12, "fmt ");
  view.setUint32(16, FMT_CHUNK_BYTES, true);
  view.setUint16(20, PCM_FORMAT, true);
  view.setUint16(22, CHANNEL_COUNT, true);
  view.setUint32(24, SOUND_SAMPLE_RATE, true);
  view.setUint32(28, SOUND_SAMPLE_RATE * CHANNEL_COUNT * BYTES_PER_SAMPLE, true);
  view.setUint16(32, CHANNEL_COUNT * BYTES_PER_SAMPLE, true);
  view.setUint16(34, BITS_PER_SAMPLE, true);
  writeText(36, "data");
  view.setUint32(40, dataBytes, true);
  for (let index = 0; index < samples.length; index += 1) {
    const sample = samples[index] ?? 0;
    const clamped = Number.isNaN(sample) ? 0 : Math.max(-1, Math.min(1, sample));
    view.setInt16(WAV_HEADER_BYTES + index * BYTES_PER_SAMPLE, Math.round(clamped < 0 ? clamped * 0x8000 : clamped * 0x7fff), true);
  }
  return bytes;
}

// --- Trimming ------------------------------------------------------------------------------------

/**
 * The largest file the trimmer opens: decoding holds the whole file in memory as 32-bit samples
 * (a 20 MB MP3 of 20 minutes is about 460 MB once decoded), and only 5.2 s are kept anyway.
 */
export const MAXIMUM_SOUND_SOURCE_BYTES = 20 * 1024 * 1024;

/** 5.2 s, Discord's longest sound. */
export const MAXIMUM_SOUND_SECONDS = MAXIMUM_SOUND_SAMPLES / SOUND_SAMPLE_RATE;
/** Shorter selections are hard to hear and to grab; a shorter file is kept whole. */
export const MINIMUM_SELECTION_SECONDS = 0.1;

/** The part of the file that becomes the sound, in seconds from its start. */
export interface SoundSelection {
  readonly startSeconds: number;
  readonly endSeconds: number;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(maximum, Math.max(minimum, value));
}

function shortestSelection(durationSeconds: number): number {
  return Math.min(MINIMUM_SELECTION_SECONDS, durationSeconds);
}

/** The start of the file, as long as a sound can be. */
export function initialSelection(durationSeconds: number): SoundSelection {
  return { startSeconds: 0, endSeconds: Math.min(durationSeconds, MAXIMUM_SOUND_SECONDS) };
}

/** Moving the start keeps the end where it was, unless the length leaves the allowed range. */
export function withSelectionStart(selection: SoundSelection, startSeconds: number, durationSeconds: number): SoundSelection {
  const shortest = shortestSelection(durationSeconds);
  const start = clamp(startSeconds, 0, durationSeconds - shortest);
  const end = clamp(selection.endSeconds, start + shortest, Math.min(start + MAXIMUM_SOUND_SECONDS, durationSeconds));
  return { startSeconds: start, endSeconds: end };
}

/** Moving the end keeps the start where it was, unless the length leaves the allowed range. */
export function withSelectionEnd(selection: SoundSelection, endSeconds: number, durationSeconds: number): SoundSelection {
  const shortest = shortestSelection(durationSeconds);
  const end = clamp(endSeconds, shortest, durationSeconds);
  const start = clamp(selection.startSeconds, Math.max(end - MAXIMUM_SOUND_SECONDS, 0), end - shortest);
  return { startSeconds: start, endSeconds: end };
}

/** A click or drag on the waveform: the selection keeps its length and centres there. */
export function withSelectionCentredAt(selection: SoundSelection, centreSeconds: number, durationSeconds: number): SoundSelection {
  const length = selection.endSeconds - selection.startSeconds;
  const start = clamp(centreSeconds - length / 2, 0, durationSeconds - length);
  return { startSeconds: start, endSeconds: start + length };
}

/** How many samples the rendered sound holds: never empty, never past the instance's limit. */
export function selectionSampleCount(selection: SoundSelection): number {
  return clamp(Math.round((selection.endSeconds - selection.startSeconds) * SOUND_SAMPLE_RATE), 1, MAXIMUM_SOUND_SAMPLES);
}

/**
 * The loudest sample of each of `bucketCount` equal slices of the file, 0 to 1, across its
 * channels: the bars of the trimmer's waveform.
 */
export function waveformPeaks(channels: readonly Float32Array[], bucketCount: number): number[] {
  const length = Math.max(0, ...channels.map((channel) => channel.length));
  const peaks = new Array<number>(bucketCount).fill(0);
  if (length === 0 || bucketCount <= 0) {
    return peaks;
  }
  for (const channel of channels) {
    for (let index = 0; index < channel.length; index += 1) {
      const bucket = Math.min(bucketCount - 1, Math.floor((index * bucketCount) / length));
      const magnitude = Math.abs(channel[index] ?? 0);
      if (magnitude > (peaks[bucket] ?? 0)) {
        peaks[bucket] = Math.min(1, magnitude);
      }
    }
  }
  return peaks;
}

// --- Names ---------------------------------------------------------------------------------------

/** Rust counts characters as code points, so the client does too. */
function characterCount(text: string): number {
  return [...text].length;
}

/** Why the instance would refuse this name, or `null`. Names need not be unique. */
export function soundNameProblem(name: string): "length" | "control-character" | null {
  const trimmed = name.trim();
  if (/\p{Cc}/u.test(trimmed)) {
    return "control-character";
  }
  const length = characterCount(trimmed);
  return length < MINIMUM_SOUND_NAME_LENGTH || length > MAXIMUM_SOUND_NAME_LENGTH ? "length" : null;
}

/** "airhorn_long-v2.mp3" → "airhorn long v2": a starting point the person can edit. */
export function soundNameFromFileName(fileName: string): string {
  const withoutExtension = fileName.replace(/\.[^.]{1,5}$/u, "");
  const spaced = withoutExtension
    .replace(/\p{Cc}/gu, "")
    .replace(/[_-]+/gu, " ")
    .replace(/\s+/gu, " ")
    .trim();
  return [...spaced].slice(0, MAXIMUM_SOUND_NAME_LENGTH).join("").trim();
}

// --- Playing -------------------------------------------------------------------------------------

/** Why the soundboard cannot play now; the server would silently ignore the frame. */
export type SoundboardBlock = "not-connected" | "no-permission" | "cannot-speak" | "deafened" | "cooling-down";

export interface SoundboardFacts {
  /** Voice is connected (not joining or reconnecting). */
  readonly isConnected: boolean;
  /** Your permissions in the connected channel; `null` while unknown (the instance decides). */
  readonly channelPermissions: PermissionBits | null;
  readonly isDeafened: boolean;
  readonly now: number;
  readonly cooldownEndsAt: number;
}

/** The first reason in the order a person can act on it; self-mute does not block (ADR-0032). */
export function soundboardBlock(facts: SoundboardFacts): SoundboardBlock | null {
  if (!facts.isConnected) {
    return "not-connected";
  }
  if (facts.channelPermissions !== null) {
    if (!hasPermission(facts.channelPermissions, Permission.USE_SOUNDBOARD)) {
      return "no-permission";
    }
    if (!hasPermission(facts.channelPermissions, Permission.SPEAK)) {
      return "cannot-speak";
    }
  }
  if (facts.isDeafened) {
    return "deafened";
  }
  return facts.now < facts.cooldownEndsAt ? "cooling-down" : null;
}

/**
 * When the soundboard can play again after a sound played at `playedAt`. Started on the click,
 * pushed back on the instance's echo: the next frame then reaches it at least 2 s after the time it
 * recorded, whatever the network's delays.
 */
export function cooldownEndAfter(currentEndsAt: number, playedAt: number): number {
  return Math.max(currentEndsAt, playedAt + MINIMUM_MILLISECONDS_BETWEEN_SOUNDS);
}

/** A played sound is heard in your channel only, never deafened, never from someone muted for you. */
export function shouldHearSound(facts: {
  readonly soundChannelId: Snowflake;
  readonly myChannelId: Snowflake | null;
  readonly isConnected: boolean;
  readonly isDeafened: boolean;
  readonly isPlayerMutedForMe: boolean;
}): boolean {
  return facts.isConnected && facts.soundChannelId === facts.myChannelId && !facts.isDeafened && !facts.isPlayerMutedForMe;
}

/**
 * What plays when someone comes into your channel: their join sound when the server still has it
 * and you would hear it, else the usual chime (deafened, muted for you, or no sound chosen).
 */
export function joinCueOf(
  joinSoundId: Snowflake | null | undefined,
  sounds: readonly SoundboardSound[],
  facts: { readonly isDeafened: boolean; readonly isMutedForMe: boolean },
): { readonly kind: "sound"; readonly sound: SoundboardSound } | { readonly kind: "chime" } {
  const sound = joinSoundId ? sounds.find((candidate) => candidate.id === joinSoundId) : undefined;
  return sound && !facts.isDeafened && !facts.isMutedForMe ? { kind: "sound", sound } : { kind: "chime" };
}

/**
 * Sounds play at your output volume (up to 200 %) times your soundboard volume (up to 100 %): the
 * soundboard slider alone never makes a sound louder than its file, the output volume can.
 */
export function soundboardGain(outputVolumePercent: number, soundboardVolumePercent: number): number {
  return (outputVolumePercent / 100) * (soundboardVolumePercent / 100);
}

/** The emoji shows by the player at least this long, longer for a longer sound. */
export const MINIMUM_SOUND_INDICATOR_MILLISECONDS = 1_500;

export function soundIndicatorMilliseconds(sound: Pick<SoundboardSound, "duration_milliseconds"> | undefined): number {
  return Math.max(MINIMUM_SOUND_INDICATOR_MILLISECONDS, sound?.duration_milliseconds ?? 0);
}

/** "0:03.2": the trimmer's times, to the tenth. */
export function formatSoundSeconds(seconds: number, decimalSeparator: string): string {
  const tenths = Math.round(Math.max(0, seconds) * 10);
  const minutes = Math.floor(tenths / 600);
  const remainingTenths = tenths - minutes * 600;
  const wholeSeconds = Math.floor(remainingTenths / 10);
  return `${minutes}:${String(wholeSeconds).padStart(2, "0")}${decimalSeparator}${remainingTenths % 10}`;
}
