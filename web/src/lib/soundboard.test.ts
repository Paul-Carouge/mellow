import { describe, expect, it } from "vitest";

import { MAXIMUM_SOUND_SAMPLES, Permission, type SoundboardSound } from "./protocol";
import {
  MAXIMUM_SOUND_SECONDS,
  MINIMUM_SELECTION_SECONDS,
  cooldownEndAfter,
  encodeWav,
  formatSoundSeconds,
  initialSelection,
  joinCueOf,
  selectionSampleCount,
  shouldHearSound,
  soundIndicatorMilliseconds,
  soundNameFromFileName,
  soundNameProblem,
  soundboardBlock,
  soundboardGain,
  waveformPeaks,
  withSelectionCentredAt,
  withSelectionEnd,
  withSelectionStart,
} from "./soundboard";

const text = (bytes: Uint8Array, start: number, length: number) => String.fromCharCode(...bytes.slice(start, start + length));

describe("encodeWav", () => {
  it("writes the canonical 44-byte header the instance checks", () => {
    const bytes = encodeWav(new Float32Array([0, 0.5, -0.5]));
    const view = new DataView(bytes.buffer);
    expect(bytes.length).toBe(44 + 6);
    expect(text(bytes, 0, 4)).toBe("RIFF");
    expect(view.getUint32(4, true)).toBe(bytes.length - 8);
    expect(text(bytes, 8, 4)).toBe("WAVE");
    expect(text(bytes, 12, 4)).toBe("fmt ");
    expect(view.getUint32(16, true)).toBe(16);
    expect(view.getUint16(20, true)).toBe(1);
    expect(view.getUint16(22, true)).toBe(1);
    expect(view.getUint32(24, true)).toBe(48_000);
    expect(view.getUint32(28, true)).toBe(96_000);
    expect(view.getUint16(32, true)).toBe(2);
    expect(view.getUint16(34, true)).toBe(16);
    expect(text(bytes, 36, 4)).toBe("data");
    expect(view.getUint32(40, true)).toBe(6);
  });

  it("stores samples as little-endian 16-bit, clamped, NaN as silence", () => {
    const bytes = encodeWav(new Float32Array([1, -1, 2, -3, Number.NaN, 0.5]));
    const view = new DataView(bytes.buffer);
    const sample = (index: number) => view.getInt16(44 + index * 2, true);
    expect(sample(0)).toBe(32_767);
    expect(sample(1)).toBe(-32_768);
    expect(sample(2)).toBe(32_767);
    expect(sample(3)).toBe(-32_768);
    expect(sample(4)).toBe(0);
    expect(sample(5)).toBe(16_384);
  });

  it("accepts 1 to 249 600 samples and refuses anything else", () => {
    expect(encodeWav(new Float32Array(1)).length).toBe(46);
    expect(encodeWav(new Float32Array(MAXIMUM_SOUND_SAMPLES)).length).toBe(44 + 499_200);
    expect(() => encodeWav(new Float32Array(0))).toThrow(RangeError);
    expect(() => encodeWav(new Float32Array(MAXIMUM_SOUND_SAMPLES + 1))).toThrow(RangeError);
  });
});

describe("trimming", () => {
  it("starts with the beginning of the file, at most 5.2 s", () => {
    expect(MAXIMUM_SOUND_SECONDS).toBeCloseTo(5.2);
    expect(initialSelection(3)).toEqual({ startSeconds: 0, endSeconds: 3 });
    expect(initialSelection(60).endSeconds).toBeCloseTo(5.2);
  });

  it("moves the start without passing the end or stretching past 5.2 s", () => {
    const selection = { startSeconds: 2, endSeconds: 6 };
    expect(withSelectionStart(selection, 1, 60)).toEqual({ startSeconds: 1, endSeconds: 6 });
    // Earlier than 5.2 s before the end: the end follows.
    expect(withSelectionStart(selection, 0, 60).endSeconds).toBeCloseTo(5.2);
    // Past the end: the end is pushed, keeping the shortest length.
    expect(withSelectionStart(selection, 7, 60)).toEqual({ startSeconds: 7, endSeconds: 7 + MINIMUM_SELECTION_SECONDS });
    expect(withSelectionStart(selection, 100, 10).startSeconds).toBeCloseTo(10 - MINIMUM_SELECTION_SECONDS);
    expect(withSelectionStart(selection, -3, 60).startSeconds).toBe(0);
  });

  it("moves the end without passing the start or stretching past 5.2 s", () => {
    const selection = { startSeconds: 2, endSeconds: 4 };
    expect(withSelectionEnd(selection, 5, 60)).toEqual({ startSeconds: 2, endSeconds: 5 });
    expect(withSelectionEnd(selection, 10, 60).startSeconds).toBeCloseTo(4.8);
    expect(withSelectionEnd(selection, 1, 60)).toEqual({ startSeconds: 1 - MINIMUM_SELECTION_SECONDS, endSeconds: 1 });
    expect(withSelectionEnd(selection, 99, 8)).toEqual({ startSeconds: 2.8, endSeconds: 8 });
  });

  it("keeps a file shorter than the shortest selection whole", () => {
    expect(withSelectionStart({ startSeconds: 0, endSeconds: 0.05 }, 0.04, 0.05)).toEqual({ startSeconds: 0, endSeconds: 0.05 });
    expect(withSelectionEnd({ startSeconds: 0, endSeconds: 0.05 }, 0, 0.05)).toEqual({ startSeconds: 0, endSeconds: 0.05 });
  });

  it("centres the selection where the waveform is clicked, inside the file", () => {
    const selection = { startSeconds: 0, endSeconds: 2 };
    expect(withSelectionCentredAt(selection, 10, 60)).toEqual({ startSeconds: 9, endSeconds: 11 });
    expect(withSelectionCentredAt(selection, 0.5, 60)).toEqual({ startSeconds: 0, endSeconds: 2 });
    expect(withSelectionCentredAt(selection, 59.5, 60)).toEqual({ startSeconds: 58, endSeconds: 60 });
  });

  it("counts samples at 48 kHz, never zero, never past the limit", () => {
    expect(selectionSampleCount({ startSeconds: 1, endSeconds: 2 })).toBe(48_000);
    expect(selectionSampleCount({ startSeconds: 1, endSeconds: 1 })).toBe(1);
    expect(selectionSampleCount({ startSeconds: 0, endSeconds: 5.2000001 })).toBe(MAXIMUM_SOUND_SAMPLES);
  });

  it("reduces the file to the loudest sample per slice, across channels", () => {
    const left = new Float32Array([0.1, -0.8, 0, 0.2]);
    const right = new Float32Array([0.3, 0, -0.4, 0]);
    expect(waveformPeaks([left, right], 2)).toEqual([expect.closeTo(0.8), expect.closeTo(0.4)]);
    expect(waveformPeaks([], 3)).toEqual([0, 0, 0]);
    expect(waveformPeaks([new Float32Array([2])], 1)).toEqual([1]);
  });

  it("prints times to the tenth", () => {
    expect(formatSoundSeconds(3.24, ",")).toBe("0:03,2");
    expect(formatSoundSeconds(65.06, ".")).toBe("1:05.1");
    expect(formatSoundSeconds(-1, ".")).toBe("0:00.0");
  });
});

describe("names", () => {
  it("follows the instance's rule: 2 to 32 characters once trimmed, no control characters", () => {
    expect(soundNameProblem("  ok  ")).toBeNull();
    expect(soundNameProblem("a")).toBe("length");
    expect(soundNameProblem(" a ")).toBe("length");
    expect(soundNameProblem("x".repeat(32))).toBeNull();
    expect(soundNameProblem("x".repeat(33))).toBe("length");
    // Code points, as the server counts: 32 emoji fit.
    expect(soundNameProblem("🎺".repeat(32))).toBeNull();
    expect(soundNameProblem("air\u0007horn")).toBe("control-character");
  });

  it("suggests a name from the file's", () => {
    expect(soundNameFromFileName("airhorn_long-v2.mp3")).toBe("airhorn long v2");
    expect(soundNameFromFileName("Sad Trombone.wav")).toBe("Sad Trombone");
    expect(soundNameFromFileName(`${"a".repeat(40)}.ogg`)).toBe("a".repeat(32));
    expect(soundNameFromFileName(".m4a")).toBe("");
  });
});

describe("playing", () => {
  const allowed = String(Permission.USE_SOUNDBOARD | Permission.SPEAK | Permission.CONNECT);
  const facts = { isConnected: true, channelPermissions: allowed, isDeafened: false, now: 10_000, cooldownEndsAt: 0 };

  it("plays only connected, with both permissions, not deafened, out of the cooldown", () => {
    expect(soundboardBlock(facts)).toBeNull();
    expect(soundboardBlock({ ...facts, isConnected: false })).toBe("not-connected");
    expect(soundboardBlock({ ...facts, channelPermissions: String(Permission.SPEAK) })).toBe("no-permission");
    expect(soundboardBlock({ ...facts, channelPermissions: String(Permission.USE_SOUNDBOARD) })).toBe("cannot-speak");
    expect(soundboardBlock({ ...facts, isDeafened: true })).toBe("deafened");
    expect(soundboardBlock({ ...facts, cooldownEndsAt: 10_001 })).toBe("cooling-down");
    expect(soundboardBlock({ ...facts, cooldownEndsAt: 10_000 })).toBeNull();
    expect(soundboardBlock({ ...facts, channelPermissions: String(Permission.ADMINISTRATOR) })).toBeNull();
    // Unknown permissions: the instance decides.
    expect(soundboardBlock({ ...facts, channelPermissions: null })).toBeNull();
  });

  it("waits 2 s from the click, then from the instance's echo if later", () => {
    const afterClick = cooldownEndAfter(0, 1_000);
    expect(afterClick).toBe(3_000);
    expect(cooldownEndAfter(afterClick, 1_150)).toBe(3_150);
    expect(cooldownEndAfter(afterClick, 900)).toBe(3_000);
  });

  it("hears sounds of your channel only, unless deafened or the player is muted for you", () => {
    const hearing = { soundChannelId: "7", myChannelId: "7", isConnected: true, isDeafened: false, isPlayerMutedForMe: false };
    expect(shouldHearSound(hearing)).toBe(true);
    expect(shouldHearSound({ ...hearing, myChannelId: "8" })).toBe(false);
    expect(shouldHearSound({ ...hearing, myChannelId: null })).toBe(false);
    expect(shouldHearSound({ ...hearing, isConnected: false })).toBe(false);
    expect(shouldHearSound({ ...hearing, isDeafened: true })).toBe(false);
    expect(shouldHearSound({ ...hearing, isPlayerMutedForMe: true })).toBe(false);
  });

  it("plays a join sound the server still has, else the chime", () => {
    const horn: SoundboardSound = { id: "9", name: "horn", duration_milliseconds: 900 };
    const hearing = { isDeafened: false, isMutedForMe: false };
    expect(joinCueOf("9", [horn], hearing)).toEqual({ kind: "sound", sound: horn });
    expect(joinCueOf("10", [horn], hearing)).toEqual({ kind: "chime" });
    expect(joinCueOf(undefined, [horn], hearing)).toEqual({ kind: "chime" });
    expect(joinCueOf(null, [horn], hearing)).toEqual({ kind: "chime" });
    expect(joinCueOf("9", [horn], { ...hearing, isDeafened: true })).toEqual({ kind: "chime" });
    expect(joinCueOf("9", [horn], { ...hearing, isMutedForMe: true })).toEqual({ kind: "chime" });
  });

  it("scales sounds by both volumes and shows the emoji at least 1.5 s", () => {
    expect(soundboardGain(100, 100)).toBe(1);
    expect(soundboardGain(150, 50)).toBe(0.75);
    expect(soundboardGain(0, 100)).toBe(0);
    expect(soundIndicatorMilliseconds({ duration_milliseconds: 400 })).toBe(1_500);
    expect(soundIndicatorMilliseconds({ duration_milliseconds: 5_200 })).toBe(5_200);
    expect(soundIndicatorMilliseconds(undefined)).toBe(1_500);
  });
});
