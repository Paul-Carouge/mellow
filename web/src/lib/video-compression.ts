// What to do with a video before it uploads (ADR-0046): pure decisions, so the Worker that runs
// WebCodecs only measures the file and carries the plan out.

/** Output fits a 1920 × 1080 box in the video's own orientation: portrait clips stay 1080 wide. */
export const MAXIMUM_OUTPUT_SHORT_SIDE = 1080;
export const MAXIMUM_OUTPUT_LONG_SIDE = 1920;
/** Faster sources are brought down to this; slower ones keep their rate. */
export const MAXIMUM_OUTPUT_FRAME_RATE = 60;
/**
 * H.264 bits per pixel per frame aimed at: 0.1 is about 6.2 Mbit/s at 1080p30, VMAF 89 and 75 on
 * the ADR's two test clips with Chrome's encoder on macOS. VBR spends less on static content.
 */
export const TARGET_BITS_PER_PIXEL = 0.1;
/** Below this, squeezing a long clip under the instance's limit would look smeared: keep the original. */
export const MINIMUM_BITS_PER_PIXEL = 0.05;
/** A source at most this much above the target is not worth a lossy second generation: remux it only. */
export const REMUX_SOURCE_BITRATE_RATIO = 1.25;
/**
 * A source in a codec not every browser plays is made H.264 at no more than this times its rate:
 * H.264 needs about half again HEVC's or VP9's bits for the same picture.
 * shortcut: one ratio for HEVC, VP9 and AV1 (AV1 needs nearer twice), measure per codec if it matters.
 */
export const COMPATIBILITY_BITRATE_RATIO = 1.5;
/** A transcode is kept only when it saves at least this share of the original's bytes. */
export const MINIMUM_TRANSCODE_SAVING = 0.2;
/** Room left under the instance's limit for container overhead and VBR overshoot. */
export const LIMIT_BUDGET_SHARE = 0.9;
export const AAC_BITRATE = 128_000;
export const OPUS_BITRATE = 96_000;
export const KEY_FRAME_INTERVAL_SECONDS = 2;

/** Audio codecs MP4 carries that every browser plays: copied as they are. */
const COPIED_AUDIO_CODECS: readonly string[] = ["aac", "opus"];

/** What the Worker measured of the file's primary tracks. */
export interface VideoSource {
  readonly byteSize: number;
  readonly durationSeconds: number;
  /** After rotation and pixel aspect ratio, as the reader sees it. */
  readonly displayWidth: number;
  readonly displayHeight: number;
  readonly frameRate: number;
  /** mediabunny's codec name ("avc", "hevc", "vp9", "av1"); null when unknown. */
  readonly videoCodec: string | null;
  readonly canDecodeVideo: boolean;
  readonly isHighDynamicRange: boolean;
  /** null without an audio track. */
  readonly audioCodec: string | null;
}

/** What this browser can encode, asked once per file. */
export interface EncoderSupport {
  readonly canEncodeH264: boolean;
  /** AAC where the browser has it (Chrome on Linux does not), else Opus; null when neither. */
  readonly audioCodec: "aac" | "opus" | null;
}

export type KeepOriginalReason =
  | "undecodable-video"
  | "unknown-duration"
  | "unknown-frame-rate"
  | "high-dynamic-range"
  | "no-h264-encoder"
  | "no-audio-encoder"
  | "too-long-for-limit";

export type AudioPlan =
  | { readonly kind: "none" }
  | { readonly kind: "copy" }
  | { readonly kind: "encode"; readonly codec: "aac" | "opus"; readonly bitrate: number };

export type VideoCompressionPlan =
  | { readonly kind: "keep-original"; readonly reason: KeepOriginalReason }
  /** Packets copied into a faststart MP4 without the source's metadata tags. */
  | { readonly kind: "remux"; readonly audio: AudioPlan }
  | {
      readonly kind: "transcode";
      readonly width: number;
      readonly height: number;
      /** null keeps the source's rate. */
      readonly frameRate: number | null;
      readonly videoBitrate: number;
      readonly audio: AudioPlan;
      /** The source's codec does not play in every browser: H.264 is worth it even without a saving. */
      readonly isForCompatibility: boolean;
    };

/** Scaled down to fit the 1080p box, never up; even sides, as H.264 encoders want. */
export function outputDimensions(width: number, height: number): { width: number; height: number } {
  const scale = Math.min(
    1,
    MAXIMUM_OUTPUT_SHORT_SIDE / Math.min(width, height),
    MAXIMUM_OUTPUT_LONG_SIDE / Math.max(width, height),
  );
  return { width: evenAtMost(width * scale), height: evenAtMost(height * scale) };
}

function evenAtMost(length: number): number {
  return Math.max(2, Math.floor(length / 2) * 2);
}

function bitrateAt(bitsPerPixel: number, width: number, height: number, frameRate: number): number {
  return Math.round(bitsPerPixel * width * height * frameRate);
}

function audioPlanFor(sourceAudioCodec: string | null, support: EncoderSupport): AudioPlan | null {
  if (sourceAudioCodec === null) {
    return { kind: "none" };
  }
  if (COPIED_AUDIO_CODECS.includes(sourceAudioCodec)) {
    return { kind: "copy" };
  }
  if (support.audioCodec === null) {
    return null;
  }
  return {
    kind: "encode",
    codec: support.audioCodec,
    bitrate: support.audioCodec === "aac" ? AAC_BITRATE : OPUS_BITRATE,
  };
}

function audioBitrateOf(audio: AudioPlan, sourceTotalBitrate: number): number {
  switch (audio.kind) {
    case "none":
      return 0;
    // Unknown without reading its packets; AAC's rate is a fair guess and only sizes the budget.
    case "copy":
      return Math.min(AAC_BITRATE, sourceTotalBitrate);
    case "encode":
      return audio.bitrate;
  }
}

/**
 * Decides once per file: transcode to H.264 at the bits-per-pixel target (and within the instance's
 * limit when it is known), remux an H.264 source already near the target and within the limit, or
 * keep the original. Other codecs are always transcoded, so every browser plays the result.
 */
export function planVideoCompression(
  source: VideoSource,
  support: EncoderSupport,
  maximumAttachmentBytes: number | null,
): VideoCompressionPlan {
  if (!source.canDecodeVideo) {
    return { kind: "keep-original", reason: "undecodable-video" };
  }
  if (!(source.durationSeconds > 0)) {
    return { kind: "keep-original", reason: "unknown-duration" };
  }
  if (!(Number.isFinite(source.frameRate) && source.frameRate > 0)) {
    return { kind: "keep-original", reason: "unknown-frame-rate" };
  }
  // Tone mapping to SDR H.264 washes HDR out; kept until the ADR handles it.
  if (source.isHighDynamicRange) {
    return { kind: "keep-original", reason: "high-dynamic-range" };
  }
  const audio = audioPlanFor(source.audioCodec, support);
  if (audio === null) {
    return { kind: "keep-original", reason: "no-audio-encoder" };
  }
  const { width, height } = outputDimensions(source.displayWidth, source.displayHeight);
  const frameRate = Math.min(source.frameRate, MAXIMUM_OUTPUT_FRAME_RATE);
  const sourceTotalBitrate = (source.byteSize * 8) / source.durationSeconds;
  const audioBitrate = audioBitrateOf(audio, sourceTotalBitrate);
  const targetVideoBitrate = bitrateAt(TARGET_BITS_PER_PIXEL, width, height, frameRate);
  const isResized = width < evenAtMost(source.displayWidth) || height < evenAtMost(source.displayHeight);
  // Only H.264 is copied: the MP4 must play in Safari and Firefox too.
  const isForCompatibility = source.videoCodec !== "avc";
  const isNearTarget =
    !isResized &&
    source.frameRate <= MAXIMUM_OUTPUT_FRAME_RATE &&
    sourceTotalBitrate <= (targetVideoBitrate + audioBitrate) * REMUX_SOURCE_BITRATE_RATIO;
  const isWithinLimit = maximumAttachmentBytes === null || source.byteSize <= maximumAttachmentBytes;
  if (isNearTarget && !isForCompatibility && isWithinLimit) {
    return { kind: "remux", audio };
  }
  if (!support.canEncodeH264) {
    return { kind: "keep-original", reason: "no-h264-encoder" };
  }
  const limitVideoBitrate =
    maximumAttachmentBytes === null
      ? Number.POSITIVE_INFINITY
      : (maximumAttachmentBytes * 8 * LIMIT_BUDGET_SHARE) / source.durationSeconds - audioBitrate;
  if (Math.min(targetVideoBitrate, limitVideoBitrate) < bitrateAt(MINIMUM_BITS_PER_PIXEL, width, height, frameRate)) {
    return { kind: "keep-original", reason: "too-long-for-limit" };
  }
  // A low-rate source is not inflated to the target: its picture has no more detail to keep.
  const compatibilityVideoBitrate = isForCompatibility
    ? sourceTotalBitrate * COMPATIBILITY_BITRATE_RATIO
    : Number.POSITIVE_INFINITY;
  const videoBitrate = Math.floor(Math.min(targetVideoBitrate, limitVideoBitrate, compatibilityVideoBitrate));
  return {
    kind: "transcode",
    width,
    height,
    frameRate: source.frameRate > MAXIMUM_OUTPUT_FRAME_RATE ? MAXIMUM_OUTPUT_FRAME_RATE : null,
    videoBitrate,
    audio,
    isForCompatibility,
  };
}

/**
 * An original over the limit would be refused: any output that fits goes. Otherwise a transcode
 * must save a fifth to be worth its generation loss, unless it is for compatibility, which only
 * must fit; a remux only must not grow.
 */
export function isOutputWorthSending(
  plan: { readonly kind: "remux" } | { readonly kind: "transcode"; readonly isForCompatibility: boolean },
  originalBytes: number,
  outputBytes: number,
  maximumAttachmentBytes: number | null,
): boolean {
  const fitsLimit = maximumAttachmentBytes === null || outputBytes <= maximumAttachmentBytes;
  if (maximumAttachmentBytes !== null && originalBytes > maximumAttachmentBytes) {
    return fitsLimit;
  }
  if (plan.kind === "remux") {
    return outputBytes <= originalBytes;
  }
  return plan.isForCompatibility ? fitsLimit : outputBytes <= originalBytes * (1 - MINIMUM_TRANSCODE_SAVING);
}

/** The compressed file's name: the original's, with the MP4 extension it now has. */
export function compressedFileName(originalName: string): string {
  const dot = originalName.lastIndexOf(".");
  return `${dot > 0 ? originalName.slice(0, dot) : originalName}.mp4`;
}
