import { describe, expect, it } from "vitest";

import { strings } from "../i18n";
import { callDurationLabel, callMessageLine, callProblemLook, incomingCardFocus } from "./calls-view";

describe("a call's message", () => {
  it("says how long the call lasted, under a minute, in minutes, then in hours", () => {
    expect(callDurationLabel(59_999)).toBe(strings.calls.underAMinute);
    expect(callDurationLabel(12 * 60_000)).toBe(strings.calls.minutes(12));
    expect(callDurationLabel(65 * 60_000)).toBe(strings.calls.hoursAndMinutes(1, "05"));
  });

  it("names a missed call and the caller's unanswered one apart", () => {
    expect(callMessageLine({ kind: "missed" })).toBe(strings.calls.missed);
    expect(callMessageLine({ kind: "unanswered" })).toBe(strings.calls.unanswered);
    expect(callMessageLine({ kind: "ongoing" })).toBe(strings.calls.ongoing);
  });
});

describe("the incoming call card's focus", () => {
  it("goes to Answer, stays put while typing or under Do not disturb, and rests on the card over a modal", () => {
    expect(incomingCardFocus({ isOverModal: false, isTyping: false, isQuiet: false })).toBe("answer");
    expect(incomingCardFocus({ isOverModal: true, isTyping: false, isQuiet: false })).toBe("answer");
    expect(incomingCardFocus({ isOverModal: false, isTyping: true, isQuiet: false })).toBe("none");
    expect(incomingCardFocus({ isOverModal: false, isTyping: false, isQuiet: true })).toBe("none");
    // Over a modal the browser would focus Decline: Space or Enter while typing would press it.
    expect(incomingCardFocus({ isOverModal: true, isTyping: true, isQuiet: false })).toBe("card");
    expect(incomingCardFocus({ isOverModal: true, isTyping: false, isQuiet: true })).toBe("card");
  });
});

describe("a call's problems", () => {
  it("tells a stop of the instance from nobody answering, and offers to call again for both", () => {
    const noAnswer = callProblemLook({ kind: "disconnected", channelId: "50", reason: "call_ended" }, "Ada");
    const stopped = callProblemLook({ kind: "disconnected", channelId: "50", reason: "server_stopping" }, "Ada");
    expect(noAnswer?.title).toBe(strings.calls.noAnswerTitle);
    expect(stopped?.title).toBe(strings.calls.interruptedTitle);
    expect(noAnswer?.canCallAgain && stopped?.canCallAgain).toBe(true);
  });

  it("says a call can no longer be joined in a call's words", () => {
    const refused = callProblemLook({ kind: "refused", channelId: "50", reason: "missing_permissions" }, "Ada");
    const cutOff = callProblemLook({ kind: "disconnected", channelId: "50", reason: "permissions_changed" }, "Ada");
    expect(refused?.sentence).toBe(strings.calls.cannotJoin);
    expect(cutOff?.sentence).toBe(strings.calls.cannotJoin);
    expect(refused?.canCallAgain).toBe(false);
  });

  it("leaves the voice channel's words where they fit", () => {
    expect(callProblemLook({ kind: "connection-lost", channelId: "50" }, "Ada")).toBeNull();
  });
});
