// Mellow's notification sound: two soft sine notes a fifth apart, made with Web Audio, so there is
// no sound file to fetch or license. Browsers start audio only after a gesture on the page, so the
// context is created on the first one and reused.

/** The two notes (A5, then E6) and their timing, in seconds: short, quiet, no attack click. */
const NOTES: readonly { frequency: number; start: number }[] = [
  { frequency: 880, start: 0 },
  { frequency: 1318.5, start: 0.11 },
];
const NOTE_LENGTH_SECONDS = 0.42;
const PEAK_GAIN = 0.07;
const ATTACK_SECONDS = 0.012;
/** Silent gain an exponential ramp can reach (it cannot reach zero). */
const SILENT_GAIN = 0.0001;

let context: AudioContext | null = null;

/** Called from a gesture: creates (or resumes) the audio context the chime will play through. */
export async function prepareChimeSound(): Promise<void> {
  if (typeof AudioContext === "undefined") {
    return;
  }
  context ??= new AudioContext();
  if (context.state === "suspended") {
    try {
      await context.resume();
    } catch (error) {
      console.warn("couldn't start audio", error);
    }
  }
}

/** Plays the chime if audio was allowed by an earlier gesture; silent otherwise. */
export function playChimeSound(): void {
  playNotes(NOTES, NOTE_LENGTH_SECONDS, PEAK_GAIN);
}

/**
 * Soft sine notes through the shared context (voice cues use it too): each starts at its offset,
 * rises without a click and fades over `noteLengthSeconds`. Silent until a gesture allowed audio.
 */
export function playNotes(
  notes: readonly { frequency: number; start: number }[],
  noteLengthSeconds: number,
  peakGain: number,
): void {
  if (context === null || context.state !== "running") {
    return;
  }
  const startsAt = context.currentTime;
  for (const note of notes) {
    const oscillator = context.createOscillator();
    const envelope = context.createGain();
    oscillator.type = "sine";
    oscillator.frequency.value = note.frequency;
    const noteStart = startsAt + note.start;
    envelope.gain.setValueAtTime(SILENT_GAIN, noteStart);
    envelope.gain.exponentialRampToValueAtTime(peakGain, noteStart + ATTACK_SECONDS);
    envelope.gain.exponentialRampToValueAtTime(SILENT_GAIN, noteStart + noteLengthSeconds);
    oscillator.connect(envelope).connect(context.destination);
    oscillator.start(noteStart);
    oscillator.stop(noteStart + noteLengthSeconds);
  }
}
