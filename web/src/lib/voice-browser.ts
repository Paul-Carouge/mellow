// The browser side of `VoicePorts`: WebRTC, the microphone, playback and the level meter. Thin on
// purpose: the rules live in `voice-session.svelte.ts`, tested with fakes. Only `AnalyserNode` is
// used for the meter: an AudioWorklet would need `worker-src`, which the policy keeps at 'none'.

import { browserStorage } from "./reactions";
import type { AudioOutput, LevelMeter, VoicePorts } from "./voice-session.svelte";

/** What Discord asks for: the browser's echo cancellation, noise suppression and gain, one channel. */
const MICROPHONE_CONSTRAINTS: MediaStreamConstraints = {
  audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true, channelCount: 1 },
  video: false,
};
const ANALYSER_FFT_SIZE = 512;

function createPeerConnection(): RTCPeerConnection | null {
  if (typeof RTCPeerConnection === "undefined") {
    return null;
  }
  // No STUN or TURN: the instance is ICE-lite with its public address, and the browser dials it (ADR-0030).
  return new RTCPeerConnection({ bundlePolicy: "max-bundle", rtcpMuxPolicy: "require" });
}

function requestMicrophone(): Promise<MediaStream> {
  // Absent outside a secure context: reported as "unavailable".
  if (!navigator.mediaDevices?.getUserMedia) {
    return Promise.reject(new TypeError("navigator.mediaDevices is unavailable"));
  }
  return navigator.mediaDevices.getUserMedia(MICROPHONE_CONSTRAINTS);
}

/** An `<audio>` element per voice: browsers only play remote WebRTC audio through a media element. */
function playTrack(track: MediaStreamTrack): AudioOutput {
  const audio = new Audio();
  audio.autoplay = true;
  audio.srcObject = new MediaStream([track]);
  audio.play().catch((error: unknown) => console.warn("couldn't play a voice", error));
  return {
    setMuted: (isMuted) => {
      audio.muted = isMuted;
    },
    stop: () => {
      audio.pause();
      audio.srcObject = null;
    },
  };
}

function meterLevel(stream: MediaStream): LevelMeter | null {
  if (typeof AudioContext === "undefined") {
    return null;
  }
  const context = new AudioContext();
  const analyser = context.createAnalyser();
  analyser.fftSize = ANALYSER_FFT_SIZE;
  context.createMediaStreamSource(stream).connect(analyser);
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
      context.close().catch((error: unknown) => console.warn("couldn't close the level meter", error));
    },
  };
}

export const BROWSER_VOICE_PORTS: Omit<VoicePorts, "sendFrame"> = {
  createPeerConnection,
  requestMicrophone,
  playTrack,
  meterLevel,
  storage: browserStorage,
};
