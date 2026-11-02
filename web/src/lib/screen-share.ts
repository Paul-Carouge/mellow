// What a shared screen is sent as (ADR-0037): three simulcast layers named by the RIDs the
// instance knows, sized to fit its per-layer budgets and picture limits (ADR-0035).

import type { SimulcastLayer, VoiceVideoRefusal } from "./protocol";

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
  const wanted = screenEncodings(quality);
  return encodings.map((encoding) => {
    const target = wanted.find((candidate) => candidate.rid === encoding.rid);
    return target === undefined ? encoding : { ...encoding, maxBitrate: target.maxBitrate, maxFramerate: target.maxFramerate };
  });
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

/** The codecs every offer keeps when it carries video (ADR-0035): Opus, then VP8 and its retransmissions. */
export interface CodecPreferences {
  readonly audio: readonly RTCRtpCodec[];
  readonly video: readonly RTCRtpCodec[];
}

export function preferredCodecs(
  audio: readonly RTCRtpCodec[],
  video: readonly RTCRtpCodec[],
): CodecPreferences | null {
  const opus = audio.filter((codec) => codec.mimeType.toLowerCase() === "audio/opus");
  const vp8 = video.filter((codec) => codec.mimeType.toLowerCase() === "video/vp8");
  const retransmissions = video.filter((codec) => codec.mimeType.toLowerCase() === "video/rtx");
  return opus.length === 0 || vp8.length === 0 ? null : { audio: opus, video: [...vp8, ...retransmissions] };
}
