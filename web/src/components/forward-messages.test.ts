import { describe, expect, it } from "vitest";

import { ApiError } from "../lib/api";
import { type Channel, type DirectMessageChannel, type Guild, Permission } from "../lib/protocol";
import {
  type ForwardAttempts,
  type ForwardDestinationSources,
  destinationLabel,
  destinationsMatching,
  forwardDestinations,
  forwardOriginPlace,
  forwardToEach,
  inOpeningOrder,
} from "./forward-messages";

const READ_AND_SEND = String(Permission.VIEW_CHANNEL | Permission.SEND_MESSAGES);

function channel(id: string, name: string, changes: Partial<Channel> = {}): Channel {
  return { id, guild_id: "g", kind: "text", name, topic: "", parent_id: null, position: Number(id), permission_overwrites: [], ...changes };
}

const rustLyon: Guild = {
  id: "g",
  name: "Rust Lyon",
  owner_id: "o",
  my_permissions: READ_AND_SEND,
  my_role_ids: [],
  roles: [],
  channels: [
    channel("2", "général"),
    channel("1", "annonces", {
      // @everyone (the role with the server's id) may read here, not write.
      permission_overwrites: [{ id: "g", kind: "role", allow: "0", deny: String(Permission.SEND_MESSAGES) }],
    }),
    channel("3", "Salon vocal", { kind: "voice" }),
    channel("4", "fichiers", {
      permission_overwrites: [{ id: "g", kind: "role", allow: String(Permission.ATTACH_FILES), deny: "0" }],
    }),
  ],
};

const ana = { id: "ana", username: "ana", display_name: "Ana" };
const bob = { id: "bob", username: "bob", display_name: "Bob" };
const me = { id: "me", username: "paul", display_name: "Paul" };
const withAna: DirectMessageChannel = { id: "dm-ana", recipients: [ana], last_message_id: null };
const withBob: DirectMessageChannel = { id: "dm-bob", recipients: [bob], last_message_id: null };
const weekend: DirectMessageChannel = {
  id: "group",
  recipients: [me, ana, bob],
  last_message_id: null,
  owner_id: "me",
  name: "Week-end à Lyon",
};

const sources: ForwardDestinationSources = {
  guilds: [rustLyon],
  conversations: [weekend, withAna, withBob],
  currentUserId: "me",
  isBlocked: (userId) => userId === "bob",
  hasAttachments: false,
};

describe("forwardDestinations", () => {
  it("lists conversations first, then the text channels you may write in, in the sidebar's order", () => {
    const destinations = forwardDestinations(sources);
    expect(destinations.map(destinationLabel)).toEqual(["Week-end à Lyon", "Ana", "#général", "#fichiers"]);
    expect(destinations.map((destination) => [destination.kind, destination.where])).toEqual([
      ["group", "Group"],
      ["person", "@ana"],
      ["channel", "Rust Lyon"],
      ["channel", "Rust Lyon"],
    ]);
  });

  it("leaves out a conversation with someone you blocked, a read-only channel and voice channels", () => {
    const channelIds = forwardDestinations(sources).map((destination) => destination.channelId);
    expect(channelIds).not.toContain("dm-bob");
    expect(channelIds).not.toContain("1");
    expect(channelIds).not.toContain("3");
  });

  it("asks for files permission too when the message carries files", () => {
    const channelIds = forwardDestinations({ ...sources, hasAttachments: true }).map((destination) => destination.channelId);
    // Conversations always allow files; of the server, only the channel that grants them is left.
    expect(channelIds).toEqual(["group", "dm-ana", "4"]);
  });

  it("offers everything to an administrator, whatever the overwrites", () => {
    const administrator = { ...rustLyon, my_permissions: String(Permission.ADMINISTRATOR) };
    const labels = forwardDestinations({ ...sources, guilds: [administrator], conversations: [] }).map(destinationLabel);
    expect(labels).toEqual(["#annonces", "#général", "#fichiers"]);
  });
});

describe("destinationsMatching", () => {
  const destinations = forwardDestinations(sources);
  const labels = (query: string) => destinationsMatching(destinations, query).map(destinationLabel);

  it("keeps everything for an empty search", () => {
    expect(labels("  ")).toHaveLength(destinations.length);
  });

  it("ignores case, accents and a leading # or @", () => {
    expect(labels("#GENERAL")).toEqual(["#général"]);
    expect(labels("@ana")).toEqual(["Week-end à Lyon", "Ana"]);
  });

  it("finds a channel by its server and a group by anyone in it, every word counting", () => {
    expect(labels("lyon fich")).toEqual(["#fichiers"]);
    expect(labels("bob")).toEqual(["Week-end à Lyon"]);
    expect(labels("zed")).toEqual([]);
  });
});

describe("forwardOriginPlace", () => {
  it("names a server channel with its # and its server, a conversation by its name", () => {
    expect(forwardOriginPlace("2", [rustLyon], [withAna], "me")).toEqual({ label: "#général", guildId: "g" });
    expect(forwardOriginPlace("dm-ana", [rustLyon], [withAna], "me")).toEqual({ label: "Ana", guildId: null });
  });

  it("knows nothing of a channel the reader cannot see", () => {
    expect(forwardOriginPlace("hidden", [rustLyon], [withAna], "me")).toBeNull();
  });
});

describe("inOpeningOrder", () => {
  it("keeps the rows where they were when the dialog opened, newcomers last", () => {
    const [group, anaDestination, general, files] = forwardDestinations(sources);
    if (!group || !anaDestination || !general || !files) {
      throw new Error("the fixture lists four places");
    }
    // Ana's conversation moved to the top after a forward; #général went away meanwhile.
    const now = [anaDestination, group, files];
    expect(inOpeningOrder(now, ["group", "2", "4"]).map(destinationLabel)).toEqual(["Week-end à Lyon", "#fichiers", "Ana"]);
  });
});

describe("forwardToEach", () => {
  const [group, anaDestination, general] = forwardDestinations(sources);
  if (!group || !anaDestination || !general) {
    throw new Error("the fixture lists these places");
  }

  /** Records every request; `refuse` decides which fail, by "forward:<channel>" or "note:<channel>". */
  function recordingSending(refuse: (request: string) => ApiError | null) {
    const requests: string[] = [];
    let nonceCount = 0;
    const attempt = (request: string, nonce: string) => {
      requests.push(`${request} ${nonce}`);
      const refusal = refuse(request);
      return refusal ? Promise.reject(refusal) : Promise.resolve();
    };
    return {
      requests,
      sending: {
        forward: (channelId: string, nonce: string) => attempt(`forward:${channelId}`, nonce),
        sendNote: (channelId: string, _text: string, nonce: string) => attempt(`note:${channelId}`, nonce),
        createNonce: () => `nonce${(nonceCount += 1)}`,
      },
    };
  }

  const freshAttempts = (): ForwardAttempts => ({ noncesByChannelId: new Map(), forwardedChannelIds: new Set() });

  it("reaches every place with the forward then the note, and nothing is refused", async () => {
    const { requests, sending } = recordingSending(() => null);
    const outcome = await forwardToEach([anaDestination, general], "Regarde", freshAttempts(), sending);
    expect(outcome.refused).toEqual([]);
    expect(outcome.reached.map(destinationLabel)).toEqual(["Ana", "#général"]);
    expect(requests).toEqual(["forward:dm-ana nonce1", "forward:2 nonce3", "note:dm-ana nonce2", "note:2 nonce4"]);
  });

  it("sends no note when none was written", async () => {
    const { requests, sending } = recordingSending(() => null);
    await forwardToEach([general], "", freshAttempts(), sending);
    expect(requests).toEqual(["forward:2 nonce1"]);
  });

  it("returns only the refused places, with the forward's own reasons", async () => {
    const { sending } = recordingSending((request) =>
      request === "forward:2" ? new ApiError(403, "MISSING_PERMISSIONS") : request === "forward:group" ? new ApiError(404, "NOT_FOUND") : null,
    );
    const outcome = await forwardToEach([group, anaDestination, general], "", freshAttempts(), sending);
    expect(outcome.reached.map(destinationLabel)).toEqual(["Ana"]);
    expect(outcome.refused.map((refusal) => [destinationLabel(refusal.destination), refusal.sentence])).toEqual([
      ["Week-end à Lyon", "Couldn't forward to Week-end à Lyon: the original was deleted, or you can no longer read it."],
      ["#général", "Couldn't forward to #général: you can't send messages or files there anymore."],
    ]);
  });

  it("on a second try reuses the nonces and only sends the note of a place whose forward landed", async () => {
    let isNoteRefused = true;
    const { requests, sending } = recordingSending((request) =>
      request === "note:dm-ana" && isNoteRefused ? new ApiError(503, "SERVER_BUSY") : null,
    );
    const attempts = freshAttempts();
    const first = await forwardToEach([anaDestination], "Regarde", attempts, sending);
    expect(first.reached).toEqual([]);
    expect(first.refused[0]?.sentence).toMatch(/^The message was forwarded to Ana, but your own message couldn't be sent/u);

    isNoteRefused = false;
    const second = await forwardToEach([anaDestination], "Regarde", attempts, sending);
    expect(second.reached.map(destinationLabel)).toEqual(["Ana"]);
    expect(requests).toEqual(["forward:dm-ana nonce1", "note:dm-ana nonce2", "note:dm-ana nonce2"]);
  });
});
