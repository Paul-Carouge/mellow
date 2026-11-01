import { describe, expect, it } from "vitest";

import type { Member } from "../lib/protocol";
import { displayName } from "./format";
import { membersSortedByDisplayName } from "./member-order";

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
