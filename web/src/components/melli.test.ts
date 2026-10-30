import { describe, expect, it } from "vitest";

import { homeMelliState } from "./melli";

describe("homeMelliState", () => {
  it("shows a lost connection before anything else", () => {
    expect(homeMelliState("reconnecting", true, true)).toBe("connection-lost");
    expect(homeMelliState("connecting", true, true)).toBe("connecting");
  });

  it("stays quiet in Do not disturb, even with something unread", () => {
    expect(homeMelliState("connected", true, true)).toBe("do-not-disturb");
    expect(homeMelliState("connected", false, true)).toBe("do-not-disturb");
  });

  it("is happy about unread conversations or mentions once connected", () => {
    expect(homeMelliState("connected", true, false)).toBe("notification");
  });

  it("idles otherwise", () => {
    expect(homeMelliState("connected", false, false)).toBe("idle");
  });
});
