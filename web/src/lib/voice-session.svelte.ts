// This tab's voice connection (ADR-0030): one RTCPeerConnection to the instance's SFU, signalled
// over the gateway. One send m-line, then `VOICE_RECEIVE_SLOTS` receive m-lines the server fills
// with whoever speaks, so people coming and going never renegotiate. Non-trickle: the offer leaves
// once ICE gathering is complete. The browser objects (peer, tracks, audio outputs) live in plain
// fields, out of Svelte's proxies; only what the interface shows is reactive.

import { type ClientFrame, type Snowflake, VOICE_RECEIVE_SLOTS, type VoiceDisconnectReason, type VoiceJoinRefusal } from "./protocol";
import { type MicrophoneProblem, microphoneProblemOf, roundTripMillisecondsOf, waitForIceGathering } from "./voice";
import { type VoiceToggles, readVoiceToggles, toggleDeafened, toggleMicrophone, writeVoiceToggles } from "./voice-toggles";

/** ICE gathering usually completes in a few milliseconds against an ICE-lite server with no STUN. */
const ICE_GATHERING_TIMEOUT_MILLISECONDS = 1_000;
/** From the offer sent to audio flowing: past it, the instance did not answer or its UDP port is unreachable. */
const MEDIA_CONNECT_TIMEOUT_MILLISECONDS = 15_000;
/** ICE `disconnected` often heals by itself; past this it counts as lost. */
const ICE_DISCONNECTED_GRACE_MILLISECONDS = 3_000;
/** Silent attempts in a row before the panel says the connection is lost. */
const MAXIMUM_RECONNECT_ATTEMPTS = 5;
/**
 * The server dropped this user's voice state (a resume after a long break): a fresh offer follows
 * unless `VOICE_DISCONNECTED` says why meanwhile, whatever order the two frames come in.
 */
const OWN_STATE_DROPPED_GRACE_MILLISECONDS = 1_000;
const ROUND_TRIP_SAMPLE_INTERVAL_MILLISECONDS = 2_000;
const LEVEL_SAMPLE_INTERVAL_MILLISECONDS = 100;
/** RMS of the microphone's samples (0 to 1) above which you count as speaking. */
const SPEAKING_LEVEL_THRESHOLD = 0.02;
/** Short pauses between words keep the ring on. */
const SPEAKING_HOLD_MILLISECONDS = 300;

export type VoicePhase = "idle" | "connecting" | "connected" | "reconnecting";

/** Why voice stopped or could not start; shown in the voice panel until dismissed or retried. */
export type VoiceProblem =
  | { kind: "unsupported" }
  | { kind: "refused"; channelId: Snowflake; reason: VoiceJoinRefusal }
  | { kind: "disconnected"; channelId: Snowflake; reason: VoiceDisconnectReason }
  /** The instance never answered the offer. */
  | { kind: "no-answer"; channelId: Snowflake }
  /** Answered, but audio never flowed: usually the instance's UDP port is closed. */
  | { kind: "unreachable"; channelId: Snowflake }
  /** Connected, then lost, and the silent attempts did not bring it back. */
  | { kind: "connection-lost"; channelId: Snowflake };

export type VoiceCue = "self-join" | "self-leave" | "peer-join" | "peer-leave" | "mute" | "unmute" | "deafen" | "undeafen";

/** One remote voice, playing. */
export interface AudioOutput {
  setMuted(isMuted: boolean): void;
  stop(): void;
}

/** The microphone's loudness, from an `AnalyserNode`. */
export interface LevelMeter {
  /** RMS of the latest samples, 0 to 1. */
  level(): number;
  stop(): void;
}

/** What the session needs from the browser and the gateway; tests give fakes. */
export interface VoicePorts {
  /** Whether it left: false while the gateway is not connected. */
  sendFrame(frame: ClientFrame): boolean;
  /** `null` where the browser has no WebRTC. */
  createPeerConnection(): RTCPeerConnection | null;
  requestMicrophone(): Promise<MediaStream>;
  playTrack(track: MediaStreamTrack): AudioOutput;
  meterLevel(stream: MediaStream): LevelMeter | null;
  storage(): Pick<Storage, "getItem" | "setItem"> | null;
}

export class VoiceSession {
  phase = $state<VoicePhase>("idle");
  /** The channel joined, or being joined. */
  channelId = $state<Snowflake | null>(null);
  problem = $state.raw<VoiceProblem | null>(null);
  /** Connected without a microphone: you hear, and show muted with this reason. */
  microphoneProblem = $state<MicrophoneProblem | null>(null);
  roundTripMilliseconds = $state<number | null>(null);
  toggles = $state.raw<VoiceToggles>(readVoiceToggles(null));
  /** From your own microphone's level, so the ring answers at once. */
  isSelfSpeaking = $state(false);
  /** Plays the join, leave and toggle sounds; set by the shell. */
  onCue: ((cue: VoiceCue) => void) | null = null;

  private peer: RTCPeerConnection | null = null;
  private sender: RTCRtpSender | null = null;
  private microphone: MediaStream | null = null;
  /** Mirrors `microphone !== null` for the interface: the stream itself stays out of Svelte's proxies. */
  private hasMicrophone = $state(false);
  private meter: LevelMeter | null = null;
  private outputs: AudioOutput[] = [];
  /** Bumped by each new offer and by stopping: older async steps see they are stale. */
  private negotiation = 0;
  private reconnectAttempts = 0;
  private hasConnectedOnce = false;
  private hasRetriedAfterMediaTimeout = false;
  private isAwaitingGateway = false;
  private lastLoudAt = Number.NEGATIVE_INFINITY;
  private connectTimer: ReturnType<typeof setTimeout> | null = null;
  private iceGraceTimer: ReturnType<typeof setTimeout> | null = null;
  private ownStateDroppedTimer: ReturnType<typeof setTimeout> | null = null;
  private roundTripTimer: ReturnType<typeof setInterval> | null = null;
  private levelTimer: ReturnType<typeof setInterval> | null = null;

  constructor(private readonly ports: VoicePorts) {
    this.toggles = readVoiceToggles(ports.storage());
  }

  /** Muted by choice, or connected with no microphone. */
  get isMicrophoneMuted(): boolean {
    return this.toggles.isMicrophoneMuted || (this.phase !== "idle" && !this.hasMicrophone);
  }

  /** From a click: the microphone permission is asked here, never on load. Joining another channel moves. */
  async join(channelId: Snowflake): Promise<void> {
    if (this.channelId === channelId && this.phase !== "idle") {
      return;
    }
    this.problem = null;
    this.channelId = channelId;
    this.phase = "connecting";
    this.reconnectAttempts = 0;
    this.hasConnectedOnce = false;
    this.hasRetriedAfterMediaTimeout = false;
    this.clearOwnStateDroppedTimer();
    if (this.microphone === null) {
      await this.acquireMicrophone();
      // Hung up, or joined elsewhere, while the browser asked.
      if (this.channelId !== channelId || !this.isActive()) {
        return;
      }
    }
    await this.negotiate();
  }

  leave(): void {
    if (this.phase === "idle") {
      return;
    }
    this.ports.sendFrame({ op: "voice_leave" });
    this.stop();
    this.onCue?.("self-leave");
  }

  /** Sign-out: the gateway closes too, so nothing is sent. */
  end(): void {
    this.stop();
    this.problem = null;
  }

  retry(): void {
    const problem = this.problem;
    if (problem !== null && "channelId" in problem) {
      void this.join(problem.channelId);
    }
  }

  dismissProblem(): void {
    this.problem = null;
  }

  async toggleMicrophone(): Promise<void> {
    const isMicrophoneMissing = this.isActive() && this.microphoneTrack() === undefined;
    // Shown muted only for want of a microphone: the click means "unmute", whatever the toggles say.
    const isOnlyMissing = isMicrophoneMissing && !this.toggles.isMicrophoneMuted;
    const next = isOnlyMissing ? this.toggles : toggleMicrophone(this.toggles);
    // Unmuting with no microphone asks for it again (this is a click), and stays muted if refused.
    if (!next.isMicrophoneMuted && isMicrophoneMissing) {
      await this.acquireMicrophone();
      const track = this.microphoneTrack();
      if (!track) {
        return;
      }
      await this.sender?.replaceTrack(track);
    }
    this.setToggles(next);
    this.onCue?.(next.isMicrophoneMuted ? "mute" : "unmute");
  }

  toggleDeafened(): void {
    const next = toggleDeafened(this.toggles);
    this.setToggles(next);
    this.onCue?.(next.isDeafened ? "deafen" : "undeafen");
  }

  // --- Signalling from the gateway -----------------------------------------------------------

  async applyAnswer(channelId: Snowflake, sdpAnswer: string): Promise<void> {
    const peer = this.peer;
    if (channelId !== this.channelId || peer === null || peer.signalingState !== "have-local-offer") {
      return;
    }
    const generation = this.negotiation;
    try {
      await peer.setRemoteDescription({ type: "answer", sdp: sdpAnswer });
    } catch (error) {
      if (generation === this.negotiation) {
        console.warn("the instance's voice answer was refused", error);
        this.connectionFailed();
      }
    }
  }

  refuse(channelId: Snowflake, reason: VoiceJoinRefusal): void {
    if (channelId !== this.channelId || this.phase === "idle") {
      return;
    }
    // A refused move may leave the old connection on the instance: it ends too.
    this.ports.sendFrame({ op: "voice_leave" });
    this.stop();
    this.problem = { kind: "refused", channelId, reason };
  }

  disconnectedByServer(channelId: Snowflake, reason: VoiceDisconnectReason): void {
    if (channelId !== this.channelId || this.phase === "idle") {
      return;
    }
    this.clearOwnStateDroppedTimer();
    if (reason === "media_timeout" && !this.hasRetriedAfterMediaTimeout) {
      this.hasRetriedAfterMediaTimeout = true;
      this.rejoin();
      return;
    }
    this.stop();
    this.problem = { kind: "disconnected", channelId, reason };
    this.onCue?.("self-leave");
  }

  /** The instance no longer lists you in the channel you are in: a fresh offer follows. */
  noteOwnStateDropped(channelId: Snowflake): void {
    if (channelId !== this.channelId || this.phase === "idle" || this.ownStateDroppedTimer !== null) {
      return;
    }
    this.ownStateDroppedTimer = setTimeout(() => {
      this.ownStateDroppedTimer = null;
      this.rejoin();
    }, OWN_STATE_DROPPED_GRACE_MILLISECONDS);
  }

  /** A new gateway session knows nothing of the old one's voice: offer again. */
  noteGatewayReady(): void {
    this.rejoin();
  }

  /** A resumed session kept its voice if audio still flows; otherwise offer again. */
  noteGatewayResumed(): void {
    if (this.isAwaitingGateway || !this.isMediaConnected()) {
      this.rejoin();
    }
  }

  /** The browser is back online: audio over the old network path will not come back. */
  noteBackOnline(): void {
    if (!this.isMediaConnected()) {
      this.rejoin();
    }
  }

  // --- Inside ----------------------------------------------------------------------------------

  /** Methods rather than inline reads: these change across awaits, which TypeScript's narrowing ignores. */
  private isActive(): boolean {
    return this.phase !== "idle";
  }

  private microphoneTrack(): MediaStreamTrack | undefined {
    return this.microphone?.getAudioTracks()[0];
  }

  private isMediaConnected(): boolean {
    const state = this.peer?.iceConnectionState;
    return state === "connected" || state === "completed";
  }

  private async acquireMicrophone(): Promise<void> {
    try {
      const stream = await this.ports.requestMicrophone();
      if (this.phase === "idle") {
        stream.getTracks().forEach((track) => track.stop());
        return;
      }
      this.microphone = stream;
      this.hasMicrophone = true;
      this.microphoneProblem = null;
      this.applyToggles();
      this.meter?.stop();
      this.meter = this.ports.meterLevel(stream);
    } catch (error) {
      this.microphoneProblem = microphoneProblemOf(error);
    }
  }

  private rejoin(): void {
    if (this.phase === "idle") {
      return;
    }
    this.phase = this.hasConnectedOnce ? "reconnecting" : "connecting";
    void this.negotiate();
  }

  /** A fresh peer connection and offer for the current channel; the server swaps the media session. */
  private async negotiate(): Promise<void> {
    const channelId = this.channelId;
    if (channelId === null) {
      return;
    }
    this.negotiation += 1;
    const generation = this.negotiation;
    this.closePeer();
    const peer = this.ports.createPeerConnection();
    if (peer === null) {
      this.stop();
      this.problem = { kind: "unsupported" };
      return;
    }
    this.peer = peer;
    const track = this.microphoneTrack() ?? null;
    this.sender = peer.addTransceiver(track ?? "audio", { direction: "sendonly" }).sender;
    for (let slot = 0; slot < VOICE_RECEIVE_SLOTS; slot += 1) {
      peer.addTransceiver("audio", { direction: "recvonly" });
    }
    peer.addEventListener("track", (event) => {
      if (generation === this.negotiation) {
        const output = this.ports.playTrack(event.track);
        output.setMuted(this.toggles.isDeafened);
        this.outputs.push(output);
      }
    });
    peer.addEventListener("iceconnectionstatechange", () => this.iceStateChanged(peer));
    this.applyToggles();
    try {
      await peer.setLocalDescription(await peer.createOffer());
      await waitForIceGathering(peer, ICE_GATHERING_TIMEOUT_MILLISECONDS);
    } catch (error) {
      if (generation === this.negotiation) {
        console.warn("couldn't prepare the voice offer", error);
        this.connectionFailed();
      }
      return;
    }
    const offer = peer.localDescription?.sdp;
    if (generation !== this.negotiation || offer === undefined) {
      return;
    }
    const isSent = this.ports.sendFrame({
      op: "voice_join",
      d: {
        channel_id: channelId,
        sdp_offer: offer,
        self_mute: this.isMicrophoneMuted,
        self_deaf: this.toggles.isDeafened,
      },
    });
    // The gateway is down: the next `ready` or `resumed` offers again.
    this.isAwaitingGateway = !isSent;
    if (isSent) {
      this.clearTimer("connectTimer");
      this.connectTimer = setTimeout(() => {
        this.connectTimer = null;
        this.connectionFailed();
      }, MEDIA_CONNECT_TIMEOUT_MILLISECONDS);
    }
  }

  private iceStateChanged(peer: RTCPeerConnection): void {
    if (peer !== this.peer) {
      return;
    }
    switch (peer.iceConnectionState) {
      case "connected":
      case "completed":
        this.clearTimer("iceGraceTimer");
        this.mediaConnected();
        return;
      case "disconnected":
        this.iceGraceTimer ??= setTimeout(() => {
          this.iceGraceTimer = null;
          this.connectionFailed();
        }, ICE_DISCONNECTED_GRACE_MILLISECONDS);
        return;
      case "failed":
        this.clearTimer("iceGraceTimer");
        this.connectionFailed();
        return;
      default:
        return;
    }
  }

  private mediaConnected(): void {
    this.clearTimer("connectTimer");
    if (this.phase === "connected" || this.phase === "idle") {
      return;
    }
    this.phase = "connected";
    this.reconnectAttempts = 0;
    if (!this.hasConnectedOnce) {
      this.hasConnectedOnce = true;
      this.onCue?.("self-join");
    }
    this.startSampling();
  }

  /** No answer, no audio, or audio lost: a few silent attempts, then the panel says so. */
  private connectionFailed(): void {
    const channelId = this.channelId;
    if (this.phase === "idle" || channelId === null) {
      return;
    }
    this.clearTimer("connectTimer");
    const hasAnswer = this.peer?.signalingState === "stable";
    // A first join that fails is not retried in silence: the reason is usually lasting.
    if (!this.hasConnectedOnce || this.reconnectAttempts >= MAXIMUM_RECONNECT_ATTEMPTS) {
      this.ports.sendFrame({ op: "voice_leave" });
      const wasConnected = this.hasConnectedOnce;
      this.stop();
      this.problem = wasConnected
        ? { kind: "connection-lost", channelId }
        : { kind: hasAnswer ? "unreachable" : "no-answer", channelId };
      return;
    }
    this.reconnectAttempts += 1;
    this.rejoin();
  }

  private setToggles(toggles: VoiceToggles): void {
    this.toggles = toggles;
    writeVoiceToggles(this.ports.storage(), toggles);
    this.applyToggles();
    if (this.phase !== "idle") {
      this.ports.sendFrame({
        op: "voice_state",
        d: { self_mute: this.isMicrophoneMuted, self_deaf: toggles.isDeafened },
      });
    }
  }

  /** Muting stops sending at once; the server's `SPEAK` check is the enforcement, not this. */
  private applyToggles(): void {
    for (const track of this.microphone?.getAudioTracks() ?? []) {
      track.enabled = !this.toggles.isMicrophoneMuted;
    }
    for (const output of this.outputs) {
      output.setMuted(this.toggles.isDeafened);
    }
  }

  private startSampling(): void {
    this.roundTripTimer ??= setInterval(() => void this.sampleRoundTrip(), ROUND_TRIP_SAMPLE_INTERVAL_MILLISECONDS);
    this.levelTimer ??= setInterval(() => this.sampleLevel(), LEVEL_SAMPLE_INTERVAL_MILLISECONDS);
    void this.sampleRoundTrip();
  }

  private async sampleRoundTrip(): Promise<void> {
    const peer = this.peer;
    if (peer === null) {
      return;
    }
    try {
      const report = await peer.getStats();
      if (peer === this.peer) {
        this.roundTripMilliseconds = roundTripMillisecondsOf(report.values()) ?? this.roundTripMilliseconds;
      }
    } catch (error) {
      console.warn("couldn't read the voice statistics", error);
    }
  }

  private sampleLevel(): void {
    const now = Date.now();
    if (!this.isMicrophoneMuted && (this.meter?.level() ?? 0) >= SPEAKING_LEVEL_THRESHOLD) {
      this.lastLoudAt = now;
    }
    const isSpeaking = this.phase === "connected" && now - this.lastLoudAt < SPEAKING_HOLD_MILLISECONDS;
    if (isSpeaking !== this.isSelfSpeaking) {
      this.isSelfSpeaking = isSpeaking;
    }
  }

  private closePeer(): void {
    this.clearTimer("connectTimer");
    this.clearTimer("iceGraceTimer");
    this.peer?.close();
    this.peer = null;
    this.sender = null;
    for (const output of this.outputs) {
      output.stop();
    }
    this.outputs = [];
  }

  /** Everything local ends; the toggles stay, as on Discord. */
  private stop(): void {
    this.negotiation += 1;
    this.closePeer();
    this.clearOwnStateDroppedTimer();
    this.clearTimer("roundTripTimer");
    this.clearTimer("levelTimer");
    this.meter?.stop();
    this.meter = null;
    this.microphone?.getTracks().forEach((track) => track.stop());
    this.microphone = null;
    this.hasMicrophone = false;
    this.microphoneProblem = null;
    this.isAwaitingGateway = false;
    this.isSelfSpeaking = false;
    this.lastLoudAt = Number.NEGATIVE_INFINITY;
    this.roundTripMilliseconds = null;
    this.channelId = null;
    this.phase = "idle";
  }

  private clearOwnStateDroppedTimer(): void {
    this.clearTimer("ownStateDroppedTimer");
  }

  private clearTimer(name: "connectTimer" | "iceGraceTimer" | "ownStateDroppedTimer" | "roundTripTimer" | "levelTimer"): void {
    const timer = this[name];
    if (timer !== null) {
      clearTimeout(timer);
      clearInterval(timer);
      this[name] = null;
    }
  }
}
