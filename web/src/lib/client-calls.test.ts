import { describe, expect, it, vi } from "vitest";

import { ApiError, type RestClient } from "./api";
import { MellowClient } from "./client.svelte";
import type { Call, DirectMessageChannel, Ready } from "./protocol";

// Private calls in the web client's state (ADR-0062), at parity with
// `mellow_client_core::state::calls`.
const ME = "1";
const ADA = "2";
const CLEO = "3";
const WITH_ADA = "500";
const GROUP = "501";

function person(id: string, name: string) {
  return { id, username: name.toLowerCase(), display_name: name };
}

const conversationWithAda: DirectMessageChannel = { id: WITH_ADA, recipients: [person(ME, "Me"), person(ADA, "Ada")], last_message_id: null };
const group: DirectMessageChannel = {
  id: GROUP,
  recipients: [person(ME, "Me"), person(ADA, "Ada"), person(CLEO, "Cleo")],
  last_message_id: null,
  owner_id: ADA,
  name: "Atelier",
};

function call(channelId: string, ringing: string[], participants: string[], callerId = ADA): Call {
  return {
    channel_id: channelId,
    message_id: "251658240000",
    caller_id: callerId,
    ringing_user_ids: ringing,
    participants: participants.map((userId) => ({ user_id: userId, self_mute: false, self_deaf: false, self_video: false, self_stream: false })),
  };
}

function ready(overrides: Partial<Ready> = {}): Ready {
  return {
    session_id: "s",
    user: { id: ME, username: "me", display_name: "Me", email: "me@example.org", is_instance_administrator: false },
    guilds: [],
    read_states: [],
    online_user_ids: [],
    direct_message_channels: [conversationWithAda, group],
    calls_enabled: true,
    ...overrides,
  };
}

/** Only what each test needs; every other request fails loudly. The cast is limited to this fake. */
function fakeRest(overrides: Partial<Record<keyof RestClient, unknown>>): RestClient {
  return new Proxy(overrides, {
    get(target, property) {
      if (property in target) {
        return target[property as keyof RestClient];
      }
      return () => Promise.reject(new Error(`unexpected call ${String(property)}`));
    },
  }) as unknown as RestClient;
}

const QUIET_REST = {
  listMessages: () => Promise.resolve([]),
  acknowledge: () => Promise.resolve(),
  gifSettings: () => Promise.reject(new ApiError(404, "NOT_FOUND")),
};

function signedInClient(rest: Partial<Record<keyof RestClient, unknown>> = {}, overrides: Partial<Ready> = {}): MellowClient {
  const client = new MellowClient(fakeRest({ ...QUIET_REST, ...rest }));
  client.applyReady(ready(overrides));
  return client;
}

describe("private calls in the client state", () => {
  it("takes the calls of known conversations from Ready, and whether calls are on", () => {
    const client = signedInClient({}, { calls: [call(WITH_ADA, [ME], [ADA]), call("999", [ME], [ADA])] });
    expect(client.isCallsEnabled).toBe(true);
    expect([...client.callsByChannelId.keys()]).toEqual([WITH_ADA]);
    expect(client.callPhaseOf(WITH_ADA)).toBe("incoming");
    expect(client.callPhaseOf(GROUP)).toBeNull();
  });

  it("hides calls on an older instance that does not say they are on", () => {
    const client = signedInClient({}, { calls_enabled: undefined });
    expect(client.isCallsEnabled).toBe(false);
  });

  it("follows a call from ringing to answered to ended", () => {
    const client = signedInClient();
    client.applyEvent({ t: "CALL_CREATE", data: call(GROUP, [ME, CLEO], [ADA]) }, 0);
    expect(client.incomingCalls.map((ringing) => ringing.channel_id)).toEqual([GROUP]);

    client.applyEvent({ t: "CALL_UPDATE", data: call(GROUP, [CLEO], [ADA, ME]) }, 0);
    expect(client.incomingCalls).toEqual([]);
    expect(client.callPhaseOf(GROUP)).toBe("ongoing");
    expect(client.isInACallOrVoiceChannel).toBe(true);

    client.applyEvent({ t: "CALL_DELETE", data: { channel_id: GROUP } }, 0);
    expect(client.callPhaseOf(GROUP)).toBeNull();
    expect(client.isInACallOrVoiceChannel).toBe(false);
  });

  it("ignores a call in a conversation it does not know", () => {
    const client = signedInClient();
    client.applyEvent({ t: "CALL_CREATE", data: call("999", [ME], [ADA]) }, 0);
    expect(client.callsByChannelId.size).toBe(0);
  });

  it("lights only the call's participants and forgets them when they leave", () => {
    const client = signedInClient();
    client.applyEvent({ t: "CALL_CREATE", data: call(GROUP, [], [ADA, CLEO]) }, 0);
    client.applyEvent({ t: "CALL_SPEAKING", data: { channel_id: GROUP, user_id: ADA, speaking: true } }, 0);
    client.applyEvent({ t: "CALL_SPEAKING", data: { channel_id: GROUP, user_id: ME, speaking: true } }, 0);
    expect([...client.speakingUserIds]).toEqual([ADA]);

    client.applyEvent({ t: "CALL_UPDATE", data: call(GROUP, [], [CLEO]) }, 0);
    expect(client.speakingUserIds.has(ADA)).toBe(false);
  });

  it("drops the call of a conversation the user can no longer read", () => {
    const client = signedInClient({}, { calls: [call(GROUP, [], [ADA])] });
    client.applyEvent({ t: "DIRECT_MESSAGE_CHANNEL_DELETE", data: group }, 0);
    expect(client.callsByChannelId.has(GROUP)).toBe(false);
  });

  it("stops ringing here at once when declining, and tells the instance", async () => {
    const declineCall = vi.fn(() => Promise.resolve());
    const client = signedInClient({ declineCall }, { calls: [call(WITH_ADA, [ME], [ADA])] });
    await client.declineCall(WITH_ADA);
    expect(declineCall).toHaveBeenCalledWith(WITH_ADA);
    expect(client.callPhaseOf(WITH_ADA)).toBe("joinable");
  });

  it("keeps a decline here when the instance cannot be told", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const client = signedInClient(
      { declineCall: () => Promise.reject(new ApiError(0, "NETWORK")) },
      { calls: [call(WITH_ADA, [ME], [ADA])] },
    );
    await client.declineCall(WITH_ADA);
    expect(client.callPhaseOf(WITH_ADA)).toBe("joinable");
    expect(warn).toHaveBeenCalled();
    warn.mockRestore();
  });

  it("starts a call, then joins it with the conversation's id and shows it", async () => {
    const started = call(WITH_ADA, [ADA], [], ME);
    const client = signedInClient({ startCall: () => Promise.resolve(started) });
    // As the session does before it asks for the microphone.
    const join = vi.spyOn(client.voice, "join").mockImplementation((channelId) => {
      client.voice.channelId = channelId;
      client.voice.phase = "connecting";
      return Promise.resolve();
    });
    await client.startCall(WITH_ADA);
    expect(join).toHaveBeenCalledWith(WITH_ADA);
    expect(client.voice.isCallViewOpen).toBe(true);
    expect(client.callsByChannelId.get(WITH_ADA)).toEqual(started);
    expect(client.callProblem).toBeNull();
  });

  it("says why a call could not start, and does not join", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const client = signedInClient({ startCall: () => Promise.reject(new ApiError(409, "CONFLICT")) });
    const join = vi.spyOn(client.voice, "join").mockResolvedValue();
    await client.startCall(WITH_ADA);
    expect(join).not.toHaveBeenCalled();
    expect(client.callProblem).toEqual({ channelId: WITH_ADA, failure: "already-calling" });
    client.dismissCallProblem();
    expect(client.callProblem).toBeNull();
    warn.mockRestore();
  });

  it("starts people the user blocked muted in a call, never in a voice channel, and lets a choice win", () => {
    const client = signedInClient({}, {
      relationships: [{ user: person(CLEO, "Cleo"), kind: "blocked", since_unix_milliseconds: 0 }],
    });
    expect(client.isMutedByDefaultInCalls(CLEO)).toBe(true);
    expect(client.voice.personSetting(CLEO).isLocallyMuted).toBe(false);

    client.voice.channelId = GROUP;
    expect(client.voice.personSetting(CLEO).isLocallyMuted).toBe(true);
    expect(client.voice.personSetting(ADA).isLocallyMuted).toBe(false);

    client.voice.togglePersonMuted(CLEO);
    expect(client.voice.personSetting(CLEO).isLocallyMuted).toBe(false);
  });
});
