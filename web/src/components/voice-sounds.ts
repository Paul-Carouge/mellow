// Voice cues (docs/design/web-voice-channels.md): Discord's grammar in Mellow's own sine voice, made
// with Web Audio like the chime, so there is no sound file. Joining rises, leaving falls; yours
// span a fifth, others' a softer, higher third; toggles are one short, quiet note, lower when muting.

import type { VoiceCue } from "../lib/voice-session.svelte";
import { playNotes } from "./chime-sound";

interface Cue {
  readonly notes: readonly { frequency: number; start: number }[];
  readonly lengthSeconds: number;
  readonly peakGain: number;
}

const C5 = 523.25;
const E5 = 659.25;
const G5 = 783.99;
const B5 = 987.77;
const A4 = 440;
const D5 = 587.33;
const F4 = 349.23;
const SECOND_NOTE_OFFSET_SECONDS = 0.09;
const TOGGLE_LENGTH_SECONDS = 0.1;
const TOGGLE_GAIN = 0.035;

const rising = (low: number, high: number) => [
  { frequency: low, start: 0 },
  { frequency: high, start: SECOND_NOTE_OFFSET_SECONDS },
];
const falling = (low: number, high: number) => [
  { frequency: high, start: 0 },
  { frequency: low, start: SECOND_NOTE_OFFSET_SECONDS },
];
const single = (frequency: number) => [{ frequency, start: 0 }];

const CUES: Record<VoiceCue, Cue> = {
  "self-join": { notes: rising(C5, G5), lengthSeconds: 0.24, peakGain: 0.06 },
  "self-leave": { notes: falling(C5, G5), lengthSeconds: 0.24, peakGain: 0.06 },
  "peer-join": { notes: rising(E5, B5), lengthSeconds: 0.18, peakGain: 0.04 },
  "peer-leave": { notes: falling(E5, B5), lengthSeconds: 0.18, peakGain: 0.04 },
  mute: { notes: single(A4), lengthSeconds: TOGGLE_LENGTH_SECONDS, peakGain: TOGGLE_GAIN },
  unmute: { notes: single(D5), lengthSeconds: TOGGLE_LENGTH_SECONDS, peakGain: TOGGLE_GAIN },
  deafen: { notes: single(F4), lengthSeconds: TOGGLE_LENGTH_SECONDS, peakGain: TOGGLE_GAIN },
  undeafen: { notes: single(C5), lengthSeconds: TOGGLE_LENGTH_SECONDS, peakGain: TOGGLE_GAIN },
};

export function playVoiceCue(cue: VoiceCue): void {
  const { notes, lengthSeconds, peakGain } = CUES[cue];
  playNotes(notes, lengthSeconds, peakGain);
}
