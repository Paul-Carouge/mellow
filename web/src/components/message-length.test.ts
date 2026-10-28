import { describe, expect, it } from "vitest";

import { MAXIMUM_MESSAGE_LENGTH } from "../lib/protocol";
import { messageCharactersLeft } from "./message-length";

describe("messageCharactersLeft", () => {
  it("counts trimmed characters, an emoji as one", () => {
    expect(messageCharactersLeft("  hi  ")).toBe(MAXIMUM_MESSAGE_LENGTH - 2);
    expect(messageCharactersLeft("🙂")).toBe(MAXIMUM_MESSAGE_LENGTH - 1);
  });

  it("goes negative past the instance's limit", () => {
    expect(messageCharactersLeft("x".repeat(MAXIMUM_MESSAGE_LENGTH))).toBe(0);
    expect(messageCharactersLeft("x".repeat(4_419))).toBe(-419);
  });

  it("runs out on bytes first for four-byte emoji", () => {
    // 2 001 emoji are 8 004 bytes: 4 over the byte bound while far under the character one.
    expect(messageCharactersLeft("🙂".repeat(2_001))).toBe(-4);
  });
});
