import { describe, expect, it } from "vitest";

import type { Channel, Guild } from "../lib/protocol";
import { type QuickSwitcherSources, quickSwitcherResults } from "./quick-switcher";

function channel(id: string, name: string, kind: Channel["kind"] = "text"): Channel {
  return { id, guild_id: "g", kind, name, topic: "", parent_id: null, position: Number(id), permission_overwrites: [] };
}

const rustLyon: Guild = {
  id: "g",
  name: "Rust Lyon",
  owner_id: "o",
  my_permissions: "0",
  my_role_ids: [],
  roles: [],
  channels: [channel("1", "general"), channel("2", "off-topic"), channel("3", "announcements")],
};

const sources: QuickSwitcherSources = {
  guilds: [rustLyon],
  conversations: [{ id: "dm", recipients: [{ id: "ana", username: "ana", display_name: "Ana" }], last_message_id: null }],
  knownPeople: [
    { id: "me", username: "paul", display_name: "Paul" },
    { id: "zoe", username: "zoe", display_name: "Zoé" },
  ],
  currentUserId: "me",
  isFriend: () => false,
  hasUnread: (channelId) => channelId === "2",
  canSendIn: (channelId) => channelId !== "3",
};

describe("quickSwitcherResults", () => {
  it("groups channels, people (never yourself) and servers", () => {
    const groups = quickSwitcherResults("", sources);
    expect(groups.map((group) => [group.label, group.items.map((item) => item.label)])).toEqual([
      ["Channels", ["general", "off-topic", "announcements"]],
      ["People", ["Ana", "Zoé"]],
      ["Servers", ["Rust Lyon"]],
    ]);
    const offTopic = groups[0]?.items[1];
    expect(offTopic).toMatchObject({ hasUnread: true, icon: "hash", where: "Rust Lyon" });
    expect(groups[0]?.items[2]?.icon).toBe("lock");
    expect(groups[1]?.items[0]?.target).toEqual({ kind: "person", userId: "ana", conversationId: "dm" });
  });

  it("shows at most twenty of a kind before anything is typed", () => {
    const crowd = Array.from({ length: 300 }, (_, index) => ({ id: `p${index}`, username: `p${index}`, display_name: "" }));
    const groups = quickSwitcherResults("", { ...sources, knownPeople: crowd });
    expect(groups.find((group) => group.label === "People")?.items).toHaveLength(20);
    expect(quickSwitcherResults("@p", { ...sources, knownPeople: crowd }).at(0)?.items).toHaveLength(300);
  });

  it("narrows with a prefix and matches anywhere in the name", () => {
    expect(quickSwitcherResults("#TOP", sources).map((group) => group.items.map((item) => item.label))).toEqual([
      ["off-topic"],
    ]);
    expect(quickSwitcherResults("@zo", sources).map((group) => group.label)).toEqual(["People"]);
    expect(quickSwitcherResults("*lyon", sources).map((group) => group.label)).toEqual(["Servers"]);
    expect(quickSwitcherResults("nothing like it", sources)).toEqual([]);
  });

  it("ignores accents both ways, as French keyboards type them or not", () => {
    const labels = (query: string) => quickSwitcherResults(query, sources).flatMap((group) => group.items.map((item) => item.label));
    expect(labels("#GÉNÉ")).toEqual(["general"]);
    expect(labels("@zoe")).toEqual(["Zoé"]);
    expect(labels("@ZOÉ")).toEqual(["Zoé"]);
  });

  it("lists groups by name or by anyone in them, and keeps people on their own conversation", () => {
    const zoe = { id: "zoe", username: "zoe", display_name: "Zoé" };
    const ana = { id: "ana", username: "ana", display_name: "Ana" };
    const ines = { id: "ines", username: "ines", display_name: "Inès" };
    const yann = { id: "yann", username: "yann", display_name: "Yann" };
    const withGroups: QuickSwitcherSources = {
      ...sources,
      conversations: [
        { id: "trip", recipients: [ana, zoe, ines, yann], last_message_id: null, owner_id: "me", name: "Trip" },
        ...sources.conversations,
        { id: "unnamed", recipients: [zoe, ana], last_message_id: null, owner_id: "zoe" },
      ],
      isFriend: (userId) => userId === "ines",
    };
    const people = quickSwitcherResults("@", withGroups)[0]?.items ?? [];
    // Yann is met only in a group and is no friend: the instance would refuse a conversation.
    expect(people.map((item) => item.label)).toEqual(["Ana", "Inès", "Zoé", "Trip", "Zoé and Ana"]);
    expect(people[0]?.target).toEqual({ kind: "person", userId: "ana", conversationId: "dm" });
    expect(people[1]?.target).toEqual({ kind: "person", userId: "ines", conversationId: null });
    // Zoé shares a loaded server: she can be messaged without a conversation yet.
    expect(people[2]?.target).toEqual({ kind: "person", userId: "zoe", conversationId: null });
    expect(people[3]).toMatchObject({ where: "Group", icon: "users", target: { kind: "conversation", conversationId: "trip" } });
    expect(quickSwitcherResults("trip", withGroups)[0]?.items.map((item) => item.label)).toEqual(["Trip"]);
    // A group still matches by anyone in it.
    expect(quickSwitcherResults("yann", withGroups)[0]?.items.map((item) => item.label)).toEqual(["Trip"]);
  });
});
