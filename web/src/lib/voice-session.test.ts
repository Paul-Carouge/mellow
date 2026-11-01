import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { type ClientFrame, VOICE_RECEIVE_SLOTS } from "./protocol";
import { type AudioDevice, DEFAULT_VOICE_SETTINGS, PERSON_VOICE_SETTINGS_STORAGE_KEY, VOICE_SETTINGS_STORAGE_KEY } from "./voice-settings";
import { type MicrophoneChoice, type VoicePorts, type VoiceCue, VoiceSession } from "./voice-session.svelte";

const CHANNEL = "500";
const OTHER_CHANNEL = "501";

/** The few RTCPeerConnection members the session uses; ICE and signalling states are driven by the test. */
class FakePeer {
  iceGatheringState: RTCIceGatheringState = "complete";
  iceConnectionState: RTCIceConnectionState = "new";
  signalingState: RTCSignalingState = "stable";
  localDescription: { type: string; sdp: string } | null = null;
  remoteDescription: { type: string; sdp?: string } | null = null;
  transceivers: { trackOrKind: unknown; direction: string | undefined }[] = [];
  replacedTracks: unknown[] = [];
  isClosed = false;
  private listeners = new Map<string, ((event: unknown) => void)[]>();

  addTransceiver(trackOrKind: unknown, init?: RTCRtpTransceiverInit) {
    this.transceivers.push({ trackOrKind, direction: init?.direction });
    return { sender: { replaceTrack: (track: unknown) => Promise.resolve(void this.replacedTracks.push(track)) } };
  }
  createOffer() {
    return Promise.resolve({ type: "offer", sdp: `offer-${this.transceivers.length}` });
  }
  setLocalDescription(description: { type: string; sdp: string }) {
    this.localDescription = description;
    this.signalingState = "have-local-offer";
    return Promise.resolve();
  }
  setRemoteDescription(description: { type: string; sdp?: string }) {
    this.remoteDescription = description;
    this.signalingState = "stable";
    return Promise.resolve();
  }
  addEventListener(type: string, listener: (event: unknown) => void) {
    this.listeners.set(type, [...(this.listeners.get(type) ?? []), listener]);
  }
  removeEventListener() {}
  getStats() {
    return Promise.resolve(
      new Map([["p", { type: "candidate-pair", id: "p", nominated: true, state: "succeeded", currentRoundTripTime: 0.042 }]]),
    );
  }
  close() {
    this.isClosed = true;
  }
  /** Test side: an ICE state change, or a remote track arriving. */
  setIce(state: RTCIceConnectionState) {
    this.iceConnectionState = state;
    this.listeners.get("iceconnectionstatechange")?.forEach((listener) => listener({}));
  }
  /** The receive transceivers' mids, as the browser numbers them after the send one. */
  receiveTrack(mid: string | null = "1") {
    this.listeners.get("track")?.forEach((listener) => listener({ track: {}, transceiver: { mid } }));
  }
}

function fakeMicrophone() {
  const endedListeners: (() => void)[] = [];
  const track = {
    enabled: true,
    isStopped: false,
    stop: () => (track.isStopped = true),
    addEventListener: (_type: string, listener: () => void) => endedListeners.push(listener),
    /** Test side: the device was unplugged. */
    end: () => endedListeners.forEach((listener) => listener()),
  };
  return { track, stream: { getAudioTracks: () => [track], getTracks: () => [track] } };
}

/** What sessions built outside the harness need besides their own ports. */
const QUIET_PORTS = {
  playTrack: () => ({ setGain: () => {}, setOutputDevice: () => {}, stop: () => {} }),
  meterLevel: () => null,
  amplify: () => null,
  listDevices: () => Promise.resolve([]),
  watchDevices: () => () => {},
  storage: () => null,
} satisfies Partial<VoicePorts>;

function memoryStorage(entries: Record<string, string> = {}) {
  const values = new Map(Object.entries(entries));
  return { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => void values.set(key, value), values };
}

function harness({ microphoneError, storage = null }: { microphoneError?: Error; storage?: ReturnType<typeof memoryStorage> | null } = {}) {
  const frames: ClientFrame[] = [];
  const peers: FakePeer[] = [];
  const cues: VoiceCue[] = [];
  const outputs: { gain: number; deviceId: string | null; isStopped: boolean }[] = [];
  const microphone = fakeMicrophone();
  /** Every microphone handed out, the first one being `microphone`, with what was asked. */
  const microphones: { choice: MicrophoneChoice; track: ReturnType<typeof fakeMicrophone>["track"] }[] = [];
  /** Device ids that are unplugged: asking for them fails as browsers do. */
  const unpluggedDeviceIds = new Set<string>();
  const devices: { list: AudioDevice[]; onChange: (() => void) | null } = { list: [], onChange: null };
  const meter = { level: 0, isStopped: false };
  const amplifiers: { track: { enabled: boolean }; gain: number; isStopped: boolean }[] = [];
  const gateway = { isUp: true };
  const microphoneFailure = { error: microphoneError };
  // The casts are limited to these fakes, which implement what the session calls.
  const session = new VoiceSession({
    sendFrame: (frame) => gateway.isUp && frames.push(frame) > 0,
    createPeerConnection: () => {
      const peer = new FakePeer();
      peers.push(peer);
      return peer as unknown as RTCPeerConnection;
    },
    requestMicrophone: (choice) => {
      if (microphoneFailure.error) {
        return Promise.reject(microphoneFailure.error);
      }
      if (choice.deviceId !== null && unpluggedDeviceIds.has(choice.deviceId)) {
        return Promise.reject(Object.assign(new Error("unplugged"), { name: "OverconstrainedError" }));
      }
      const next = microphones.length === 0 ? microphone : fakeMicrophone();
      microphones.push({ choice, track: next.track });
      return Promise.resolve(next.stream as unknown as MediaStream);
    },
    playTrack: () => {
      const output = { gain: 1, deviceId: null as string | null, isStopped: false };
      outputs.push(output);
      return {
        setGain: (gain) => (output.gain = gain),
        setOutputDevice: (deviceId) => (output.deviceId = deviceId),
        stop: () => (output.isStopped = true),
      };
    },
    meterLevel: () => ({ level: () => meter.level, stop: () => (meter.isStopped = true) }),
    amplify: (_track, gain) => {
      const amplifier = { track: { enabled: true }, gain, isStopped: false };
      amplifiers.push(amplifier);
      return {
        track: amplifier.track as unknown as MediaStreamTrack,
        setGain: (next) => (amplifier.gain = next),
        stop: () => (amplifier.isStopped = true),
      };
    },
    listDevices: () => Promise.resolve(devices.list),
    watchDevices: (onChange) => {
      devices.onChange = onChange;
      return () => (devices.onChange = null);
    },
    storage: () => storage,
  });
  session.onCue = (cue) => cues.push(cue);
  const lastPeer = () => {
    const peer = peers.at(-1);
    if (!peer) {
      throw new Error("no peer connection yet");
    }
    return peer;
  };
  /** Joins, answers and connects: the usual path. */
  async function connect(channelId = CHANNEL) {
    await session.join(channelId);
    await session.applyAnswer(channelId, session.currentOfferId, "answer");
    lastPeer().setIce("connected");
  }
  const joins = () => frames.filter((frame) => frame.op === "voice_join");
  return {
    session,
    frames,
    joins,
    peers,
    lastPeer,
    cues,
    outputs,
    microphone,
    microphones,
    unpluggedDeviceIds,
    devices,
    meter,
    amplifiers,
    microphoneFailure,
    gateway,
    connect,
  };
}

const flush = () => vi.advanceTimersByTimeAsync(0);

describe("VoiceSession", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("offers one send line, then the receive slots, once ICE gathering is complete", async () => {
    const { session, joins, lastPeer, microphone } = harness();
    await session.join(CHANNEL);
    const peer = lastPeer();
    expect(peer.transceivers[0]).toEqual({ trackOrKind: microphone.track, direction: "sendonly" });
    expect(peer.transceivers.slice(1)).toHaveLength(VOICE_RECEIVE_SLOTS);
    expect(peer.transceivers.slice(1).every((line) => line.trackOrKind === "audio" && line.direction === "recvonly")).toBe(true);
    expect(joins()).toEqual([
      {
        op: "voice_join",
        d: { channel_id: CHANNEL, offer_id: session.currentOfferId, sdp_offer: "offer-17", self_mute: false, self_deaf: false },
      },
    ]);
    expect(session.phase).toBe("connecting");
  });

  it("connects on the answer and ICE, with the join sound once", async () => {
    const { session, lastPeer, cues, connect } = harness();
    await connect();
    expect(lastPeer().remoteDescription).toEqual({ type: "answer", sdp: "answer" });
    expect(session.phase).toBe("connected");
    expect(cues).toEqual(["self-join"]);
    await flush();
    expect(session.roundTripMilliseconds).toBe(42);
  });

  it("ignores an answer for another channel or with no offer waiting", async () => {
    const { session, lastPeer } = harness();
    await session.join(CHANNEL);
    const offerId = session.currentOfferId;
    await session.applyAnswer(OTHER_CHANNEL, offerId, "answer");
    expect(lastPeer().remoteDescription).toBeNull();
    await session.applyAnswer(CHANNEL, offerId, "answer");
    await session.applyAnswer(CHANNEL, offerId, "second answer");
    expect(lastPeer().remoteDescription?.sdp).toBe("answer");
  });

  it("applies only the answer to its latest offer, and ignores a refusal of an older one", async () => {
    const { session, joins, lastPeer, connect } = harness();
    await connect();
    lastPeer().setIce("failed");
    await flush();
    // A reconnection sent a second offer: the answer to the first one arrives late.
    const [first, second] = joins().map((frame) => (frame.op === "voice_join" ? frame.d.offer_id : -1));
    expect(second).not.toBe(first);
    await session.applyAnswer(CHANNEL, first ?? -1, "stale answer");
    expect(lastPeer().remoteDescription).toBeNull();
    session.refuse(CHANNEL, first ?? -1, "rate_limited");
    expect(session.phase).toBe("reconnecting");
    await session.applyAnswer(CHANNEL, second ?? -1, "fresh answer");
    expect(lastPeer().remoteDescription?.sdp).toBe("fresh answer");
  });

  it("mutes by stopping the track and telling the instance; deafening also mutes every voice", async () => {
    const { session, frames, outputs, microphone, lastPeer, connect, cues } = harness();
    await connect();
    lastPeer().receiveTrack();
    await session.toggleMicrophone();
    expect(microphone.track.enabled).toBe(false);
    expect(frames.at(-1)).toEqual({ op: "voice_state", d: { self_mute: true, self_deaf: false } });
    await session.toggleMicrophone();
    session.toggleDeafened();
    expect(outputs[0]?.gain).toBe(0);
    expect(microphone.track.enabled).toBe(false);
    expect(frames.at(-1)).toEqual({ op: "voice_state", d: { self_mute: true, self_deaf: true } });
    session.toggleDeafened();
    expect(outputs[0]?.gain).toBe(1);
    expect(microphone.track.enabled).toBe(true);
    expect(cues.slice(1)).toEqual(["mute", "unmute", "deafen", "undeafen"]);
  });

  it("keeps the toggles while not connected, without sending anything", () => {
    const { session, frames } = harness();
    session.toggleDeafened();
    expect(session.toggles).toMatchObject({ isDeafened: true, isMicrophoneMuted: true });
    expect(frames).toEqual([]);
  });

  it("joins to listen when there is no microphone, shown muted with the reason", async () => {
    const missing = Object.assign(new Error("none"), { name: "NotFoundError" });
    const { session, joins, lastPeer } = harness({ microphoneError: missing });
    await session.join(CHANNEL);
    expect(lastPeer().transceivers[0]).toEqual({ trackOrKind: "audio", direction: "sendonly" });
    expect(joins()[0]).toMatchObject({ d: { self_mute: true } });
    expect(session.microphoneProblem).toBe("missing");
    expect(session.isMicrophoneMuted).toBe(true);
  });

  it("asks for the microphone again when unmuting without one, and keeps it muted if refused", async () => {
    const denied = Object.assign(new Error("no"), { name: "NotAllowedError" });
    const { session, connect, frames } = harness({ microphoneError: denied });
    await connect();
    const sentBefore = frames.length;
    await session.toggleMicrophone();
    expect(session.isMicrophoneMuted).toBe(true);
    expect(session.microphoneProblem).toBe("denied");
    expect(frames).toHaveLength(sentBefore);
  });

  it("starts sending once a microphone is allowed, without renegotiating", async () => {
    const denied = Object.assign(new Error("no"), { name: "NotAllowedError" });
    const { session, connect, frames, lastPeer, joins, microphone, microphoneFailure } = harness({ microphoneError: denied });
    await connect();
    microphoneFailure.error = undefined;
    await session.toggleMicrophone();
    expect(lastPeer().replacedTracks).toEqual([microphone.track]);
    expect(session.microphoneProblem).toBeNull();
    expect(session.isMicrophoneMuted).toBe(false);
    expect(frames.at(-1)).toEqual({ op: "voice_state", d: { self_mute: false, self_deaf: false } });
    expect(joins()).toHaveLength(1);
  });

  it("leaves: tells the instance, closes everything and frees the microphone", async () => {
    const { session, frames, lastPeer, microphone, connect, cues } = harness();
    await connect();
    session.leave();
    expect(frames.at(-1)).toEqual({ op: "voice_leave" });
    expect(lastPeer().isClosed).toBe(true);
    expect(microphone.track.isStopped).toBe(true);
    expect(session.phase).toBe("idle");
    expect(session.channelId).toBeNull();
    expect(cues.at(-1)).toBe("self-leave");
  });

  it("moves to another channel with a fresh offer, keeping the microphone", async () => {
    const { session, joins, peers, connect, microphone } = harness();
    await connect();
    await session.join(OTHER_CHANNEL);
    expect(peers[0]?.isClosed).toBe(true);
    expect(joins().at(-1)).toMatchObject({ d: { channel_id: OTHER_CHANNEL } });
    expect(microphone.track.isStopped).toBe(false);
  });

  it("stops on a refusal, says why, and ends any old connection", async () => {
    const { session, frames } = harness();
    await session.join(CHANNEL);
    session.refuse(CHANNEL, session.currentOfferId, "channel_full");
    expect(frames.at(-1)).toEqual({ op: "voice_leave" });
    expect(session.phase).toBe("idle");
    expect(session.problem).toEqual({ kind: "refused", channelId: CHANNEL, reason: "channel_full" });
  });

  it("refuses at once, without asking for the microphone, when the instance has no voice", async () => {
    const { session, frames, peers } = harness();
    session.isVoiceEnabledOnInstance = false;
    await session.join(CHANNEL);
    expect(frames).toEqual([]);
    expect(peers).toEqual([]);
    expect(session.phase).toBe("idle");
    expect(session.problem).toEqual({ kind: "refused", channelId: CHANNEL, reason: "voice_unavailable" });
  });

  it("does not reconnect when voice moved to another tab or device", async () => {
    const { session, joins, connect } = harness();
    await connect();
    session.disconnectedByServer(CHANNEL, "moved_elsewhere");
    await vi.advanceTimersByTimeAsync(10_000);
    expect(session.phase).toBe("idle");
    expect(joins()).toHaveLength(1);
    expect(session.problem).toEqual({ kind: "disconnected", channelId: CHANNEL, reason: "moved_elsewhere" });
  });

  it("reconnects once after a media timeout, then shows the error", async () => {
    const { session, joins, connect } = harness();
    await connect();
    session.disconnectedByServer(CHANNEL, "media_timeout");
    await flush();
    expect(session.phase).toBe("reconnecting");
    expect(joins()).toHaveLength(2);
    session.disconnectedByServer(CHANNEL, "media_timeout");
    expect(session.phase).toBe("idle");
    expect(session.problem).toMatchObject({ kind: "disconnected", reason: "media_timeout" });
  });

  it("retries once per connection: a media timeout after audio came back retries again", async () => {
    const { session, joins, lastPeer, connect } = harness();
    await connect();
    session.disconnectedByServer(CHANNEL, "media_timeout");
    await flush();
    await session.applyAnswer(CHANNEL, session.currentOfferId, "answer");
    lastPeer().setIce("connected");
    expect(session.phase).toBe("connected");
    session.disconnectedByServer(CHANNEL, "media_timeout");
    await flush();
    expect(session.phase).toBe("reconnecting");
    expect(joins()).toHaveLength(3);
  });

  it("sends a mute made while the gateway was resuming once it is back", async () => {
    const { session, frames, joins, gateway, connect } = harness();
    await connect();
    gateway.isUp = false;
    await session.toggleMicrophone();
    expect(frames.some((frame) => frame.op === "voice_state")).toBe(false);
    gateway.isUp = true;
    session.noteGatewayResumed();
    await flush();
    expect(joins()).toHaveLength(1);
    expect(frames.at(-1)).toEqual({ op: "voice_state", d: { self_mute: true, self_deaf: false } });
    session.noteGatewayResumed();
    expect(frames.filter((frame) => frame.op === "voice_state")).toHaveLength(1);
  });

  it("stops the second microphone stream of a quick second join", async () => {
    const pending: ((stream: MediaStream) => void)[] = [];
    const microphones = [fakeMicrophone(), fakeMicrophone()];
    const session = new VoiceSession({
      sendFrame: () => true,
      createPeerConnection: () => new FakePeer() as unknown as RTCPeerConnection,
      requestMicrophone: () => new Promise((resolve) => pending.push(resolve)),
      ...QUIET_PORTS,
    });
    const first = session.join(CHANNEL);
    const second = session.join(OTHER_CHANNEL);
    pending[0]?.(microphones[0]?.stream as unknown as MediaStream);
    await first;
    pending[1]?.(microphones[1]?.stream as unknown as MediaStream);
    await second;
    expect(microphones[0]?.track.isStopped).toBe(false);
    expect(microphones[1]?.track.isStopped).toBe(true);
    expect(session.channelId).toBe(OTHER_CHANNEL);
  });

  it("offers again when ICE stays disconnected for 3 seconds, not before", async () => {
    const { session, joins, lastPeer, connect } = harness();
    await connect();
    lastPeer().setIce("disconnected");
    await vi.advanceTimersByTimeAsync(2_999);
    expect(joins()).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(session.phase).toBe("reconnecting");
    expect(joins()).toHaveLength(2);
  });

  it("lets a short ICE hiccup heal by itself", async () => {
    const { joins, lastPeer, connect } = harness();
    await connect();
    lastPeer().setIce("disconnected");
    await vi.advanceTimersByTimeAsync(1_000);
    lastPeer().setIce("connected");
    await vi.advanceTimersByTimeAsync(5_000);
    expect(joins()).toHaveLength(1);
  });

  it("says the voice port may be closed when a first join never gets audio", async () => {
    const { session, frames, lastPeer } = harness();
    await session.join(CHANNEL);
    await session.applyAnswer(CHANNEL, session.currentOfferId, "answer");
    lastPeer().setIce("failed");
    expect(session.problem).toEqual({ kind: "unreachable", channelId: CHANNEL });
    expect(frames.at(-1)).toEqual({ op: "voice_leave" });
  });

  it("says the instance did not answer after 15 seconds without an answer", async () => {
    const { session } = harness();
    await session.join(CHANNEL);
    await vi.advanceTimersByTimeAsync(15_000);
    expect(session.problem).toEqual({ kind: "no-answer", channelId: CHANNEL });
  });

  it("waits for the gateway, then offers on its next ready", async () => {
    const { session, joins, gateway } = harness();
    gateway.isUp = false;
    await session.join(CHANNEL);
    expect(joins()).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(20_000);
    expect(session.problem).toBeNull();
    gateway.isUp = true;
    session.noteGatewayReady();
    await flush();
    expect(joins()).toHaveLength(1);
  });

  it("offers again after a resume only if audio stopped", async () => {
    const { session, joins, lastPeer, connect } = harness();
    await connect();
    session.noteGatewayResumed();
    await flush();
    expect(joins()).toHaveLength(1);
    lastPeer().setIce("checking");
    session.noteGatewayResumed();
    await flush();
    expect(joins()).toHaveLength(2);
  });

  it("offers again when the instance drops your state, unless it says why", async () => {
    const first = harness();
    await first.connect();
    first.session.noteOwnStateDropped(CHANNEL);
    await vi.advanceTimersByTimeAsync(1_000);
    expect(first.joins()).toHaveLength(2);

    const second = harness();
    await second.connect();
    second.session.noteOwnStateDropped(CHANNEL);
    second.session.disconnectedByServer(CHANNEL, "permissions_changed");
    await vi.advanceTimersByTimeAsync(5_000);
    expect(second.joins()).toHaveLength(1);
    expect(second.session.problem).toMatchObject({ reason: "permissions_changed" });
  });

  it("says when the browser has no WebRTC", async () => {
    const { session } = harness();
    const unsupported = new VoiceSession({
      sendFrame: () => true,
      createPeerConnection: () => null,
      requestMicrophone: () => Promise.reject(new TypeError("none")),
      ...QUIET_PORTS,
    });
    await unsupported.join(CHANNEL);
    expect(unsupported.problem).toEqual({ kind: "unsupported" });
    expect(unsupported.phase).toBe("idle");
    expect(session.problem).toBeNull();
  });

  it("retries the channel of a problem", async () => {
    const { session, joins } = harness();
    await session.join(CHANNEL);
    session.refuse(CHANNEL, session.currentOfferId, "rate_limited");
    session.retry();
    await flush();
    expect(session.problem).toBeNull();
    expect(joins()).toHaveLength(2);
  });

  describe("comfort (stage 16.3)", () => {
    /** RMS for a level in dBFS. */
    const levelAt = (decibels: number) => 10 ** (decibels / 20);

    it("plays each slot at the volume of the person it carries, and forgets who on a new answer", async () => {
      const { session, outputs, lastPeer, connect } = harness();
      await connect();
      lastPeer().receiveTrack("1");
      lastPeer().receiveTrack("2");
      session.setPersonVolume("7", 150);
      session.assignSlot(CHANNEL, "1", "7");
      expect(outputs.map((output) => output.gain)).toEqual([1.5, 1]);
      session.togglePersonMuted("7");
      expect(outputs[0]?.gain).toBe(0);
      // The slot now carries someone else.
      session.assignSlot(CHANNEL, "1", "8");
      expect(outputs[0]?.gain).toBe(1);
      session.assignSlot(CHANNEL, "2", "7");
      expect(outputs[1]?.gain).toBe(0);
      // Another channel's assignment is not this media session's.
      session.assignSlot(OTHER_CHANNEL, "1", "7");
      expect(outputs[0]?.gain).toBe(1);
      // A fresh answer: the mapping is gone until the server assigns again.
      session.noteGatewayReady();
      await flush();
      await session.applyAnswer(CHANNEL, session.currentOfferId, "answer");
      lastPeer().receiveTrack("2");
      expect(outputs.at(-1)?.gain).toBe(1);
    });

    it("applies the output volume to every voice, and deafen still silences all", async () => {
      const { session, outputs, lastPeer, connect } = harness();
      await connect();
      lastPeer().receiveTrack("1");
      session.setPersonVolume("7", 200);
      session.assignSlot(CHANNEL, "1", "7");
      session.updateSettings({ ...session.settings, outputVolumePercent: 50 });
      expect(outputs[0]?.gain).toBe(1);
      session.toggleDeafened();
      expect(outputs[0]?.gain).toBe(0);
    });

    it("routes voices to the chosen output, and to the default one while it is unplugged", async () => {
      const { session, outputs, lastPeer, connect, devices } = harness();
      session.updateSettings({ ...session.settings, outputDeviceId: "headset" });
      devices.list = [{ kind: "audiooutput", deviceId: "headset", label: "Casque" }];
      await connect();
      lastPeer().receiveTrack("1");
      expect(outputs[0]?.deviceId).toBe("headset");
      devices.list = [{ kind: "audiooutput", deviceId: "speakers", label: "Haut-parleurs" }];
      devices.onChange?.();
      await flush();
      expect(session.isOutputDeviceMissing).toBe(true);
      expect(outputs[0]?.deviceId).toBeNull();
      devices.list = [{ kind: "audiooutput", deviceId: "headset", label: "Casque" }];
      devices.onChange?.();
      await flush();
      expect(session.isOutputDeviceMissing).toBe(false);
      expect(outputs[0]?.deviceId).toBe("headset");
    });

    it("sends only while the push-to-talk key is held, then after the release delay", async () => {
      const { session, microphone, connect, cues, meter } = harness();
      session.updateSettings({
        ...session.settings,
        inputMode: "push-to-talk",
        pushToTalkKey: { code: "ControlRight", label: "Ctrl droit" },
        pushToTalkReleaseDelayMilliseconds: 200,
        arePushToTalkSoundsOn: true,
      });
      await connect();
      meter.level = levelAt(-20);
      await vi.advanceTimersByTimeAsync(40);
      expect(microphone.track.enabled).toBe(false);
      expect(session.isSelfSpeaking).toBe(false);
      session.pressPushToTalk();
      session.pressPushToTalk();
      expect(microphone.track.enabled).toBe(true);
      await vi.advanceTimersByTimeAsync(40);
      expect(session.isSelfSpeaking).toBe(true);
      session.releasePushToTalk();
      await vi.advanceTimersByTimeAsync(199);
      expect(microphone.track.enabled).toBe(true);
      await vi.advanceTimersByTimeAsync(1);
      expect(microphone.track.enabled).toBe(false);
      expect(cues.filter((cue) => cue.startsWith("push-to-talk"))).toEqual(["push-to-talk-press", "push-to-talk-release"]);
    });

    it("keeps the microphone muted whatever push-to-talk does, and ignores the key outside a call", async () => {
      const { session, microphone, connect, cues } = harness();
      session.updateSettings({ ...session.settings, inputMode: "push-to-talk", arePushToTalkSoundsOn: true });
      session.pressPushToTalk();
      await connect();
      expect(microphone.track.enabled).toBe(false);
      await session.toggleMicrophone();
      session.pressPushToTalk();
      expect(microphone.track.enabled).toBe(false);
      expect(cues.filter((cue) => cue.startsWith("push-to-talk"))).toEqual([]);
    });

    it("opens the manual sensitivity gate above the threshold and holds it a quarter second", async () => {
      const { session, microphone, connect, meter } = harness();
      session.updateSettings({ ...session.settings, isSensitivityAutomatic: false, sensitivityThresholdDecibels: -40 });
      await connect();
      meter.level = levelAt(-60);
      await vi.advanceTimersByTimeAsync(40);
      expect(microphone.track.enabled).toBe(false);
      meter.level = levelAt(-30);
      await vi.advanceTimersByTimeAsync(20);
      expect(microphone.track.enabled).toBe(true);
      expect(session.isSelfSpeaking).toBe(true);
      meter.level = levelAt(-60);
      await vi.advanceTimersByTimeAsync(200);
      expect(microphone.track.enabled).toBe(true);
      await vi.advanceTimersByTimeAsync(80);
      expect(microphone.track.enabled).toBe(false);
      expect(session.isSelfSpeaking).toBe(false);
      // Automatic: no gate in the client.
      session.updateSettings({ ...session.settings, isSensitivityAutomatic: true });
      expect(microphone.track.enabled).toBe(true);
    });

    it("keeps the manual gate open in a hidden tab, where timers are throttled", async () => {
      const { session, microphone, connect, meter } = harness();
      session.updateSettings({ ...session.settings, isSensitivityAutomatic: false, sensitivityThresholdDecibels: -40 });
      await connect();
      meter.level = levelAt(-60);
      await vi.advanceTimersByTimeAsync(40);
      expect(microphone.track.enabled).toBe(false);
      vi.stubGlobal("document", { visibilityState: "hidden" });
      try {
        await vi.advanceTimersByTimeAsync(20);
        expect(microphone.track.enabled).toBe(true);
      } finally {
        vi.unstubAllGlobals();
        vi.stubGlobal("navigator", { languages: ["en-GB"] });
      }
      await vi.advanceTimersByTimeAsync(20);
      expect(microphone.track.enabled).toBe(false);
    });

    it("compares the level after the input volume", async () => {
      const { session, connect, meter, amplifiers } = harness();
      session.updateSettings({ ...session.settings, isSensitivityAutomatic: false, sensitivityThresholdDecibels: -40 });
      await connect();
      meter.level = levelAt(-44);
      await vi.advanceTimersByTimeAsync(20);
      expect(session.isSelfSpeaking).toBe(false);
      // 200 % is about +6 dB.
      session.updateSettings({ ...session.settings, inputVolumePercent: 200 });
      await vi.advanceTimersByTimeAsync(20);
      expect(session.isSelfSpeaking).toBe(true);
      expect(amplifiers[0]?.track.enabled).toBe(true);
    });

    it("sends the amplified track for an input volume other than 100 %, without renegotiating", async () => {
      const { session, connect, lastPeer, amplifiers, microphone, joins } = harness();
      await connect();
      session.updateSettings({ ...session.settings, inputVolumePercent: 150 });
      await flush();
      expect(amplifiers).toHaveLength(1);
      expect(amplifiers[0]?.gain).toBe(1.5);
      expect(lastPeer().replacedTracks.at(-1)).toBe(amplifiers[0]?.track);
      session.updateSettings({ ...session.settings, inputVolumePercent: 80 });
      expect(amplifiers).toHaveLength(1);
      expect(amplifiers[0]?.gain).toBe(0.8);
      session.updateSettings({ ...session.settings, inputVolumePercent: 100 });
      await flush();
      expect(amplifiers[0]?.isStopped).toBe(true);
      expect(lastPeer().replacedTracks.at(-1)).toBe(microphone.track);
      expect(joins()).toHaveLength(1);
    });

    it("switches the microphone live when another one or noise suppression is chosen", async () => {
      const { session, connect, lastPeer, microphones, joins } = harness();
      await connect();
      session.updateSettings({ ...session.settings, inputDeviceId: "usb", isNoiseSuppressionOn: false });
      await flush();
      expect(microphones.map((entry) => entry.choice)).toEqual([
        { deviceId: null, isNoiseSuppressionOn: true },
        { deviceId: "usb", isNoiseSuppressionOn: false },
      ]);
      expect(microphones[0]?.track.isStopped).toBe(true);
      expect(lastPeer().replacedTracks.at(-1)).toBe(microphones[1]?.track);
      expect(joins()).toHaveLength(1);
    });

    it("falls back to the default microphone when the remembered one is unplugged, and says so", async () => {
      const { session, connect, microphones, unpluggedDeviceIds, devices, lastPeer } = harness();
      session.updateSettings({ ...session.settings, inputDeviceId: "usb" });
      unpluggedDeviceIds.add("usb");
      await connect();
      expect(microphones.map((entry) => entry.choice.deviceId)).toEqual([null]);
      expect(session.isInputDeviceMissing).toBe(true);
      // Plugged back in: it is used again.
      unpluggedDeviceIds.clear();
      devices.list = [{ kind: "audioinput", deviceId: "usb", label: "Micro USB" }];
      devices.onChange?.();
      await flush();
      expect(microphones.map((entry) => entry.choice.deviceId)).toEqual([null, "usb"]);
      expect(session.isInputDeviceMissing).toBe(false);
      // Unplugged mid-call: its track ends, and the default one takes over.
      unpluggedDeviceIds.add("usb");
      microphones[1]?.track.end();
      await flush();
      expect(microphones.map((entry) => entry.choice.deviceId)).toEqual([null, "usb", null]);
      expect(session.isInputDeviceMissing).toBe(true);
      expect(lastPeer().replacedTracks.at(-1)).toBe(microphones[2]?.track);
    });

    it("tells the instance when the only microphone is unplugged, and recovers when it is back", async () => {
      const { session, connect, microphones, microphoneFailure, devices, lastPeer, frames } = harness();
      await connect();
      const states = () => frames.filter((frame) => frame.op === "voice_state");
      microphoneFailure.error = Object.assign(new Error("gone"), { name: "NotFoundError" });
      microphones[0]?.track.end();
      await flush();
      expect(session.isMicrophoneMuted).toBe(true);
      expect(session.microphoneProblem).toBe("missing");
      expect(lastPeer().replacedTracks.at(-1)).toBeNull();
      expect(states().at(-1)).toEqual({ op: "voice_state", d: { self_mute: true, self_deaf: false } });
      microphoneFailure.error = undefined;
      devices.onChange?.();
      await flush();
      expect(session.isMicrophoneMuted).toBe(false);
      expect(session.microphoneProblem).toBeNull();
      expect(lastPeer().replacedTracks.at(-1)).toBe(microphones[1]?.track);
      expect(states().at(-1)).toEqual({ op: "voice_state", d: { self_mute: false, self_deaf: false } });
    });

    it("tests the microphone outside a call, and reads the live one during a call", async () => {
      const { session, connect, microphones, meter } = harness();
      expect(session.microphoneDecibels()).toBeNull();
      await session.startMicrophoneTest();
      meter.level = levelAt(-30);
      expect(session.microphoneDecibels()).toBeCloseTo(-30);
      session.stopMicrophoneTest();
      expect(microphones[0]?.track.isStopped).toBe(true);
      expect(session.microphoneDecibels()).toBeNull();
      await connect();
      expect(session.isMicrophoneLive).toBe(true);
      expect(session.microphoneDecibels()).toBeCloseTo(-30);
    });

    it("frees the older microphone when test starts overlap", async () => {
      const { session, microphones } = harness();
      const first = session.startMicrophoneTest();
      session.stopMicrophoneTest();
      const second = session.startMicrophoneTest();
      await Promise.all([first, second]);
      expect(microphones).toHaveLength(2);
      expect(microphones[0]?.track.isStopped).toBe(true);
      expect(microphones[1]?.track.isStopped).toBe(false);
      // Two quick restarts (another microphone, then noise suppression) keep only the last one.
      session.updateSettings({ ...session.settings, inputDeviceId: "usb" });
      session.updateSettings({ ...session.settings, isNoiseSuppressionOn: false });
      await flush();
      expect(microphones.slice(0, -1).every((entry) => entry.track.isStopped)).toBe(true);
      expect(microphones.at(-1)?.track.isStopped).toBe(false);
      session.stopMicrophoneTest();
      expect(microphones.every((entry) => entry.track.isStopped)).toBe(true);
    });

    it("says why the microphone test could not start", async () => {
      const { session } = harness({ microphoneError: Object.assign(new Error("no"), { name: "NotAllowedError" }) });
      await session.startMicrophoneTest();
      expect(session.microphoneTestProblem).toBe("denied");
      expect(session.isMicrophoneTestRunning).toBe(false);
    });

    it("remembers the settings and each person's volume in this browser", () => {
      const storage = memoryStorage();
      const { session } = harness({ storage });
      session.updateSettings({ ...DEFAULT_VOICE_SETTINGS, inputMode: "push-to-talk", outputVolumePercent: 120 });
      session.setPersonVolume("7", 40);
      session.setPersonVolume("8", 100);
      expect(storage.values.has(VOICE_SETTINGS_STORAGE_KEY)).toBe(true);
      expect(JSON.parse(storage.values.get(PERSON_VOICE_SETTINGS_STORAGE_KEY) ?? "{}")).toEqual({
        "7": { volumePercent: 40, isLocallyMuted: false },
      });
      const { session: reloaded } = harness({ storage });
      expect(reloaded.settings.inputMode).toBe("push-to-talk");
      expect(reloaded.settings.outputVolumePercent).toBe(120);
      expect(reloaded.personSetting("7").volumePercent).toBe(40);
      expect(reloaded.personSetting("9").volumePercent).toBe(100);
    });
  });
});
