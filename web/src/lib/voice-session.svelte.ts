// This tab's voice connection (ADR-0030): one RTCPeerConnection to the instance's SFU, signalled
// over the gateway. One send m-line, then `VOICE_RECEIVE_SLOTS` receive m-lines the server fills
// with whoever speaks, so people coming and going never renegotiate. Non-trickle: the offer leaves
// once ICE gathering is complete. The browser objects (peer, tracks, audio outputs) live in plain
// fields, out of Svelte's proxies; only what the interface shows is reactive.

import { type ClientFrame, type Snowflake, VOICE_RECEIVE_SLOTS, type VoiceDisconnectReason, type VoiceJoinRefusal } from "./protocol";
import { type MicrophoneProblem, microphoneProblemOf, roundTripMillisecondsOf, waitForIceGathering } from "./voice";
import {
  type MicrophoneChoice,
  type MicrophoneProcessing,
  type NoiseSuppressorProblem,
  isMicrophoneReopenNeeded,
  microphoneChoiceOf,
  microphoneProcessingOf,
  processingStagesOf,
} from "./voice-processing";
import {
  type AudioDevice,
  DEFAULT_PERSON_VOICE_SETTING,
  type EqualizerDecibels,
  type PersonVoiceSetting,
  type VoiceSettings,
  decibelsOfLevel,
  isRememberedDevicePresent,
  playbackGain,
  readPersonVoiceSettings,
  readVoiceSettings,
  writePersonVoiceSettings,
  writeVoiceSettings,
} from "./voice-settings";
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
/** Often enough for the manual sensitivity gate to open on the first syllable. */
const LEVEL_SAMPLE_INTERVAL_MILLISECONDS = 20;
/** RMS of the microphone's samples (0 to 1) above which you count as speaking. */
const SPEAKING_LEVEL_THRESHOLD = 0.02;
/** Short pauses between words keep the ring on. */
const SPEAKING_HOLD_MILLISECONDS = 300;
/** The manual sensitivity gate stays open this long after the level falls under the threshold. */
const SENSITIVITY_GATE_HOLD_MILLISECONDS = 250;

/**
 * shortcut: hidden tabs throttle timers, so the manual gate stays open there (Opus DTX and the
 * server's speaking detection handle silence); upgrade when an AudioWorklet is allowed by the CSP.
 */
function isPageHidden(): boolean {
  return typeof document !== "undefined" && document.visibilityState === "hidden";
}

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

export type VoiceCue =
  | "self-join"
  | "self-leave"
  | "peer-join"
  | "peer-leave"
  | "mute"
  | "unmute"
  | "deafen"
  | "undeafen"
  | "push-to-talk-press"
  | "push-to-talk-release";

/** One receive slot's voice, playing. */
export interface AudioOutput {
  /** 0 is silent, 1 as received, up to 4 (200 % of 200 %). */
  setGain(gain: number): void;
  /** `null`: the system's default output. Ignored where the browser cannot choose. */
  setOutputDevice(deviceId: string | null): void;
  stop(): void;
}

/** The microphone's loudness, from an `AnalyserNode` on a copy of its track. */
export interface LevelMeter {
  /** RMS of the latest samples, 0 to 1. */
  level(): number;
  stop(): void;
}

/** The microphone through a gain, for an input volume other than 100 %. */
export interface InputAmplifier {
  /** What is sent instead of the microphone's own track. */
  readonly track: MediaStreamTrack;
  setGain(gain: number): void;
  stop(): void;
}

export type { MicrophoneChoice } from "./voice-processing";

/** The microphone through the processing chain (ADR-0031): RNNoise, equaliser, compressor, limiter. */
export interface MicrophoneProcessor {
  /** What is metered, gated and sent instead of the microphone's own track. */
  readonly track: MediaStreamTrack;
  /** The sliders, live. */
  setEqualizer(equalizer: EqualizerDecibels): void;
  stop(): void;
}

/** A microphone as opened: the browser's stream, and the processing on it when any is on. */
interface OpenedMicrophone {
  readonly stream: MediaStream;
  readonly processor: MicrophoneProcessor | null;
}

function closeOpenedMicrophone(opened: OpenedMicrophone): void {
  opened.processor?.stop();
  opened.stream.getTracks().forEach((track) => track.stop());
}

/** What the session needs from the browser and the gateway; tests give fakes. */
export interface VoicePorts {
  /** Whether it left: false while the gateway is not connected. */
  sendFrame(frame: ClientFrame): boolean;
  /** `null` where the browser has no WebRTC. */
  createPeerConnection(): RTCPeerConnection | null;
  requestMicrophone(choice: MicrophoneChoice): Promise<MediaStream>;
  /** Loads RNNoise once; `null` when it is ready, else why it cannot run here. */
  prepareNoiseSuppressor(): Promise<NoiseSuppressorProblem | null>;
  /**
   * Builds the chain on `track`. `null` where the browser has no Web Audio (the track is sent as
   * it is); rejects when RNNoise, though prepared, cannot start.
   */
  processMicrophone(track: MediaStreamTrack, processing: MicrophoneProcessing): Promise<MicrophoneProcessor | null>;
  playTrack(track: MediaStreamTrack): AudioOutput;
  meterLevel(track: MediaStreamTrack): LevelMeter | null;
  /** `null` where the browser has no Web Audio: the microphone is then sent at 100 %. */
  amplify(track: MediaStreamTrack, gain: number): InputAmplifier | null;
  listDevices(): Promise<readonly AudioDevice[]>;
  /** Calls `onChange` when a device is plugged or unplugged; returns how to stop. */
  watchDevices(onChange: () => void): () => void;
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
  /** From `ready`: false when the instance has no voice, so joining fails at once without asking for the microphone. */
  isVoiceEnabledOnInstance = $state(true);
  settings = $state.raw<VoiceSettings>(readVoiceSettings(null));
  /** Volume and local mute per person, for you only. */
  personSettings = $state.raw<ReadonlyMap<Snowflake, PersonVoiceSetting>>(new Map());
  /** The remembered microphone is unplugged: the default one is used meanwhile. */
  isInputDeviceMissing = $state(false);
  /** The remembered output is unplugged: the default one plays meanwhile. */
  isOutputDeviceMissing = $state(false);
  isMicrophoneTestRunning = $state(false);
  microphoneTestProblem = $state<MicrophoneProblem | null>(null);
  /** Why "advanced" noise suppression fell back to the browser's; forgotten when the choice changes. */
  noiseSuppressorProblem = $state<NoiseSuppressorProblem | null>(null);
  /** "S'écouter": the processed microphone plays back, from the settings page only. */
  isMonitoring = $state(false);
  /** Plays the join, leave and toggle sounds; set by the shell. */
  onCue: ((cue: VoiceCue) => void) | null = null;

  private peer: RTCPeerConnection | null = null;
  private sender: RTCRtpSender | null = null;
  private microphone: MediaStream | null = null;
  /** Mirrors `microphone !== null` for the interface: the stream itself stays out of Svelte's proxies. */
  private hasMicrophone = $state(false);
  private processor: MicrophoneProcessor | null = null;
  private meter: LevelMeter | null = null;
  private amplifier: InputAmplifier | null = null;
  /** Bumped by each microphone switch: a slower earlier one sees it is stale. */
  private microphoneSwitch = 0;
  /** Bumped by each test start and stop: a slower earlier start sees it is stale and frees its stream. */
  private microphoneTestAttempt = 0;
  /** Each receive slot's voice, by the SDP mid of its transceiver. */
  private outputs: { mid: string | null; output: AudioOutput }[] = [];
  /** Who each receive slot carries now (`VOICE_SLOT_ASSIGNED`), for the current media session only. */
  private slotUserIds = new Map<string, Snowflake>();
  private isPushToTalkOpen = false;
  private lastAboveThresholdAt = Number.NEGATIVE_INFINITY;
  private stopWatchingDevices: (() => void) | null = null;
  private testMicrophone: MediaStream | null = null;
  private testProcessor: MicrophoneProcessor | null = null;
  private testMeter: LevelMeter | null = null;
  /** The track "S'écouter" plays, and where. */
  private monitor: { track: MediaStreamTrack; output: AudioOutput } | null = null;
  /** Bumped by each new offer and by stopping: older async steps see they are stale. */
  private negotiation = 0;
  private reconnectAttempts = 0;
  private hasConnectedOnce = false;
  private hasRetriedAfterMediaTimeout = false;
  private isAwaitingGateway = false;
  /** A `voice_state` could not leave (the gateway was down): sent again once it is back. */
  private hasUnsentVoiceState = false;
  private lastLoudAt = Number.NEGATIVE_INFINITY;
  private connectTimer: ReturnType<typeof setTimeout> | null = null;
  private iceGraceTimer: ReturnType<typeof setTimeout> | null = null;
  private ownStateDroppedTimer: ReturnType<typeof setTimeout> | null = null;
  private roundTripTimer: ReturnType<typeof setInterval> | null = null;
  private levelTimer: ReturnType<typeof setInterval> | null = null;
  private pushToTalkReleaseTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(private readonly ports: VoicePorts) {
    const storage = ports.storage();
    this.toggles = readVoiceToggles(storage);
    this.settings = readVoiceSettings(storage);
    this.personSettings = readPersonVoiceSettings(storage);
  }

  /** Muted by choice, or connected with no microphone. */
  get isMicrophoneMuted(): boolean {
    return this.toggles.isMicrophoneMuted || (this.phase !== "idle" && !this.hasMicrophone);
  }

  /** Connected with a working microphone: the settings' meter reads it rather than opening another. */
  get isMicrophoneLive(): boolean {
    return this.phase !== "idle" && this.hasMicrophone;
  }

  /** From a click: the microphone permission is asked here, never on load. Joining another channel moves. */
  async join(channelId: Snowflake): Promise<void> {
    if (this.channelId === channelId && this.phase !== "idle") {
      return;
    }
    if (!this.isVoiceEnabledOnInstance) {
      this.problem = { kind: "refused", channelId, reason: "voice_unavailable" };
      return;
    }
    this.problem = null;
    this.channelId = channelId;
    this.phase = "connecting";
    this.reconnectAttempts = 0;
    this.hasConnectedOnce = false;
    this.hasRetriedAfterMediaTimeout = false;
    this.clearOwnStateDroppedTimer();
    this.stopWatchingDevices ??= this.ports.watchDevices(() => void this.checkDevices());
    void this.checkDevices();
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
      const track = this.sentTrack();
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

  // --- Settings ------------------------------------------------------------------------------

  /** Stores the settings and applies them at once, during a call too (no renegotiation). */
  updateSettings(next: VoiceSettings): void {
    const previous = this.settings;
    this.settings = next;
    writeVoiceSettings(this.ports.storage(), next);
    if (next.inputDeviceId !== previous.inputDeviceId) {
      this.isInputDeviceMissing = false;
    }
    if (next.noiseSuppression !== previous.noiseSuppression) {
      // Tried again on the next microphone: a failed load may have been a network hiccup.
      this.noiseSuppressorProblem = null;
    }
    if (isMicrophoneReopenNeeded(previous, next)) {
      void this.switchMicrophone();
      void this.restartMicrophoneTest();
    } else {
      if (next.equalizerDecibels !== previous.equalizerDecibels) {
        this.processor?.setEqualizer(next.equalizerDecibels);
        this.testProcessor?.setEqualizer(next.equalizerDecibels);
      }
      if (next.inputVolumePercent !== previous.inputVolumePercent && this.updateAmplifier()) {
        void this.sender?.replaceTrack(this.sentTrack() ?? null);
      }
    }
    if (next.inputMode !== previous.inputMode || next.pushToTalkKey?.code !== previous.pushToTalkKey?.code) {
      this.clearTimer("pushToTalkReleaseTimer");
      this.isPushToTalkOpen = false;
    }
    if (next.outputDeviceId !== previous.outputDeviceId) {
      this.isOutputDeviceMissing = false;
      this.applyOutputDevice();
      void this.checkDevices();
    }
    this.applyTransmission();
    this.applyGains();
    this.refreshMonitor();
  }

  personSetting(userId: Snowflake): PersonVoiceSetting {
    return this.personSettings.get(userId) ?? DEFAULT_PERSON_VOICE_SETTING;
  }

  setPersonVolume(userId: Snowflake, volumePercent: number): void {
    this.setPersonSetting(userId, { ...this.personSetting(userId), volumePercent });
  }

  togglePersonMuted(userId: Snowflake): void {
    const current = this.personSetting(userId);
    this.setPersonSetting(userId, { ...current, isLocallyMuted: !current.isLocallyMuted });
  }

  /** The push-to-talk key went down (repeats included): the microphone opens while connected. */
  pressPushToTalk(): void {
    if (this.settings.inputMode !== "push-to-talk" || this.phase === "idle") {
      return;
    }
    this.clearTimer("pushToTalkReleaseTimer");
    if (this.isPushToTalkOpen) {
      return;
    }
    this.isPushToTalkOpen = true;
    this.applyTransmission();
    if (this.settings.arePushToTalkSoundsOn && !this.isMicrophoneMuted) {
      this.onCue?.("push-to-talk-press");
    }
  }

  /** The key came up, or the window lost focus with it held: closes after the release delay. */
  releasePushToTalk(): void {
    if (!this.isPushToTalkOpen || this.pushToTalkReleaseTimer !== null) {
      return;
    }
    if (this.settings.arePushToTalkSoundsOn && !this.isMicrophoneMuted) {
      this.onCue?.("push-to-talk-release");
    }
    const close = () => {
      this.pushToTalkReleaseTimer = null;
      this.isPushToTalkOpen = false;
      this.applyTransmission();
    };
    if (this.settings.pushToTalkReleaseDelayMilliseconds === 0) {
      close();
    } else {
      this.pushToTalkReleaseTimer = setTimeout(close, this.settings.pushToTalkReleaseDelayMilliseconds);
    }
  }

  /** Opens the microphone for the settings' meter, from a click; outside a call only. */
  async startMicrophoneTest(): Promise<void> {
    if (this.isMicrophoneTestRunning) {
      return;
    }
    this.isMicrophoneTestRunning = true;
    this.microphoneTestProblem = null;
    this.microphoneTestAttempt += 1;
    const attempt = this.microphoneTestAttempt;
    try {
      const opened = await this.openMicrophone();
      if (attempt !== this.microphoneTestAttempt || !this.isMicrophoneTestRunning) {
        closeOpenedMicrophone(opened);
        return;
      }
      this.testMicrophone = opened.stream;
      this.testProcessor = opened.processor;
      const track = this.testTrack();
      this.testMeter = track ? this.ports.meterLevel(track) : null;
      this.refreshMonitor();
    } catch (error) {
      if (attempt === this.microphoneTestAttempt) {
        this.microphoneTestProblem = microphoneProblemOf(error);
        this.isMicrophoneTestRunning = false;
      }
    }
  }

  /** Leaving the settings page stops the test, frees the microphone and stops listening to yourself. */
  stopMicrophoneTest(): void {
    this.isMonitoring = false;
    this.closeMicrophoneTest();
  }

  /**
   * "S'écouter": plays what others would hear, at your input volume and on your output. Outside a
   * call it starts the microphone test, since this is a click.
   */
  setMonitoring(isOn: boolean): void {
    this.isMonitoring = isOn;
    if (isOn && !this.isMicrophoneLive && !this.isMicrophoneTestRunning) {
      void this.startMicrophoneTest();
    }
    this.refreshMonitor();
  }

  /** Asks for the microphone only so the browser names the devices; nothing keeps listening. */
  async allowMicrophone(): Promise<MicrophoneProblem | null> {
    try {
      const stream = await this.ports.requestMicrophone(microphoneChoiceOf(this.settings, null, false));
      stream.getTracks().forEach((track) => track.stop());
      return null;
    } catch (error) {
      return microphoneProblemOf(error);
    }
  }

  /** The level the settings' meter shows, in dB after the input volume; `null` while nothing listens. */
  microphoneDecibels(): number | null {
    const meter = this.testMeter ?? (this.isMicrophoneLive ? this.meter : null);
    return meter === null ? null : decibelsOfLevel(meter.level() * this.inputGain());
  }

  // --- Signalling from the gateway -----------------------------------------------------------

  /**
   * The id of the latest offer: answers and refusals of older offers (sent before a reconnection or
   * a move) are ignored, since they do not match this peer connection.
   */
  get currentOfferId(): number {
    return this.negotiation;
  }

  async applyAnswer(channelId: Snowflake, offerId: number, sdpAnswer: string): Promise<void> {
    const peer = this.peer;
    if (
      channelId !== this.channelId ||
      offerId !== this.currentOfferId ||
      peer === null ||
      peer.signalingState !== "have-local-offer"
    ) {
      return;
    }
    const generation = this.negotiation;
    // A new media session: the slots' speakers of the old one mean nothing any more.
    this.slotUserIds.clear();
    this.applyGains();
    try {
      await peer.setRemoteDescription({ type: "answer", sdp: sdpAnswer });
    } catch (error) {
      if (generation === this.negotiation) {
        console.warn("the instance's voice answer was refused", error);
        this.connectionFailed();
      }
    }
  }

  refuse(channelId: Snowflake, offerId: number, reason: VoiceJoinRefusal): void {
    if (channelId !== this.channelId || offerId !== this.currentOfferId || this.phase === "idle") {
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

  /** The instance now relays `userId` on the receive slot whose transceiver has `mid`. */
  assignSlot(channelId: Snowflake, mid: string, userId: Snowflake): void {
    if (channelId !== this.channelId || this.phase === "idle") {
      return;
    }
    this.slotUserIds.set(mid, userId);
    this.applyGains();
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
      // The offer carries the current mute and deafen.
      this.rejoin();
    } else if (this.hasUnsentVoiceState) {
      this.sendVoiceState();
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

  /** The processed microphone when processing is on, else the browser's track. */
  private microphoneTrack(): MediaStreamTrack | undefined {
    return this.processor?.track ?? this.microphone?.getAudioTracks()[0];
  }

  private testTrack(): MediaStreamTrack | undefined {
    return this.testProcessor?.track ?? this.testMicrophone?.getAudioTracks()[0];
  }

  /** What the send transceiver carries: the amplified microphone, or the microphone itself at 100 %. */
  private sentTrack(): MediaStreamTrack | undefined {
    return this.amplifier?.track ?? this.microphoneTrack();
  }

  private inputGain(): number {
    return this.settings.inputVolumePercent / 100;
  }

  private outputDeviceId(): string | null {
    return this.isOutputDeviceMissing ? null : this.settings.outputDeviceId;
  }

  /**
   * The microphone with its processing. RNNoise that cannot start falls back to the browser's
   * noise suppression, which has to be asked of `getUserMedia`: the microphone is opened again.
   */
  private async openMicrophone(): Promise<OpenedMicrophone> {
    const isNoiseSuppressorUsable = await this.isNoiseSuppressorUsable();
    const stream = await this.requestChosenMicrophone(isNoiseSuppressorUsable);
    const track = stream.getAudioTracks()[0];
    const processing = microphoneProcessingOf(this.settings, isNoiseSuppressorUsable);
    if (!track || processingStagesOf(processing).length === 0) {
      return { stream, processor: null };
    }
    try {
      return { stream, processor: await this.ports.processMicrophone(track, processing) };
    } catch (error) {
      if (!processing.isNoiseSuppressorOn) {
        console.warn("couldn't process the microphone, it is sent as it is", error);
        return { stream, processor: null };
      }
      console.warn("couldn't start the noise suppressor", error);
      stream.getTracks().forEach((stoppedTrack) => stoppedTrack.stop());
      this.noiseSuppressorProblem = "failed";
      return this.openMicrophone();
    }
  }

  /** Whether RNNoise is wanted and can run; loads it the first time. */
  private async isNoiseSuppressorUsable(): Promise<boolean> {
    if (this.settings.noiseSuppression !== "advanced" || this.noiseSuppressorProblem !== null) {
      return false;
    }
    const problem = await this.ports.prepareNoiseSuppressor();
    // The choice may have changed while it loaded: a later microphone asks again.
    if (this.settings.noiseSuppression === "advanced") {
      this.noiseSuppressorProblem = problem;
    }
    return problem === null;
  }

  /** The remembered microphone, or the default one when it is unplugged (said in the panel). */
  private async requestChosenMicrophone(isNoiseSuppressorUsable: boolean): Promise<MediaStream> {
    const { inputDeviceId } = this.settings;
    if (inputDeviceId !== null) {
      try {
        const stream = await this.ports.requestMicrophone(microphoneChoiceOf(this.settings, inputDeviceId, isNoiseSuppressorUsable));
        this.isInputDeviceMissing = false;
        return stream;
      } catch (error) {
        if (microphoneProblemOf(error) !== "missing") {
          throw error;
        }
        this.isInputDeviceMissing = true;
      }
    }
    return this.ports.requestMicrophone(microphoneChoiceOf(this.settings, null, isNoiseSuppressorUsable));
  }

  /** Takes `opened` as the microphone, with its meter and amplifier; the previous one stops. */
  private installMicrophone(opened: OpenedMicrophone): void {
    this.releaseMicrophone();
    const { stream } = opened;
    this.microphone = stream;
    this.processor = opened.processor;
    this.hasMicrophone = true;
    this.microphoneProblem = null;
    // Unplugged mid-call: the device's track ends (never the processed one), and the default microphone takes over.
    stream.getAudioTracks()[0]?.addEventListener("ended", () => {
      if (stream === this.microphone) {
        this.isInputDeviceMissing = this.settings.inputDeviceId !== null;
        void this.switchMicrophone();
      }
    });
    // A slider may have moved while the chain was being built.
    if (this.settings.isVoiceEnhancementOn) {
      this.processor?.setEqualizer(this.settings.equalizerDecibels);
    }
    const track = this.microphoneTrack();
    this.meter = track ? this.ports.meterLevel(track) : null;
    this.updateAmplifier();
    this.applyTransmission();
    this.refreshMonitor();
  }

  private releaseMicrophone(): void {
    this.meter?.stop();
    this.meter = null;
    this.amplifier?.stop();
    this.amplifier = null;
    this.processor?.stop();
    this.processor = null;
    this.microphone?.getTracks().forEach((track) => track.stop());
    this.microphone = null;
    this.hasMicrophone = false;
    this.refreshMonitor();
  }

  /**
   * Another device or processing, during a call: a new track replaces the sent one, without
   * renegotiating. Also brings back a microphone that was unplugged (`missing`) once one is there.
   * Others are told when this mutes or unmutes you, as a toggle would.
   */
  private async switchMicrophone(): Promise<void> {
    if (this.phase === "idle" || (this.microphone === null && this.microphoneProblem !== "missing")) {
      return;
    }
    this.microphoneSwitch += 1;
    const attempt = this.microphoneSwitch;
    const wasMuted = this.isMicrophoneMuted;
    let opened: OpenedMicrophone;
    try {
      opened = await this.openMicrophone();
    } catch (error) {
      if (attempt === this.microphoneSwitch && this.isActive()) {
        this.releaseMicrophone();
        this.microphoneProblem = microphoneProblemOf(error);
        await this.sender?.replaceTrack(null);
        this.sendVoiceStateIfMuteChanged(wasMuted);
      }
      return;
    }
    if (attempt !== this.microphoneSwitch || !this.isActive()) {
      closeOpenedMicrophone(opened);
      return;
    }
    this.installMicrophone(opened);
    await this.sender?.replaceTrack(this.sentTrack() ?? null);
    this.sendVoiceStateIfMuteChanged(wasMuted);
  }

  private sendVoiceStateIfMuteChanged(wasMuted: boolean): void {
    if (this.isActive() && this.isMicrophoneMuted !== wasMuted) {
      this.sendVoiceState();
    }
  }

  /** The amplifier follows the input volume; true when the sent track changed. */
  private updateAmplifier(): boolean {
    const gain = this.inputGain();
    if (gain === 1) {
      if (this.amplifier === null) {
        return false;
      }
      this.amplifier.stop();
      this.amplifier = null;
      return true;
    }
    if (this.amplifier !== null) {
      this.amplifier.setGain(gain);
      return false;
    }
    const track = this.microphoneTrack();
    this.amplifier = track ? this.ports.amplify(track, gain) : null;
    return this.amplifier !== null;
  }

  /** Another microphone or processing: the test reopens, and "S'écouter" follows it. */
  private async restartMicrophoneTest(): Promise<void> {
    if (this.isMicrophoneTestRunning) {
      this.closeMicrophoneTest();
      await this.startMicrophoneTest();
    }
  }

  private closeMicrophoneTest(): void {
    this.microphoneTestAttempt += 1;
    this.isMicrophoneTestRunning = false;
    this.testMeter?.stop();
    this.testMeter = null;
    this.testProcessor?.stop();
    this.testProcessor = null;
    this.testMicrophone?.getTracks().forEach((track) => track.stop());
    this.testMicrophone = null;
    this.refreshMonitor();
  }

  /**
   * The test's microphone first, as the settings' meter reads it, at the input volume. In a call,
   * what is sent: it already carries the input volume, push-to-talk and the gate, so you hear
   * exactly what the others hear.
   */
  private monitoredTrack(): { track: MediaStreamTrack; gain: number } | null {
    if (this.testMicrophone !== null) {
      const track = this.testTrack();
      return track ? { track, gain: this.inputGain() } : null;
    }
    const track = this.isMicrophoneLive ? this.sentTrack() : undefined;
    return track ? { track, gain: 1 } : null;
  }

  /** Plays the test's or the call's processed microphone while "S'écouter" is on; nothing otherwise. */
  private refreshMonitor(): void {
    const monitored = this.isMonitoring ? this.monitoredTrack() : null;
    if (this.monitor !== null && this.monitor.track !== monitored?.track) {
      this.monitor.output.stop();
      this.monitor = null;
    }
    if (monitored === null) {
      return;
    }
    this.monitor ??= { track: monitored.track, output: this.ports.playTrack(monitored.track) };
    this.monitor.output.setGain(monitored.gain);
    this.monitor.output.setOutputDevice(this.outputDeviceId());
  }

  /** A device came or went: the remembered ones are used again when back, the defaults while away. */
  private async checkDevices(): Promise<void> {
    let devices: readonly AudioDevice[];
    try {
      devices = await this.ports.listDevices();
    } catch (error) {
      console.warn("couldn't list the audio devices", error);
      return;
    }
    if (!this.isActive()) {
      return;
    }
    const isOutputMissing = !isRememberedDevicePresent(devices, "audiooutput", this.settings.outputDeviceId);
    if (isOutputMissing !== this.isOutputDeviceMissing) {
      this.isOutputDeviceMissing = isOutputMissing;
      this.applyOutputDevice();
    }
    const isUnpluggedMicrophoneBack =
      this.isInputDeviceMissing && isRememberedDevicePresent(devices, "audioinput", this.settings.inputDeviceId);
    // The only microphone was unplugged: any device change may be it coming back.
    const mayMicrophoneBeBack = this.microphone === null && this.microphoneProblem === "missing";
    if (isUnpluggedMicrophoneBack || mayMicrophoneBeBack) {
      await this.switchMicrophone();
    }
  }

  private isMediaConnected(): boolean {
    const state = this.peer?.iceConnectionState;
    return state === "connected" || state === "completed";
  }

  private async acquireMicrophone(): Promise<void> {
    try {
      const opened = await this.openMicrophone();
      // Hung up meanwhile, or a second click got a stream first: this one would never be stopped.
      if (this.phase === "idle" || this.microphone !== null) {
        closeOpenedMicrophone(opened);
        return;
      }
      this.installMicrophone(opened);
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
    const track = this.sentTrack() ?? null;
    this.sender = peer.addTransceiver(track ?? "audio", { direction: "sendonly" }).sender;
    this.slotUserIds.clear();
    for (let slot = 0; slot < VOICE_RECEIVE_SLOTS; slot += 1) {
      peer.addTransceiver("audio", { direction: "recvonly" });
    }
    peer.addEventListener("track", (event) => {
      if (generation === this.negotiation) {
        const output = this.ports.playTrack(event.track);
        const mid = event.transceiver.mid;
        output.setOutputDevice(this.outputDeviceId());
        output.setGain(this.gainOfSlot(mid));
        this.outputs.push({ mid, output });
      }
    });
    peer.addEventListener("iceconnectionstatechange", () => this.iceStateChanged(peer));
    this.applyTransmission();
    this.applyGains();
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
        offer_id: generation,
        sdp_offer: offer,
        self_mute: this.isMicrophoneMuted,
        self_deaf: this.toggles.isDeafened,
      },
    });
    // The gateway is down: the next `ready` or `resumed` offers again.
    this.isAwaitingGateway = !isSent;
    if (isSent) {
      this.hasUnsentVoiceState = false;
    }
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
    // Audio flows again: a later media timeout deserves its own silent retry.
    this.hasRetriedAfterMediaTimeout = false;
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
    this.applyTransmission();
    this.applyGains();
    if (this.phase !== "idle") {
      this.sendVoiceState();
    }
  }

  private sendVoiceState(): void {
    this.hasUnsentVoiceState = !this.ports.sendFrame({
      op: "voice_state",
      d: { self_mute: this.isMicrophoneMuted, self_deaf: this.toggles.isDeafened },
    });
  }

  /** Whether the input mode lets sound out now (mute aside). */
  private isTransmitting(): boolean {
    if (this.settings.inputMode === "push-to-talk") {
      return this.isPushToTalkOpen;
    }
    return (
      this.settings.isSensitivityAutomatic ||
      isPageHidden() ||
      Date.now() - this.lastAboveThresholdAt < SENSITIVITY_GATE_HOLD_MILLISECONDS
    );
  }

  /**
   * Muting, push-to-talk and the sensitivity gate stop sending at once; the server's `SPEAK` check
   * is the enforcement, not this. The meter reads its own copy of the track, so the gate can reopen.
   */
  private applyTransmission(): void {
    const isMuted = this.toggles.isMicrophoneMuted;
    const isOpen = !isMuted && this.isTransmitting();
    const track = this.microphoneTrack();
    if (track) {
      track.enabled = !isMuted && (this.amplifier !== null || isOpen);
    }
    if (this.amplifier) {
      this.amplifier.track.enabled = isOpen;
    }
    if (this.processor) {
      // The device's track feeds the chain, so a closed gate or push-to-talk leaves it on for the
      // meter; muted (or deafened), it goes off too, so the browser's microphone indicator does.
      this.microphone?.getAudioTracks().forEach((deviceTrack) => (deviceTrack.enabled = !isMuted));
    }
  }

  private setPersonSetting(userId: Snowflake, setting: PersonVoiceSetting): void {
    const next = new Map(this.personSettings);
    next.set(userId, setting);
    this.personSettings = next;
    writePersonVoiceSettings(this.ports.storage(), next);
    this.applyGains();
  }

  private gainOfSlot(mid: string | null): number {
    const userId = mid === null ? undefined : this.slotUserIds.get(mid);
    const person = userId === undefined ? null : this.personSetting(userId);
    return playbackGain(this.settings.outputVolumePercent, person, this.toggles.isDeafened);
  }

  /** Output volume, each person's volume and local mute, and deafen (which wins). */
  private applyGains(): void {
    for (const { mid, output } of this.outputs) {
      output.setGain(this.gainOfSlot(mid));
    }
  }

  private applyOutputDevice(): void {
    const deviceId = this.outputDeviceId();
    for (const { output } of this.outputs) {
      output.setOutputDevice(deviceId);
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
    const level = this.isMicrophoneMuted ? 0 : (this.meter?.level() ?? 0) * this.inputGain();
    if (level >= SPEAKING_LEVEL_THRESHOLD) {
      this.lastLoudAt = now;
    }
    if (decibelsOfLevel(level) >= this.settings.sensitivityThresholdDecibels && level > 0) {
      this.lastAboveThresholdAt = now;
    }
    this.applyTransmission();
    const isManualGate = this.settings.inputMode === "voice-activity" && !this.settings.isSensitivityAutomatic;
    // Your ring follows what leaves: the gate itself when manual, else being heard and loud.
    const isHeard = !this.isMicrophoneMuted && this.isTransmitting();
    const isSpeaking =
      this.phase === "connected" && isHeard && (isManualGate || now - this.lastLoudAt < SPEAKING_HOLD_MILLISECONDS);
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
    for (const { output } of this.outputs) {
      output.stop();
    }
    this.outputs = [];
    this.slotUserIds.clear();
  }

  /** Everything local ends; the toggles stay, as on Discord. */
  private stop(): void {
    this.negotiation += 1;
    this.closePeer();
    this.clearOwnStateDroppedTimer();
    this.clearTimer("roundTripTimer");
    this.clearTimer("levelTimer");
    this.clearTimer("pushToTalkReleaseTimer");
    this.microphoneSwitch += 1;
    // Listening to the call's microphone ends with it; the settings' test keeps its own.
    if (!this.isMicrophoneTestRunning) {
      this.isMonitoring = false;
    }
    this.releaseMicrophone();
    this.stopWatchingDevices?.();
    this.stopWatchingDevices = null;
    this.microphoneProblem = null;
    this.isInputDeviceMissing = false;
    this.isOutputDeviceMissing = false;
    this.isPushToTalkOpen = false;
    this.lastAboveThresholdAt = Number.NEGATIVE_INFINITY;
    this.isAwaitingGateway = false;
    this.hasUnsentVoiceState = false;
    this.isSelfSpeaking = false;
    this.lastLoudAt = Number.NEGATIVE_INFINITY;
    this.roundTripMilliseconds = null;
    this.channelId = null;
    this.phase = "idle";
  }

  private clearOwnStateDroppedTimer(): void {
    this.clearTimer("ownStateDroppedTimer");
  }

  private clearTimer(
    name: "connectTimer" | "iceGraceTimer" | "ownStateDroppedTimer" | "roundTripTimer" | "levelTimer" | "pushToTalkReleaseTimer",
  ): void {
    const timer = this[name];
    if (timer !== null) {
      clearTimeout(timer);
      clearInterval(timer);
      this[name] = null;
    }
  }
}
