import { describe, expect, it } from "vitest";

import { strings } from "../i18n";
import { callDurationLabel, callMessageLine } from "./calls-view";

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
