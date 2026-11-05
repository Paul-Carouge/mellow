import { afterEach, describe, expect, it, vi } from "vitest";

import { ApiError, RestClient, uploadTimeoutMilliseconds } from "./api";

/** Records each request and answers 200 with `{}`. */
function recordFetches(): { url: string; body: unknown }[] {
  const requests: { url: string; body: unknown }[] = [];
  vi.stubGlobal("fetch", (url: string, init: RequestInit) => {
    requests.push({ url, body: init.body === undefined ? undefined : JSON.parse(String(init.body)) });
    return Promise.resolve(new Response("{}", { status: 200 }));
  });
  return requests;
}

describe("RestClient custom emoji", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("sends the picture raw with its name in the query, and renames and deletes by id", async () => {
    const requests: { url: string; method: string | undefined; body: unknown }[] = [];
    vi.stubGlobal("fetch", (url: string, init: RequestInit) => {
      requests.push({ url, method: init.method, body: init.body });
      return Promise.resolve(new Response(init.method === "DELETE" ? null : "{}", { status: init.method === "DELETE" ? 204 : 200 }));
    });
    const file = new File(["gif"], "parrot.gif");
    const rest = new RestClient();
    await rest.createEmoji("100", "party parrot", file);
    await rest.renameEmoji("100", "500", "fiesta");
    await rest.deleteEmoji("100", "500");
    await rest.listEmojis("100");
    expect(requests).toEqual([
      { url: "/api/v1/guilds/100/emojis?name=party%20parrot", method: "POST", body: file },
      { url: "/api/v1/guilds/100/emojis/500", method: "PATCH", body: JSON.stringify({ name: "fiesta" }) },
      { url: "/api/v1/guilds/100/emojis/500", method: "DELETE", body: undefined },
      { url: "/api/v1/guilds/100/emojis", method: "GET", body: undefined },
    ]);
  });
});

describe("RestClient invites", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("sends an invite's limits under the instance's names", async () => {
    const requests = recordFetches();
    await new RestClient().createInvite("100", { maxUses: 25, expiresInSeconds: 3_600 });
    expect(requests).toEqual([
      { url: "/api/v1/guilds/100/invites", body: { max_uses: 25, expires_in_seconds: 3_600 } },
    ]);
  });

  it("joins with a typed code without its spaces, and refuses anything but a plain code", async () => {
    const requests = recordFetches();
    await new RestClient().joinWithInvite(" ae8c 78a9-k2qf ");
    expect(requests.map((request) => request.url)).toEqual(["/api/v1/invites/ae8c78a9-k2qf"]);
    await expect(new RestClient().joinWithInvite("ab/../x")).rejects.toBeInstanceOf(ApiError);
    expect(requests).toHaveLength(1);
  });
});

describe("RestClient message search", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("searches a server or the conversations with the query in the body, never in the URL", async () => {
    const requests = recordFetches();
    const rest = new RestClient();
    const query = { q: 'from:ada "café au lait"', sort: "newest", offset: 25, utc_offset_minutes: -300 } as const;
    await rest.searchMessages("100", query);
    await rest.searchMessages(null, { q: "hello" });
    expect(requests).toEqual([
      { url: "/api/v1/guilds/100/messages/search", body: query },
      { url: "/api/v1/users/@me/messages/search", body: { q: "hello" } },
    ]);
  });
});

describe("RestClient read state", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("sends a read position with keepalive so a closing tab still delivers it, and nothing else", async () => {
    const keepalives: (boolean | undefined)[] = [];
    vi.stubGlobal("fetch", (_url: string, init: RequestInit) => {
      keepalives.push(init.keepalive);
      return Promise.resolve(new Response(null, { status: 204 }));
    });
    const rest = new RestClient();
    await rest.acknowledge("100", "200");
    await rest.startTyping("100");
    expect(keepalives).toEqual([true, undefined]);
  });
});

describe("RestClient profile images", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("sends an avatar's crop as a square and a banner's as a width", async () => {
    const urls: string[] = [];
    vi.stubGlobal("fetch", (url: string) => {
      urls.push(url);
      return Promise.resolve(new Response("{}", { status: 200 }));
    });
    const rest = new RestClient();
    const file = new File(["png"], "me.png");
    await rest.setProfileImage("avatar", file, { x: 10, y: 20, width: 300 });
    await rest.setProfileImage("banner", file, { x: 0, y: 40, width: 500 });
    await rest.setProfileImage("avatar", file, null);
    expect(urls).toEqual([
      "/api/v1/users/@me/avatar?crop_x=10&crop_y=20&crop_size=300",
      "/api/v1/users/@me/banner?crop_x=0&crop_y=40&crop_width=500",
      "/api/v1/users/@me/avatar",
    ]);
  });
});

describe("RestClient forwarding", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("posts a forward with empty content and its origin only, and a plain message without forward fields", async () => {
    const requests = recordFetches();
    const rest = new RestClient();
    await rest.forwardMessage("40", { channel_id: "30", message_id: "800" }, "nonce-1");
    await rest.sendMessage("40", "Regarde ça", "nonce-2");
    expect(requests).toEqual([
      { url: "/api/v1/channels/40/messages", body: { content: "", forwarded_from: { channel_id: "30", message_id: "800" }, nonce: "nonce-1" } },
      { url: "/api/v1/channels/40/messages", body: { content: "Regarde ça", reply_to_id: null, nonce: "nonce-2" } },
    ]);
  });
});

describe("RestClient uploads", () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  /** Requests that never answer: each ends only when its signal aborts. */
  function stallFetches(): AbortSignal[] {
    const signals: AbortSignal[] = [];
    vi.stubGlobal("fetch", (_url: string, init: RequestInit) => {
      const signal = init.signal;
      if (!signal) {
        return Promise.reject(new Error("the upload has no signal"));
      }
      signals.push(signal);
      return new Promise((_resolve, reject) => signal.addEventListener("abort", () => reject(signal.reason)));
    });
    return signals;
  }

  it("aborts an attachment's request when its caller aborts", async () => {
    const signals = stallFetches();
    const controller = new AbortController();
    const upload = new RestClient().uploadAttachment("40", new File(["cat"], "cat.png"), controller.signal);
    controller.abort();
    await expect(upload).rejects.toEqual(new ApiError(0, "NETWORK"));
    expect(signals.map((signal) => signal.aborted)).toEqual([true]);
  });

  it("gives up on a stalled attachment, avatar, emoji or sound at its deadline", async () => {
    vi.useFakeTimers();
    stallFetches();
    const rest = new RestClient();
    const file = new File(["x".repeat(1000)], "small.png");
    let settledCount = 0;
    const outcomes = [
      rest.uploadAttachment("40", file, new AbortController().signal),
      rest.setProfileImage("avatar", file, null),
      rest.createEmoji("100", "wave", file),
      rest.createSound("100", "boop", null, file),
    ].map((upload) =>
      upload.then(
        () => "answered",
        (error: unknown) => error,
      ).finally(() => settledCount++),
    );
    await vi.advanceTimersByTimeAsync(uploadTimeoutMilliseconds(file.size) - 1);
    expect(settledCount).toBe(0);
    await vi.advanceTimersByTimeAsync(1);
    expect(await Promise.all(outcomes)).toEqual(outcomes.map(() => new ApiError(0, "NETWORK")));
  });

  it("waits longer for a larger file", () => {
    expect(uploadTimeoutMilliseconds(8 * 1024 * 1024)).toBeGreaterThan(uploadTimeoutMilliseconds(1000));
  });
});
