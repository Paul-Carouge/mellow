import { describe, expect, it } from "vitest";

import { ApiError, type RestClient } from "./api";
import { MellowClient, errorReason } from "./client.svelte";
import type { Channel, DirectMessageChannel, Guild, Message, Ready, Relationship, Role } from "./protocol";

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

  it("keeps why the instance refused a message, for Not sent", async () => {
    const refusal = new ApiError(400, "VALIDATION_FAILED");
    const client = new MellowClient(fakeRest({ ...QUIET_REST, sendMessage: () => Promise.reject(refusal) }));
    client.applyReady(ready());
    client.selectGuild(GUILD, GENERAL);
    await client.sendMessage("hello");
    expect(client.pendingMessages[0]?.status).toBe("failed");
    expect(client.pendingMessages[0]?.failureReason).toBe(errorReason(refusal));
  });

  it("knows from Ready whether the user may create servers, and assumes so with older servers", () => {
    const client = new MellowClient(fakeRest(QUIET_REST));
    client.applyReady(ready({ may_create_guilds: false }));
    expect(client.mayCreateGuilds).toBe(false);
    client.applyReady(ready());
    expect(client.mayCreateGuilds).toBe(true);
  });

  it("opens a created text channel but stays put on a created category", async () => {
    const category: Channel = { ...channel("104"), kind: "category" };
    const text: Channel = { ...channel("105"), parent_id: "104" };
    const created = [category, text];
    const client = new MellowClient(
      fakeRest({ ...QUIET_REST, createChannel: () => Promise.resolve(created.shift()) }),
    );
    client.applyReady(ready());
    client.selectGuild(GUILD, GENERAL);
    await client.createChannel("category", "Games");
    expect(client.selectedChannelId).toBe(GENERAL);
    await client.createChannel("text", "Chess", "", "104");
    expect(client.selectedChannelId).toBe("105");
  });

  it("removes a deleted category at once and leaves its channels uncategorised", async () => {
    const category: Channel = { ...channel("104"), kind: "category" };
    const client = new MellowClient(fakeRest({ ...QUIET_REST, deleteChannel: () => Promise.resolve() }));
    client.applyReady(ready({ guilds: [guild([category, { ...channel(GENERAL), parent_id: "104" }])] }));
    await client.deleteChannel(GUILD, "104");
    expect(client.guilds[0]?.channels.map((candidate) => [candidate.id, candidate.parent_id])).toEqual([
      [GENERAL, null],
    ]);
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

  it("never brings back a message deleted while a history page was on its way", async () => {
    let answerFirstPage: (page: Message[]) => void = () => {};
    const client = new MellowClient(
      fakeRest({
        ...QUIET_REST,
        listMessages: () => new Promise<Message[]>((resolve) => (answerFirstPage = resolve)),
      }),
    );
    client.applyReady(ready());
    client.applyEvent({ t: "MESSAGE_DELETE", data: { channel_id: GENERAL, id: "301" } }, 0);
    answerFirstPage([message("300"), message("301")]);
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(client.messages(GENERAL).map((cached) => cached.id)).toEqual(["300"]);
  });

  it("drops an older page that comes back after a Ready emptied the cache, so no gap opens", async () => {
    let answerOlderPage: (page: Message[]) => void = () => {};
    const latestPage = Array.from({ length: 50 }, (_, index) => message(String(300 + index)));
    const client = new MellowClient(
      fakeRest({
        ...QUIET_REST,
        listMessages: (_channelId: string, before?: string) =>
          before ? new Promise<Message[]>((resolve) => (answerOlderPage = resolve)) : Promise.resolve(latestPage),
      }),
    );
    client.applyReady(ready());
    await new Promise((resolve) => setTimeout(resolve, 0));
    const olderLoad = client.loadOlderMessages();
    client.applyReady(ready());
    await new Promise((resolve) => setTimeout(resolve, 0));
    answerOlderPage([message("100"), message("101")]);
    await olderLoad;
    expect(client.messages(GENERAL).map((cached) => cached.id)).toEqual(latestPage.map((page) => page.id));
  });

  it("deletes in the message's own channel, and treats an already deleted message as deleted", async () => {
    const deletions: string[] = [];
    const client = new MellowClient(
      fakeRest({
        ...QUIET_REST,
        listMessages: (channelId: string) => Promise.resolve(channelId === GENERAL ? [message("300")] : []),
        deleteMessage: (channelId: string, messageId: string) => {
          deletions.push(`${channelId}/${messageId}`);
          return Promise.reject(new ApiError(404, "NOT_FOUND"));
        },
      }),
    );
    client.applyReady(ready({ guilds: [guild([channel(GENERAL), channel(RULES)])] }));
    await new Promise((resolve) => setTimeout(resolve, 0));
    const deleted = client.messages(GENERAL)[0];
    expect(deleted).toBeDefined();
    client.selectChannel(RULES);
    await client.deleteMessage(deleted as Message);
    expect(deletions).toEqual([`${GENERAL}/300`]);
    expect(client.messages(GENERAL)).toEqual([]);
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

  it("renames a server and changes its icon from GUILD_UPDATE", () => {
    const client = new MellowClient(fakeRest(QUIET_REST));
    client.applyReady(ready());
    const icon = { id: "900", is_animated: true };
    client.applyEvent({ t: "GUILD_UPDATE", data: { id: GUILD, name: "Book club", icon } }, 0);
    expect(client.selectedGuild?.name).toBe("Book club");
    expect(client.selectedGuild?.icon).toEqual(icon);
    client.applyEvent({ t: "GUILD_UPDATE", data: { id: GUILD, name: "Book club" } }, 0);
    expect(client.selectedGuild?.icon).toBeNull();
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

  it("never lets a late settings answer undo newer state from the gateway", async () => {
    const helper: Role = { id: "201", name: "Helper", color: 0, position: 1, permissions: "0" };
    const everyone: Role = { id: GUILD, name: "@everyone", color: 0, position: 0, permissions: "6" };
    const member = { user: { id: OTHER, username: "other", display_name: "Other" }, role_ids: [], joined_at_unix_milliseconds: 0 };
    const answers: (() => void)[] = [];
    const late = <T>(value: T) => new Promise<T>((resolve) => answers.push(() => resolve(value)));
    const client = new MellowClient(
      fakeRest({
        ...QUIET_REST,
        listMembers: () => Promise.resolve([member]),
        renameGuild: () => late({ ...guild(), name: "Old answer" }),
        updateRole: () => late({ ...helper, name: "Old answer" }),
        reorderRoles: () => late([everyone, helper]),
        addMemberRole: () => late(undefined),
      }),
    );
    client.applyReady(ready({ guilds: [{ ...guild(), roles: [everyone, helper] }] }));
    await new Promise((resolve) => setTimeout(resolve, 0));
    const requests = [
      client.renameServer(GUILD, "Old answer"),
      client.updateRole(GUILD, helper.id, { name: "Old answer" }),
      client.reorderRoles(GUILD, [helper.id]),
      client.setMemberRole(GUILD, OTHER, helper.id, true),
    ];
    // Newer changes reach the gateway before the answers.
    client.applyEvent({ t: "GUILD_UPDATE", data: { id: GUILD, name: "Newest" } }, 0);
    client.applyEvent({ t: "GUILD_ROLES_UPDATE", data: { guild_id: GUILD, roles: [everyone, { ...helper, name: "Newest" }] } }, 0);
    for (const answer of answers) {
      answer();
    }
    await Promise.all(requests);
    expect(client.selectedGuild?.name).toBe("Newest");
    expect(client.selectedGuild?.roles.map((role) => role.name)).toEqual(["@everyone", "Newest"]);
    expect(client.membersByGuild.get(GUILD)?.[0]?.role_ids).toEqual([]);
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

describe("unread state and mention counts", () => {
  const CONVERSATION = "500";
  const conversation = {
    id: CONVERSATION,
    recipients: [{ id: OTHER, username: "other", display_name: "Other" }],
    last_message_id: "400",
  };
  const mentioning = (id: string, channelId = RULES): Message => ({
    ...message(id, channelId),
    mention_ids: [ME],
  });
  const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

  /** GENERAL is open and read; RULES was read up to 300, has 310 and 3 unread mentions. */
  async function clientWithUnreadRules(rest: Partial<Record<keyof RestClient, unknown>> = {}) {
    const client = new MellowClient(fakeRest({ ...QUIET_REST, ...rest }));
    client.applyReady(
      ready({
        guilds: [guild([channel(GENERAL), channel(RULES)])],
        read_states: [{ channel_id: RULES, last_read_message_id: "300" }],
        direct_message_channels: [conversation],
        channel_unreads: [
          { channel_id: GENERAL, last_message_id: "290", mention_count: 0 },
          { channel_id: RULES, last_message_id: "310", mention_count: 3 },
          {
            channel_id: CONVERSATION,
            last_message_id: "400",
            mention_count: 2,
          },
        ],
      }),
    );
    await flush();
    return client;
  }

  it("show a channel never opened as unread, with its mentions, from Ready", async () => {
    const client = await clientWithUnreadRules();
    expect(client.hasUnread(RULES)).toBe(true);
    expect(client.mentionCount(RULES)).toBe(3);
    expect(client.guildMentionCount(client.guilds[0] as Guild)).toBe(3);
    expect(client.conversationsMentionCount()).toBe(2);
    expect(client.doesServerFindMentions).toBe(true);
  });

  it("keep the unread state and the counts through GUILD_CREATE and CHANNEL_UPDATE", async () => {
    const client = await clientWithUnreadRules();
    client.applyEvent({ t: "GUILD_CREATE", data: guild([channel(GENERAL), channel(RULES)]) }, 0);
    client.applyEvent({ t: "CHANNEL_UPDATE", data: { ...channel(RULES), name: "renamed" } }, 0);
    expect(client.hasUnread(RULES)).toBe(true);
    expect(client.mentionCount(RULES)).toBe(3);
  });

  it("show a channel read up to its newest message as read", async () => {
    const client = new MellowClient(fakeRest(QUIET_REST));
    client.applyReady(
      ready({
        guilds: [guild([channel(GENERAL), channel(RULES)])],
        read_states: [{ channel_id: RULES, last_read_message_id: "310" }],
        channel_unreads: [{ channel_id: RULES, last_message_id: "310", mention_count: 0 }],
      }),
    );
    expect(client.hasUnread(RULES)).toBe(false);
    client.applyEvent({ t: "MESSAGE_CREATE", data: message("320", RULES) }, 0);
    expect(client.hasUnread(RULES)).toBe(true);
    expect(client.mentionCount(RULES)).toBe(0);
  });

  it("count a new mention outside the open channel, never one in it or from the user", async () => {
    const client = await clientWithUnreadRules();
    client.applyEvent({ t: "MESSAGE_CREATE", data: mentioning("320") }, 0);
    client.applyEvent({ t: "MESSAGE_CREATE", data: mentioning("320") }, 0);
    client.applyEvent({ t: "MESSAGE_CREATE", data: message("321", RULES) }, 0);
    client.applyEvent(
      {
        t: "MESSAGE_CREATE",
        data: {
          ...mentioning("322"),
          author: { id: ME, username: "me", display_name: "Me" },
        },
      },
      0,
    );
    client.applyEvent({ t: "MESSAGE_CREATE", data: mentioning("330", GENERAL) }, 0);
    expect(client.mentionCount(RULES)).toBe(4);
    expect(client.mentionCount(GENERAL)).toBe(0);
  });

  it("count every message from someone else in a conversation", async () => {
    const client = await clientWithUnreadRules();
    client.applyEvent({ t: "MESSAGE_CREATE", data: message("410", CONVERSATION) }, 0);
    client.applyEvent(
      {
        t: "MESSAGE_CREATE",
        data: {
          ...message("411", CONVERSATION),
          author: { id: ME, username: "me", display_name: "Me" },
        },
      },
      0,
    );
    expect(client.conversationsMentionCount()).toBe(3);
  });

  it("clear the count when the channel is opened and read", async () => {
    const client = await clientWithUnreadRules({
      listMessages: () => Promise.resolve([mentioning("310")]),
    });
    client.selectChannel(RULES);
    await flush();
    expect(client.mentionCount(RULES)).toBe(0);
    expect(client.hasUnread(RULES)).toBe(false);
    expect(client.guildMentionCount(client.guilds[0] as Guild)).toBe(0);
  });

  it("follow edits that add or remove the user, and deletions, never below zero", async () => {
    const client = await clientWithUnreadRules();
    client.applyEvent({ t: "MESSAGE_CREATE", data: message("320", RULES) }, 0);
    client.applyEvent(
      {
        t: "MESSAGE_UPDATE",
        data: { ...mentioning("320"), edited_at_unix_milliseconds: 1 },
      },
      0,
    );
    expect(client.mentionCount(RULES)).toBe(4);
    client.applyEvent({ t: "MESSAGE_UPDATE", data: { ...message("320", RULES, 2) } }, 0);
    expect(client.mentionCount(RULES)).toBe(3);
    client.applyEvent(
      {
        t: "MESSAGE_UPDATE",
        data: { ...mentioning("320"), edited_at_unix_milliseconds: 3 },
      },
      0,
    );
    client.applyEvent({ t: "MESSAGE_DELETE", data: { channel_id: RULES, id: "320" } }, 0);
    expect(client.mentionCount(RULES)).toBe(3);
    // Counted in Ready only: the client cannot tell, so the count stays.
    client.applyEvent({ t: "MESSAGE_DELETE", data: { channel_id: RULES, id: "305" } }, 0);
    expect(client.mentionCount(RULES)).toBe(3);
  });

  it("count a message Ready already counted once, even when the gateway replays it", async () => {
    const client = await clientWithUnreadRules();
    // 310 was committed while the server built Ready: counted there, then replayed.
    client.applyEvent({ t: "MESSAGE_CREATE", data: mentioning("310") }, 0);
    expect(client.mentionCount(RULES)).toBe(3);
    client.applyEvent({ t: "MESSAGE_DELETE", data: { channel_id: RULES, id: "310" } }, 0);
    expect(client.mentionCount(RULES)).toBe(2);
    client.applyEvent({ t: "MESSAGE_CREATE", data: mentioning("311") }, 0);
    expect(client.mentionCount(RULES)).toBe(3);
  });

  it("keep a saturated count when a counted message goes", async () => {
    const client = new MellowClient(fakeRest(QUIET_REST));
    client.applyReady(
      ready({
        guilds: [guild([channel(GENERAL), channel(RULES)])],
        channel_unreads: [{ channel_id: RULES, last_message_id: "310", mention_count: 100 }],
      }),
    );
    await flush();
    client.applyEvent({ t: "MESSAGE_CREATE", data: mentioning("320") }, 0);
    client.applyEvent({ t: "MESSAGE_DELETE", data: { channel_id: RULES, id: "320" } }, 0);
    expect(client.mentionCount(RULES)).toBe(101);
  });

  it("leave counts alone for an older server, which highlights mentions on its own", () => {
    const client = new MellowClient(fakeRest(QUIET_REST));
    client.applyReady(ready({ direct_message_channels: [conversation] }));
    expect(client.doesServerFindMentions).toBe(false);
    expect(client.mentionCount(GENERAL)).toBe(0);
    client.applyEvent({ t: "MESSAGE_CREATE", data: message("410", CONVERSATION) }, 0);
    client.applyEvent({ t: "MESSAGE_CREATE", data: mentioning("411", GENERAL) }, 0);
    expect(client.mentionCount(CONVERSATION)).toBe(0);
    expect(client.mentionCount(GENERAL)).toBe(0);
  });
});

describe("Presence", () => {
  it("reads statuses from Ready, and every online user of an older server as online", () => {
    const client = new MellowClient(fakeRest(QUIET_REST));
    client.applyReady(
      ready({
        online_user_ids: [OTHER],
        presences: [{ user_id: OTHER, status: "do_not_disturb", custom_status: "Focus" }],
      }),
    );
    expect(client.presenceOf(OTHER)).toBe("do_not_disturb");
    expect(client.customStatusOf(OTHER)).toBe("Focus");

    client.applyReady(ready({ online_user_ids: [OTHER] }));
    expect(client.presenceOf(OTHER)).toBe("online");
    expect(client.customStatusOf(OTHER)).toBeNull();
  });

  it("follows presence updates, offline dropping the custom status", () => {
    const client = new MellowClient(fakeRest(QUIET_REST));
    client.applyReady(ready());
    expect(client.presenceOf(OTHER)).toBe("offline");
    client.applyEvent({ t: "PRESENCE_UPDATE", data: { user_id: OTHER, status: "idle", custom_status: "Lunch" } }, 0);
    expect(client.presenceOf(OTHER)).toBe("idle");
    expect(client.customStatusOf(OTHER)).toBe("Lunch");
    client.applyEvent({ t: "PRESENCE_UPDATE", data: { user_id: OTHER, status: "offline" } }, 0);
    expect(client.presenceOf(OTHER)).toBe("offline");
    expect(client.customStatusOf(OTHER)).toBeNull();
  });

  it("shows the signed-in user their own pick, invisible as offline", async () => {
    const me = ready().user;
    const client = new MellowClient(
      fakeRest({
        ...QUIET_REST,
        updatePresence: (status: string, customStatus: string | null) =>
          Promise.resolve({ ...me, status, custom_status: customStatus }),
      }),
    );
    client.user = me;
    client.applyReady(ready());
    expect(client.presenceOf(ME)).toBe("online");
    await client.updatePresence("invisible", "  Hidden  ");
    expect(client.user?.custom_status).toBe("Hidden");
    expect(client.presenceOf(ME)).toBe("offline");
    expect(client.customStatusOf(ME)).toBeNull();
    await client.updatePresence("idle", "   ");
    expect(client.presenceOf(ME)).toBe("idle");
    expect(client.user?.custom_status).toBeNull();
  });

  it("takes the signed-in user's pick from another device", () => {
    const client = new MellowClient(fakeRest(QUIET_REST));
    client.user = { ...ready().user, custom_status: "Old" };
    client.applyReady(ready());
    client.applyEvent({ t: "PRESENCE_UPDATE", data: { user_id: ME, status: "idle", custom_status: "Lunch" } }, 0);
    expect(client.user?.status).toBe("idle");
    expect(client.user?.custom_status).toBe("Lunch");
    client.applyEvent({ t: "PRESENCE_UPDATE", data: { user_id: ME, status: "offline" } }, 0);
    expect(client.user?.status).toBe("invisible");
    expect(client.user?.custom_status).toBe("Lunch");
    expect(client.presenceOf(ME)).toBe("offline");
  });
});

describe("Friends", () => {
  const relationshipWith = (id: string, kind: Relationship["kind"], name = `user${id}`): Relationship => ({
    user: { id, username: name, display_name: "" },
    kind,
    since_unix_milliseconds: 0,
  });

  it("follow Ready, gateway events and the answers to the user's own actions", async () => {
    const sentTo: string[] = [];
    const client = new MellowClient(
      fakeRest({
        ...QUIET_REST,
        sendFriendRequest: (username: string) => {
          sentTo.push(username);
          return Promise.resolve(relationshipWith("30", "outgoing_request", username));
        },
        setRelationship: (userId: string, kind: "friend" | "blocked") => Promise.resolve(relationshipWith(userId, kind)),
        removeRelationship: () => Promise.resolve(),
      }),
    );
    client.applyReady(
      ready({
        relationships: [relationshipWith("20", "incoming_request"), relationshipWith("21", "friend", "zoe"), relationshipWith("22", "friend", "anna")],
      }),
    );
    expect(client.incomingRequestCount()).toBe(1);
    expect(client.relationshipsOfKind("friend").map((relationship) => relationship.user.username)).toEqual([
      "anna",
      "zoe",
    ]);

    await client.acceptFriendRequest("20");
    await client.sendFriendRequest("  carol ");
    await client.blockUser("22");
    await client.removeRelationship("21");
    client.applyEvent({ t: "RELATIONSHIP_ADD", data: relationshipWith("40", "incoming_request") }, 0);
    client.applyEvent({ t: "RELATIONSHIP_REMOVE", data: { user_id: "30" } }, 0);
    client.applyEvent({ t: "USER_UPDATE", data: { id: "20", username: "renamed", display_name: "" } }, 0);

    expect(sentTo).toEqual(["carol"]);
    expect(client.relationshipByUserId.get("20")?.kind).toBe("friend");
    expect(client.isBlocked("22")).toBe(true);
    expect(client.relationshipByUserId.has("21")).toBe(false);
    expect(client.relationshipByUserId.has("30")).toBe(false);
    expect(client.incomingRequestCount()).toBe(1);
    expect(client.relationshipByUserId.get("20")?.user.username).toBe("renamed");
  });

  it("open at Home without a conversation, as Discord's Home opens Friends", () => {
    const client = new MellowClient(fakeRest(QUIET_REST));
    expect(client.isFriendsSpaceOpen).toBe(false);
    const conversation = { id: "500", recipients: [{ id: OTHER, username: "other", display_name: "" }], last_message_id: "501" };
    client.applyReady(ready({ guilds: [], direct_message_channels: [conversation] }));
    expect(client.selectedChannelId).toBeNull();
    expect(client.isFriendsSpaceOpen).toBe(true);
    client.selectGuild(null, "500");
    expect(client.isFriendsSpaceOpen).toBe(false);
    client.selectGuild(null, null);
    expect(client.isFriendsSpaceOpen).toBe(true);
  });

  it("stay open across a new Ready, and take over when the open conversation is gone", () => {
    const client = new MellowClient(fakeRest(QUIET_REST));
    const conversation = { id: "500", recipients: [{ id: OTHER, username: "other", display_name: "" }], last_message_id: "501" };
    client.applyReady(ready({ direct_message_channels: [conversation] }));
    expect(client.selectedGuildId).toBe(GUILD);
    client.selectGuild(null, null);
    client.applyReady(ready({ direct_message_channels: [conversation] }));
    expect(client.isFriendsSpaceOpen).toBe(true);

    client.selectGuild(null, "500");
    expect(client.isFriendsSpaceOpen).toBe(false);
    client.applyReady(ready({ direct_message_channels: [] }));
    expect(client.selectedGuildId).toBeNull();
    expect(client.isFriendsSpaceOpen).toBe(true);
  });

  it("count a removal already done elsewhere as done, and report other refusals", async () => {
    let refusal = new ApiError(404, "NOT_FOUND");
    const client = new MellowClient(
      fakeRest({ ...QUIET_REST, removeRelationship: () => Promise.reject(refusal) }),
    );
    client.applyReady(ready({ relationships: [relationshipWith("21", "friend"), relationshipWith("22", "blocked")] }));
    await client.removeRelationship("21");
    expect(client.relationshipByUserId.has("21")).toBe(false);

    refusal = new ApiError(429, "RATE_LIMITED");
    await expect(client.removeRelationship("22")).rejects.toBe(refusal);
    expect(client.isBlocked("22")).toBe(true);
  });

  it("start empty with an older server", () => {
    const client = new MellowClient(fakeRest(QUIET_REST));
    client.applyReady(ready());
    expect(client.relationshipByUserId.size).toBe(0);
  });
});

describe("Group conversations", () => {
  const group = (id: string, recipientIds: string[], name = ""): DirectMessageChannel => ({
    id,
    recipients: recipientIds.map((recipientId) => ({ id: recipientId, username: `user${recipientId}`, display_name: "" })),
    last_message_id: null,
    owner_id: ME,
    name,
  });

  it("open one person's conversation, or a new group for several, then follow its changes", async () => {
    const calls: string[] = [];
    const client = new MellowClient(
      fakeRest({
        ...QUIET_REST,
        openDirectMessage: (userId: string) => {
          calls.push(`open:${userId}`);
          return Promise.resolve({ id: "600", recipients: [], last_message_id: null });
        },
        createGroupConversation: (recipientIds: string[], name: string) => {
          calls.push(`group:${recipientIds.join(",")}:${name}`);
          return Promise.resolve(group("700", recipientIds, name));
        },
        removeGroupParticipant: () => Promise.resolve(),
      }),
    );
    client.applyReady(ready());

    await client.startConversation(["20"]);
    await client.startConversation(["20", "21"], "  Trip ");
    expect(calls).toEqual(["open:20", "group:20,21:Trip"]);
    expect(client.selectedChannelId).toBe("700");

    client.applyEvent({ t: "DIRECT_MESSAGE_CHANNEL_UPDATE", data: group("700", ["20", "21", "22"], "Trip") }, 0);
    expect(client.conversations.find((channel) => channel.id === "700")?.recipients).toHaveLength(3);

    await client.leaveGroupConversation("700");
    expect(client.conversations.some((channel) => channel.id === "700")).toBe(false);
    expect(client.selectedChannelId).toBeNull();
    expect(client.isFriendsSpaceOpen).toBe(true);
    expect(client.closedConversation).toMatchObject({ reason: "left", conversation: { id: "700", name: "Trip" } });
  });

  it("never bring back a group whose delete event came before the answer to a rename or an add", async () => {
    const answers: (() => void)[] = [];
    const client = new MellowClient(
      fakeRest({
        ...QUIET_REST,
        renameGroupConversation: (channelId: string, name: string) =>
          new Promise((resolve) => answers.push(() => resolve(group(channelId, ["20", "21"], name)))),
        addGroupParticipant: (channelId: string, userId: string) =>
          new Promise((resolve) => answers.push(() => resolve(group(channelId, ["20", "21", userId])))),
      }),
    );
    client.applyReady(ready({ direct_message_channels: [group("860", ["20", "21"])] }));
    const renaming = client.renameGroupConversation("860", "Trip");
    const adding = client.addGroupParticipant("860", "22");
    client.applyEvent({ t: "DIRECT_MESSAGE_CHANNEL_DELETE", data: { id: "860" } }, 0);
    for (const answer of answers) {
      answer();
    }
    await Promise.all([renaming, adding]);
    expect(client.conversations).toEqual([]);
  });

  it("take someone the owner removed out of the group at once", async () => {
    const client = new MellowClient(fakeRest({ ...QUIET_REST, removeGroupParticipant: () => Promise.resolve() }));
    client.applyReady(ready({ direct_message_channels: [group("850", ["20", "21", "22"], "Trip")] }));
    await client.removeGroupParticipant("850", "21");
    expect(client.conversations[0]?.recipients.map((recipient) => recipient.id)).toEqual(["20", "22"]);
    expect(client.conversations[0]?.name).toBe("Trip");
  });

  it("drop a group the user was removed from, and say so only when it was open", () => {
    const client = new MellowClient(fakeRest(QUIET_REST));
    client.applyReady(ready({ direct_message_channels: [group("800", ["20", "21"]), group("801", ["20", "22"])] }));
    client.applyEvent({ t: "DIRECT_MESSAGE_CHANNEL_DELETE", data: { id: "801" } }, 0);
    expect(client.closedConversation).toBeNull();

    client.selectGuild(null, "800");
    client.applyEvent({ t: "DIRECT_MESSAGE_CHANNEL_DELETE", data: { id: "800" } }, 0);
    expect(client.conversations).toEqual([]);
    expect(client.isFriendsSpaceOpen).toBe(true);
    expect(client.closedConversation).toMatchObject({ reason: "removed", conversation: { id: "800" } });
  });

  it("count leaving as leaving when the gateway's delete comes before the instance's answer", async () => {
    let answer: () => void = () => undefined;
    const client = new MellowClient(
      fakeRest({
        ...QUIET_REST,
        removeGroupParticipant: () => new Promise<void>((resolve) => (answer = resolve)),
      }),
    );
    client.applyReady(ready({ direct_message_channels: [group("900", ["20", "21"])] }));
    client.selectGuild(null, "900");
    const leaving = client.leaveGroupConversation("900");
    client.applyEvent({ t: "DIRECT_MESSAGE_CHANNEL_DELETE", data: { id: "900" } }, 0);
    answer();
    await leaving;
    expect(client.closedConversation?.reason).toBe("left");
  });
  it("create an invite and leave a server for the server given, not the open one", async () => {
    const invited: string[] = [];
    const left: string[] = [];
    const other = { ...guild([channel("202")]), id: "200" };
    const client = new MellowClient(
      fakeRest({
        ...QUIET_REST,
        createInvite: (guildId: string) => {
          invited.push(guildId);
          return Promise.resolve({ code: `code-${guildId}` });
        },
        leaveGuild: (guildId: string) => {
          left.push(guildId);
          return Promise.resolve();
        },
      }),
    );
    client.applyReady(ready({ guilds: [guild(), other] }));
    expect(client.selectedGuildId).toBe(GUILD);

    expect(await client.createInvite("200")).toBe("code-200");
    expect(invited).toEqual(["200"]);

    await client.leaveServer("200");
    expect(left).toEqual(["200"]);
    expect(client.guilds.map((candidate) => candidate.id)).toEqual([GUILD]);
    expect(client.selectedGuildId).toBe(GUILD);
  });

  it("leave the open server and move to another one", async () => {
    const other = { ...guild([channel("202")]), id: "200" };
    const client = new MellowClient(fakeRest({ ...QUIET_REST, leaveGuild: () => Promise.resolve() }));
    client.applyReady(ready({ guilds: [guild(), other] }));
    await client.leaveServer(GUILD);
    expect(client.selectedGuildId).toBe("200");
  });

  it("keep the server when leaving fails", async () => {
    const client = new MellowClient(fakeRest({ ...QUIET_REST, leaveGuild: () => Promise.reject(new ApiError(500, "SERVER_BUSY")) }));
    client.applyReady(ready());
    await expect(client.leaveServer(GUILD)).rejects.toBeInstanceOf(ApiError);
    expect(client.guilds).toHaveLength(1);
  });
});
