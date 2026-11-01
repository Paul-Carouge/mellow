// The browser side of `VoicePorts`: WebRTC, the microphone, playback, devices and the level meter.
// Thin on purpose: the rules live in `voice-session.svelte.ts`, tested with fakes. Only built-in
// Web Audio nodes are used (AnalyserNode, GainNode): an AudioWorklet would need `worker-src`,
// which the policy keeps at 'none'.

import { browserStorage } from "./reactions";
import type { AudioDevice } from "./voice-settings";
import type { AudioOutput, InputAmplifier, LevelMeter, MicrophoneChoice, VoicePorts } from "./voice-session.svelte";

const ANALYSER_FFT_SIZE = 512;

function createPeerConnection(): RTCPeerConnection | null {
  if (typeof RTCPeerConnection === "undefined") {
    return null;
  }
  // No STUN or TURN: the instance is ICE-lite with its public address, and the browser dials it (ADR-0030).
  return new RTCPeerConnection({ bundlePolicy: "max-bundle", rtcpMuxPolicy: "require" });
}

/**
 * What Discord asks for: the browser's echo cancellation and gain always, noise suppression as
 * chosen, one channel. A chosen device is required (`exact`), so its absence fails and the
 * session falls back to the default one, saying so.
 */
function requestMicrophone(choice: MicrophoneChoice): Promise<MediaStream> {
  // Absent outside a secure context: reported as "unavailable".
  if (!navigator.mediaDevices?.getUserMedia) {
    return Promise.reject(new TypeError("navigator.mediaDevices is unavailable"));
  }
  return navigator.mediaDevices.getUserMedia({
    audio: {
      echoCancellation: true,
      noiseSuppression: choice.isNoiseSuppressionOn,
      autoGainControl: true,
      channelCount: 1,
      ...(choice.deviceId === null ? {} : { deviceId: { exact: choice.deviceId } }),
    },
    video: false,
  });
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

/** Voices above 100 % go through one shared Web Audio graph, played by one element routed like the others. */
let amplifiedPlayback: { context: AudioContext; destination: MediaStreamAudioDestinationNode; element: HTMLAudioElement } | null = null;

function amplifiedPlaybackGraph(deviceId: string | null) {
  if (amplifiedPlayback === null) {
    const context = new AudioContext();
    const destination = context.createMediaStreamDestination();
    const element = new Audio();
    element.srcObject = destination.stream;
    element.play().catch((error: unknown) => console.warn("couldn't play amplified voices", error));
    amplifiedPlayback = { context, destination, element };
  }
  if (amplifiedPlayback.context.state === "suspended") {
    amplifiedPlayback.context.resume().catch((error: unknown) => console.warn("couldn't start amplified voices", error));
  }
  routeTo(amplifiedPlayback.element, deviceId);
  return amplifiedPlayback;
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
  return {
    setGain: (gain) => {
      if (gain <= 1 || typeof AudioContext === "undefined") {
        amplified?.source.disconnect();
        amplified?.gain.disconnect();
        amplified = null;
        audio.muted = gain === 0;
        audio.volume = Math.min(1, gain);
        return;
      }
      if (amplified === null) {
        const graph = amplifiedPlaybackGraph(deviceId);
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
      amplified?.source.disconnect();
      amplified?.gain.disconnect();
      amplified = null;
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
  playTrack,
  meterLevel,
  amplify,
  listDevices: listAudioDevices,
  watchDevices: watchAudioDevices,
  storage: browserStorage,
};
