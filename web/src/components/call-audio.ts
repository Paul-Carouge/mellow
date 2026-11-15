// The sounds of private calls (ADR-0062, DESIGN.md §5.6.3), through the chime's shared Web Audio
// context, so they reach the output chosen for voices and stay silent until a gesture on the page
// allowed audio: the owner's ringtone looped while a call rings, the caller's generated waiting
// tone (425 Hz, 1 s on and 4 s off, the European ringback), and a short cue for a call that rings
// someone already in a call.

import { playNotes, runningAudioContext } from "./chime-sound";

/** Stops a sound that loops; calling it twice is harmless. */
export type StopSound = () => void;

const RINGBACK_FREQUENCY_HERTZ = 425;
const RINGBACK_ON_SECONDS = 1;
const RINGBACK_CYCLE_SECONDS = 5;
/** Soft, under the chime's 0.07, so "I am calling" never covers the call's first words. */
const RINGBACK_PEAK_GAIN = 0.05;
/** Edges without a click. */
const RINGBACK_EDGE_SECONDS = 0.03;
/** Cycles scheduled ahead, so a throttled background timer never leaves a gap. */
const RINGBACK_CYCLES_AHEAD = 2;
/** The ringtone's level before the output volume: the files are mastered loud. */
const RINGTONE_GAIN = 0.5;
/** Two soft notes a third apart, lower than the chime: "another call" without a ring. */
const CALL_WAITING_NOTES: readonly { frequency: number; start: number }[] = [
  { frequency: 659.25, start: 0 },
  { frequency: 523.25, start: 0.16 },
];
const CALL_WAITING_NOTE_SECONDS = 0.3;
const CALL_WAITING_GAIN = 0.05;
/** A ringtone the browser cannot read rings with those notes, this often. */
const FALLBACK_RING_INTERVAL_MILLISECONDS = 2_000;
/** Silent gain an exponential ramp can reach (it cannot reach zero). */
const SILENT_GAIN = 0.0001;

/** Decoded once per file and context: the ringtone may ring many times in a session. */
const decodedRingtones = new WeakMap<AudioContext, Map<string, Promise<AudioBuffer>>>();

function decodeRingtone(context: AudioContext, url: string): Promise<AudioBuffer> {
  let byUrl = decodedRingtones.get(context);
  if (!byUrl) {
    byUrl = new Map();
    decodedRingtones.set(context, byUrl);
  }
  let decoding = byUrl.get(url);
  if (!decoding) {
    decoding = fetch(url)
      .then((response) => {
        if (!response.ok) {
          throw new Error(`the ringtone answered ${response.status}`);
        }
        return response.arrayBuffer();
      })
      .then((bytes) => context.decodeAudioData(bytes));
    // A failure is not kept: the next call tries again.
    decoding.catch(() => byUrl.delete(url));
    byUrl.set(url, decoding);
  }
  return decoding;
}

/**
 * Loops the ringtone at `volume` (0 to 2, the output volume) until stopped. `null` when audio is
 * not allowed yet (no gesture on the page): the caller tries again once it is. A ringtone that
 * cannot be read (a browser without Ogg Opus) rings with the call-waiting notes instead.
 */
export function startRingtone(url: string, volume: number): StopSound | null {
  const context = runningAudioContext();
  if (context === null) {
    return null;
  }
  let isStopped = false;
  let source: AudioBufferSourceNode | null = null;
  let fallbackTimer: ReturnType<typeof setInterval> | null = null;
  const output = context.createGain();
  output.gain.value = RINGTONE_GAIN * volume;
  output.connect(context.destination);
  decodeRingtone(context, url)
    .then((buffer) => {
      if (isStopped) {
        return;
      }
      source = context.createBufferSource();
      source.buffer = buffer;
      source.loop = true;
      source.connect(output);
      source.start();
    })
    .catch((error: unknown) => {
      console.warn("couldn't play the ringtone; ringing with notes instead", error);
      if (isStopped) {
        return;
      }
      playCallWaitingCue();
      fallbackTimer = setInterval(playCallWaitingCue, FALLBACK_RING_INTERVAL_MILLISECONDS);
    });
  return () => {
    if (isStopped) {
      return;
    }
    isStopped = true;
    source?.stop();
    source?.disconnect();
    output.disconnect();
    if (fallbackTimer !== null) {
      clearInterval(fallbackTimer);
    }
  };
}

/** The caller's waiting tone until stopped; `null` while audio is not allowed yet. */
export function startRingback(volume: number): StopSound | null {
  const context = runningAudioContext();
  if (context === null) {
    return null;
  }
  const oscillator = context.createOscillator();
  const envelope = context.createGain();
  oscillator.type = "sine";
  oscillator.frequency.value = RINGBACK_FREQUENCY_HERTZ;
  envelope.gain.value = SILENT_GAIN;
  oscillator.connect(envelope).connect(context.destination);
  const peak = Math.max(SILENT_GAIN, RINGBACK_PEAK_GAIN * volume);
  let nextCycleAt = context.currentTime + RINGBACK_EDGE_SECONDS;
  const scheduleCycles = () => {
    while (nextCycleAt < context.currentTime + RINGBACK_CYCLES_AHEAD * RINGBACK_CYCLE_SECONDS) {
      const start = nextCycleAt;
      envelope.gain.setValueAtTime(SILENT_GAIN, start);
      envelope.gain.exponentialRampToValueAtTime(peak, start + RINGBACK_EDGE_SECONDS);
      envelope.gain.setValueAtTime(peak, start + RINGBACK_ON_SECONDS - RINGBACK_EDGE_SECONDS);
      envelope.gain.exponentialRampToValueAtTime(SILENT_GAIN, start + RINGBACK_ON_SECONDS);
      nextCycleAt += RINGBACK_CYCLE_SECONDS;
    }
  };
  scheduleCycles();
  oscillator.start();
  const timer = setInterval(scheduleCycles, RINGBACK_CYCLE_SECONDS * 1000);
  let isStopped = false;
  return () => {
    if (isStopped) {
      return;
    }
    isStopped = true;
    clearInterval(timer);
    envelope.gain.cancelScheduledValues(context.currentTime);
    envelope.gain.setValueAtTime(SILENT_GAIN, context.currentTime);
    oscillator.stop();
    oscillator.disconnect();
    envelope.disconnect();
  };
}

/** Once, for a call ringing someone already in a call or a voice channel. Silent without a gesture. */
export function playCallWaitingCue(): void {
  playNotes(CALL_WAITING_NOTES, CALL_WAITING_NOTE_SECONDS, CALL_WAITING_GAIN);
}
