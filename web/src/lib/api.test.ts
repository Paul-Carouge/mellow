import { afterEach, describe, expect, it, vi } from "vitest";

import { ApiError, RestClient } from "./api";

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

  it("searches a server or the conversations, sending only the given parameters, encoded", async () => {
    const requests = recordFetches();
    const rest = new RestClient();
    await rest.searchMessages("100", { q: 'from:ada "café au lait" -x&y', sort: "newest", offset: 25, utc_offset_minutes: -300 });
    await rest.searchMessages(null, { q: "hello" });
    expect(requests.map((request) => request.url)).toEqual([
      "/api/v1/guilds/100/messages/search?q=from%3Aada+%22caf%C3%A9+au+lait%22+-x%26y&sort=newest&offset=25&utc_offset_minutes=-300",
      "/api/v1/users/@me/messages/search?q=hello",
    ]);
  });
});
