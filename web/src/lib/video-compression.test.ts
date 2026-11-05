import { describe, expect, it } from "vitest";

import {
  AAC_BITRATE,
  COMPATIBILITY_BITRATE_RATIO,
  type EncoderSupport,
  MAXIMUM_OUTPUT_FRAME_RATE,
  OPUS_BITRATE,
  TARGET_BITS_PER_PIXEL,
  type VideoSource,
  compressedFileName,
  isOutputWorthSending,
  outputDimensions,
  planVideoCompression,
} from "./video-compression";

const MEBIBYTE = 1024 * 1024;
const FULL_SUPPORT: EncoderSupport = { canEncodeH264: true, audioCodec: "aac" };
/** A phone clip: 10 s of 4K H.264 at 30 fps with AAC, about 48 Mbit/s. */
const PHONE_CLIP: VideoSource = {
  byteSize: 60 * MEBIBYTE,
  durationSeconds: 10,
  displayWidth: 3840,
  displayHeight: 2160,
  frameRate: 30,
  videoCodec: "avc",
  canDecodeVideo: true,
  isHighDynamicRange: false,
  audioCodec: "aac",
};

function clip(overrides: Partial<VideoSource>): VideoSource {
  return { ...PHONE_CLIP, ...overrides };
}

describe("outputDimensions", () => {
  it("brings 4K down to 1080p", () => {
    expect(outputDimensions(3840, 2160)).toEqual({ width: 1920, height: 1080 });
  });

  it("keeps a portrait clip 1080 wide", () => {
    expect(outputDimensions(2160, 3840)).toEqual({ width: 1080, height: 1920 });
  });

  it("never upscales", () => {
    expect(outputDimensions(640, 360)).toEqual({ width: 640, height: 360 });
  });

  it("fits an ultra-wide clip by its long side, with even sides", () => {
    expect(outputDimensions(5120, 1440)).toEqual({ width: 1920, height: 540 });
    expect(outputDimensions(3001, 1001)).toEqual({ width: 1920, height: 640 });
  });
});

describe("planVideoCompression", () => {
  it("transcodes a 4K phone clip to 1080p H.264 at the bits-per-pixel target", () => {
    expect(planVideoCompression(PHONE_CLIP, FULL_SUPPORT, null)).toEqual({
      kind: "transcode",
      width: 1920,
      height: 1080,
      frameRate: null,
      videoBitrate: Math.round(TARGET_BITS_PER_PIXEL * 1920 * 1080 * 30),
      audio: { kind: "copy" },
      isForCompatibility: false,
    });
  });

  it("brings a 120 fps clip down to 60 and keeps 60 fps clips as they are", () => {
    const fast = planVideoCompression(clip({ frameRate: 120 }), FULL_SUPPORT, null);
    expect(fast).toMatchObject({ kind: "transcode", frameRate: MAXIMUM_OUTPUT_FRAME_RATE });
    const sixty = planVideoCompression(clip({ frameRate: 60 }), FULL_SUPPORT, null);
    expect(sixty).toMatchObject({ kind: "transcode", frameRate: null });
  });

  it("keeps HDR sources, undecodable video and files without a duration", () => {
    expect(planVideoCompression(clip({ isHighDynamicRange: true }), FULL_SUPPORT, null)).toEqual({
      kind: "keep-original",
      reason: "high-dynamic-range",
    });
    expect(planVideoCompression(clip({ canDecodeVideo: false }), FULL_SUPPORT, null)).toEqual({
      kind: "keep-original",
      reason: "undecodable-video",
    });
    expect(planVideoCompression(clip({ durationSeconds: 0 }), FULL_SUPPORT, null)).toEqual({
      kind: "keep-original",
      reason: "unknown-duration",
    });
  });

  it("keeps the original without an H.264 encoder", () => {
    expect(planVideoCompression(PHONE_CLIP, { canEncodeH264: false, audioCodec: "aac" }, null)).toEqual({
      kind: "keep-original",
      reason: "no-h264-encoder",
    });
  });

  it("encodes other audio to AAC, or Opus where the browser has no AAC encoder", () => {
    const pcm = clip({ audioCodec: "pcm-s16" });
    expect(planVideoCompression(pcm, FULL_SUPPORT, null)).toMatchObject({
      audio: { kind: "encode", codec: "aac", bitrate: AAC_BITRATE },
    });
    expect(planVideoCompression(pcm, { canEncodeH264: true, audioCodec: "opus" }, null)).toMatchObject({
      audio: { kind: "encode", codec: "opus", bitrate: OPUS_BITRATE },
    });
    expect(planVideoCompression(pcm, { canEncodeH264: true, audioCodec: null }, null)).toEqual({
      kind: "keep-original",
      reason: "no-audio-encoder",
    });
    expect(planVideoCompression(clip({ audioCodec: null }), FULL_SUPPORT, null)).toMatchObject({
      audio: { kind: "none" },
    });
  });

  it("only remuxes an H.264 source already near the target", () => {
    // 1080p30 at 4.2 Mbit/s: under 1.25 × (6.2 Mbit/s + AAC).
    const small = clip({ displayWidth: 1920, displayHeight: 1080, byteSize: 5 * MEBIBYTE });
    expect(planVideoCompression(small, FULL_SUPPORT, null)).toEqual({ kind: "remux", audio: { kind: "copy" } });
    // Remuxing needs no encoder.
    expect(planVideoCompression(small, { canEncodeH264: false, audioCodec: null }, null)).toMatchObject({
      kind: "remux",
    });
  });

  it("lowers the bitrate to fit the instance's limit, down to the quality floor", () => {
    const limit = 25 * MEBIBYTE;
    // 45 s at the target would be about 35 MB.
    const fortyFiveSeconds = planVideoCompression(clip({ durationSeconds: 45, byteSize: 270 * MEBIBYTE }), FULL_SUPPORT, limit);
    expect(fortyFiveSeconds.kind).toBe("transcode");
    if (fortyFiveSeconds.kind === "transcode") {
      expect(fortyFiveSeconds.videoBitrate).toBeLessThan(Math.round(TARGET_BITS_PER_PIXEL * 1920 * 1080 * 30));
      expect(((fortyFiveSeconds.videoBitrate + AAC_BITRATE) * 45) / 8).toBeLessThanOrEqual(limit);
    }
    // Ten minutes would need a bitrate too low to watch.
    const long = planVideoCompression(clip({ durationSeconds: 600, byteSize: 3600 * MEBIBYTE }), FULL_SUPPORT, limit);
    expect(long).toEqual({ kind: "keep-original", reason: "too-long-for-limit" });
  });
});

describe("planVideoCompression for codecs not every browser plays", () => {
  // An iPhone's 1080p30 HEVC: 10 s at about 5.2 Mbit/s, near the H.264 target.
  const IPHONE_HEVC = clip({ displayWidth: 1920, displayHeight: 1080, byteSize: 6.2 * MEBIBYTE, videoCodec: "hevc" });

  it("turns a small SDR HEVC clip into H.264, at most half again its rate", () => {
    const plan = planVideoCompression(IPHONE_HEVC, FULL_SUPPORT, null);
    expect(plan).toMatchObject({ kind: "transcode", width: 1920, height: 1080, isForCompatibility: true });
    if (plan.kind === "transcode") {
      const sourceBitrate = (IPHONE_HEVC.byteSize * 8) / IPHONE_HEVC.durationSeconds;
      expect(plan.videoBitrate).toBeLessThanOrEqual(sourceBitrate * COMPATIBILITY_BITRATE_RATIO);
    }
  });

  it("does not spend the target's rate on a tiny VP9 clip", () => {
    const tiny = clip({ displayWidth: 640, displayHeight: 360, byteSize: 250_000, videoCodec: "vp9" });
    const plan = planVideoCompression(tiny, FULL_SUPPORT, null);
    expect(plan).toMatchObject({ kind: "transcode", isForCompatibility: true });
    if (plan.kind === "transcode") {
      expect(plan.videoBitrate).toBeLessThanOrEqual(((250_000 * 8) / 10) * COMPATIBILITY_BITRATE_RATIO);
    }
  });

  it("keeps HEVC as it is without an H.264 encoder", () => {
    expect(planVideoCompression(IPHONE_HEVC, { canEncodeH264: false, audioCodec: "aac" }, null)).toEqual({
      kind: "keep-original",
      reason: "no-h264-encoder",
    });
  });
});

describe("planVideoCompression against the instance's limit", () => {
  const LIMIT = 25 * MEBIBYTE;

  it("transcodes a near-target H.264 clip over the limit instead of remuxing it", () => {
    // 40 s of 1080p30 at 6 Mbit/s: about 30 MB, near the target but over 25 MiB.
    const overLimit = clip({ displayWidth: 1920, displayHeight: 1080, durationSeconds: 40, byteSize: 30_600_000 });
    expect(planVideoCompression(overLimit, FULL_SUPPORT, null)).toMatchObject({ kind: "remux" });
    const plan = planVideoCompression(overLimit, FULL_SUPPORT, LIMIT);
    expect(plan).toMatchObject({ kind: "transcode", isForCompatibility: false });
    if (plan.kind === "transcode") {
      expect(((plan.videoBitrate + AAC_BITRATE) * 40) / 8).toBeLessThanOrEqual(LIMIT);
    }
  });

  it("keeps the original when the frame rate is unknown", () => {
    for (const frameRate of [0, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(planVideoCompression(clip({ frameRate }), FULL_SUPPORT, LIMIT)).toEqual({
        kind: "keep-original",
        reason: "unknown-frame-rate",
      });
    }
  });
});

describe("isOutputWorthSending", () => {
  const TRANSCODE = { kind: "transcode", isForCompatibility: false } as const;
  const COMPATIBILITY_TRANSCODE = { kind: "transcode", isForCompatibility: true } as const;
  const REMUX = { kind: "remux" } as const;

  it("wants a transcode at least 20 % smaller", () => {
    expect(isOutputWorthSending(TRANSCODE, 100, 80, null)).toBe(true);
    expect(isOutputWorthSending(TRANSCODE, 100, 81, null)).toBe(false);
  });

  it("takes a remux that does not grow", () => {
    expect(isOutputWorthSending(REMUX, 100, 100, null)).toBe(true);
    expect(isOutputWorthSending(REMUX, 100, 101, null)).toBe(false);
  });

  it("takes any output that fits when the original is over the limit", () => {
    expect(isOutputWorthSending(TRANSCODE, 120, 100, 100)).toBe(true);
    expect(isOutputWorthSending(TRANSCODE, 120, 101, 100)).toBe(false);
    expect(isOutputWorthSending(REMUX, 120, 101, 100)).toBe(false);
  });

  it("takes a transcode for compatibility that fits the limit, even a little larger", () => {
    expect(isOutputWorthSending(COMPATIBILITY_TRANSCODE, 100, 130, 200)).toBe(true);
    expect(isOutputWorthSending(COMPATIBILITY_TRANSCODE, 100, 130, null)).toBe(true);
    expect(isOutputWorthSending(COMPATIBILITY_TRANSCODE, 100, 201, 200)).toBe(false);
  });
});

describe("compressedFileName", () => {
  it("gives the name the MP4 extension", () => {
    expect(compressedFileName("IMG_0042.MOV")).toBe("IMG_0042.mp4");
    expect(compressedFileName("clip.final.webm")).toBe("clip.final.mp4");
    expect(compressedFileName("video")).toBe("video.mp4");
    expect(compressedFileName(".hidden")).toBe(".hidden.mp4");
  });
});
