import { describe, expect, it } from "vitest";

import { MAXIMUM_CUSTOM_STATUS_LENGTH } from "../lib/protocol";
import { customStatusCharactersLeft } from "./custom-status";

describe("custom status length", () => {
  it("counts the trimmed text, as the instance does", () => {
    expect(customStatusCharactersLeft("")).toBe(MAXIMUM_CUSTOM_STATUS_LENGTH);
    expect(customStatusCharactersLeft("  En réunion  ")).toBe(MAXIMUM_CUSTOM_STATUS_LENGTH - 10);
  });

  it("counts an emoji outside the basic plane as one character, not two", () => {
    expect(customStatusCharactersLeft("🎧")).toBe(MAXIMUM_CUSTOM_STATUS_LENGTH - 1);
  });

  it("goes negative past the limit", () => {
    expect(customStatusCharactersLeft("é".repeat(MAXIMUM_CUSTOM_STATUS_LENGTH))).toBe(0);
    expect(customStatusCharactersLeft("é".repeat(MAXIMUM_CUSTOM_STATUS_LENGTH + 3))).toBe(-3);
  });
});
