import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { type ClientFrame, VOICE_RECEIVE_SLOTS } from "./protocol";
import { type VoiceCue, VoiceSession } from "./voice-session.svelte";

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
  receiveTrack() {
    this.listeners.get("track")?.forEach((listener) => listener({ track: {} }));
  }
}

function fakeMicrophone() {
  const track = { enabled: true, isStopped: false, stop: () => (track.isStopped = true) };
  return { track, stream: { getAudioTracks: () => [track], getTracks: () => [track] } };
}

function harness({ microphoneError }: { microphoneError?: Error } = {}) {
  const frames: ClientFrame[] = [];
  const peers: FakePeer[] = [];
  const cues: VoiceCue[] = [];
  const outputs: { isMuted: boolean; isStopped: boolean }[] = [];
  const microphone = fakeMicrophone();
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
    requestMicrophone: () =>
      microphoneFailure.error
        ? Promise.reject(microphoneFailure.error)
        : Promise.resolve(microphone.stream as unknown as MediaStream),
    playTrack: () => {
      const output = { isMuted: false, isStopped: false };
      outputs.push(output);
      return { setMuted: (isMuted) => (output.isMuted = isMuted), stop: () => (output.isStopped = true) };
    },
    meterLevel: () => null,
    storage: () => null,
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
    await session.applyAnswer(channelId, "answer");
    lastPeer().setIce("connected");
  }
  const joins = () => frames.filter((frame) => frame.op === "voice_join");
  return { session, frames, joins, peers, lastPeer, cues, outputs, microphone, microphoneFailure, gateway, connect };
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
      { op: "voice_join", d: { channel_id: CHANNEL, sdp_offer: "offer-17", self_mute: false, self_deaf: false } },
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
    await session.applyAnswer(OTHER_CHANNEL, "answer");
    expect(lastPeer().remoteDescription).toBeNull();
    await session.applyAnswer(CHANNEL, "answer");
    await session.applyAnswer(CHANNEL, "second answer");
    expect(lastPeer().remoteDescription?.sdp).toBe("answer");
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
    expect(outputs[0]?.isMuted).toBe(true);
    expect(microphone.track.enabled).toBe(false);
    expect(frames.at(-1)).toEqual({ op: "voice_state", d: { self_mute: true, self_deaf: true } });
    session.toggleDeafened();
    expect(outputs[0]?.isMuted).toBe(false);
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
    session.refuse(CHANNEL, "channel_full");
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
    await session.applyAnswer(CHANNEL, "answer");
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
      playTrack: () => ({ setMuted: () => {}, stop: () => {} }),
      meterLevel: () => null,
      storage: () => null,
    });
    await unsupported.join(CHANNEL);
    expect(unsupported.problem).toEqual({ kind: "unsupported" });
    expect(unsupported.phase).toBe("idle");
    expect(session.problem).toBeNull();
  });

  it("retries the channel of a problem", async () => {
    const { session, joins } = harness();
    await session.join(CHANNEL);
    session.refuse(CHANNEL, "rate_limited");
    session.retry();
    await flush();
    expect(session.problem).toBeNull();
    expect(joins()).toHaveLength(2);
  });
});
