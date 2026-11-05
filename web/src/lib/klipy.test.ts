import { afterEach, describe, expect, it, vi } from "vitest";

import {
  KLIPY_PAGE_SIZE,
  KlipyError,
  fetchKlipyPage,
  klipyItemsUrl,
  klipyListUrl,
  klipyLocaleOf,
  klipyMediaUrl,
  klipyShareUrl,
  messageGifMediaOf,
  parseKlipyItems,
  parseKlipyPage,
  triggerKlipyShare,
} from "./klipy";

const SETTINGS = { klipy_app_key: "app key/1", customer_id: "0123456789abcdef0123456789abcdef" };
const STATIC = "https://static.klipy.com/ii/935d7ab9d8c6202580a668421940ec81/14/af";

function format(name: string, width = 220, height = 180) {
  return { url: `${STATIC}/${name}`, width, height, size: 1000 };
}

/** Shaped as Klipy's documentation shows an item (docs.klipy.com, GIF Search API). */
function klipyItem(slug: string, overrides: Record<string, unknown> = {}) {
  return {
    id: 8041071659142944,
    slug,
    title: "Hello",
    type: "gif",
    blur_preview: "data:image/jpeg;base64,/9j//gAQTGF2YzU5==",
    file: {
      hd: { gif: format("hd.gif", 498, 498), webp: format("hd.webp", 498, 498) },
      md: { gif: format("md.gif"), webp: format("md.webp"), mp4: format("md.mp4"), jpg: format("md.jpg") },
      sm: { gif: format("sm.gif"), webp: format("sm.webp"), mp4: format("sm.mp4"), jpg: format("sm.jpg") },
      xs: { webp: format("xs.webp", 90, 74), jpg: format("xs.jpg", 90, 74) },
    },
    ...overrides,
  };
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("klipyMediaUrl", () => {
  it("accepts only https addresses on Klipy's media hosts", () => {
    expect(klipyMediaUrl(`${STATIC}/a.webp`)).toBe(`${STATIC}/a.webp`);
    expect(klipyMediaUrl("https://static2.klipy.com/x.gif")).toBe("https://static2.klipy.com/x.gif");
    for (const refused of [
      "http://static.klipy.com/x.gif",
      "https://static3.klipy.com/x.gif",
      "https://static.klipy.com.evil.example/x.gif",
      "https://evil.example/static.klipy.com/x.gif",
      "https://user:pass@static.klipy.com/x.gif",
      "https://static.klipy.com:8443/x.gif",
      "javascript:alert(1)",
      "/relative.gif",
      42,
      null,
    ]) {
      expect(klipyMediaUrl(refused), String(refused)).toBeNull();
    }
  });
});

describe("parseKlipyPage", () => {
  it("reads Klipy's page and keeps its order", () => {
    const page = parseKlipyPage({
      result: true,
      data: { data: [klipyItem("hello-hi-662"), klipyItem("cat-2")], current_page: 1, per_page: 24, has_next: true },
    });
    expect(page?.hasNext).toBe(true);
    expect(page?.items.map((item) => (item.kind === "gif" ? item.slug : "ad"))).toEqual(["hello-hi-662", "cat-2"]);
    const [first] = page?.items ?? [];
    expect(first).toEqual({
      kind: "gif",
      slug: "hello-hi-662",
      title: "Hello",
      tile: { url: `${STATIC}/sm.webp`, width: 220, height: 180, isVideo: false },
      still: { url: `${STATIC}/sm.jpg`, width: 220, height: 180, isVideo: false },
      messageUrl: `${STATIC}/md.webp`,
      blurPreview: "data:image/jpeg;base64,/9j//gAQTGF2YzU5==",
    });
  });

  it("refuses an answer of another shape", () => {
    expect(parseKlipyPage(null)).toBeNull();
    expect(parseKlipyPage({ result: false, data: { data: [] } })).toBeNull();
    expect(parseKlipyPage({ result: true, data: [] })).toBeNull();
    expect(parseKlipyPage({ result: true, data: { data: [], has_next: "yes" } })).toEqual({ items: [], hasNext: false });
  });
});

describe("parseKlipyItems", () => {
  it("skips an item it cannot show, never the others", () => {
    const items = parseKlipyItems([
      klipyItem("ok-1"),
      "not an object",
      klipyItem("bad slug!"),
      klipyItem("no-file", { file: null }),
      klipyItem("elsewhere", { file: { md: { webp: { url: "https://evil.example/x.webp", width: 1, height: 1 } } } }),
      klipyItem("no-size", { file: { md: { webp: { url: `${STATIC}/x.webp` } } } }),
      klipyItem("a-sticker", { type: "sticker" }),
      klipyItem("ok-2"),
    ]);
    expect(items.map((item) => (item.kind === "gif" ? item.slug : "ad"))).toEqual(["ok-1", "ok-2"]);
  });

  it("keeps an ad in its place as a tile, without its HTML", () => {
    const items = parseKlipyItems([klipyItem("a"), { type: "ad", content: "<script>x</script>", width: 300, height: 250 }, klipyItem("b")]);
    expect(items[1]).toEqual({ kind: "ad", position: 1, width: 300, height: 250 });
  });

  it("falls back from the medium WebP to the medium GIF, then a small one, for messages", () => {
    const withoutMediumWebp = klipyItem("a", { file: { md: { gif: format("md.gif") }, sm: { webp: format("sm.webp") } } });
    const smallOnly = klipyItem("b", { file: { sm: { gif: format("sm.gif") } } });
    const [first, second] = parseKlipyItems([withoutMediumWebp, smallOnly]);
    expect(first?.kind === "gif" && first.messageUrl).toBe(`${STATIC}/md.gif`);
    expect(second?.kind === "gif" && second.messageUrl).toBe(`${STATIC}/sm.gif`);
    expect(second?.kind === "gif" && second.tile.url).toBe(`${STATIC}/sm.gif`);
  });

  it("uses the small MP4 for a tile when there is no small WebP", () => {
    const [item] = parseKlipyItems([klipyItem("a", { file: { md: { webp: format("md.webp") }, sm: { mp4: format("sm.mp4") } } })]);
    expect(item?.kind === "gif" && item.tile).toEqual({ url: `${STATIC}/sm.mp4`, width: 220, height: 180, isVideo: true });
  });

  it("drops a blurred preview that is not a plain base64 picture", () => {
    const [unsafe, fine] = parseKlipyItems([
      klipyItem("a", { blur_preview: 'data:image/jpeg;base64,AAA") url("https://evil.example' }),
      klipyItem("b", { blur_preview: "data:image/png;base64,iVBORw0KGgo=" }),
    ]);
    expect(unsafe?.kind === "gif" && unsafe.blurPreview).toBeNull();
    expect(fine?.kind === "gif" && fine.blurPreview).toBe("data:image/png;base64,iVBORw0KGgo=");
  });

  it("accepts items without a type, as the items endpoint may send them", () => {
    expect(parseKlipyItems([klipyItem("a", { type: undefined })])).toHaveLength(1);
  });
});

describe("request addresses", () => {
  it("asks for trending, or a search when there is a query", () => {
    const trending = new URL(klipyListUrl(SETTINGS, "  ", 2, "fr"));
    expect(trending.origin + trending.pathname).toBe("https://api.klipy.com/api/v1/app%20key%2F1/gifs/trending");
    expect(Object.fromEntries(trending.searchParams)).toEqual({
      page: "2",
      per_page: String(KLIPY_PAGE_SIZE),
      customer_id: SETTINGS.customer_id,
      locale: "fr",
    });
    const search = new URL(klipyListUrl(SETTINGS, " chat & chien ", 1, "fr"));
    expect(search.pathname).toBe("/api/v1/app%20key%2F1/gifs/search");
    expect(search.searchParams.get("q")).toBe("chat & chien");
  });

  it("asks for items by slugs and shares by slug", () => {
    expect(new URL(klipyItemsUrl(SETTINGS, ["a-1", "b_2"])).searchParams.get("slugs")).toBe("a-1,b_2");
    expect(klipyShareUrl(SETTINGS, "hello-hi-662")).toBe("https://api.klipy.com/api/v1/app%20key%2F1/gifs/share/hello-hi-662");
  });

  it("gives Klipy a country code from the reader's locale", () => {
    expect(klipyLocaleOf("fr-FR")).toBe("fr");
    expect(klipyLocaleOf("en-GB")).toBe("gb");
    expect(klipyLocaleOf("fr-CA")).toBe("ca");
    expect(klipyLocaleOf("fr")).toBe("fr");
    expect(klipyLocaleOf("zh-Hant-TW")).toBe("zh");
    expect(klipyLocaleOf("")).toBe("us");
  });
});

describe("fetchKlipyPage", () => {
  it("says why Klipy did not answer", async () => {
    const answer = (status: number) => vi.stubGlobal("fetch", () => Promise.resolve(new Response("{}", { status })));
    const failureOf = (error: unknown) => (error instanceof KlipyError ? error.failure : String(error));
    answer(429);
    expect(await fetchKlipyPage("https://api.klipy.com/x", new AbortController().signal).catch(failureOf)).toBe("rate-limited");
    answer(403);
    expect(await fetchKlipyPage("https://api.klipy.com/x", new AbortController().signal).catch(failureOf)).toBe("refused");
    answer(200);
    expect(await fetchKlipyPage("https://api.klipy.com/x", new AbortController().signal).catch(failureOf)).toBe("failed");
    vi.stubGlobal("fetch", () => Promise.reject(new TypeError("network")));
    expect(await fetchKlipyPage("https://api.klipy.com/x", new AbortController().signal).catch(failureOf)).toBe("failed");
  });

  it("sends no cookie and no page address", async () => {
    let init: RequestInit | undefined;
    vi.stubGlobal("fetch", (_url: string, requestInit: RequestInit) => {
      init = requestInit;
      return Promise.resolve(Response.json({ result: true, data: { data: [klipyItem("a")], has_next: false } }));
    });
    const page = await fetchKlipyPage("https://api.klipy.com/x", new AbortController().signal);
    expect(page.items).toHaveLength(1);
    expect(init?.credentials).toBe("omit");
    expect(init?.referrerPolicy).toBe("no-referrer");
  });

  it("lets the caller's abort through as is", async () => {
    const controller = new AbortController();
    vi.stubGlobal("fetch", (_url: string, init: RequestInit) => {
      return new Promise((_resolve, reject) => {
        init.signal?.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
      });
    });
    const pending = fetchKlipyPage("https://api.klipy.com/x", controller.signal);
    controller.abort();
    await expect(pending).rejects.toHaveProperty("name", "AbortError");
  });
});

describe("triggerKlipyShare", () => {
  it("posts the customer id and the search, and swallows nothing silently", async () => {
    const calls: { url: string; init: RequestInit }[] = [];
    vi.stubGlobal("fetch", (url: string, init: RequestInit) => {
      calls.push({ url, init });
      return Promise.reject(new TypeError("offline"));
    });
    const debug = vi.spyOn(console, "debug").mockImplementation(() => undefined);
    triggerKlipyShare(SETTINGS, "hello-hi-662", " hello ");
    await vi.waitFor(() => expect(debug).toHaveBeenCalled());
    expect(calls[0]?.url).toBe("https://api.klipy.com/api/v1/app%20key%2F1/gifs/share/hello-hi-662");
    expect(calls[0]?.init.method).toBe("POST");
    expect(JSON.parse(String(calls[0]?.init.body))).toEqual({ customer_id: SETTINGS.customer_id, q: "hello" });
    debug.mockRestore();
  });
});

describe("messageGifMediaOf", () => {
  it("shows a message that is only a Klipy media address as the GIF", () => {
    expect(messageGifMediaOf(`  ${STATIC}/md.webp\n`)).toEqual({ url: `${STATIC}/md.webp`, isVideo: false });
    expect(messageGifMediaOf(`${STATIC}/md.GIF`)).toEqual({ url: `${STATIC}/md.GIF`, isVideo: false });
    expect(messageGifMediaOf(`${STATIC}/md.mp4`)).toEqual({ url: `${STATIC}/md.mp4`, isVideo: true });
  });

  it("keeps the link for anything more or anything else", () => {
    for (const content of [
      `look ${STATIC}/md.webp`,
      `${STATIC}/md.webp ${STATIC}/sm.webp`,
      `<${STATIC}/md.webp>`,
      `${STATIC}/page.html`,
      `${STATIC}/no-extension`,
      "https://static.example.org/a.gif",
      "https://static.klipy.com.evil.example/a.gif",
      "http://static.klipy.com/a.gif",
      "",
    ]) {
      expect(messageGifMediaOf(content), content).toBeNull();
    }
  });
});
