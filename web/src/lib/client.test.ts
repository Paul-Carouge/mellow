import { describe, expect, it } from "vitest";

import { ApiError, type RestClient } from "./api";
import { MellowClient } from "./client.svelte";
import type { Channel, Guild, Message, Ready, Role } from "./protocol";

const ME = "1";
const OTHER = "2";
const GUILD = "100";
const GENERAL = "102";
const RULES = "103";

function channel(id: string, overwrites: Channel["permission_overwrites"] = []): Channel {
  return {
    id,
    guild_id: GUILD,
    kind: "text",
    name: `c${id}`,
    topic: "",
    parent_id: null,
    position: Number(id),
    permission_overwrites: overwrites,
  };
}

function guild(channels: Channel[] = [channel(GENERAL)]): Guild {
  return {
    id: GUILD,
    name: "Club",
    owner_id: OTHER,
    my_permissions: String((1n << 1n) | (1n << 2n) | (1n << 3n)),
    my_role_ids: [],
    roles: [],
    channels,
  };
}

function ready(overrides: Partial<Ready> = {}): Ready {
  return {
    session_id: "s",
    user: { id: ME, username: "me", display_name: "Me", email: "me@example.org", is_instance_administrator: false },
    guilds: [guild()],
    read_states: [],
    online_user_ids: [],
    direct_message_channels: [],
    ...overrides,
  };
}

function message(id: string, channelId = GENERAL, editedAt: number | null = null): Message {
  return {
    id,
    channel_id: channelId,
    author: { id: OTHER, username: "other", display_name: "Other" },
    content: `message ${id}`,
    reply_to_id: null,
    edited_at_unix_milliseconds: editedAt,
  };
}

/** Only what each test needs; every other call fails loudly. The cast is limited to this fake. */
function fakeRest(overrides: Partial<Record<keyof RestClient, unknown>>): RestClient {
  return new Proxy(overrides, {
    get(target, property) {
      if (property in target) {
        return target[property as keyof RestClient];
      }
      return () => Promise.reject(new Error(`unexpected call ${String(property)}`));
    },
  }) as unknown as RestClient;
}

const QUIET_REST = { listMessages: () => Promise.resolve([]), listMembers: () => Promise.resolve([]), acknowledge: () => Promise.resolve() };

describe("MellowClient", () => {
  it("opens the first server at the first Ready, not home", () => {
    const client = new MellowClient(fakeRest(QUIET_REST));
    client.user = ready().user;
    client.applyReady(ready());
    expect(client.selectedGuildId).toBe(GUILD);
    expect(client.selectedChannelId).toBe(GENERAL);
  });

  it("resends failed messages in order and stops at a failure that may pass", async () => {
    const sent: string[] = [];
    let isInstanceBusy = true;
    const client = new MellowClient(
      fakeRest({
        ...QUIET_REST,
        sendMessage: (_channelId: string, content: string) => {
          if (content === "second" && isInstanceBusy) {
            return Promise.reject(new ApiError(503, "SERVER_BUSY"));
          }
          sent.push(content);
          return Promise.resolve(message(String(200 + sent.length)));
        },
      }),
    );
    client.applyReady(ready());
    client.pendingMessages = ["first", "second", "third"].map((content, index) => ({
      nonce: `n${index}`,
      channelId: GENERAL,
      content,
      replyToId: null,
      attachments: [],
      status: "failed" as const,
    }));
    await client["resendFailedMessages"]();
    expect(sent).toEqual(["first"]);
    expect(client.pendingMessages.map((pending) => [pending.content, pending.status])).toEqual([
      ["second", "failed"],
      ["third", "failed"],
    ]);
    isInstanceBusy = false;
    await client["resendFailedMessages"]();
    expect(sent).toEqual(["first", "second", "third"]);
  });

  it("never discards a message while it is being sent", () => {
    const client = new MellowClient(fakeRest(QUIET_REST));
    client.pendingMessages = [{ nonce: "n", channelId: GENERAL, content: "x", replyToId: null, attachments: [], status: "sending" }];
    client.discardMessage("n");
    expect(client.pendingMessages).toHaveLength(1);
  });

  it("loads a server's history again after leaving and joining it", async () => {
    let historyRequests = 0;
    const client = new MellowClient(
      fakeRest({
        ...QUIET_REST,
        listMessages: () => {
          historyRequests += 1;
          return Promise.resolve([message("150")]);
        },
      }),
    );
    client.applyReady(ready());
    await Promise.resolve();
    client.applyEvent({ t: "GUILD_DELETE", data: { id: GUILD } }, 0);
    client.applyEvent({ t: "GUILD_CREATE", data: guild() }, 0);
    client.selectGuild(GUILD, null);
    await Promise.resolve();
    expect(historyRequests).toBe(2);
  });

  it("keeps a hidden channel hidden when a late update about it arrives", () => {
    const client = new MellowClient(fakeRest(QUIET_REST));
    client.applyReady(ready({ guilds: [guild([channel(GENERAL), channel(RULES)])] }));
    client.applyEvent({ t: "GUILD_CREATE", data: guild([channel(GENERAL)]) }, 0);
    client.applyEvent({ t: "CHANNEL_UPDATE", data: { ...channel(RULES), name: "renamed" } }, 0);
    expect(client.guilds[0]?.channels.map((candidate) => candidate.id)).toEqual([GENERAL]);
  });

  it("applies a new avatar and a removed banner from USER_UPDATE to authors and to the signed-in user", () => {
    const client = new MellowClient(fakeRest(QUIET_REST));
    client.user = { ...ready().user, banner: { id: "70", is_animated: false } };
    client.applyReady(ready({ user: client.user }));
    client.applyEvent({ t: "MESSAGE_CREATE", data: message("150") }, 0);
    const avatar = { id: "71", is_animated: true };
    client.applyEvent({ t: "USER_UPDATE", data: { id: OTHER, username: "other", display_name: "Other", avatar } }, 0);
    client.applyEvent({ t: "USER_UPDATE", data: { id: ME, username: "me", display_name: "Me" } }, 0);
    expect(client.messages(GENERAL)[0]?.author.avatar).toEqual(avatar);
    expect(client.user?.banner).toBeNull();
    expect(client.user?.email).toBe("me@example.org");
  });

  it("keeps the newer edit when an older one comes back late", () => {
    const client = new MellowClient(fakeRest(QUIET_REST));
    client.applyReady(ready());
    client.applyEvent({ t: "MESSAGE_CREATE", data: message("150") }, 0);
    client.applyEvent({ t: "MESSAGE_UPDATE", data: { ...message("150", GENERAL, 20), content: "newer" } }, 0);
    client.applyEvent({ t: "MESSAGE_UPDATE", data: { ...message("150", GENERAL, 10), content: "older" } }, 0);
    expect(client.messages(GENERAL)[0]?.content).toBe("newer");
  });

  it("reads channel overwrites: a read-only channel refuses sending", () => {
    const client = new MellowClient(fakeRest(QUIET_REST));
    const readOnly = channel(RULES, [{ id: GUILD, kind: "role", allow: "0", deny: String(1n << 2n) }]);
    client.applyReady(ready({ guilds: [guild([channel(GENERAL), readOnly])] }));
    expect(BigInt(client.myChannelPermissions(GENERAL) ?? "0") & (1n << 2n)).toBe(1n << 2n);
    expect(BigInt(client.myChannelPermissions(RULES) ?? "0") & (1n << 2n)).toBe(0n);
  });

  it("stops asking for older history after a failure until Try again", async () => {
    let olderRequests = 0;
    const client = new MellowClient(
      fakeRest({
        ...QUIET_REST,
        listMessages: (_channelId: string, before?: string) => {
          if (!before) {
            return Promise.resolve(Array.from({ length: 50 }, (_, index) => message(String(300 + index))));
          }
          olderRequests += 1;
          return Promise.reject(new ApiError(0, "NETWORK"));
        },
      }),
    );
    client.applyReady(ready());
    await new Promise((resolve) => setTimeout(resolve, 0));
    await client.loadOlderMessages();
    await client.loadOlderMessages();
    expect(olderRequests).toBe(1);
    expect(client.failedHistoryChannelIds.has(GENERAL)).toBe(true);
    client.retryLoadingHistory();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(olderRequests).toBe(2);
  });

  it("shows a new display name at once, in the user panel and on own messages", async () => {
    const client = new MellowClient(
      fakeRest({
        ...QUIET_REST,
        updateCurrentUser: (changes: { display_name?: string }) =>
          Promise.resolve({ ...ready().user, display_name: changes.display_name ?? "Me" }),
      }),
    );
    client.applyReady(ready());
    client.applyEvent({ t: "MESSAGE_CREATE", data: { ...message("150"), author: { id: ME, username: "me", display_name: "Me" } } }, 0);
    await client.updateDisplayName("  Paul  ");
    expect(client.user?.display_name).toBe("Paul");
    expect(client.messages(GENERAL)[0]?.author.display_name).toBe("Paul");
  });

  it("keeps the last read message of the visit for the New divider, until the channel is left", async () => {
    const acknowledged: string[] = [];
    const client = new MellowClient(
      fakeRest({
        ...QUIET_REST,
        listMessages: () => Promise.resolve([message("200"), message("201")]),
        acknowledge: (_channelId: string, messageId: string) => {
          acknowledged.push(messageId);
          return Promise.resolve();
        },
      }),
    );
    const start = ready({
      guilds: [guild([channel(GENERAL), channel(RULES)])],
      read_states: [{ channel_id: GENERAL, last_read_message_id: "200" }],
    });
    client.user = start.user;
    client.applyReady(start);
    await new Promise((resolve) => setTimeout(resolve, 0));

    expect(client.unreadMarkers.get(GENERAL)).toEqual({ lastReadMessageId: "200", newestAtOpenMessageId: "201" });
    expect(acknowledged).toEqual(["201"]);
    // A message arriving during the visit is read at once and moves no line.
    client.applyEvent({ t: "MESSAGE_CREATE", data: message("202") }, Date.now());
    expect(client.unreadMarkers.get(GENERAL)).toEqual({ lastReadMessageId: "200", newestAtOpenMessageId: "201" });

    client.selectChannel(RULES);
    expect(client.unreadMarkers.has(GENERAL)).toBe(false);
  });

  it("draws no New divider for a channel read up to its last message", async () => {
    const client = new MellowClient(fakeRest({ ...QUIET_REST, listMessages: () => Promise.resolve([message("200")]) }));
    const start = ready({ read_states: [{ channel_id: GENERAL, last_read_message_id: "200" }] });
    client.user = start.user;
    client.applyReady(start);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(client.unreadMarkers.has(GENERAL)).toBe(false);
  });

  it("renames a server from GUILD_UPDATE", () => {
    const client = new MellowClient(fakeRest(QUIET_REST));
    client.applyReady(ready());
    client.applyEvent({ t: "GUILD_UPDATE", data: { id: GUILD, name: "Book club" } }, 0);
    expect(client.selectedGuild?.name).toBe("Book club");
  });

  it("drops a deleted role from members and from the user's own roles", async () => {
    const helper: Role = { id: "201", name: "Helper", color: 0, position: 1, permissions: "0" };
    const everyone: Role = { id: GUILD, name: "@everyone", color: 0, position: 0, permissions: "6" };
    const member = { user: { id: OTHER, username: "other", display_name: "Other" }, role_ids: [helper.id], joined_at_unix_milliseconds: 0 };
    const client = new MellowClient(fakeRest({ ...QUIET_REST, listMembers: () => Promise.resolve([member]) }));
    client.applyReady(ready({ guilds: [{ ...guild(), roles: [everyone, helper], my_role_ids: [helper.id] }] }));
    await new Promise((resolve) => setTimeout(resolve, 0));
    client.applyEvent({ t: "GUILD_ROLES_UPDATE", data: { guild_id: GUILD, roles: [everyone] } }, 0);
    expect(client.selectedGuild?.my_role_ids).toEqual([]);
    expect(client.membersByGuild.get(GUILD)?.[0]?.role_ids).toEqual([]);
  });

  it("puts a created role above @everyone at once, and once only when the gateway came first", async () => {
    const everyone: Role = { id: GUILD, name: "@everyone", color: 0, position: 0, permissions: "6" };
    const helper: Role = { id: "201", name: "Helper", color: 0, position: 1, permissions: "0" };
    const created: Role = { id: "202", name: "new role", color: 0, position: 1, permissions: "0" };
    const client = new MellowClient(fakeRest({ ...QUIET_REST, createRole: () => Promise.resolve(created) }));
    client.applyReady(ready({ guilds: [{ ...guild(), roles: [everyone, helper] }] }));
    await client.createRole(GUILD, "new role");
    const positions = () => client.selectedGuild?.roles.map((role) => [role.id, role.position]);
    expect(positions()).toEqual([
      [GUILD, 0],
      ["201", 2],
      ["202", 1],
    ]);
    await client.createRole(GUILD, "new role");
    expect(positions()).toHaveLength(3);
  });
});

describe("reactions and pins", () => {
  async function clientWithHistory(rest: Partial<Record<keyof RestClient, unknown>> = {}) {
    const client = new MellowClient(
      fakeRest({ ...QUIET_REST, listMessages: () => Promise.resolve([message("200")]), ...rest }),
    );
    const start = ready();
    client.user = start.user;
    client.applyReady(start);
    await new Promise((resolve) => setTimeout(resolve, 0));
    return client;
  }
  const reactionsOf = (client: MellowClient) => client.messages(GENERAL)[0]?.reactions ?? [];
  const reaction = (userId: string, emoji = "👍") => ({ channel_id: GENERAL, message_id: "200", user_id: userId, emoji });

  it("count reactions from events, with `me` for the user's own", async () => {
    const client = await clientWithHistory();
    client.applyEvent({ t: "MESSAGE_REACTION_ADD", data: reaction(OTHER) }, 0);
    client.applyEvent({ t: "MESSAGE_REACTION_ADD", data: reaction(ME) }, 0);
    client.applyEvent({ t: "MESSAGE_REACTION_ADD", data: reaction(OTHER, "🔥") }, 0);
    expect(reactionsOf(client)).toEqual([
      { emoji: "👍", count: 2, me: true },
      { emoji: "🔥", count: 1, me: false },
    ]);
    client.applyEvent({ t: "MESSAGE_REACTION_REMOVE", data: reaction(ME) }, 0);
    client.applyEvent({ t: "MESSAGE_REACTION_REMOVE_EMOJI", data: { channel_id: GENERAL, message_id: "200", emoji: "🔥" } }, 0);
    expect(reactionsOf(client)).toEqual([{ emoji: "👍", count: 1, me: false }]);
    client.applyEvent({ t: "MESSAGE_REACTION_REMOVE_ALL", data: { channel_id: GENERAL, message_id: "200" } }, 0);
    expect(reactionsOf(client)).toEqual([]);
  });

  it("show the user's reaction at once, count its event once, and undo it if refused", async () => {
    let refuse = false;
    const client = await clientWithHistory({
      addReaction: () => (refuse ? Promise.reject(new ApiError(403, "MISSING_PERMISSIONS")) : Promise.resolve()),
      removeOwnReaction: () => Promise.resolve(),
    });
    const adding = client.toggleReaction(GENERAL, "200", "❤️");
    expect(reactionsOf(client)).toEqual([{ emoji: "❤", count: 1, me: true }]);
    await adding;
    client.applyEvent({ t: "MESSAGE_REACTION_ADD", data: reaction(ME, "❤") }, 0);
    expect(reactionsOf(client)).toEqual([{ emoji: "❤", count: 1, me: true }]);

    await client.toggleReaction(GENERAL, "200", "❤");
    expect(reactionsOf(client)).toEqual([]);

    refuse = true;
    const refused = client.toggleReaction(GENERAL, "200", "👍");
    expect(reactionsOf(client)).toEqual([{ emoji: "👍", count: 1, me: true }]);
    await expect(refused).rejects.toBeInstanceOf(ApiError);
    expect(reactionsOf(client)).toEqual([]);
  });

  it("ignore a second click while the first is on its way", async () => {
    let finish: () => void = () => undefined;
    let calls = 0;
    const client = await clientWithHistory({
      addReaction: () => {
        calls += 1;
        return new Promise<void>((resolve) => (finish = resolve));
      },
    });
    const first = client.toggleReaction(GENERAL, "200", "👍");
    await client.toggleReaction(GENERAL, "200", "👍");
    finish();
    await first;
    expect(calls).toBe(1);
    expect(reactionsOf(client)).toEqual([{ emoji: "👍", count: 1, me: true }]);
  });

  it("keep loaded pins in step with pins, unpins, edits and deletions", async () => {
    const pinned = { ...message("150"), pinned: true };
    const client = await clientWithHistory({
      listPins: () => Promise.resolve([pinned]),
      pinMessage: () => Promise.resolve(),
    });
    await client.loadPins(GENERAL);
    expect(client.pinnedMessages(GENERAL).map((candidate) => candidate.id)).toEqual(["150"]);

    await client.setPinned(message("200"), true);
    expect(client.messages(GENERAL)[0]?.pinned).toBe(true);
    expect(client.pinnedMessages(GENERAL).map((candidate) => candidate.id)).toEqual(["200", "150"]);

    client.applyEvent({ t: "MESSAGE_UPDATE", data: { ...pinned, content: "edited" } }, 0);
    expect(client.pinnedMessages(GENERAL)[1]?.content).toBe("edited");
    client.applyEvent({ t: "MESSAGE_UPDATE", data: { ...message("200"), pinned: false } }, 0);
    client.applyEvent({ t: "MESSAGE_DELETE", data: { channel_id: GENERAL, id: "150" } }, 0);
    expect(client.pinnedMessages(GENERAL)).toEqual([]);
    expect(client.pinsByChannel.get(GENERAL)?.status).toBe("loaded");
  });

  it("tell a failed pins load, and load again on Try again", async () => {
    let fail = true;
    const client = await clientWithHistory({
      listPins: () => (fail ? Promise.reject(new ApiError(0, "NETWORK")) : Promise.resolve([])),
    });
    await client.loadPins(GENERAL);
    expect(client.pinsByChannel.get(GENERAL)?.status).toBe("failed");
    fail = false;
    await client.loadPins(GENERAL, true);
    expect(client.pinsByChannel.get(GENERAL)).toEqual({ status: "loaded", messages: [] });
  });
});

describe("reaction and pin edge cases", () => {
  async function clientWithHistory(rest: Partial<Record<keyof RestClient, unknown>> = {}) {
    const client = new MellowClient(
      fakeRest({ ...QUIET_REST, listMessages: () => Promise.resolve([message("200")]), ...rest }),
    );
    const start = ready();
    client.user = start.user;
    client.applyReady(start);
    await new Promise((resolve) => setTimeout(resolve, 0));
    return client;
  }
  const reaction = (userId: string, emoji = "👍") => ({ channel_id: GENERAL, message_id: "200", user_id: userId, emoji });

  it("put back a reaction whose removal the instance refused", async () => {
    const client = await clientWithHistory({
      removeOwnReaction: () => Promise.reject(new ApiError(503, "SERVER_BUSY")),
    });
    client.applyEvent({ t: "MESSAGE_REACTION_ADD", data: reaction(ME) }, 0);
    const removing = client.toggleReaction(GENERAL, "200", "👍");
    expect(client.messages(GENERAL)[0]?.reactions).toEqual([]);
    await expect(removing).rejects.toBeInstanceOf(ApiError);
    expect(client.messages(GENERAL)[0]?.reactions).toEqual([{ emoji: "👍", count: 1, me: true }]);
  });

  it("ignore the removal of the user's own reaction when it is not counted", async () => {
    const client = await clientWithHistory();
    client.applyEvent({ t: "MESSAGE_REACTION_ADD", data: reaction(OTHER) }, 0);
    client.applyEvent({ t: "MESSAGE_REACTION_REMOVE", data: reaction(ME) }, 0);
    expect(client.messages(GENERAL)[0]?.reactions).toEqual([{ emoji: "👍", count: 1, me: false }]);
  });

  it("count reactions on loaded pins too, and keep them when a pin arrives by MESSAGE_UPDATE", async () => {
    const client = await clientWithHistory({ listPins: () => Promise.resolve([]) });
    await client.loadPins(GENERAL);
    client.applyEvent({ t: "MESSAGE_REACTION_ADD", data: reaction(OTHER) }, 0);
    client.applyEvent({ t: "MESSAGE_UPDATE", data: { ...message("200"), pinned: true } }, 0);
    expect(client.pinnedMessages(GENERAL)[0]?.reactions).toEqual([{ emoji: "👍", count: 1, me: false }]);
    client.applyEvent({ t: "MESSAGE_REACTION_ADD", data: reaction(ME) }, 0);
    expect(client.pinnedMessages(GENERAL)[0]?.reactions).toEqual([{ emoji: "👍", count: 2, me: true }]);
    expect(client.messages(GENERAL)[0]?.reactions).toEqual([{ emoji: "👍", count: 2, me: true }]);
  });

  it("ask for the pins again when one changed while they were loading", async () => {
    const answers = [[], [{ ...message("200"), pinned: true }]];
    let requests = 0;
    let answerFirst: () => void = () => undefined;
    const client = await clientWithHistory({
      listPins: () => {
        const answer = answers[requests] ?? [];
        requests += 1;
        return requests === 1
          ? new Promise((resolve) => (answerFirst = () => resolve(answer)))
          : Promise.resolve(answer);
      },
    });
    const loading = client.loadPins(GENERAL);
    client.applyEvent({ t: "MESSAGE_UPDATE", data: { ...message("200"), pinned: true } }, 0);
    answerFirst();
    await loading;
    expect(requests).toBe(2);
    expect(client.pinnedMessages(GENERAL).map((pinned) => pinned.id)).toEqual(["200"]);
  });
});
