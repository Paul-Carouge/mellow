import { describe, expect, it } from "vitest";

import type { Guild, Member, Role } from "../lib/protocol";
import { profileFactsOf } from "./profile-facts";

function role(id: string, position: number, unicodeEmoji: string | null = null): Role {
  return { id, name: id, color: 0, position, permissions: "0", is_displayed_separately: false, unicode_emoji: unicodeEmoji };
}

const atelier: Guild = {
  id: "1",
  name: "Atelier",
  owner_id: "4194304",
  my_permissions: "0",
  my_role_ids: [],
  roles: [role("1", 0), role("10", 2, "🎨"), role("11", 5)],
  channels: [],
};

function member(id: string, roleIds: string[], joinedAt: number): Member {
  return { user: { id, username: id, display_name: id }, role_ids: roleIds, joined_at_unix_milliseconds: joinedAt };
}

describe("profile facts", () => {
  // 4194304 is 1 << 22: one millisecond after Mellow's epoch, 2026-01-01T00:00:00Z.
  const ownerId = "4194304";

  it("reads the account's creation from its id and the server's join date from the member", () => {
    const facts = profileFactsOf(ownerId, atelier, [member(ownerId, ["10", "11"], 1_790_000_000_000)]);
    expect(facts.joinedMellowAtUnixMilliseconds).toBe(1_767_225_600_001);
    expect(facts.joinedServerAtUnixMilliseconds).toBe(1_790_000_000_000);
    expect(facts.isOwner).toBe(true);
    expect(facts.roleIcon?.id).toBe("10");
  });

  it("knows nothing of a server outside one, before its members arrive or once they left", () => {
    for (const facts of [
      profileFactsOf(ownerId, null, undefined),
      profileFactsOf(ownerId, atelier, undefined),
      profileFactsOf(ownerId, atelier, [member("8388608", [], 0)]),
    ]) {
      expect(facts.member).toBeNull();
      expect(facts.isOwner).toBe(false);
      expect(facts.roleIcon).toBeNull();
      expect(facts.joinedServerAtUnixMilliseconds).toBeNull();
    }
  });
});
