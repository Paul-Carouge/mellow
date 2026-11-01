import { describe, expect, it } from "vitest";

import {
  DEFAULT_VOICE_SETTINGS,
  PERSON_VOICE_SETTINGS_STORAGE_KEY,
  VOICE_SETTINGS_STORAGE_KEY,
  decibelsOfLevel,
  deviceChoices,
  isCharacterKey,
  isRememberedDevicePresent,
  meterFractionOfDecibels,
  playbackGain,
  pushToTalkKeyOf,
  readPersonVoiceSettings,
  readVoiceSettings,
  writePersonVoiceSettings,
  writeVoiceSettings,
} from "./voice-settings";

const storageWith = (key: string, value: string) => ({ getItem: (asked: string) => (asked === key ? value : null) });

describe("voice settings", () => {
  it("reads defaults from nothing, garbage, or another shape", () => {
    expect(readVoiceSettings(null)).toEqual(DEFAULT_VOICE_SETTINGS);
    expect(readVoiceSettings(storageWith(VOICE_SETTINGS_STORAGE_KEY, "{not json"))).toEqual(DEFAULT_VOICE_SETTINGS);
    expect(readVoiceSettings(storageWith(VOICE_SETTINGS_STORAGE_KEY, "[1,2]"))).toEqual(DEFAULT_VOICE_SETTINGS);
  });

  it("validates each field: clamps numbers, drops unknown modes and unusable keys", () => {
    const stored = JSON.stringify({
      inputDeviceId: 42,
      outputDeviceId: "headset",
      inputVolumePercent: 999,
      outputVolumePercent: -5,
      inputMode: "always",
      pushToTalkKey: { code: "Escape", label: "Échap" },
      pushToTalkReleaseDelayMilliseconds: 2500.4,
      sensitivityThresholdDecibels: -42.6,
      isNoiseSuppressionOn: "no",
    });
    expect(readVoiceSettings(storageWith(VOICE_SETTINGS_STORAGE_KEY, stored))).toEqual({
      ...DEFAULT_VOICE_SETTINGS,
      outputDeviceId: "headset",
      inputVolumePercent: 200,
      outputVolumePercent: 0,
      pushToTalkReleaseDelayMilliseconds: 2000,
      sensitivityThresholdDecibels: -43,
    });
  });

  it("reads back what it wrote", () => {
    const values = new Map<string, string>();
    const settings = {
      ...DEFAULT_VOICE_SETTINGS,
      inputMode: "push-to-talk" as const,
      pushToTalkKey: { code: "KeyV", label: "V" },
      isSensitivityAutomatic: false,
    };
    writeVoiceSettings({ setItem: (key, value) => void values.set(key, value) }, settings);
    expect(readVoiceSettings({ getItem: (key) => values.get(key) ?? null })).toEqual(settings);
  });

  it("keeps a blocked storage from throwing", () => {
    expect(() =>
      writeVoiceSettings(
        {
          setItem: () => {
            throw new Error("quota");
          },
        },
        DEFAULT_VOICE_SETTINGS,
      ),
    ).not.toThrow();
  });
});

describe("levels", () => {
  it("converts an RMS level to dBFS on the meter's −100 to 0 scale", () => {
    expect(decibelsOfLevel(1)).toBe(0);
    expect(decibelsOfLevel(0.1)).toBeCloseTo(-20);
    expect(decibelsOfLevel(0)).toBe(-100);
    expect(decibelsOfLevel(1e-9)).toBe(-100);
    expect(decibelsOfLevel(4)).toBe(0);
    expect(meterFractionOfDecibels(-50)).toBe(0.5);
    expect(meterFractionOfDecibels(-150)).toBe(0);
  });
});

describe("push-to-talk keys", () => {
  const named = { ControlRight: "Ctrl droit", Space: "Espace" };

  it("records the printed letter, or the key's name", () => {
    expect(pushToTalkKeyOf({ code: "KeyQ", key: "a" }, named)).toEqual({ code: "KeyQ", label: "A" });
    expect(pushToTalkKeyOf({ code: "ControlRight", key: "Control" }, named)).toEqual({ code: "ControlRight", label: "Ctrl droit" });
    expect(pushToTalkKeyOf({ code: "Space", key: " " }, named)).toEqual({ code: "Space", label: "Espace" });
    expect(pushToTalkKeyOf({ code: "F13", key: "F13" }, named)).toEqual({ code: "F13", label: "F13" });
  });

  it("refuses Esc, which cancels, and Tab, which moves focus", () => {
    expect(pushToTalkKeyOf({ code: "Escape", key: "Escape" }, named)).toBeNull();
    expect(pushToTalkKeyOf({ code: "Tab", key: "Tab" }, named)).toBeNull();
  });

  it("tells a key that types a character", () => {
    const plain = { ctrlKey: false, metaKey: false, altKey: false };
    expect(isCharacterKey({ ...plain, key: "v" })).toBe(true);
    expect(isCharacterKey({ ...plain, key: " " })).toBe(true);
    expect(isCharacterKey({ ...plain, key: "Control" })).toBe(false);
    expect(isCharacterKey({ ...plain, key: "v", ctrlKey: true })).toBe(false);
  });
});

describe("devices", () => {
  const fallbackName = (position: number) => `Micro ${position}`;

  it("lists named devices without the system's duplicates, and names the default one", () => {
    const choices = deviceChoices(
      [
        { kind: "audioinput", deviceId: "default", label: "Par défaut - MacBook Pro Microphone (Built-in)" },
        { kind: "audioinput", deviceId: "communications", label: "Communications - Casque" },
        { kind: "audioinput", deviceId: "a1", label: "MacBook Pro Microphone (Built-in)" },
        { kind: "audioinput", deviceId: "a2", label: "" },
        { kind: "audiooutput", deviceId: "o1", label: "Haut-parleurs" },
      ],
      "audioinput",
      fallbackName,
    );
    expect(choices).toEqual({
      defaultDeviceName: "MacBook Pro Microphone (Built-in)",
      devices: [
        { deviceId: "a1", label: "MacBook Pro Microphone (Built-in)" },
        { deviceId: "a2", label: "Micro 2" },
      ],
      areLabelsHidden: false,
    });
  });

  it("hides the list until the microphone is allowed", () => {
    expect(deviceChoices([{ kind: "audioinput", deviceId: "", label: "" }], "audioinput", fallbackName)).toEqual({
      defaultDeviceName: null,
      devices: [],
      areLabelsHidden: true,
    });
  });

  it("knows whether a remembered device is plugged in, or cannot tell", () => {
    const devices = [{ kind: "audioinput" as const, deviceId: "a1", label: "Micro" }];
    expect(isRememberedDevicePresent(devices, "audioinput", "a1")).toBe(true);
    expect(isRememberedDevicePresent(devices, "audioinput", "gone")).toBe(false);
    expect(isRememberedDevicePresent(devices, "audioinput", null)).toBe(true);
    expect(isRememberedDevicePresent([{ kind: "audioinput", deviceId: "", label: "" }], "audioinput", "gone")).toBe(true);
  });
});

describe("per-person volume", () => {
  it("reads only valid people and values", () => {
    const stored = JSON.stringify({
      "7": { volumePercent: 250, isLocallyMuted: true },
      "not-an-id": { volumePercent: 50 },
      "8": "loud",
    });
    const settings = readPersonVoiceSettings(storageWith(PERSON_VOICE_SETTINGS_STORAGE_KEY, stored));
    expect([...settings]).toEqual([["7", { volumePercent: 200, isLocallyMuted: true }]]);
  });

  it("stores only the people changed from the default", () => {
    const values = new Map<string, string>();
    writePersonVoiceSettings(
      { setItem: (key, value) => void values.set(key, value) },
      new Map([
        ["7", { volumePercent: 100, isLocallyMuted: false }],
        ["8", { volumePercent: 60, isLocallyMuted: false }],
      ]),
    );
    expect(JSON.parse(values.get(PERSON_VOICE_SETTINGS_STORAGE_KEY) ?? "{}")).toEqual({
      "8": { volumePercent: 60, isLocallyMuted: false },
    });
  });

  it("multiplies the output and the person's volume; deafen and local mute silence", () => {
    expect(playbackGain(100, null, false)).toBe(1);
    expect(playbackGain(200, { volumePercent: 200, isLocallyMuted: false }, false)).toBe(4);
    expect(playbackGain(50, { volumePercent: 150, isLocallyMuted: false }, false)).toBe(0.75);
    expect(playbackGain(100, { volumePercent: 100, isLocallyMuted: true }, false)).toBe(0);
    expect(playbackGain(100, null, true)).toBe(0);
  });
});
