import { describe, expect, it } from "vitest";

import { ApiError } from "./api";
import {
  callDurationMinutes,
  callMessageSummary,
  callPhase,
  callSound,
  callStartFailureOf,
  incomingCallsOf,
  ringingEndsAtUnixMilliseconds,
} from "./calls";
import type { Call, CallParticipant, MessageCall, Snowflake } from "./protocol";

// The same cases as `mellow_client_core::state::calls`'s tests: both clients must agree.
const ANA = "1";
const BOB = "2";
const CLEO = "3";

function connected(userId: Snowflake): CallParticipant {
  return { user_id: userId, self_mute: false, self_deaf: false, self_video: false, self_stream: false };
}

function call(ringing: Snowflake[], participants: Snowflake[], messageId = "251658240000"): Call {
  return {
    channel_id: "50",
    message_id: messageId,
    caller_id: ANA,
    ringing_user_ids: ringing,
    participants: participants.map(connected),
  };
}

describe("callPhase", () => {
  it("is incoming, outgoing, ongoing or joinable", () => {
    const ringingBob = call([BOB, CLEO], [ANA]);
    expect(callPhase(ringingBob, BOB)).toBe("incoming");
    expect(callPhase(ringingBob, ANA)).toBe("outgoing");
    const answered = call([CLEO], [ANA, BOB]);
    expect(callPhase(answered, ANA)).toBe("ongoing");
    expect(callPhase(answered, CLEO)).toBe("incoming");
    const declinedByCleo = call([], [ANA, BOB]);
    expect(callPhase(declinedByCleo, CLEO)).toBe("joinable");
    const aloneAfterTheOthersLeft = call([], [BOB]);
    expect(callPhase(aloneAfterTheOthersLeft, BOB)).toBe("ongoing");
  });
});

describe("callSound", () => {
  it("is silent under do not disturb and a short cue for someone already in a call", () => {
    expect(callSound("incoming", false, false)).toBe("ringtone");
    expect(callSound("incoming", true, false)).toBe("call-waiting");
    expect(callSound("incoming", true, true)).toBeNull();
    expect(callSound("incoming", false, true)).toBeNull();
  });

  it("plays the waiting tone to the caller, do not disturb or not, and nothing once answered", () => {
    expect(callSound("outgoing", false, true)).toBe("ringback");
    expect(callSound("ongoing", false, false)).toBeNull();
    expect(callSound("joinable", false, false)).toBeNull();
  });
});

describe("ringingEndsAtUnixMilliseconds", () => {
  it("is thirty seconds after the call started", () => {
    expect(ringingEndsAtUnixMilliseconds(1_000_000)).toBe(1_030_000);
  });
});

describe("callMessageSummary", () => {
  const startedAt = 1_000_000;

  it("is ongoing until the call ends", () => {
    const ongoing: MessageCall = { participant_ids: [] };
    expect(callMessageSummary(ongoing, startedAt, BOB)).toEqual({ kind: "ongoing" });
  });

  it("gives its length to those who took part and is missed for the others", () => {
    const ended: MessageCall = { participant_ids: [ANA, BOB], ended_at_unix_milliseconds: startedAt + 720_000 };
    expect(callMessageSummary(ended, startedAt, BOB)).toEqual({ kind: "ended", durationMilliseconds: 720_000 });
    expect(callMessageSummary(ended, startedAt, CLEO)).toEqual({ kind: "missed" });
  });

  it("is unanswered for the caller who rang alone, and missed for the callee", () => {
    const unanswered: MessageCall = { participant_ids: [ANA], ended_at_unix_milliseconds: startedAt + 30_000 };
    expect(callMessageSummary(unanswered, startedAt, ANA)).toEqual({ kind: "unanswered" });
    expect(callMessageSummary(unanswered, startedAt, BOB)).toEqual({ kind: "missed" });
  });

  it("never gives a negative length, whatever the clocks say", () => {
    const ended: MessageCall = { participant_ids: [ANA, BOB], ended_at_unix_milliseconds: startedAt - 5 };
    expect(callMessageSummary(ended, startedAt, ANA)).toEqual({ kind: "ended", durationMilliseconds: 0 });
  });
});

describe("incomingCallsOf", () => {
  it("lists the calls ringing the user, oldest first", () => {
    const newer = { ...call([BOB], [ANA], "900000000000"), channel_id: "61" };
    const older = { ...call([BOB], [ANA], "100000000000"), channel_id: "60" };
    const answered = { ...call([], [ANA, BOB]), channel_id: "62" };
    expect(incomingCallsOf([newer, answered, older], BOB).map((ringing) => ringing.channel_id)).toEqual(["60", "61"]);
    expect(incomingCallsOf([newer, older], ANA)).toEqual([]);
  });
});

describe("callStartFailureOf", () => {
  it("names the refusals the call button can meet", () => {
    expect(callStartFailureOf(new ApiError(409, "CONFLICT"))).toBe("already-calling");
    expect(callStartFailureOf(new ApiError(429, "RATE_LIMITED"))).toBe("rate-limited");
    expect(callStartFailureOf(new ApiError(404, "NOT_FOUND"))).toBe("unreachable");
    expect(callStartFailureOf(new ApiError(400, "VALIDATION_FAILED"))).toBe("voice-unavailable");
    expect(callStartFailureOf(new ApiError(0, "NETWORK"))).toBe("other");
    expect(callStartFailureOf(new Error("boom"))).toBe("other");
  });
});

describe("callDurationMinutes", () => {
  it("counts whole minutes", () => {
    expect(callDurationMinutes(59_999)).toBe(0);
    expect(callDurationMinutes(720_000)).toBe(12);
  });
});
