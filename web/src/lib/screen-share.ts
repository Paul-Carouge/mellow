// What a shared screen is sent as (ADR-0037): three simulcast layers named by the RIDs the
// instance knows, sized to fit its per-layer budgets and picture limits (ADR-0035).

import type { SimulcastLayer, VoiceVideoRefusal } from "./protocol";
import type { AudioDevice } from "./voice-settings";
import { withTargetsOf } from "./camera";

/** The qualities a screen can be shared at (owner, 2026-11-02), all at 30 frames per second. */
export type ScreenShareQuality = "720p" | "1080p" | "1440p";
export const SCREEN_SHARE_QUALITIES: readonly ScreenShareQuality[] = ["720p", "1080p", "1440p"];
export const DEFAULT_SCREEN_SHARE_QUALITY: ScreenShareQuality = "1080p";
export const SCREEN_SHARE_QUALITY_STORAGE_KEY = "mellow.screen-share-quality";
export const SCREEN_FRAMES_PER_SECOND = 30;

interface ScreenShareProfile {
  readonly width: number;
  readonly height: number;
  /** The full-size layer's bit rate; the instance allows 10 Mbit/s on it (ADR-0037). */
  readonly topBitrate: number;
}

const SCREEN_SHARE_PROFILES: Readonly<Record<ScreenShareQuality, ScreenShareProfile>> = {
  "720p": { width: 1_280, height: 720, topBitrate: 2_500_000 },
  "1080p": { width: 1_920, height: 1_080, topBitrate: 5_000_000 },
  "1440p": { width: 2_560, height: 1_440, topBitrate: 8_000_000 },
};

export function isScreenShareQuality(value: string | null): value is ScreenShareQuality {
  return SCREEN_SHARE_QUALITIES.some((quality) => quality === value);
}

/** What the browser captures: at most the quality's size, at 30 frames per second. */
export function screenCaptureConstraints(quality: ScreenShareQuality): MediaTrackConstraints {
  const profile = SCREEN_SHARE_PROFILES[quality];
  return {
    width: { max: profile.width },
    height: { max: profile.height },
    frameRate: { ideal: SCREEN_FRAMES_PER_SECOND, max: SCREEN_FRAMES_PER_SECOND },
  };
}

/**
 * Chromium's `getDisplayMedia` options beyond TypeScript's DOM types: `systemAudio` offers the
 * whole system's sound when a screen is shared (Windows, ChromeOS), and excluding the current tab
 * keeps people from sharing the call into itself.
 */
export interface ScreenCaptureOptions extends DisplayMediaStreamOptions {
  readonly audio?: boolean | (MediaTrackConstraints & { readonly restrictOwnAudio?: boolean });
  readonly systemAudio?: "include" | "exclude";
  readonly selfBrowserSurface?: "include" | "exclude";
}

/**
 * The picture at the quality's size, and the sound of the tab, window or screen if the person
 * ticks it in the browser's picker (ADR-0037), untouched: no echo cancellation or noise suppression
 * meant for a microphone. `restrictOwnAudio` leaves the call's own sound out of a whole system's
 * sound where the browser supports it (ignored elsewhere), so watchers neither hear themselves nor
 * someone they muted.
 */
export function screenCaptureOptions(quality: ScreenShareQuality): ScreenCaptureOptions {
  return {
    video: screenCaptureConstraints(quality),
    audio: { echoCancellation: false, noiseSuppression: false, autoGainControl: false, restrictOwnAudio: true },
    systemAudio: "include",
    selfBrowserSurface: "exclude",
  };
}

/**
 * Lowest first, as browsers list simulcast layers. The half and quarter layers keep the bit rates
 * of the instance's `l` and `q` budgets, whatever the quality; the full one follows the quality.
 */
export function screenEncodings(quality: ScreenShareQuality): RTCRtpEncodingParameters[] {
  const frames = SCREEN_FRAMES_PER_SECOND;
  return [
    { rid: "q", scaleResolutionDownBy: 4, maxBitrate: 150_000, maxFramerate: frames },
    { rid: "l", scaleResolutionDownBy: 2, maxBitrate: 500_000, maxFramerate: frames },
    { rid: "h", scaleResolutionDownBy: 1, maxBitrate: SCREEN_SHARE_PROFILES[quality].topBitrate, maxFramerate: frames },
  ];
}

/** The live encodings with the bit rates of `quality`, keeping which layers are paused. */
export function withQuality(
  encodings: readonly RTCRtpEncodingParameters[],
  quality: ScreenShareQuality,
): RTCRtpEncodingParameters[] {
  return withTargetsOf(encodings, screenEncodings(quality));
}

/**
 * The encodings with only the layers someone watches left on (`VOICE_VIDEO_LAYERS`); an empty
 * list leaves them as they are, since the instance only sends it once the video is off.
 */
export function withActiveLayers(
  encodings: readonly RTCRtpEncodingParameters[],
  active: readonly SimulcastLayer[],
): RTCRtpEncodingParameters[] {
  if (active.length === 0) {
    return [...encodings];
  }
  return encodings.map((encoding) => ({
    ...encoding,
    active: encoding.rid === undefined || active.some((layer) => layer === encoding.rid),
  }));
}

// --- The share's sound (docs/design/web-stream-watching.md) ----------------------------------------

/**
 * Where your share's sound comes from, remembered on this device: absent or empty for the shared
 * tab's, window's or screen's own sound (what the browser's picker gives), else an audio input's id.
 */
export const SCREEN_SOUND_DEVICE_STORAGE_KEY = "mellow.screen-share-sound";
/** Device ids are opaque, but a stored value longer than this is not one. */
const MAXIMUM_STORED_DEVICE_ID_LENGTH = 512;

/** An audio input the sharer may send as their screen's sound. */
export interface ScreenSoundDevice {
  readonly deviceId: string;
  readonly label: string;
  /** Its label says it carries what the computer plays (a PipeWire or PulseAudio monitor, Stereo Mix). */
  readonly isSystemSound: boolean;
}

/** Chrome and Edge list the system's choices as extra entries, duplicates of real devices. */
const SYSTEM_CHOICE_DEVICE_IDS: ReadonlySet<string> = new Set(["default", "communications"]);
/**
 * PipeWire and PulseAudio name their monitors "Monitor of …" ("Moniteur de …" in French), Windows
 * "Stereo Mix" ("Mixage stéréo"); BlackHole and Loopback are the usual virtual devices on macOS.
 */
const SYSTEM_SOUND_LABEL_PATTERN = /\bmonitor\b|\bmoniteur\b|stereo mix|mixage st[ée]r[ée]o|what u hear|loopback|blackhole/iu;

/**
 * The audio inputs that can be a share's sound, the system's sound first, in the browser's order
 * otherwise. Empty while the browser hides their names (before the microphone is allowed).
 *
 * The microphone your voice uses is left out: sent as the share's sound, it would be heard even
 * while you mute yourself. `voiceInputDeviceId` is the chosen one, or `null` for the default: the
 * device Chrome's "default" entry names, else (Firefox, Safari) the first input, which browsers
 * list first as the default, unless it carries the system's sound.
 */
export function screenSoundDevicesOf(devices: readonly AudioDevice[], voiceInputDeviceId: string | null): ScreenSoundDevice[] {
  const audioInputs = devices.filter((device) => device.kind === "audioinput");
  const inputs = audioInputs
    .filter((device) => device.deviceId !== "" && device.label !== "" && !SYSTEM_CHOICE_DEVICE_IDS.has(device.deviceId))
    .map((device) => ({ deviceId: device.deviceId, label: device.label, isSystemSound: SYSTEM_SOUND_LABEL_PATTERN.test(device.label) }));
  const voiceDeviceId = voiceInputDeviceId ?? defaultInputDeviceId(audioInputs, inputs);
  const choices = inputs.filter((device) => device.deviceId !== voiceDeviceId);
  return [...choices.filter((device) => device.isSystemSound), ...choices.filter((device) => !device.isSystemSound)];
}

/** The device the system's default input is, by the name in Chrome's "Default - …" entry, else the first one. */
function defaultInputDeviceId(audioInputs: readonly AudioDevice[], inputs: readonly ScreenSoundDevice[]): string | null {
  const systemDefault = audioInputs.find((device) => device.deviceId === "default");
  if (systemDefault !== undefined) {
    const name = systemDefault.label.replace(/^[^-–]+[-–]\s+/u, "");
    return inputs.find((device) => device.label === name)?.deviceId ?? null;
  }
  const first = inputs[0];
  return first !== undefined && !first.isSystemSound ? first.deviceId : null;
}

export function readScreenSoundDeviceId(storage: Pick<Storage, "getItem"> | null): string | null {
  const stored = storage?.getItem(SCREEN_SOUND_DEVICE_STORAGE_KEY) ?? null;
  return stored !== null && stored.length > 0 && stored.length <= MAXIMUM_STORED_DEVICE_ID_LENGTH ? stored : null;
}

/**
 * The choice the share's options show checked: the remembered input while it is plugged in (or
 * while the list is unknown), else the screen's own sound, which is what a share then falls back to.
 */
export function shownScreenSoundDeviceId(devices: readonly ScreenSoundDevice[], deviceId: string | null): string | null {
  return deviceId !== null && (devices.length === 0 || devices.some((device) => device.deviceId === deviceId)) ? deviceId : null;
}

/** An audio input taken as it is: the processing meant for a microphone would flatten music and games. */
export function soundSourceConstraints(deviceId: string): MediaStreamConstraints {
  return {
    audio: { deviceId: { exact: deviceId }, echoCancellation: false, noiseSuppression: false, autoGainControl: false },
    video: false,
  };
}

/** Said once in the voice panel about your share's sound. */
export type ScreenSoundNotice =
  /** The share started without any sound. */
  | "no-sound"
  /** The chosen audio input could not open, or was unplugged: the screen's own sound, if any, goes instead. */
  | "source-unavailable";

/** What to tell someone whose share has no sound, by what their browser's picker can offer. */
export type SoundlessShareAdvice =
  /** Chromium on Windows and ChromeOS: a tab's sound, or the whole system's with the entire screen. */
  | "tab-or-screen"
  /** Chromium elsewhere (Linux, macOS): only a tab's sound. */
  | "tab"
  /** Firefox and Safari share no sound from their picker: only an audio input can carry it. */
  | "sound-source";

/**
 * `chromiumPlatform` is `navigator.userAgentData.platform`, which only Chromium browsers have, so
 * its absence is what says the picker offers no sound at all.
 */
export function soundlessShareAdvice(chromiumPlatform: string | null): SoundlessShareAdvice {
  if (chromiumPlatform === null) {
    return "sound-source";
  }
  return /^(windows|chrome ?os)$/iu.test(chromiumPlatform) ? "tab-or-screen" : "tab";
}

/**
 * One app's sound on Linux, without any extension: a virtual output the app plays into, whose
 * monitor the browser lists as an audio input, and a loopback so its sound is still heard. `pactl`
 * speaks to PulseAudio and to PipeWire's `pipewire-pulse` alike; both modules last until a restart.
 */
export const LINUX_SHARE_OUTPUT_COMMAND =
  "pactl load-module module-null-sink sink_name=mellow_partage sink_properties=device.description=Partage-Mellow";
export const LINUX_SHARE_LOOPBACK_COMMAND = "pactl load-module module-loopback source=mellow_partage.monitor";

/** Linux desktops, where the help with PipeWire's and PulseAudio's commands applies; not Android. */
export function isLinuxDesktop(platform: string): boolean {
  return /linux/iu.test(platform) && !/android/iu.test(platform);
}

// --- Watching a share's sound ----------------------------------------------------------------------

/** Each sharer's sound volume and mute, for you only, apart from their voice (as on Discord). */
export const STREAM_SOUND_SETTINGS_STORAGE_KEY = "mellow.stream-sound-settings";
/** Twice a second is enough to say whether a share has sound; never per frame. */
export const STREAM_SOUND_SAMPLE_INTERVAL_MILLISECONDS = 500;
/** RMS (0 to 1) above which the received sound counts as heard: about -50 dBFS, over Opus's comfort noise. */
export const STREAM_SOUND_LEVEL_THRESHOLD = 0.003;
/** No sound slot for this long once the picture shows: the share has no sound. */
export const STREAM_SOUND_HOLD_MILLISECONDS = 5_000;

export type StreamSoundStatus =
  /** Not known yet: the picture has not come, or came only a moment ago. */
  | "listening"
  | "sound"
  | "silent";

/**
 * The instance gives a screen's sound a slot only at its first packet, so a slot carrying the
 * sharer proves a sound track is sent: quiet passages (a silent scene, an idle tab) are never "no
 * sound". The level only confirms "sound", once heard on that slot. Without a slot for 5 s of
 * shown picture, there is none.
 *
 * `pictureSince`: since when the picture has been shown (`null` while it is not: not arrived,
 * paused, or you are deafened and the instance relays no sound to you).
 */
export function streamSoundStatus(
  pictureSince: number | null,
  hasSoundSlot: boolean,
  hasBeenHeard: boolean,
  now: number,
): StreamSoundStatus {
  if (hasSoundSlot) {
    return hasBeenHeard ? "sound" : "listening";
  }
  return pictureSince !== null && now - pictureSince >= STREAM_SOUND_HOLD_MILLISECONDS ? "silent" : "listening";
}

/** Why a screen is not (or no longer) shared. */
export type ScreenShareProblem =
  /** The person closed the browser's picker, or the browser refused. */
  | "denied"
  /** This browser cannot share a screen into a call. */
  | "unsupported"
  /** The instance stopped relaying it (permission taken away, or nothing sent for 10 s). */
  | "stopped"
  | VoiceVideoRefusal;

/** `getDisplayMedia` rejects with `NotAllowedError` when the picker is cancelled. */
export function screenShareProblemOf(error: unknown): ScreenShareProblem {
  const name = error instanceof Error || error instanceof DOMException ? error.name : "";
  return name === "NotAllowedError" || name === "AbortError" ? "denied" : "unsupported";
}

/**
 * The codecs every offer keeps when it carries video (ADR-0035): Opus, then VP8 and its
 * retransmissions to send; to receive, H.264 Constrained Baseline too where the browser decodes it
 * (ADR-0052), so pictures from hardware encoders reach this browser.
 */
export interface CodecPreferences {
  readonly audio: readonly RTCRtpCodec[];
  /** Your camera or screen: VP8 and its retransmissions only. */
  readonly video: readonly RTCRtpCodec[];
  /** The video receive slots: VP8, then H.264 if decoded, then retransmissions. */
  readonly receivedVideo: readonly RTCRtpCodec[];
  readonly receivesH264: boolean;
}

/** `profile_idc` 66 with `constraint_set1_flag` and no flag of a higher profile (RFC 6184, 8.1). */
const CONSTRAINED_BASELINE_PROFILE_IDC = 0x42;
const CONSTRAINED_BASELINE_IOP_MASK = 0x4f;
const CONSTRAINED_BASELINE_IOP = 0x40;

/** H.264 as the instance forwards it (ADR-0052): Constrained Baseline in packetisation mode 1. */
export function isConstrainedBaselineInPacketizationMode1(codec: RTCRtpCodec): boolean {
  if (codec.mimeType.toLowerCase() !== "video/h264") {
    return false;
  }
  const parameters = new Map(
    (codec.sdpFmtpLine ?? "").split(";").map((parameter) => {
      const [name = "", value = ""] = parameter.split("=");
      return [name.trim().toLowerCase(), value.trim().toLowerCase()] as const;
    }),
  );
  const profileLevelId = parameters.get("profile-level-id") ?? "";
  const profileIdc = Number.parseInt(profileLevelId.slice(0, 2), 16);
  const profileIop = Number.parseInt(profileLevelId.slice(2, 4), 16);
  return (
    parameters.get("packetization-mode") === "1" &&
    /^[0-9a-f]{6}$/.test(profileLevelId) &&
    profileIdc === CONSTRAINED_BASELINE_PROFILE_IDC &&
    (profileIop & CONSTRAINED_BASELINE_IOP_MASK) === CONSTRAINED_BASELINE_IOP
  );
}

export function preferredCodecs(
  audio: readonly RTCRtpCodec[],
  video: readonly RTCRtpCodec[],
): CodecPreferences | null {
  const opus = audio.filter((codec) => codec.mimeType.toLowerCase() === "audio/opus");
  const vp8 = video.filter((codec) => codec.mimeType.toLowerCase() === "video/vp8");
  const h264 = video.filter(isConstrainedBaselineInPacketizationMode1);
  const retransmissions = video.filter((codec) => codec.mimeType.toLowerCase() === "video/rtx");
  return opus.length === 0 || vp8.length === 0
    ? null
    : {
        audio: opus,
        video: [...vp8, ...retransmissions],
        receivedVideo: [...vp8, ...h264, ...retransmissions],
        receivesH264: h264.length > 0,
      };
}
