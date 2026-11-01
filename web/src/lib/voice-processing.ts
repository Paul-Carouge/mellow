// The microphone's processing chain (ADR-0031, docs/design/web-voice-enhancement.md), as plain
// data: which `getUserMedia` processing to ask for, and which Web Audio stages to build, in order.
// `voice-browser.ts` turns the stages into nodes; everything here is tested without a browser.

import type { EqualizerBand, EqualizerDecibels, VoiceSettings } from "./voice-settings";

/** Why RNNoise is not running: the browser lacks AudioWorklet or WebAssembly, or loading it failed. */
export type NoiseSuppressorProblem = "unsupported" | "failed";

/** Which microphone, and what the browser's own processing does to it. */
export interface MicrophoneChoice {
  /** `null`: the system's default. */
  readonly deviceId: string | null;
  readonly isBrowserNoiseSuppressionOn: boolean;
  /** Off while the voice enhancement's compressor sets the loudness: the two would fight. */
  readonly isAutomaticGainOn: boolean;
}

/** What the page does to the microphone after the browser. */
export interface MicrophoneProcessing {
  readonly isNoiseSuppressorOn: boolean;
  /** `null`: the voice enhancement is off. */
  readonly equalizer: EqualizerDecibels | null;
}

/** RNNoise works on 10 ms frames at 48 kHz only. */
export const NOISE_SUPPRESSOR_SAMPLE_RATE_HERTZ = 48_000;

/** Below the lowest voices: desk thumps, traffic and fan rumble. */
const RUMBLE_CUTOFF_HERTZ = 90;
/** A Butterworth slope: flat above the cutoff, no bump. */
const RUMBLE_FILTER_QUALITY = Math.SQRT1_2;
const BASS_SHELF_HERTZ = 120;
/** Where a voice sounds boxy or muddy in a small room. */
const MIDS_PEAK_HERTZ = 350;
/** About 1.5 octaves wide: a gentle bell, never a notch. */
const MIDS_PEAK_QUALITY = 0.9;
/** Presence and air, above the sibilance-heavy 3–4 kHz. */
const TREBLE_SHELF_HERTZ = 5_000;
/** Shelves ignore their quality in Web Audio. */
const SHELF_QUALITY = 1;

/**
 * About 3:1 above −24 dBFS: quiet words come up, shouts come down. Web Audio's compressor adds its
 * own make-up gain from these settings (about +9 dB here), which replaces the browser's automatic gain.
 */
const COMPRESSOR = { thresholdDecibels: -24, kneeDecibels: 10, ratio: 3, attackSeconds: 0.005, releaseSeconds: 0.2 } as const;
/** The hardest the compressor goes; with its own make-up gain, peaks stop just under −1 dBFS. */
const LIMITER = { thresholdDecibels: -2, kneeDecibels: 0, ratio: 20, attackSeconds: 0.001, releaseSeconds: 0.05 } as const;

export type ProcessingStage =
  | { readonly kind: "noise-suppressor" }
  | {
      readonly kind: "filter";
      /** `"rumble"`: the fixed high-pass; otherwise the slider it follows. */
      readonly band: "rumble" | EqualizerBand;
      readonly type: "highpass" | "lowshelf" | "peaking" | "highshelf";
      readonly frequencyHertz: number;
      readonly quality: number;
      readonly gainDecibels: number;
    }
  | {
      readonly kind: "dynamics";
      readonly role: "compressor" | "limiter";
      readonly thresholdDecibels: number;
      readonly kneeDecibels: number;
      readonly ratio: number;
      readonly attackSeconds: number;
      readonly releaseSeconds: number;
    };

/**
 * `isNoiseSuppressorUsable`: RNNoise loaded (or not tried yet). Where it cannot run, "advanced"
 * falls back to the browser's filter, so the reader is never left with no suppression at all.
 */
export function microphoneChoiceOf(settings: VoiceSettings, deviceId: string | null, isNoiseSuppressorUsable: boolean): MicrophoneChoice {
  const isNoiseSuppressorWanted = settings.noiseSuppression === "advanced" && isNoiseSuppressorUsable;
  return {
    deviceId,
    isBrowserNoiseSuppressionOn: settings.noiseSuppression !== "off" && !isNoiseSuppressorWanted,
    isAutomaticGainOn: !settings.isVoiceEnhancementOn,
  };
}

export function microphoneProcessingOf(settings: VoiceSettings, isNoiseSuppressorUsable: boolean): MicrophoneProcessing {
  return {
    isNoiseSuppressorOn: settings.noiseSuppression === "advanced" && isNoiseSuppressorUsable,
    equalizer: settings.isVoiceEnhancementOn ? settings.equalizerDecibels : null,
  };
}

/**
 * What `getUserMedia` is asked for: echo cancellation always, one channel. A chosen device is
 * required (`exact`), so its absence fails and the session falls back to the default one.
 */
export function microphoneConstraintsOf(choice: MicrophoneChoice): MediaTrackConstraints {
  return {
    echoCancellation: true,
    noiseSuppression: choice.isBrowserNoiseSuppressionOn,
    autoGainControl: choice.isAutomaticGainOn,
    channelCount: 1,
    ...(choice.deviceId === null ? {} : { deviceId: { exact: choice.deviceId } }),
  };
}

function equalizerStage(band: EqualizerBand, gainDecibels: number): ProcessingStage {
  switch (band) {
    case "bass":
      return { kind: "filter", band, type: "lowshelf", frequencyHertz: BASS_SHELF_HERTZ, quality: SHELF_QUALITY, gainDecibels };
    case "mids":
      return { kind: "filter", band, type: "peaking", frequencyHertz: MIDS_PEAK_HERTZ, quality: MIDS_PEAK_QUALITY, gainDecibels };
    case "treble":
      return { kind: "filter", band, type: "highshelf", frequencyHertz: TREBLE_SHELF_HERTZ, quality: SHELF_QUALITY, gainDecibels };
  }
}

/**
 * The stages, source to track. Empty when nothing is on: the microphone's own track is sent, with
 * no Web Audio in the way. Noise goes first, so the equaliser and compressor never lift it.
 */
export function processingStagesOf(processing: MicrophoneProcessing): readonly ProcessingStage[] {
  const stages: ProcessingStage[] = processing.isNoiseSuppressorOn ? [{ kind: "noise-suppressor" }] : [];
  const { equalizer } = processing;
  if (equalizer === null) {
    return stages;
  }
  return [
    ...stages,
    { kind: "filter", band: "rumble", type: "highpass", frequencyHertz: RUMBLE_CUTOFF_HERTZ, quality: RUMBLE_FILTER_QUALITY, gainDecibels: 0 },
    equalizerStage("bass", equalizer.bass),
    equalizerStage("mids", equalizer.mids),
    equalizerStage("treble", equalizer.treble),
    { kind: "dynamics", role: "compressor", ...COMPRESSOR },
    { kind: "dynamics", role: "limiter", ...LIMITER },
  ];
}

/** Whether `next` needs another microphone (its browser processing changed), not only new slider values. */
export function isMicrophoneReopenNeeded(previous: VoiceSettings, next: VoiceSettings): boolean {
  return (
    next.inputDeviceId !== previous.inputDeviceId ||
    next.noiseSuppression !== previous.noiseSuppression ||
    next.isVoiceEnhancementOn !== previous.isVoiceEnhancementOn
  );
}
