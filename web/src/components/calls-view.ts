// What the call interface says (DESIGN.md §5.6.3, ADR-0062): where the incoming card puts focus,
// the words of a call's message and of a call's problems, and who is being rung. The rules
// themselves are in `lib/calls.ts`.

import { strings } from "../i18n";
import { type CallMessageSummary, callDurationMinutes } from "../lib/calls";
import type { Call, DirectMessageChannel, PublicUser, Snowflake } from "../lib/protocol";
import type { VoiceProblem } from "../lib/voice-session.svelte";
import { displayName } from "./format";
import { conversationDisplayName, isGroupConversation } from "./group-conversations";

const MINUTES_PER_HOUR = 60;

/** Where focus goes when the incoming call card shows. */
export type IncomingCardFocus = "answer" | "card" | "none";

/**
 * Answer, unless the reader is typing (Enter would answer by accident) or under Do not disturb.
 * Then focus stays where it was; over a modal it cannot (the page under it is inert, and the
 * browser would focus Decline), so it rests on the card itself, where Space and Enter press nothing.
 */
export function incomingCardFocus(facts: { isOverModal: boolean; isTyping: boolean; isQuiet: boolean }): IncomingCardFocus {
  if (!facts.isTyping && !facts.isQuiet) {
    return "answer";
  }
  return facts.isOverModal ? "card" : "none";
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

/** A voice problem's card, in a call's own words. */
export interface CallProblemLook {
  readonly title: string;
  readonly sentence: string;
  readonly tone: "error" | "notice";
  /** "Rappeler": calling again may work. */
  readonly canCallAgain: boolean;
  readonly icon: "phone-x" | "warning-circle";
}

/**
 * A voice problem in a call (`place` is the conversation's name), when a call says it otherwise
 * than a voice channel: nobody answered, the instance stopped, the conversation no longer open to
 * the reader. `null`: the voice channel's words fit.
 */
export function callProblemLook(problem: VoiceProblem, place: string): CallProblemLook | null {
  if (problem.kind === "disconnected") {
    switch (problem.reason) {
      case "call_ended":
        return { title: strings.calls.noAnswerTitle, sentence: strings.calls.noAnswer, tone: "notice", canCallAgain: true, icon: "phone-x" };
      case "server_stopping":
        return { title: strings.calls.interruptedTitle, sentence: strings.calls.interrupted, tone: "notice", canCallAgain: true, icon: "phone-x" };
      case "permissions_changed":
        return { title: strings.voice.disconnectedFrom(place), sentence: strings.calls.cannotJoin, tone: "error", canCallAgain: false, icon: "warning-circle" };
      default:
        return null;
    }
  }
  if (problem.kind === "refused" && problem.reason === "missing_permissions") {
    return { title: strings.voice.couldNotJoin(place), sentence: strings.calls.cannotJoin, tone: "error", canCallAgain: false, icon: "warning-circle" };
  }
  return null;
}
