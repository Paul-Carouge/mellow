import { SvelteMap } from "svelte/reactivity";

import { waveformPeaks } from "../lib/soundboard";
import { type Attachment, type Snowflake, attachmentUrl } from "../lib/protocol";
import {
  MAXIMUM_WAVEFORM_SOURCE_BYTES,
  MAXIMUM_WAVEFORM_SOURCE_SECONDS,
  normalizedPeaks,
  WAVEFORM_DECODE_SAMPLE_RATE,
} from "./audio-player";

// What the list knows of each sound's shape (DESIGN.md 5.4, "Sound player"): its waveform and its
// length, learnt the first time it plays and kept for the tab's life. Nothing is fetched for a
// sound nobody played (the instance decrypts a whole file per request), and nothing outlives the
// tab: a shared computer keeps no trace of what was listened to.

export interface SoundShape {
  /** One level per bar, 0 to 1, or null while unknown. */
  peaks: readonly number[] | null;
  durationSeconds: number | null;
}

/** A long session's worth of sounds; the oldest is forgotten first. */
const MAXIMUM_REMEMBERED_SOUNDS = 200;

const shapes = new SvelteMap<Snowflake, SoundShape>();
const soundsBeingRead = new Set<Snowflake>();
// Sounds whose shape could not be read: not asked again on every resume (a busy instance would
// decrypt the whole file each time). Bounded like the shapes, oldest forgotten first.
const soundsThatFailed = new Set<Snowflake>();

function rememberFailure(id: Snowflake): void {
  soundsThatFailed.add(id);
  if (soundsThatFailed.size > MAXIMUM_REMEMBERED_SOUNDS) {
    const oldest = soundsThatFailed.values().next().value;
    if (oldest !== undefined) {
      soundsThatFailed.delete(oldest);
    }
  }
}

function remember(id: Snowflake, shape: SoundShape): void {
  shapes.delete(id);
  shapes.set(id, shape);
  if (shapes.size > MAXIMUM_REMEMBERED_SOUNDS) {
    const oldest = shapes.keys().next().value;
    if (oldest !== undefined) {
      shapes.delete(oldest);
    }
  }
}

export function soundShapeOf(id: Snowflake): SoundShape | null {
  return shapes.get(id) ?? null;
}

/** The player learnt the sound's length from its metadata. */
export function rememberSoundDuration(id: Snowflake, durationSeconds: number): void {
  if (!Number.isFinite(durationSeconds) || durationSeconds <= 0) {
    return;
  }
  const known = shapes.get(id);
  if (known?.durationSeconds !== durationSeconds) {
    remember(id, { peaks: known?.peaks ?? null, durationSeconds });
  }
}

function isShortEnoughToDecode(durationSeconds: number): boolean {
  return Number.isFinite(durationSeconds) && durationSeconds > 0 && durationSeconds <= MAXIMUM_WAVEFORM_SOURCE_SECONDS;
}

async function decodeSoundShape(bytes: ArrayBuffer, barCount: number): Promise<SoundShape> {
  // Decoded at a low rate: bars a 48th of the sound wide need nothing finer, and the loop over the
  // samples stays short on the main thread (7.2 million samples for the longest, 15-minute sound).
  const context = new OfflineAudioContext(1, 1, WAVEFORM_DECODE_SAMPLE_RATE);
  const buffer = await context.decodeAudioData(bytes);
  // shortcut: the length checked before decoding is the one the file's header gave the player; a
  // file lying about it is still decoded whole before this check. Only its samples are not walked.
  // A server-computed waveform (DESIGN.md 9, question 42) removes the decoding altogether.
  if (!isShortEnoughToDecode(buffer.duration)) {
    throw new Error(`the sound decoded to ${Math.round(buffer.duration)} s, longer than its header said`);
  }
  const channels = Array.from({ length: buffer.numberOfChannels }, (_, index) => buffer.getChannelData(index));
  return { peaks: normalizedPeaks(waveformPeaks(channels, barCount)), durationSeconds: buffer.duration };
}

/**
 * Reads the sound's waveform once it played: the file again (the browser revalidates it with its
 * entity tag), decoded off the page's audio. `durationSeconds` is the player's, known once it plays.
 * Files over `MAXIMUM_WAVEFORM_SOURCE_BYTES` or sounds over `MAXIMUM_WAVEFORM_SOURCE_SECONDS` keep
 * the flat line. A failure keeps it too, is logged without the file's name and is not retried.
 */
export async function readSoundShape(attachment: Attachment, durationSeconds: number, barCount: number): Promise<void> {
  const known = shapes.get(attachment.id);
  if (
    known?.peaks ||
    soundsBeingRead.has(attachment.id) ||
    soundsThatFailed.has(attachment.id) ||
    attachment.size_bytes > MAXIMUM_WAVEFORM_SOURCE_BYTES ||
    !isShortEnoughToDecode(durationSeconds)
  ) {
    return;
  }
  soundsBeingRead.add(attachment.id);
  try {
    const response = await fetch(attachmentUrl(attachment));
    if (!response.ok) {
      throw new Error(`the instance answered ${response.status}`);
    }
    const shape = await decodeSoundShape(await response.arrayBuffer(), barCount);
    remember(attachment.id, { peaks: shape.peaks, durationSeconds: known?.durationSeconds ?? shape.durationSeconds });
  } catch (error: unknown) {
    rememberFailure(attachment.id);
    console.warn("couldn't read a sound's waveform", error);
  } finally {
    soundsBeingRead.delete(attachment.id);
  }
}
