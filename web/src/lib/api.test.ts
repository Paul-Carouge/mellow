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
