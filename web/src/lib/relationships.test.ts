import { describe, expect, it } from "vitest";

import { ApiError, type FriendRequestTarget } from "./api";
import type { Relationship, RelationshipKind } from "./protocol";
import { Relationships } from "./relationships.svelte";

function relationship(id: string, displayName: string, kind: RelationshipKind): Relationship {
  return { user: { id, username: displayName.toLowerCase(), display_name: displayName }, kind, since_unix_milliseconds: 0 };
}

const ZOE = relationship("2", "Zoé", "friend");
const ANA = relationship("3", "Ana", "friend");
const BOB = relationship("4", "Bob", "incoming_request");

type Rest = ConstructorParameters<typeof Relationships>[0];

function relationships(rest: Partial<Rest> = {}): Relationships {
  return new Relationships({
    sendFriendRequest: () => Promise.reject(new Error("unexpected sendFriendRequest")),
    setRelationship: () => Promise.reject(new Error("unexpected setRelationship")),
    removeRelationship: () => Promise.resolve(),
    ...rest,
  });
}

describe("Relationships", () => {
  it("lists each kind by name, and counts the requests to answer", () => {
    const book = relationships();
    book.replaceAll([ZOE, BOB, ANA]);
    expect(book.ofKind("friend")).toEqual([ANA, ZOE]);
    expect(book.incomingRequestCount()).toBe(1);
    book.replaceAll(undefined);
    expect(book.byUserId.size).toBe(0);
  });

  it("follows the gateway's adds, removes and profile changes", () => {
    const book = relationships();
    book.apply(ZOE);
    book.apply({ ...ZOE, kind: "blocked" });
    expect(book.isBlocked(ZOE.user.id)).toBe(true);
    book.updateUser(ZOE.user.id, (user) => ({ ...user, display_name: "Zoé B." }));
    book.updateUser("999", () => {
      throw new Error("no relationship, no update");
    });
    expect(book.byUserId.get(ZOE.user.id)?.user.display_name).toBe("Zoé B.");
    book.forget(ZOE.user.id);
    expect(book.isBlocked(ZOE.user.id)).toBe(false);
  });

  it("applies the instance's answer at once, with the name or friend code trimmed", async () => {
    const asked: FriendRequestTarget[] = [];
    const book = relationships({
      sendFriendRequest: (target) => {
        asked.push(target);
        return Promise.resolve({ ...BOB, kind: "outgoing_request" });
      },
      setRelationship: (userId, kind) => Promise.resolve({ ...BOB, user: { ...BOB.user, id: userId }, kind }),
    });
    await book.sendFriendRequest({ username: "  bob " });
    await book.sendFriendRequest({ friend_code: " ld0a-0da8 " });
    expect(asked).toEqual([{ username: "bob" }, { friend_code: "ld0a-0da8" }]);
    expect(book.ofKind("outgoing_request")).toHaveLength(1);
    await book.acceptFriendRequest(BOB.user.id);
    expect(book.byUserId.get(BOB.user.id)?.kind).toBe("friend");
    await book.blockUser(BOB.user.id);
    expect(book.isBlocked(BOB.user.id)).toBe(true);
  });

  it("counts a relationship already gone as removed, and keeps it on any other failure", async () => {
    const gone = relationships({ removeRelationship: () => Promise.reject(new ApiError(404, "NOT_FOUND")) });
    gone.apply(ZOE);
    await gone.removeRelationship(ZOE.user.id);
    expect(gone.byUserId.has(ZOE.user.id)).toBe(false);

    const failing = relationships({ removeRelationship: () => Promise.reject(new Error("offline")) });
    failing.apply(ZOE);
    await expect(failing.removeRelationship(ZOE.user.id)).rejects.toThrow("offline");
    expect(failing.byUserId.has(ZOE.user.id)).toBe(true);
  });
});
