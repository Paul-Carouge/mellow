import { describe, expect, it } from "vitest";

import { ringtoneFor, ringtoneUrl } from "./ringtones";

describe("ringtones", () => {
  it("plays the Christmas ringtone all December and only then, by the local date", () => {
    expect(ringtoneFor(new Date(2026, 11, 1, 0, 0))).toBe("christmas");
    expect(ringtoneFor(new Date(2026, 11, 31, 23, 59))).toBe("christmas");
    expect(ringtoneFor(new Date(2026, 10, 30, 23, 59))).toBe("standard");
    expect(ringtoneFor(new Date(2027, 0, 1, 0, 0))).toBe("standard");
  });

  it("serves each ringtone from its own file", () => {
    expect(ringtoneUrl("standard")).toMatch(/ringtone.*\.opus/);
    expect(ringtoneUrl("christmas")).toMatch(/ringtone-christmas.*\.opus/);
  });
});
