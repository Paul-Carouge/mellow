// Soundboard audio in the browser (ADR-0032, docs/design/web-soundboard.md): each client plays the
// sounds itself, through the chime's shared context, so they reach the output chosen for voices.
// A sound is downloaded and decoded once, then kept while its server still lists it. Uploads are
// turned into the instance's one format here: decode any file the browser reads, cut, mix to
// mono at 48 kHz with the chosen volume, then `encodeWav`.

import { type Snowflake, SOUND_SAMPLE_RATE, soundUrl } from "../lib/protocol";
import { type SoundSelection, selectionSampleCount } from "../lib/soundboard";
import { prepareChimeSound, runningAudioContext } from "./chime-sound";

/** By sound id; a failed load is forgotten, so the next play tries again. */
const decodedSounds = new Map<Snowflake, Promise<AudioBuffer>>();

function loadSound(context: AudioContext, soundId: Snowflake): Promise<AudioBuffer> {
  const cached = decodedSounds.get(soundId);
  if (cached) {
    return cached;
  }
  const loading = fetch(soundUrl(soundId), { credentials: "same-origin" }).then(async (response) => {
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

function playBuffer(context: AudioContext, buffer: AudioBuffer, gain: number): PlayingSound {
  const source = context.createBufferSource();
  source.buffer = buffer;
  const volume = context.createGain();
  volume.gain.value = gain;
  source.connect(volume).connect(context.destination);
  const ended = new Promise<void>((resolve) => source.addEventListener("ended", () => resolve(), { once: true }));
  source.start();
  return { stop: () => source.stop(), ended };
}

/**
 * Plays a server's sound here at `gain`; `null` when it cannot (no gesture allowed audio yet, the
 * download or the decoding failed), so the caller can fall back to the chime.
 */
export async function playSoundOnThisDevice(soundId: Snowflake, gain: number): Promise<PlayingSound | null> {
  const context = runningAudioContext();
  if (context === null) {
    return null;
  }
  try {
    return playBuffer(context, await loadSound(context, soundId), gain);
  } catch (error) {
    console.warn("couldn't play a soundboard sound", error);
    return null;
  }
}

/** From a click: starts audio first if this is the page's first gesture. */
export async function previewSound(soundId: Snowflake, gain: number): Promise<PlayingSound | null> {
  await prepareChimeSound();
  return playSoundOnThisDevice(soundId, gain);
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
  return playBuffer(context, buffer, gain);
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
