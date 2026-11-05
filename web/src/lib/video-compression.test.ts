import { describe, expect, it } from "vitest";

import {
  AAC_BITRATE,
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

  it("only remuxes an H.264 source already near the target, and keeps other codecs as they are", () => {
    // 1080p30 at 4.2 Mbit/s: under 1.25 × (6.2 Mbit/s + AAC).
    const small = clip({ displayWidth: 1920, displayHeight: 1080, byteSize: 5 * MEBIBYTE });
    expect(planVideoCompression(small, FULL_SUPPORT, null)).toEqual({ kind: "remux", audio: { kind: "copy" } });
    expect(planVideoCompression({ ...small, videoCodec: "vp9" }, FULL_SUPPORT, null)).toEqual({
      kind: "keep-original",
      reason: "already-small",
    });
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

describe("isOutputWorthSending", () => {
  it("wants a transcode at least 20 % smaller", () => {
    expect(isOutputWorthSending("transcode", 100, 80)).toBe(true);
    expect(isOutputWorthSending("transcode", 100, 81)).toBe(false);
  });

  it("takes a remux that does not grow", () => {
    expect(isOutputWorthSending("remux", 100, 100)).toBe(true);
    expect(isOutputWorthSending("remux", 100, 101)).toBe(false);
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
