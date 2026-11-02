import { describe, expect, it } from "vitest";

import type { Member, Role } from "../lib/protocol";
import { displayName } from "./format";
import { type MemberListGroup, memberListGroups, membersSortedByDisplayName } from "./member-order";

function member(id: string, username: string, displayNameText: string): Member {
  return { user: { id, username, display_name: displayNameText }, role_ids: [], joined_at_unix_milliseconds: 0 };
}

describe("membersSortedByDisplayName", () => {
  const members = [
    member("1", "zed", "Zoé"),
    member("2", "bob", ""),
    member("3", "eve", "élodie"),
    member("4", "ana", "Ana"),
    member("5", "al", "anatole"),
    member("6", "max", "Émile"),
  ];

  it("orders as localeCompare did, display name first, else username", () => {
    const expected = [...members].sort((left, right) => displayName(left.user).localeCompare(displayName(right.user)));
    expect(membersSortedByDisplayName(members)).toEqual(expected);
    expect(membersSortedByDisplayName(members).map((each) => displayName(each.user))).toEqual([
      "Ana",
      "anatole",
      "bob",
      "élodie",
      "Émile",
      "Zoé",
    ]);
  });

  it("leaves the given list untouched", () => {
    const before = [...members];
    membersSortedByDisplayName(members);
    expect(members).toEqual(before);
  });
});

describe("memberListGroups", () => {
  function role(id: string, position: number, isDisplayedSeparately: boolean): Role {
    return { id, name: `role ${id}`, color: 0, position, permissions: "0", is_displayed_separately: isDisplayedSeparately };
  }
  function withRoles(id: string, roleIds: string[]): Member {
    return { ...member(id, `user${id}`, `User ${id}`), role_ids: roleIds };
  }
  const everyone = role("100", 0, false);
  const helpers = role("201", 1, true);
  const quiet = role("202", 2, false);
  const admins = role("203", 3, true);
  const roles = [everyone, helpers, quiet, admins];
  const offlineUserIds = new Set(["5"]);
  const isOnline = (userId: string) => !offlineUserIds.has(userId);
  const summary = (groups: MemberListGroup[]) =>
    groups.map((group) => [group.kind === "role" ? group.role.id : group.kind, group.members.map((each) => each.user.id)]);

  it("lists online members under their highest role displayed separately, highest role first", () => {
    const members = [
      withRoles("1", []),
      withRoles("2", ["201"]),
      withRoles("3", ["201", "203"]),
      withRoles("4", ["202"]),
      withRoles("5", ["203"]),
    ];

    expect(summary(memberListGroups(members, roles, isOnline))).toEqual([
      ["203", ["3"]],
      ["201", ["2"]],
      ["online", ["1", "4"]],
      ["offline", ["5"]],
    ]);
  });

  it("keeps the given order inside each group and leaves empty groups out", () => {
    const members = [withRoles("2", ["201"]), withRoles("1", ["201"])];

    expect(summary(memberListGroups(members, roles, isOnline))).toEqual([["201", ["2", "1"]]]);
    expect(memberListGroups([], roles, isOnline)).toEqual([]);
  });

  it("is Online and Offline only when no role is displayed separately", () => {
    const members = [withRoles("1", ["202"]), withRoles("5", [])];

    expect(summary(memberListGroups(members, [everyone, quiet], isOnline))).toEqual([
      ["online", ["1"]],
      ["offline", ["5"]],
    ]);
  });
});
