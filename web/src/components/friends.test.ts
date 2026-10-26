import { describe, expect, it } from "vitest";

import type { PresenceStatus, Relationship, RelationshipKind } from "../lib/protocol";
import { friendRequestOutcome, friendsGroupsOf } from "./friends";

const relationship = (id: string, kind: RelationshipKind): Relationship => ({
  user: { id, username: `user${id}`, display_name: "" },
  kind,
  since_unix_milliseconds: 0,
});

const RELATIONSHIPS = [
  relationship("1", "friend"),
  relationship("2", "friend"),
  relationship("3", "friend"),
  relationship("4", "incoming_request"),
  relationship("5", "outgoing_request"),
  relationship("6", "blocked"),
];
const ofKind = (kind: RelationshipKind) => RELATIONSHIPS.filter((candidate) => candidate.kind === kind);
const PRESENCE: Record<string, PresenceStatus> = { "1": "online", "2": "do_not_disturb", "4": "online", "6": "online" };
const presenceOf = (userId: string) => PRESENCE[userId] ?? "offline";
const idsIn = (tab: Parameters<typeof friendsGroupsOf>[0]) =>
  friendsGroupsOf(tab, ofKind, presenceOf).map((group) => [
    group.kind,
    group.relationships.map((candidate) => candidate.user.id),
  ]);

describe("friendsGroupsOf", () => {
  it("lists friends who are not offline under Online, whatever their status", () => {
    expect(idsIn("online")).toEqual([["online", ["1", "2"]]]);
  });

  it("lists every friend under All, and only friends", () => {
    expect(idsIn("all")).toEqual([["all", ["1", "2", "3"]]]);
  });

  it("lists requests received before requests sent under Pending", () => {
    expect(idsIn("pending")).toEqual([
      ["incoming", ["4"]],
      ["outgoing", ["5"]],
    ]);
  });

  it("lists blocks under Blocked, online or not", () => {
    expect(idsIn("blocked")).toEqual([["blocked", ["6"]]]);
  });

  it("leaves empty groups out, so an empty tab has no group", () => {
    expect(friendsGroupsOf("pending", () => [], presenceOf)).toEqual([]);
    expect(friendsGroupsOf("online", ofKind, () => "offline")).toEqual([]);
  });
});

describe("friendRequestOutcome", () => {
  it("tells a new request from one already waiting", () => {
    expect(friendRequestOutcome(undefined, "outgoing_request")).toBe("sent");
    expect(friendRequestOutcome("outgoing_request", "outgoing_request")).toBe("already-sent");
  });

  it("tells accepting their request from an existing friendship", () => {
    expect(friendRequestOutcome("incoming_request", "friend")).toBe("accepted");
    expect(friendRequestOutcome(undefined, "friend")).toBe("accepted");
    expect(friendRequestOutcome("friend", "friend")).toBe("already-friends");
  });
});
