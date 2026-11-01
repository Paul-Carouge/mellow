// Voice settings (docs/design/web-voice-comfort.md, ROADMAP stage 16.3): Discord's Voice settings
// in Mellow's words, kept per browser like the toggles. Every field is validated on read: storage
// is shared with other tabs and older versions, so anything unreadable falls back to its default.

import type { Snowflake } from "./protocol";

export type VoiceInputMode = "voice-activity" | "push-to-talk";

/** A key recorded for push-to-talk: `code` is what is matched, `label` what was printed on it then. */
export interface PushToTalkKey {
  readonly code: string;
  readonly label: string;
}

export interface VoiceSettings {
  /** `null`: the system's default device. */
  readonly inputDeviceId: string | null;
  readonly outputDeviceId: string | null;
  readonly inputVolumePercent: number;
  readonly outputVolumePercent: number;
  readonly inputMode: VoiceInputMode;
  /** No default: a letter would stop typing, so the reader records their own. */
  readonly pushToTalkKey: PushToTalkKey | null;
  readonly pushToTalkReleaseDelayMilliseconds: number;
  readonly arePushToTalkSoundsOn: boolean;
  /** Automatic: no gate in the client, the browser's processing and Opus DTX handle silence. */
  readonly isSensitivityAutomatic: boolean;
  readonly sensitivityThresholdDecibels: number;
  readonly isNoiseSuppressionOn: boolean;
}

export const MINIMUM_VOLUME_PERCENT = 0;
export const MAXIMUM_VOLUME_PERCENT = 200;
export const DEFAULT_VOLUME_PERCENT = 100;
export const MAXIMUM_PUSH_TO_TALK_RELEASE_DELAY_MILLISECONDS = 2_000;
/** Discord's scale: the meter and the threshold run from silence (−100 dB) to full scale (0 dB). */
export const MINIMUM_DECIBELS = -100;
export const MAXIMUM_DECIBELS = 0;

export const DEFAULT_VOICE_SETTINGS: VoiceSettings = {
  inputDeviceId: null,
  outputDeviceId: null,
  inputVolumePercent: DEFAULT_VOLUME_PERCENT,
  outputVolumePercent: DEFAULT_VOLUME_PERCENT,
  inputMode: "voice-activity",
  pushToTalkKey: null,
  // Long enough not to clip the last syllable when the key comes up with it.
  pushToTalkReleaseDelayMilliseconds: 200,
  arePushToTalkSoundsOn: false,
  isSensitivityAutomatic: true,
  // Above a quiet room's hum, below normal speech.
  sensitivityThresholdDecibels: -50,
  isNoiseSuppressionOn: true,
};

export const VOICE_SETTINGS_STORAGE_KEY = "mellow.voice-settings";

const MAXIMUM_STORED_TEXT_LENGTH = 256;

function clampedInteger(value: unknown, minimum: number, maximum: number, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? Math.min(maximum, Math.max(minimum, Math.round(value))) : fallback;
}

function storedText(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 && value.length <= MAXIMUM_STORED_TEXT_LENGTH ? value : null;
}

function storedPushToTalkKey(value: unknown): PushToTalkKey | null {
  if (typeof value !== "object" || value === null) {
    return null;
  }
  const fields = value as Record<string, unknown>;
  const code = storedText(fields["code"]);
  const label = storedText(fields["label"]);
  return code !== null && label !== null && isRecordablePushToTalkCode(code) ? { code, label } : null;
}

function storedObject(storage: Pick<Storage, "getItem"> | null, key: string): Record<string, unknown> | null {
  try {
    const stored: unknown = JSON.parse(storage?.getItem(key) ?? "null");
    return typeof stored === "object" && stored !== null && !Array.isArray(stored) ? (stored as Record<string, unknown>) : null;
  } catch (error) {
    console.warn("unreadable voice settings", error);
    return null;
  }
}

export function readVoiceSettings(storage: Pick<Storage, "getItem"> | null): VoiceSettings {
  const fields = storedObject(storage, VOICE_SETTINGS_STORAGE_KEY);
  if (fields === null) {
    return DEFAULT_VOICE_SETTINGS;
  }
  const defaults = DEFAULT_VOICE_SETTINGS;
  const storedBoolean = (name: keyof VoiceSettings, fallback: boolean) =>
    typeof fields[name] === "boolean" ? (fields[name] as boolean) : fallback;
  return {
    inputDeviceId: storedText(fields["inputDeviceId"]),
    outputDeviceId: storedText(fields["outputDeviceId"]),
    inputVolumePercent: clampedInteger(fields["inputVolumePercent"], MINIMUM_VOLUME_PERCENT, MAXIMUM_VOLUME_PERCENT, defaults.inputVolumePercent),
    outputVolumePercent: clampedInteger(fields["outputVolumePercent"], MINIMUM_VOLUME_PERCENT, MAXIMUM_VOLUME_PERCENT, defaults.outputVolumePercent),
    inputMode: fields["inputMode"] === "push-to-talk" ? "push-to-talk" : "voice-activity",
    pushToTalkKey: storedPushToTalkKey(fields["pushToTalkKey"]),
    pushToTalkReleaseDelayMilliseconds: clampedInteger(
      fields["pushToTalkReleaseDelayMilliseconds"],
      0,
      MAXIMUM_PUSH_TO_TALK_RELEASE_DELAY_MILLISECONDS,
      defaults.pushToTalkReleaseDelayMilliseconds,
    ),
    arePushToTalkSoundsOn: storedBoolean("arePushToTalkSoundsOn", defaults.arePushToTalkSoundsOn),
    isSensitivityAutomatic: storedBoolean("isSensitivityAutomatic", defaults.isSensitivityAutomatic),
    sensitivityThresholdDecibels: clampedInteger(
      fields["sensitivityThresholdDecibels"],
      MINIMUM_DECIBELS,
      MAXIMUM_DECIBELS,
      defaults.sensitivityThresholdDecibels,
    ),
    isNoiseSuppressionOn: storedBoolean("isNoiseSuppressionOn", defaults.isNoiseSuppressionOn),
  };
}

/** A full or blocked storage keeps them for this tab only. */
export function writeVoiceSettings(storage: Pick<Storage, "setItem"> | null, settings: VoiceSettings): void {
  try {
    storage?.setItem(VOICE_SETTINGS_STORAGE_KEY, JSON.stringify(settings));
  } catch (error) {
    console.warn("couldn't store the voice settings", error);
  }
}

// --- Levels --------------------------------------------------------------------------------------

/** An RMS level (0 to 1) in dBFS, floored at the meter's −100 dB. */
export function decibelsOfLevel(level: number): number {
  return level > 0 ? Math.max(MINIMUM_DECIBELS, Math.min(MAXIMUM_DECIBELS, 20 * Math.log10(level))) : MINIMUM_DECIBELS;
}

/** Where a level sits on the meter, 0 to 1. */
export function meterFractionOfDecibels(decibels: number): number {
  return (Math.min(MAXIMUM_DECIBELS, Math.max(MINIMUM_DECIBELS, decibels)) - MINIMUM_DECIBELS) / (MAXIMUM_DECIBELS - MINIMUM_DECIBELS);
}

// --- Push-to-talk keys ----------------------------------------------------------------------------

/** Esc cancels recording, and Tab must keep moving focus: neither can be the key. */
export function isRecordablePushToTalkCode(code: string): boolean {
  return code !== "Escape" && code !== "Tab" && code !== "" && code !== "Unidentified";
}

/** The key a keydown records, or `null` when it cannot be one. */
export function pushToTalkKeyOf(event: { readonly code: string; readonly key: string }, namedKeys: Readonly<Record<string, string>>): PushToTalkKey | null {
  if (!isRecordablePushToTalkCode(event.code)) {
    return null;
  }
  const printed = [...event.key].length === 1 && event.key.trim() !== "" ? event.key.toLocaleUpperCase() : null;
  return { code: event.code, label: namedKeys[event.code] ?? printed ?? event.code };
}

/** A key that types a character: held for push-to-talk, it must not also type into the composer. */
export function isCharacterKey(event: { readonly key: string; readonly ctrlKey: boolean; readonly metaKey: boolean; readonly altKey: boolean }): boolean {
  return [...event.key].length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey;
}

// --- Devices --------------------------------------------------------------------------------------

/** What `enumerateDevices` gives, reduced to what the settings use. */
export interface AudioDevice {
  readonly kind: "audioinput" | "audiooutput";
  readonly deviceId: string;
  readonly label: string;
}

/** Chrome and Edge list the system's choices as extra entries; Mellow offers its own "Default". */
const SYSTEM_CHOICE_DEVICE_IDS: ReadonlySet<string> = new Set(["default", "communications"]);

export interface DeviceChoices {
  /** The default device's name when the browser tells it (Chrome's "default" entry), else `null`. */
  readonly defaultDeviceName: string | null;
  readonly devices: readonly { readonly deviceId: string; readonly label: string }[];
  /** Labels are empty until the microphone is allowed: the list cannot be shown yet. */
  readonly areLabelsHidden: boolean;
}

/** The choices of one kind, without the system's duplicates; unnamed devices get `fallbackName(n)`. */
export function deviceChoices(
  devices: readonly AudioDevice[],
  kind: AudioDevice["kind"],
  fallbackName: (position: number) => string,
): DeviceChoices {
  const ofKind = devices.filter((device) => device.kind === kind);
  const areLabelsHidden = ofKind.length === 0 || ofKind.every((device) => device.label === "");
  const systemDefault = ofKind.find((device) => device.deviceId === "default");
  return {
    // "Default - MacBook Pro Microphone": the part after the browser's own prefix.
    defaultDeviceName: systemDefault?.label ? systemDefault.label.replace(/^[^-–]+[-–]\s+/u, "") : null,
    devices: areLabelsHidden
      ? []
      : ofKind
          .filter((device) => device.deviceId !== "" && !SYSTEM_CHOICE_DEVICE_IDS.has(device.deviceId))
          .map((device, index) => ({ deviceId: device.deviceId, label: device.label || fallbackName(index + 1) })),
    areLabelsHidden,
  };
}

/**
 * Whether a remembered device is plugged in. Unknown (`true`) while the browser hides the list:
 * without the microphone permission it gives no device ids to compare.
 */
export function isRememberedDevicePresent(devices: readonly AudioDevice[], kind: AudioDevice["kind"], deviceId: string | null): boolean {
  if (deviceId === null) {
    return true;
  }
  const ofKind = devices.filter((device) => device.kind === kind);
  if (ofKind.every((device) => device.deviceId === "")) {
    return true;
  }
  return ofKind.some((device) => device.deviceId === deviceId);
}

// --- Per-person volume ----------------------------------------------------------------------------

/** One person's volume and local mute, as you set them for you only. */
export interface PersonVoiceSetting {
  readonly volumePercent: number;
  readonly isLocallyMuted: boolean;
}

export const DEFAULT_PERSON_VOICE_SETTING: PersonVoiceSetting = { volumePercent: DEFAULT_VOLUME_PERCENT, isLocallyMuted: false };

export const PERSON_VOICE_SETTINGS_STORAGE_KEY = "mellow.voice-person-settings";

const SNOWFLAKE_PATTERN = /^\d{1,20}$/u;

/** Only the people changed from the default are stored. */
export function readPersonVoiceSettings(storage: Pick<Storage, "getItem"> | null): ReadonlyMap<Snowflake, PersonVoiceSetting> {
  const stored = storedObject(storage, PERSON_VOICE_SETTINGS_STORAGE_KEY);
  const settings = new Map<Snowflake, PersonVoiceSetting>();
  for (const [userId, value] of Object.entries(stored ?? {})) {
    if (!SNOWFLAKE_PATTERN.test(userId) || typeof value !== "object" || value === null) {
      continue;
    }
    const fields = value as Record<string, unknown>;
    settings.set(userId, {
      volumePercent: clampedInteger(fields["volumePercent"], MINIMUM_VOLUME_PERCENT, MAXIMUM_VOLUME_PERCENT, DEFAULT_VOLUME_PERCENT),
      isLocallyMuted: fields["isLocallyMuted"] === true,
    });
  }
  return settings;
}

export function writePersonVoiceSettings(
  storage: Pick<Storage, "setItem"> | null,
  settings: ReadonlyMap<Snowflake, PersonVoiceSetting>,
): void {
  const changed = [...settings].filter(
    ([, setting]) => setting.volumePercent !== DEFAULT_VOLUME_PERCENT || setting.isLocallyMuted,
  );
  try {
    storage?.setItem(PERSON_VOICE_SETTINGS_STORAGE_KEY, JSON.stringify(Object.fromEntries(changed)));
  } catch (error) {
    console.warn("couldn't store the per-person voice settings", error);
  }
}

/**
 * The gain one incoming voice plays at: your output volume times that person's volume; nothing
 * while deafened or muted for you. `null` person: the slot carries nobody known yet.
 */
export function playbackGain(outputVolumePercent: number, person: PersonVoiceSetting | null, isDeafened: boolean): number {
  if (isDeafened || person?.isLocallyMuted === true) {
    return 0;
  }
  return (outputVolumePercent / 100) * ((person?.volumePercent ?? DEFAULT_VOLUME_PERCENT) / 100);
}
