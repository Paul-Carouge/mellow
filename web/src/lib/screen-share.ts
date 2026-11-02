// What a shared screen is sent as (ADR-0037): three simulcast layers named by the RIDs the
// instance knows, sized to fit its per-layer budgets and picture limits (ADR-0035).

import type { SimulcastLayer, VoiceVideoRefusal } from "./protocol";

/** The capture asked of the browser: text stays readable, and still screens cost little. */
export const SCREEN_MAXIMUM_WIDTH = 1920;
export const SCREEN_MAXIMUM_HEIGHT = 1080;
export const SCREEN_MAXIMUM_FRAMES_PER_SECOND = 15;

/** Lowest first, as browsers list simulcast layers. */
export const SCREEN_ENCODINGS: readonly RTCRtpEncodingParameters[] = [
  { rid: "q", scaleResolutionDownBy: 4, maxBitrate: 150_000, maxFramerate: SCREEN_MAXIMUM_FRAMES_PER_SECOND },
  { rid: "l", scaleResolutionDownBy: 2, maxBitrate: 500_000, maxFramerate: SCREEN_MAXIMUM_FRAMES_PER_SECOND },
  { rid: "h", scaleResolutionDownBy: 1, maxBitrate: 2_500_000, maxFramerate: SCREEN_MAXIMUM_FRAMES_PER_SECOND },
];

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
