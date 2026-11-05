import { describe, expect, it } from "vitest";

import { embedPlayerKey, embedPlayers } from "./embed-players.svelte";

describe("embedPlayers (ADR-0044: one player at a time)", () => {
  const first = embedPlayerKey("800", "900");
  const second = embedPlayerKey("801", "900");

  it("keeps one player open: opening another closes the first", () => {
    embedPlayers.open(first);
    expect(embedPlayers.isOpen(first)).toBe(true);
    embedPlayers.open(second);
    expect(embedPlayers.isOpen(first)).toBe(false);
    expect(embedPlayers.isOpen(second)).toBe(true);
    embedPlayers.close(second);
  });

  it("tells one preview's players apart under two messages", () => {
    expect(first).not.toBe(second);
  });

  it("leaves the player opened since alone when a closed one closes again", () => {
    embedPlayers.open(first);
    embedPlayers.open(second);
    embedPlayers.close(first);
    expect(embedPlayers.isOpen(second)).toBe(true);
    embedPlayers.close(second);
    expect(embedPlayers.isOpen(second)).toBe(false);
  });
});
