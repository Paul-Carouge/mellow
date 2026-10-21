import { describe, expect, it } from "vitest";

import { homeMelliState } from "./melli";

describe("homeMelliState", () => {
  it("shows a lost connection before unread messages", () => {
    expect(homeMelliState("reconnecting", true)).toBe("connection-lost");
    expect(homeMelliState("connecting", true)).toBe("connecting");
  });

  it("is happy about unread direct messages once connected", () => {
    expect(homeMelliState("connected", true)).toBe("notification");
  });

  it("idles otherwise", () => {
    expect(homeMelliState("connected", false)).toBe("idle");
  });
});
