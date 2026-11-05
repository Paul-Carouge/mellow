// The video compression Worker (ADR-0046): measures the file with mediabunny, asks the pure plan in
// `video-compression.ts`, and carries it out with WebCodecs, off the main thread.

import {
  BlobSource,
  BufferTarget,
  Conversion,
  type ConversionAudioOptions,
  Input,
  MATROSKA,
  MP4,
  Mp4OutputFormat,
  Output,
  QTFF,
  WEBM,
  canEncodeAudio,
  canEncodeVideo,
} from "mediabunny";

import {
  type CompressVideo,
  type CompressionWorkerScope,
  type VideoCompressionRequest,
  answerVideoCompressionRequests,
} from "./video-compression-protocol";
import {
  type AudioPlan,
  type EncoderSupport,
  KEY_FRAME_INTERVAL_SECONDS,
  type VideoSource,
  isOutputWorthSending,
  outputDimensions,
  planVideoCompression,
} from "./video-compression";

/** Enough packets to know the frame rate without reading a whole WebM. */
const FRAME_RATE_SAMPLE_PACKETS = 120;

function audioOptions(plan: AudioPlan): ConversionAudioOptions {
  switch (plan.kind) {
    case "none":
    case "copy":
      return {};
    case "encode":
      return { codec: plan.codec, bitrate: plan.bitrate };
  }
}

const compressVideo: CompressVideo = async (request, onprogress) => {
  const input = new Input({ source: new BlobSource(request.file), formats: [MP4, QTFF, WEBM, MATROSKA] });
  try {
    return await compressInput(input, request, onprogress);
  } finally {
    input.dispose();
  }
};

async function compressInput(
  input: Input,
  { file, maximumAttachmentBytes }: VideoCompressionRequest,
  onprogress: (fraction: number) => void,
): ReturnType<CompressVideo> {
  const videoTrack = await input.getPrimaryVideoTrack();
  if (videoTrack === null) {
    return { keptOriginalBecause: "no-video-track" };
  }
  const audioTrack = await input.getPrimaryAudioTrack();
  const displayWidth = await videoTrack.getDisplayWidth();
  const displayHeight = await videoTrack.getDisplayHeight();
  const source: VideoSource = {
    byteSize: file.size,
    durationSeconds: await input.computeDuration(),
    displayWidth,
    displayHeight,
    frameRate: (await videoTrack.computePacketStats(FRAME_RATE_SAMPLE_PACKETS)).averagePacketRate,
    videoCodec: await videoTrack.getCodec(),
    canDecodeVideo: await videoTrack.canDecode(),
    isHighDynamicRange: await videoTrack.hasHighDynamicRange(),
    audioCodec: audioTrack === null ? null : await audioTrack.getCodec(),
  };
  const { width, height } = outputDimensions(displayWidth, displayHeight);
  const support: EncoderSupport = {
    canEncodeH264: await canEncodeVideo("avc", { width, height }),
    audioCodec: (await canEncodeAudio("aac")) ? "aac" : (await canEncodeAudio("opus")) ? "opus" : null,
  };
  const plan = planVideoCompression(source, support, maximumAttachmentBytes);
  if (plan.kind === "keep-original") {
    return { keptOriginalBecause: plan.reason };
  }
  const output = new Output({ format: new Mp4OutputFormat({ fastStart: "in-memory" }), target: new BufferTarget() });
  const conversion = await Conversion.init({
    input,
    output,
    tracks: "primary",
    video:
      plan.kind === "remux"
        ? {}
        : {
            codec: "avc",
            width: plan.width,
            height: plan.height,
            fit: "fill",
            // Baked into the frames: the box sizes were computed on the rotated picture.
            allowTransformationMetadata: false,
            ...(plan.frameRate === null ? {} : { frameRate: plan.frameRate }),
            bitrate: plan.videoBitrate,
            keyFrameInterval: KEY_FRAME_INTERVAL_SECONDS,
          },
    audio: audioOptions(plan.audio),
    // The source's tags (location, device, dates) are not copied.
    tags: {},
    showWarnings: false,
  });
  // A dropped track would lose the sound or the picture.
  if (!conversion.isValid || conversion.discardedTracks.length > 0) {
    return { keptOriginalBecause: "track-not-convertible" };
  }
  conversion.onProgress = onprogress;
  await conversion.execute();
  const bytes = output.target.buffer;
  if (bytes === null) {
    throw new Error("the conversion finished without an output buffer");
  }
  return isOutputWorthSending(plan.kind, file.size, bytes.byteLength)
    ? { bytes }
    : { keptOriginalBecause: "not-smaller" };
}

// The project's TypeScript knows the window's globals only; in this file `self` is the Worker's
// scope, whose `onmessage` and `postMessage` are the ones `CompressionWorkerScope` names.
answerVideoCompressionRequests(self as unknown as CompressionWorkerScope, compressVideo);
