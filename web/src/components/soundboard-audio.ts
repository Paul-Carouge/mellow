// Soundboard audio in the browser (ADR-0032, docs/design/web-soundboard.md): each client plays the
// sounds itself, through the chime's shared context, so they reach the output chosen for voices.
// A sound is downloaded and decoded once, then kept while its server still lists it. Uploads are
// turned into the instance's one format here: decode any file the browser reads, cut, mix to
// mono at 48 kHz with the chosen volume, then `encodeWav`.

import { type Snowflake, SOUND_SAMPLE_RATE, soundUrl } from "../lib/protocol";
import { type SoundSelection, selectionSampleCount } from "../lib/soundboard";
import { prepareChimeSound, runningAudioContext } from "./chime-sound";

/**
 * A sound (at most about 488 KB) that has not arrived by then is given up: a join sound then
 * gives way to its chime while the arrival is still news.
 */
const SOUND_DOWNLOAD_TIMEOUT_MILLISECONDS = 5_000;

/** By sound id; a failed load is forgotten, so the next play tries again. */
const decodedSounds = new Map<Snowflake, Promise<AudioBuffer>>();

/** What plays now, by who played it (`null`: a preview, yours only). */
const playingSounds = new Set<{ readonly playerId: Snowflake | null; readonly stop: () => void }>();

function loadSound(context: AudioContext, soundId: Snowflake): Promise<AudioBuffer> {
  const cached = decodedSounds.get(soundId);
  if (cached) {
    return cached;
  }
  const loading = fetch(soundUrl(soundId), {
    credentials: "same-origin",
    signal: AbortSignal.timeout(SOUND_DOWNLOAD_TIMEOUT_MILLISECONDS),
  }).then(async (response) => {
    if (!response.ok) {
      throw new Error(`sound ${soundId} answered ${response.status}`);
    }
    return context.decodeAudioData(await response.arrayBuffer());
  });
  decodedSounds.set(soundId, loading);
  loading.catch(() => decodedSounds.delete(soundId));
  return loading;
}

/** Drops the decoded sounds no server lists any more. */
export function forgetSoundsExcept(soundIds: ReadonlySet<Snowflake>): void {
  for (const soundId of decodedSounds.keys()) {
    if (!soundIds.has(soundId)) {
      decodedSounds.delete(soundId);
    }
  }
}

/** A playing sound; overlapping ones are allowed, as on Discord. */
export interface PlayingSound {
  stop(): void;
  /** Resolves when it ends or is stopped. */
  readonly ended: Promise<void>;
}

function playBuffer(context: AudioContext, buffer: AudioBuffer, gain: number, playerId: Snowflake | null): PlayingSound {
  const source = context.createBufferSource();
  source.buffer = buffer;
  const volume = context.createGain();
  volume.gain.value = gain;
  source.connect(volume).connect(context.destination);
  const entry = { playerId, stop: () => source.stop() };
  playingSounds.add(entry);
  const ended = new Promise<void>((resolve) =>
    source.addEventListener(
      "ended",
      () => {
        playingSounds.delete(entry);
        resolve();
      },
      { once: true },
    ),
  );
  source.start();
  return { stop: entry.stop, ended };
}

/** Stops the sounds whose player `shouldStop` names: everyone's when you deafen, one person's when you mute them for you. */
export function stopPlayingSounds(shouldStop: (playerId: Snowflake | null) => boolean): void {
  for (const entry of [...playingSounds]) {
    if (shouldStop(entry.playerId)) {
      playingSounds.delete(entry);
      entry.stop();
    }
  }
}

/**
 * Plays a server's sound here at `gain`, as played by `playerId` (`null` for a preview).
 * `"failed"` when it cannot (no gesture allowed audio yet, the download or the decoding failed),
 * so the caller can fall back to the chime; `"skipped"` when `isStillWanted` said no once it was
 * ready (you deafened or muted the player meanwhile).
 */
export async function playSoundOnThisDevice(
  soundId: Snowflake,
  gain: number,
  playerId: Snowflake | null,
  isStillWanted: () => boolean = () => true,
): Promise<PlayingSound | "failed" | "skipped"> {
  const context = runningAudioContext();
  if (context === null) {
    return "failed";
  }
  let buffer: AudioBuffer;
  try {
    buffer = await loadSound(context, soundId);
  } catch (error) {
    console.warn("couldn't play a soundboard sound", error);
    return "failed";
  }
  return isStillWanted() ? playBuffer(context, buffer, gain, playerId) : "skipped";
}

/** From a click: starts audio first if this is the page's first gesture. */
export async function previewSound(soundId: Snowflake, gain: number): Promise<PlayingSound | null> {
  await prepareChimeSound();
  const playing = await playSoundOnThisDevice(soundId, gain, null);
  return typeof playing === "string" ? null : playing;
}

/** A rendered selection in the upload dialog, before it exists on the instance. */
export async function previewSamples(samples: Float32Array<ArrayBuffer>, gain: number): Promise<PlayingSound | null> {
  await prepareChimeSound();
  const context = runningAudioContext();
  if (context === null) {
    return null;
  }
  const buffer = context.createBuffer(1, samples.length, SOUND_SAMPLE_RATE);
  buffer.copyToChannel(samples, 0);
  return playBuffer(context, buffer, gain, null);
}

/**
 * Any file the browser can decode, resampled to 48 kHz (`decodeAudioData` converts to its
 * context's rate), its channels kept for the waveform. Rejects when the file is not audio.
 */
export async function decodeSoundFile(file: Blob): Promise<AudioBuffer> {
  const context = new OfflineAudioContext(1, 1, SOUND_SAMPLE_RATE);
  return context.decodeAudioData(await file.arrayBuffer());
}

/**
 * The selection as the instance will store it: mono (the destination's single channel downmixes,
 * as speakers would), 48 kHz, at `volumePercent`. `encodeWav` clamps what the gain pushes past full scale.
 */
export async function renderSound(source: AudioBuffer, selection: SoundSelection, volumePercent: number): Promise<Float32Array<ArrayBuffer>> {
  const length = selectionSampleCount(selection);
  const context = new OfflineAudioContext(1, length, SOUND_SAMPLE_RATE);
  const player = context.createBufferSource();
  player.buffer = source;
  const volume = context.createGain();
  volume.gain.value = volumePercent / 100;
  player.connect(volume).connect(context.destination);
  player.start(0, selection.startSeconds, length / SOUND_SAMPLE_RATE);
  const rendered = await context.startRendering();
  return rendered.getChannelData(0);
}
