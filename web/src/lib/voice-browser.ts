// The browser side of `VoicePorts`: WebRTC, the microphone, playback, devices, the level meter and
// the processing chain. Thin on purpose: the rules live in `voice-session.svelte.ts` and
// `voice-processing.ts`, tested with fakes. RNNoise runs in an AudioWorklet loaded from this origin
// (`script-src 'self'`, not a worker) and compiles its WebAssembly under `'wasm-unsafe-eval'` (ADR-0031).

import rnnoiseWasmUrl from "@sapphi-red/web-noise-suppressor/rnnoise.wasm?url";
import rnnoiseSimdWasmUrl from "@sapphi-red/web-noise-suppressor/rnnoise_simd.wasm?url";
import rnnoiseWorkletUrl from "@sapphi-red/web-noise-suppressor/rnnoiseWorklet.js?url";

import { browserStorage } from "./reactions";
import {
  type MicrophoneChoice,
  type MicrophoneProcessing,
  NOISE_SUPPRESSOR_SAMPLE_RATE_HERTZ,
  type NoiseSuppressorProblem,
  type ProcessingStage,
  microphoneConstraintsOf,
  processingStagesOf,
} from "./voice-processing";
import type { AudioDevice, EqualizerBand } from "./voice-settings";
import type { AudioOutput, InputAmplifier, LevelMeter, MicrophoneProcessor, VoicePorts } from "./voice-session.svelte";

const ANALYSER_FFT_SIZE = 512;

function createPeerConnection(): RTCPeerConnection | null {
  if (typeof RTCPeerConnection === "undefined") {
    return null;
  }
  // No STUN or TURN: the instance is ICE-lite with its public address, and the browser dials it (ADR-0030).
  return new RTCPeerConnection({ bundlePolicy: "max-bundle", rtcpMuxPolicy: "require" });
}

function requestMicrophone(choice: MicrophoneChoice): Promise<MediaStream> {
  // Absent outside a secure context: reported as "unavailable".
  if (!navigator.mediaDevices?.getUserMedia) {
    return Promise.reject(new TypeError("navigator.mediaDevices is unavailable"));
  }
  return navigator.mediaDevices.getUserMedia({ audio: microphoneConstraintsOf(choice), video: false });
}

/** Where RNNoise can run at all; the settings say so before anything is loaded. */
export const CAN_SUPPRESS_NOISE_IN_PAGE =
  typeof AudioWorkletNode !== "undefined" && typeof WebAssembly !== "undefined" && typeof AudioContext !== "undefined";

/** RNNoise's binary, fetched once per page; forgotten on failure so a later microphone tries again. */
let noiseSuppressorBinary: Promise<ArrayBuffer> | null = null;

async function loadNoiseSuppressorBinary(): Promise<ArrayBuffer> {
  // Imported here, not at the top: the package defines its node class by extending
  // `AudioWorkletNode`, which would throw on load in a browser without it.
  const { loadRnnoise } = await import("@sapphi-red/web-noise-suppressor");
  const binary = await loadRnnoise({ url: rnnoiseWasmUrl, simdUrl: rnnoiseSimdWasmUrl });
  // The worklet compiles it under this page's policy; compiling it here first finds a refusal
  // before the microphone is opened without the browser's own suppression.
  await WebAssembly.compile(binary);
  return binary;
}

async function prepareNoiseSuppressor(): Promise<NoiseSuppressorProblem | null> {
  if (!CAN_SUPPRESS_NOISE_IN_PAGE) {
    return "unsupported";
  }
  noiseSuppressorBinary ??= loadNoiseSuppressorBinary();
  try {
    await noiseSuppressorBinary;
    return null;
  } catch (error) {
    console.warn("couldn't load the noise suppressor", error);
    noiseSuppressorBinary = null;
    return "failed";
  }
}

async function createStageNode(
  context: AudioContext,
  stage: ProcessingStage,
  bands: Map<EqualizerBand, BiquadFilterNode>,
): Promise<AudioNode> {
  switch (stage.kind) {
    case "noise-suppressor": {
      if (noiseSuppressorBinary === null) {
        throw new Error("the noise suppressor was not prepared");
      }
      const [{ RnnoiseWorkletNode }, wasmBinary] = await Promise.all([
        import("@sapphi-red/web-noise-suppressor"),
        noiseSuppressorBinary,
        context.audioWorklet.addModule(rnnoiseWorkletUrl),
      ]);
      const node = new RnnoiseWorkletNode(context, { wasmBinary, maxChannels: 1 });
      node.addEventListener("processorerror", (event) => console.warn("the noise suppressor stopped", event));
      return node;
    }
    case "filter": {
      const filter = context.createBiquadFilter();
      filter.type = stage.type;
      filter.frequency.value = stage.frequencyHertz;
      filter.Q.value = stage.quality;
      filter.gain.value = stage.gainDecibels;
      if (stage.band !== "rumble") {
        bands.set(stage.band, filter);
      }
      return filter;
    }
    case "dynamics": {
      const compressor = context.createDynamicsCompressor();
      compressor.threshold.value = stage.thresholdDecibels;
      compressor.knee.value = stage.kneeDecibels;
      compressor.ratio.value = stage.ratio;
      compressor.attack.value = stage.attackSeconds;
      compressor.release.value = stage.releaseSeconds;
      return compressor;
    }
  }
}

/**
 * The chain in an AudioContext of its own, into a track of its own. RNNoise needs 48 kHz; Firefox
 * refuses a microphone at another rate, which rejects here and falls back to the browser's filter.
 */
async function processMicrophone(track: MediaStreamTrack, processing: MicrophoneProcessing): Promise<MicrophoneProcessor | null> {
  if (typeof AudioContext === "undefined") {
    return null;
  }
  const context = new AudioContext(processing.isNoiseSuppressorOn ? { sampleRate: NOISE_SUPPRESSOR_SAMPLE_RATE_HERTZ } : {});
  try {
    const bands = new Map<EqualizerBand, BiquadFilterNode>();
    const nodes: AudioNode[] = [context.createMediaStreamSource(new MediaStream([track]))];
    for (const stage of processingStagesOf(processing)) {
      nodes.push(await createStageNode(context, stage, bands));
    }
    const destination = context.createMediaStreamDestination();
    destination.channelCount = 1;
    nodes.push(destination);
    nodes.reduce((from, to) => from.connect(to));
    const processedTrack = destination.stream.getAudioTracks()[0];
    if (!processedTrack) {
      throw new Error("the processing graph gave no track");
    }
    if (context.state === "suspended") {
      // Not awaited: without the page's activation it would wait for the next click.
      context.resume().catch((error: unknown) => console.warn("couldn't start the microphone processing", error));
    }
    return {
      track: processedTrack,
      setEqualizer: (equalizer) => {
        for (const [band, filter] of bands) {
          filter.gain.value = equalizer[band];
        }
      },
      stop: () => {
        processedTrack.stop();
        for (const node of nodes) {
          node.disconnect();
          if ("destroy" in node && typeof node.destroy === "function") {
            // RNNoise's node frees its WebAssembly memory in the worklet.
            node.destroy();
          }
        }
        context.close().catch((error: unknown) => console.warn("couldn't close the microphone processing", error));
      },
    };
  } catch (error) {
    context.close().catch((closeError: unknown) => console.warn("couldn't close the microphone processing", closeError));
    throw error;
  }
}

/** Where the browser lets a page choose its output (not Safari). */
export const CAN_CHOOSE_OUTPUT_DEVICE = typeof HTMLMediaElement !== "undefined" && "setSinkId" in HTMLMediaElement.prototype;

function routeTo(element: HTMLMediaElement, deviceId: string | null): void {
  if (!CAN_CHOOSE_OUTPUT_DEVICE) {
    return;
  }
  // An unplugged device rejects: the session notices through `devicechange` and goes back to default.
  element.setSinkId(deviceId ?? "").catch((error: unknown) => console.warn("couldn't route a voice to the chosen output", error));
}

/**
 * Voices above 100 % go through one shared Web Audio graph, played by one element routed like the
 * others. Counted by the voices using it: the last one to leave closes it, so nothing keeps the
 * output device after a call.
 */
let amplifiedPlayback: {
  context: AudioContext;
  destination: MediaStreamAudioDestinationNode;
  element: HTMLAudioElement;
  voiceCount: number;
} | null = null;

function useAmplifiedPlayback(deviceId: string | null) {
  if (amplifiedPlayback === null) {
    const context = new AudioContext();
    const destination = context.createMediaStreamDestination();
    const element = new Audio();
    element.srcObject = destination.stream;
    element.play().catch((error: unknown) => console.warn("couldn't play amplified voices", error));
    amplifiedPlayback = { context, destination, element, voiceCount: 0 };
  }
  amplifiedPlayback.voiceCount += 1;
  if (amplifiedPlayback.context.state === "suspended") {
    amplifiedPlayback.context.resume().catch((error: unknown) => console.warn("couldn't start amplified voices", error));
  }
  routeTo(amplifiedPlayback.element, deviceId);
  return amplifiedPlayback;
}

function leaveAmplifiedPlayback(): void {
  if (amplifiedPlayback === null) {
    return;
  }
  amplifiedPlayback.voiceCount -= 1;
  if (amplifiedPlayback.voiceCount > 0) {
    return;
  }
  const { context, element } = amplifiedPlayback;
  amplifiedPlayback = null;
  element.pause();
  element.srcObject = null;
  context.close().catch((error: unknown) => console.warn("couldn't close the amplified voices", error));
}

/**
 * One receive slot's voice. Up to 100 % it plays through its own `<audio>` element, as in 16.2:
 * browsers only play remote WebRTC audio through a media element, and Chrome's echo cancellation
 * hears what plays there. Above 100 % the element goes quiet and a `GainNode` takes over (the
 * element stays attached, which Chrome needs to feed remote audio into Web Audio).
 */
function playTrack(track: MediaStreamTrack): AudioOutput {
  const stream = new MediaStream([track]);
  const audio = new Audio();
  audio.autoplay = true;
  audio.srcObject = stream;
  audio.play().catch((error: unknown) => console.warn("couldn't play a voice", error));
  let deviceId: string | null = null;
  let amplified: { source: MediaStreamAudioSourceNode; gain: GainNode } | null = null;
  const stopAmplifying = () => {
    if (amplified !== null) {
      amplified.source.disconnect();
      amplified.gain.disconnect();
      amplified = null;
      leaveAmplifiedPlayback();
    }
  };
  return {
    setGain: (gain) => {
      if (gain <= 1 || typeof AudioContext === "undefined") {
        stopAmplifying();
        audio.muted = gain === 0;
        audio.volume = Math.min(1, gain);
        return;
      }
      if (amplified === null) {
        const graph = useAmplifiedPlayback(deviceId);
        const source = graph.context.createMediaStreamSource(stream);
        const gainNode = graph.context.createGain();
        source.connect(gainNode).connect(graph.destination);
        amplified = { source, gain: gainNode };
      }
      amplified.gain.gain.value = gain;
      audio.muted = true;
    },
    setOutputDevice: (nextDeviceId) => {
      deviceId = nextDeviceId;
      routeTo(audio, deviceId);
      if (amplifiedPlayback !== null) {
        routeTo(amplifiedPlayback.element, deviceId);
      }
    },
    stop: () => {
      stopAmplifying();
      audio.pause();
      audio.srcObject = null;
    },
  };
}

/** The meter reads a copy of the track: the session disables the sent one to gate it. */
function meterLevel(track: MediaStreamTrack): LevelMeter | null {
  if (typeof AudioContext === "undefined") {
    return null;
  }
  const copy = track.clone();
  const context = new AudioContext();
  const analyser = context.createAnalyser();
  analyser.fftSize = ANALYSER_FFT_SIZE;
  context.createMediaStreamSource(new MediaStream([copy])).connect(analyser);
  const samples = new Float32Array(analyser.fftSize);
  return {
    level: () => {
      analyser.getFloatTimeDomainData(samples);
      let sumOfSquares = 0;
      for (const sample of samples) {
        sumOfSquares += sample * sample;
      }
      return Math.sqrt(sumOfSquares / samples.length);
    },
    stop: () => {
      copy.stop();
      context.close().catch((error: unknown) => console.warn("couldn't close the level meter", error));
    },
  };
}

/** Input volume other than 100 %: the microphone through a `GainNode` into a track of its own. */
function amplify(track: MediaStreamTrack, gain: number): InputAmplifier | null {
  if (typeof AudioContext === "undefined") {
    return null;
  }
  const context = new AudioContext();
  const gainNode = context.createGain();
  gainNode.gain.value = gain;
  const destination = context.createMediaStreamDestination();
  context.createMediaStreamSource(new MediaStream([track])).connect(gainNode).connect(destination);
  const amplifiedTrack = destination.stream.getAudioTracks()[0];
  if (!amplifiedTrack) {
    void context.close();
    return null;
  }
  return {
    track: amplifiedTrack,
    setGain: (nextGain) => {
      gainNode.gain.value = nextGain;
    },
    stop: () => {
      amplifiedTrack.stop();
      context.close().catch((error: unknown) => console.warn("couldn't close the input amplifier", error));
    },
  };
}

export async function listAudioDevices(): Promise<readonly AudioDevice[]> {
  if (!navigator.mediaDevices?.enumerateDevices) {
    return [];
  }
  const devices = await navigator.mediaDevices.enumerateDevices();
  return devices.flatMap((device) =>
    device.kind === "audioinput" || device.kind === "audiooutput"
      ? [{ kind: device.kind, deviceId: device.deviceId, label: device.label }]
      : [],
  );
}

export function watchAudioDevices(onChange: () => void): () => void {
  const mediaDevices = navigator.mediaDevices;
  if (!mediaDevices) {
    return () => {};
  }
  mediaDevices.addEventListener("devicechange", onChange);
  return () => mediaDevices.removeEventListener("devicechange", onChange);
}

export const BROWSER_VOICE_PORTS: Omit<VoicePorts, "sendFrame"> = {
  createPeerConnection,
  requestMicrophone,
  prepareNoiseSuppressor,
  processMicrophone,
  playTrack,
  meterLevel,
  amplify,
  listDevices: listAudioDevices,
  watchDevices: watchAudioDevices,
  storage: browserStorage,
};
