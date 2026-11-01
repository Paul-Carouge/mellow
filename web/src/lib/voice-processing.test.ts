import { describe, expect, it } from "vitest";

import {
  isMicrophoneReopenNeeded,
  microphoneChoiceOf,
  microphoneConstraintsOf,
  microphoneProcessingOf,
  NOISE_SUPPRESSOR_SILENCE_LIMIT_MILLISECONDS,
  noiseSuppressorProblemOf,
  processingStagesOf,
  silentNoiseSuppressorMilliseconds,
} from "./voice-processing";
import { DEFAULT_VOICE_SETTINGS, EQUALIZER_OF_PRESET, type VoiceSettings, withEqualizerBand } from "./voice-settings";

const settingsWith = (change: Partial<VoiceSettings>): VoiceSettings => ({ ...DEFAULT_VOICE_SETTINGS, ...change });

describe("microphone choice", () => {
  it("turns the browser's suppression off only while RNNoise runs", () => {
    const advanced = settingsWith({ noiseSuppression: "advanced" });
    expect(microphoneChoiceOf(advanced, null, true).isBrowserNoiseSuppressionOn).toBe(false);
    expect(microphoneChoiceOf(advanced, null, false).isBrowserNoiseSuppressionOn).toBe(true);
    expect(microphoneChoiceOf(settingsWith({ noiseSuppression: "browser" }), null, true).isBrowserNoiseSuppressionOn).toBe(true);
    expect(microphoneChoiceOf(settingsWith({ noiseSuppression: "off" }), null, true).isBrowserNoiseSuppressionOn).toBe(false);
  });

  it("hands loudness to the compressor while the voice enhancement is on", () => {
    expect(microphoneChoiceOf(settingsWith({ isVoiceEnhancementOn: false }), null, true).isAutomaticGainOn).toBe(true);
    expect(microphoneChoiceOf(settingsWith({ isVoiceEnhancementOn: true }), null, true).isAutomaticGainOn).toBe(false);
  });

  it("asks getUserMedia for echo cancellation always, one channel, and the exact device", () => {
    expect(microphoneConstraintsOf({ deviceId: null, isBrowserNoiseSuppressionOn: false, isAutomaticGainOn: false })).toEqual({
      echoCancellation: true,
      noiseSuppression: false,
      autoGainControl: false,
      channelCount: 1,
    });
    expect(microphoneConstraintsOf({ deviceId: "yeti", isBrowserNoiseSuppressionOn: true, isAutomaticGainOn: true })).toEqual({
      echoCancellation: true,
      noiseSuppression: true,
      autoGainControl: true,
      channelCount: 1,
      deviceId: { exact: "yeti" },
    });
  });
});

describe("processing chain", () => {
  const kinds = (settings: VoiceSettings, isNoiseSuppressorUsable = true) =>
    processingStagesOf(microphoneProcessingOf(settings, isNoiseSuppressorUsable)).map((stage) =>
      stage.kind === "noise-suppressor" ? stage.kind : stage.kind === "filter" ? `${stage.type}:${stage.band}` : stage.role,
    );

  it("is empty when nothing is on, so the device's track is sent as it is", () => {
    expect(kinds(settingsWith({ noiseSuppression: "off", isVoiceEnhancementOn: false }))).toEqual([]);
    expect(kinds(settingsWith({ noiseSuppression: "browser", isVoiceEnhancementOn: false }))).toEqual([]);
    expect(kinds(settingsWith({ noiseSuppression: "advanced" }), false)).toEqual([]);
  });

  it("holds RNNoise alone when only it is on", () => {
    expect(kinds(settingsWith({ noiseSuppression: "advanced" }))).toEqual(["noise-suppressor"]);
  });

  it("runs noise first, then rumble, the three bands, the compressor and the limiter", () => {
    expect(kinds(settingsWith({ noiseSuppression: "advanced", isVoiceEnhancementOn: true }))).toEqual([
      "noise-suppressor",
      "highpass:rumble",
      "lowshelf:bass",
      "peaking:mids",
      "highshelf:treble",
      "compressor",
      "limiter",
    ]);
    expect(kinds(settingsWith({ noiseSuppression: "off", isVoiceEnhancementOn: true }))[0]).toBe("highpass:rumble");
  });

  it("sets the bands from the sliders, and keeps the limiter's ceiling under full scale", () => {
    const settings = withEqualizerBand(settingsWith({ isVoiceEnhancementOn: true }), "treble", 7);
    const stages = processingStagesOf(microphoneProcessingOf(settings, false));
    const gains = Object.fromEntries(stages.flatMap((stage) => (stage.kind === "filter" ? [[stage.band, stage.gainDecibels]] : [])));
    expect(gains).toEqual({ rumble: 0, bass: EQUALIZER_OF_PRESET.streamer.bass, mids: EQUALIZER_OF_PRESET.streamer.mids, treble: 7 });
    const limiter = stages.find((stage) => stage.kind === "dynamics" && stage.role === "limiter");
    expect(limiter?.kind === "dynamics" && limiter.thresholdDecibels).toBeLessThan(-1);
  });

  it("opens the microphone again only when the browser's processing changes", () => {
    const base = DEFAULT_VOICE_SETTINGS;
    expect(isMicrophoneReopenNeeded(base, { ...base, noiseSuppression: "browser" })).toBe(true);
    expect(isMicrophoneReopenNeeded(base, { ...base, isVoiceEnhancementOn: true })).toBe(true);
    expect(isMicrophoneReopenNeeded(base, { ...base, inputDeviceId: "yeti" })).toBe(true);
    expect(isMicrophoneReopenNeeded(base, withEqualizerBand(base, "bass", 4))).toBe(false);
    expect(isMicrophoneReopenNeeded(base, { ...base, inputVolumePercent: 150 })).toBe(false);
  });
});

describe("noise suppressor problems", () => {
  it("counts a refused sample rate as unsupported, anything else as a failure worth retrying", () => {
    expect(noiseSuppressorProblemOf(new DOMException("rates differ", "NotSupportedError"))).toBe("unsupported");
    expect(noiseSuppressorProblemOf(new DOMException("aborted", "AbortError"))).toBe("failed");
    expect(noiseSuppressorProblemOf(new TypeError("network"))).toBe("failed");
    expect(noiseSuppressorProblemOf("weird")).toBe("failed");
  });
});

describe("noise suppressor watchdog", () => {
  const heard = new Float32Array([0.1, -0.2, 0.05]);
  const silence = new Float32Array(3);
  const quiet = new Float32Array([0.0003, -0.0001, 0]);

  it("counts exact silence out while the microphone is heard, up to the limit", () => {
    let silent = 0;
    for (let look = 0; look < 4; look++) {
      silent = silentNoiseSuppressorMilliseconds(silent, heard, silence, 250);
    }
    expect(silent).toBe(NOISE_SUPPRESSOR_SILENCE_LIMIT_MILLISECONDS);
  });

  it("starts again from zero when RNNoise gives anything, or the microphone is muted", () => {
    expect(silentNoiseSuppressorMilliseconds(750, heard, quiet, 250)).toBe(0);
    expect(silentNoiseSuppressorMilliseconds(750, silence, silence, 250)).toBe(0);
  });
});
