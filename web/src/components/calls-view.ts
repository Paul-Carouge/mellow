// What the call interface says (DESIGN.md §5.6.3, ADR-0062): how long a call rings, the words of
// a call's message, and who is being rung. The rules themselves are in `lib/calls.ts`.

import { strings } from "../i18n";
import { type CallMessageSummary, callDurationMinutes, ringingEndsAtUnixMilliseconds } from "../lib/calls";
import type { Call, DirectMessageChannel, PublicUser, Snowflake } from "../lib/protocol";
import { displayName, unixMillisecondsOfSnowflake } from "./format";
import { conversationDisplayName, isGroupConversation } from "./group-conversations";

const MINUTES_PER_HOUR = 60;

/** When the call stops ringing at the latest, from its message's time. */
export function ringingEndsAt(call: Call): number {
  return ringingEndsAtUnixMilliseconds(unixMillisecondsOfSnowflake(call.message_id));
}

/** A call rings until it says otherwise or its 30 seconds are over, whichever comes first. */
export function isStillRinging(call: Call, now: number): boolean {
  return call.ringing_user_ids.length > 0 && now < ringingEndsAt(call);
}

/** "moins d'une minute", "12 min", "1 h 05". */
export function callDurationLabel(durationMilliseconds: number): string {
  const minutes = callDurationMinutes(durationMilliseconds);
  if (minutes === 0) {
    return strings.calls.underAMinute;
  }
  if (minutes < MINUTES_PER_HOUR) {
    return strings.calls.minutes(minutes);
  }
  const minutesPastTheHour = String(minutes % MINUTES_PER_HOUR).padStart(2, "0");
  return strings.calls.hoursAndMinutes(Math.floor(minutes / MINUTES_PER_HOUR), minutesPastTheHour);
}

/** The words of a call's message: "Appel manqué", "Appel sans réponse", "Appel · 12 min", "Appel en cours". */
export function callMessageLine(summary: CallMessageSummary): string {
  switch (summary.kind) {
    case "ongoing":
      return strings.calls.ongoing;
    case "missed":
      return strings.calls.missed;
    case "unanswered":
      return strings.calls.unanswered;
    case "ended":
      return strings.calls.ended(callDurationLabel(summary.durationMilliseconds));
  }
}

/** Someone of the conversation, or a stand-in for an id it no longer lists. */
export function callPersonOf(conversation: DirectMessageChannel | null, userId: Snowflake): PublicUser {
  return conversation?.recipients.find((recipient) => recipient.id === userId) ?? { id: userId, username: userId, display_name: strings.voice.someone };
}

/** "Ada t'appelle", or "Ada t'appelle dans Atelier" for a group. */
export function incomingCallSentence(call: Call, conversation: DirectMessageChannel, currentUserId: Snowflake | null): string {
  const caller = displayName(callPersonOf(conversation, call.caller_id));
  return isGroupConversation(conversation)
    ? strings.calls.callsYouIn(caller, conversationDisplayName(conversation, currentUserId))
    : strings.calls.callsYou(caller);
}

/** The people being rung, named as a group's people are ("Ada, Bob et 2 autres"). */
export function ringingNames(call: Call, conversation: DirectMessageChannel | null): string {
  return strings.groups.participantNames(call.ringing_user_ids.map((userId) => displayName(callPersonOf(conversation, userId))));
}
