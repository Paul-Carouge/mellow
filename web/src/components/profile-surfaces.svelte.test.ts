import { afterEach, describe, expect, it, vi } from "vitest";

import type { UserProfile } from "../lib/protocol";
import { readRecentUserProfile } from "./profile-surfaces.svelte";

function profileOf(userId: string): UserProfile {
  return { user_id: userId, about_me: `About ${userId}`, links: [] };
}

describe("profiles read by the hover card and the modal", () => {
  afterEach(() => {
    vi.useRealTimers();
  });

  it("reads a profile once when the card then the modal show it within a minute", async () => {
    vi.useFakeTimers({ now: 1_000_000 });
    const load = vi.fn((userId: string) => Promise.resolve(profileOf(userId)));
    const fromCard = readRecentUserProfile(load, "10", "1");
    vi.setSystemTime(1_000_000 + 59_000);
    const fromModal = readRecentUserProfile(load, "10", "1");
    expect(fromModal).toBe(fromCard);
    expect(load).toHaveBeenCalledTimes(1);
    vi.setSystemTime(1_000_000 + 61_000);
    await readRecentUserProfile(load, "10", "1");
    expect(load).toHaveBeenCalledTimes(2);
  });

  it("always reads your own profile again: you may just have edited it", () => {
    const load = vi.fn((userId: string) => Promise.resolve(profileOf(userId)));
    readRecentUserProfile(load, "11", "11");
    readRecentUserProfile(load, "11", "11");
    expect(load).toHaveBeenCalledTimes(2);
  });

  it("starts afresh when someone else signs in on this page", () => {
    const load = vi.fn((userId: string) => Promise.resolve(profileOf(userId)));
    readRecentUserProfile(load, "13", "1");
    readRecentUserProfile(load, "13", "2");
    readRecentUserProfile(load, "13", "2");
    expect(load).toHaveBeenCalledTimes(2);
  });

  it("forgets a failure, so the next look asks again", async () => {
    const load = vi
      .fn<(userId: string) => Promise<UserProfile>>()
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValue(profileOf("12"));
    await expect(readRecentUserProfile(load, "12", "1")).rejects.toThrow("offline");
    await expect(readRecentUserProfile(load, "12", "1")).resolves.toEqual(profileOf("12"));
    expect(load).toHaveBeenCalledTimes(2);
  });
});
