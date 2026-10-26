// What the Friends space lists under each tab (DESIGN.md §5.6.1, ADR-0025), and what a friend
// request's answer means for the person who sent it.

import type { MellowClient } from "../lib/client.svelte";
import type { PresenceStatus, Relationship, RelationshipKind, Snowflake } from "../lib/protocol";

export type FriendsTab = "online" | "all" | "pending" | "blocked" | "add";

export const LIST_TABS = ["online", "all", "pending", "blocked"] as const satisfies readonly FriendsTab[];

export interface FriendsGroup {
  /** "online", "all", "incoming", "outgoing" or "blocked": names the group's header. */
  kind: "online" | "all" | "incoming" | "outgoing" | "blocked";
  relationships: Relationship[];
}

/**
 * The groups a list tab shows, empty groups left out: friends who are not offline (invisible
 * people read as offline), every friend, requests received then sent, or blocks. `relationshipsOfKind`
 * gives each kind sorted by name, as `MellowClient.relationshipsOfKind` does.
 */
export function friendsGroupsOf(
  tab: Exclude<FriendsTab, "add">,
  relationshipsOfKind: (kind: RelationshipKind) => Relationship[],
  presenceOf: (userId: Snowflake) => PresenceStatus,
): FriendsGroup[] {
  const groups: FriendsGroup[] = (() => {
    switch (tab) {
      case "online":
        return [
          {
            kind: "online",
            relationships: relationshipsOfKind("friend").filter(
              (relationship) => presenceOf(relationship.user.id) !== "offline",
            ),
          },
        ];
      case "all":
        return [{ kind: "all", relationships: relationshipsOfKind("friend") }];
      case "pending":
        return [
          { kind: "incoming", relationships: relationshipsOfKind("incoming_request") },
          { kind: "outgoing", relationships: relationshipsOfKind("outgoing_request") },
        ];
      case "blocked":
        return [{ kind: "blocked", relationships: relationshipsOfKind("blocked") }];
    }
  })();
  return groups.filter((group) => group.relationships.length > 0);
}

/** The groups of `tab` in the client's current state. */
export function friendsGroupsIn(client: MellowClient, tab: Exclude<FriendsTab, "add">): FriendsGroup[] {
  return friendsGroupsOf(
    tab,
    (kind) => client.relationshipsOfKind(kind),
    (userId) => client.presenceOf(userId),
  );
}

export type FriendRequestOutcome = "sent" | "accepted" | "already-friends" | "already-sent";

/**
 * What sending a request did, from the relationship before it and the instance's answer: the
 * instance answers an unchanged friendship or request with that same relationship.
 */
export function friendRequestOutcome(
  before: RelationshipKind | undefined,
  after: RelationshipKind,
): FriendRequestOutcome {
  if (after === "friend") {
    return before === "friend" ? "already-friends" : "accepted";
  }
  return before === "outgoing_request" ? "already-sent" : "sent";
}
