import { describe, expect, it } from "vitest";

import { CALL_RING_TIMEOUT_MILLISECONDS, type Call } from "../lib/protocol";
import { unixMillisecondsOfSnowflake } from "../lib/snowflake-time";
import { plannedIncomingSound, plannedRingback } from "./call-alert-plan";

const ME = "1";
const ADA = "2";
// A message id made 60 s after the Mellow epoch.
const MESSAGE_ID = String(60_000n << 22n);
const STARTED_AT = unixMillisecondsOfSnowflake(MESSAGE_ID);

function ringing(): Call {
  return { channel_id: "50", message_id: MESSAGE_ID, caller_id: ADA, ringing_user_ids: [ME], participants: [] };
}

describe("the incoming call's sound", () => {
  it("keeps one key per ringing, whether the reader is in a call or not, so answering never swaps it", () => {
    const facts = { isDoNotDisturb: false, now: STARTED_AT };
    const alone = plannedIncomingSound(ringing(), { ...facts, isInAnotherCallOrVoiceChannel: false });
    const joining = plannedIncomingSound(ringing(), { ...facts, isInAnotherCallOrVoiceChannel: true });
    expect(alone?.kind).toBe("ringtone");
    expect(joining?.kind).toBe("call-waiting");
    expect(joining?.key).toBe(alone?.key);
  });

  it("is silent under Do not disturb, under a new key, so turning it on stops a ring", () => {
    const ringingOut = plannedIncomingSound(ringing(), { isInAnotherCallOrVoiceChannel: false, isDoNotDisturb: false, now: STARTED_AT });
    expect(plannedIncomingSound(ringing(), { isInAnotherCallOrVoiceChannel: false, isDoNotDisturb: true, now: STARTED_AT })).toBeNull();
    expect(ringingOut?.key).not.toContain("true");
  });

  it("rings for what is left of the 30 seconds, on this device's clock", () => {
    const facts = { isInAnotherCallOrVoiceChannel: false, isDoNotDisturb: false };
    expect(plannedIncomingSound(ringing(), { ...facts, now: STARTED_AT + 10_000 })?.durationMilliseconds).toBe(20_000);
    // A clock behind the server's: never more than the whole timeout.
    expect(plannedIncomingSound(ringing(), { ...facts, now: STARTED_AT - 60_000 })?.durationMilliseconds).toBe(CALL_RING_TIMEOUT_MILLISECONDS);
    // A clock ahead: the deadline looks past while the server says it rings, so the whole timeout.
    expect(plannedIncomingSound(ringing(), { ...facts, now: STARTED_AT + 120_000 })?.durationMilliseconds).toBe(CALL_RING_TIMEOUT_MILLISECONDS);
  });

  it("is nothing without a ringing call", () => {
    expect(plannedIncomingSound(null, { isInAnotherCallOrVoiceChannel: false, isDoNotDisturb: false, now: 0 })).toBeNull();
  });
});

describe("the caller's waiting tone", () => {
  it("plays under its own key, for the time left", () => {
    const plan = plannedRingback(ringing(), STARTED_AT + 5_000);
    expect(plan).toEqual({ kind: "ringback", key: `ringback:50:${MESSAGE_ID}`, durationMilliseconds: 25_000 });
    expect(plannedRingback(null, STARTED_AT)).toBeNull();
  });
});
