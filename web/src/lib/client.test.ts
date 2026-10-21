import { describe, expect, it } from "vitest";

import { ApiError, type RestClient } from "./api";
import { MellowClient } from "./client.svelte";
import type { Channel, Guild, Message, Ready } from "./protocol";

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
    client.pendingMessages = [{ nonce: "n", channelId: GENERAL, content: "x", replyToId: null, status: "sending" }];
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
});
