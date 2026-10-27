import { describe, expect, it } from "vitest";

import { en } from "../i18n/en";
import { fr } from "../i18n/fr";
import type { DirectMessageChannel, PublicUser } from "../lib/protocol";
import {
  blockedRecipientOf,
  conversationDisplayName,
  friendsMatching,
  isGroupConversation,
  nextOwnerAfterLeaving,
  participantCount,
  pickerRoom,
} from "./group-conversations";

function person(id: string, displayName: string, username = displayName.toLowerCase()): PublicUser {
  return { id, username, display_name: displayName };
}

const ana = person("30", "Ana");
const bob = person("12", "Bob");
const chloe = person("25", "Chloé", "chloe_b");
const david = person("9000000000000000001", "David");
const emma = person("41", "Emma");

function group(recipients: PublicUser[], name = "", ownerId = "me"): DirectMessageChannel {
  return { id: "g1", recipients, last_message_id: null, owner_id: ownerId, name };
}

describe("conversationDisplayName", () => {
  it("names a one-to-one conversation after the other person, even with a name on it", () => {
    const oneToOne: DirectMessageChannel = { id: "d1", recipients: [ana], last_message_id: null, name: "ignored" };
    expect(isGroupConversation(oneToOne)).toBe(false);
    expect(conversationDisplayName(oneToOne, "me")).toBe("Ana");
  });

  it("uses a group's own name, trimmed, when it has one", () => {
    expect(conversationDisplayName(group([ana, bob], "  Trip to Lyon "), "me")).toBe("Trip to Lyon");
  });

  it("lists up to three people, then two and how many others", () => {
    expect(conversationDisplayName(group([ana, bob]), "me")).toBe("Ana and Bob");
    expect(conversationDisplayName(group([ana, bob, chloe]), "me")).toBe("Ana, Bob and Chloé");
    expect(conversationDisplayName(group([ana, bob, chloe, emma]), "me")).toBe("Ana, Bob and 2 others");
    expect(conversationDisplayName(group([ana, bob, chloe, emma, david]), "me")).toBe("Ana, Bob and 3 others");
    expect(conversationDisplayName(group([], "  "), "me")).toBe("Just you");
  });

  it("never names the signed-in user among the people", () => {
    expect(conversationDisplayName(group([person("me", "Paul"), ana, bob]), "me")).toBe("Ana and Bob");
    expect(participantCount(group([person("me", "Paul"), ana, bob]), "me")).toBe(3);
  });

  it("words the names in French with its own plural", () => {
    expect(fr.groups.participantNames(["Ana", "Bob", "Chloé"])).toBe("Ana, Bob et Chloé");
    expect(fr.groups.participantNames(["Ana", "Bob", "Chloé", "Emma"])).toBe("Ana, Bob et 2 autres");
    expect(fr.groups.memberCount(1)).toBe("1 membre");
    expect(en.groups.memberCount(4)).toBe("4 members");
    expect(fr.groups.participantNames([])).toBe("Rien que toi");
  });
});

describe("the picker", () => {
  const friends = [ana, bob, chloe, david];

  it("finds friends by display name or username, ignoring case and accents", () => {
    expect(friendsMatching(friends, "", new Set())).toEqual(friends);
    expect(friendsMatching(friends, "CHLOE", new Set())).toEqual([chloe]);
    expect(friendsMatching(friends, "_b", new Set())).toEqual([chloe]);
    expect(friendsMatching(friends, "  bo ", new Set())).toEqual([bob]);
    expect(friendsMatching(friends, "zed", new Set())).toEqual([]);
  });

  it("leaves out people already in the group", () => {
    expect(friendsMatching(friends, "", new Set([bob.id, david.id]))).toEqual([ana, chloe]);
  });

  it("offers 9 others for a new group and what is left of 10 places for an existing one", () => {
    expect(pickerRoom(1, 0)).toBe(9);
    expect(pickerRoom(1, 9)).toBe(0);
    expect(pickerRoom(7, 1)).toBe(2);
    expect(pickerRoom(10, 0)).toBe(0);
    expect(pickerRoom(10, 3)).toBe(0);
    expect(en.groups.roomLeft(1)).toBe("You can add 1 more friend.");
    expect(fr.groups.roomLeft(2)).toBe("Tu peux encore ajouter 2 amis.");
  });
});

describe("blockedRecipientOf", () => {
  const isBlocked = (userId: string) => userId === bob.id;

  it("locks a one-to-one conversation with someone you blocked", () => {
    const withBob: DirectMessageChannel = { id: "d2", recipients: [bob], last_message_id: null };
    expect(blockedRecipientOf(withBob, "me", isBlocked)).toEqual(bob);
    expect(blockedRecipientOf({ ...withBob, recipients: [ana] }, "me", isBlocked)).toBeNull();
  });

  it("never locks a group, even with someone you blocked in it", () => {
    expect(blockedRecipientOf(group([ana, bob]), "me", isBlocked)).toBeNull();
  });
});

describe("nextOwnerAfterLeaving", () => {
  it("is the participant with the lowest id, compared as 64-bit numbers", () => {
    expect(nextOwnerAfterLeaving(group([ana, chloe, bob, david]), "me")).toEqual(bob);
    // As strings "10" would come before "9"; as doubles the last two would be equal.
    expect(nextOwnerAfterLeaving(group([person("10", "Yann"), person("9", "Zoé")]), "me")?.display_name).toBe("Zoé");
    const high = [person("9007199254740993", "Hugo"), person("9007199254740992", "Inès")];
    expect(nextOwnerAfterLeaving(group(high), "me")?.display_name).toBe("Inès");
  });

  it("is nobody when you do not own the group or are the last one in it", () => {
    expect(nextOwnerAfterLeaving(group([ana, bob], "", "30"), "me")).toBeNull();
    expect(nextOwnerAfterLeaving(group([]), "me")).toBeNull();
  });
});
