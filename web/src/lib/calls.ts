// Private calls in conversations (ADR-0062): what a call going on is for the signed-in user, the
// sound to play for it, and what its message says. Mirrors `mellow_client_core::state::calls`, rule
// for rule: both clients must ring, wait and summarise alike.

import { ApiError } from "./api";
import { CALL_RING_TIMEOUT_MILLISECONDS, type Call, type MessageCall, type Snowflake } from "./protocol";

/**
 * What a call is for the signed-in user. `incoming`: it rings them (answer or decline).
 * `outgoing`: they are in it alone while others are rung. `ongoing`: they are in it with others,
 * or after the others left. `joinable`: it goes on without them and does not ring them.
 */
export type CallPhase = "incoming" | "outgoing" | "ongoing" | "joinable";

/**
 * The sound a client plays for a call. `ringtone`: looped while it rings the user.
 * `call-waiting`: a short discreet cue, for a user already in a call or a voice channel.
 * `ringback`: the caller's waiting tone while others are rung.
 */
export type CallSound = "ringtone" | "call-waiting" | "ringback";

/** What a call's message shows its reader; `ended` carries the call's length. */
export type CallMessageSummary =
  | { readonly kind: "ongoing" }
  | { readonly kind: "missed" }
  | { readonly kind: "unanswered" }
  | { readonly kind: "ended"; readonly durationMilliseconds: number };

export function callPhase(call: Call, userId: Snowflake): CallPhase {
  const isIn = call.participants.some((participant) => participant.user_id === userId);
  if (!isIn) {
    return call.ringing_user_ids.includes(userId) ? "incoming" : "joinable";
  }
  const isAlone = call.participants.length === 1;
  return isAlone && call.ringing_user_ids.length > 0 ? "outgoing" : "ongoing";
}

/**
 * None under do not disturb (the call still shows), a discreet cue for someone already in a call
 * or a voice channel, otherwise the ringtone; the caller hears the waiting tone, do not disturb or
 * not (it is about being disturbed, not calling).
 */
export function callSound(
  phase: CallPhase,
  isInAnotherCallOrVoiceChannel: boolean,
  isDoNotDisturb: boolean,
): CallSound | null {
  switch (phase) {
    case "incoming":
      if (isDoNotDisturb) {
        return null;
      }
      return isInAnotherCallOrVoiceChannel ? "call-waiting" : "ringtone";
    case "outgoing":
      return "ringback";
    case "ongoing":
    case "joinable":
      return null;
  }
}

/**
 * When the call stops ringing at the latest: its start (its message's time) plus the shared
 * timeout. A client stops its ringtone then, even if no event came.
 */
export function ringingEndsAtUnixMilliseconds(callStartedAtUnixMilliseconds: number): number {
  return callStartedAtUnixMilliseconds + CALL_RING_TIMEOUT_MILLISECONDS;
}

/**
 * What a call's message shows `readerId`: going on; missed when it ended without them; unanswered
 * when they were its only participant (the caller who rang alone); else its length.
 */
export function callMessageSummary(
  call: MessageCall,
  callStartedAtUnixMilliseconds: number,
  readerId: Snowflake,
): CallMessageSummary {
  const endedAt = call.ended_at_unix_milliseconds;
  if (endedAt === undefined) {
    return { kind: "ongoing" };
  }
  if (!call.participant_ids.includes(readerId)) {
    return { kind: "missed" };
  }
  if (call.participant_ids.length === 1) {
    return { kind: "unanswered" };
  }
  return { kind: "ended", durationMilliseconds: Math.max(0, endedAt - callStartedAtUnixMilliseconds) };
}

/** The calls ringing `userId`, oldest first: the incoming call card shows the first. */
export function incomingCallsOf(calls: Iterable<Call>, userId: Snowflake): Call[] {
  return [...calls]
    .filter((call) => callPhase(call, userId) === "incoming")
    .sort((left, right) => compareIds(left.message_id, right.message_id));
}

function compareIds(left: Snowflake, right: Snowflake): number {
  const difference = BigInt(left) - BigInt(right);
  return difference === 0n ? 0 : difference < 0n ? -1 : 1;
}

/** Why starting a call failed, in the words the voice panel's card uses. */
export type CallStartFailure = "already-calling" | "rate-limited" | "unreachable" | "voice-unavailable" | "other";

/**
 * `CONFLICT`: the user already has an unanswered call going on (one at a time). `NOT_FOUND`: the
 * conversation is not callable for them (a block, no shared server or friendship any more).
 */
export function callStartFailureOf(error: unknown): CallStartFailure {
  if (!(error instanceof ApiError)) {
    return "other";
  }
  switch (error.code) {
    case "CONFLICT":
      return "already-calling";
    case "RATE_LIMITED":
      return "rate-limited";
    case "NOT_FOUND":
      return "unreachable";
    case "VALIDATION_FAILED":
      return "voice-unavailable";
    default:
      return "other";
  }
}

/** Whole minutes of a call, as "Call · 12 min" counts them: a call under a minute reads 0. */
export function callDurationMinutes(durationMilliseconds: number): number {
  return Math.floor(durationMilliseconds / 60_000);
}
