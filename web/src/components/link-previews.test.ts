import { describe, expect, it } from "vitest";

import type { LinkPreview } from "../lib/protocol";
import {
  knownLinkEmbedOf,
  linkHostName,
  linkPreviewBarColor,
  linkPreviewImageUrl,
  linkPreviewLayout,
  thumbnailWidth,
  youTubeEmbedUrl,
  youTubeEmbedOf,
} from "./link-previews";

const ARTICLE: LinkPreview = { id: "900", url: "https://example.org/article", title: "An article", description: "What it says" };

const VIDEO = { videoId: "dQw4w9WgXcQ", playlistId: null, startSeconds: null, isShort: false };
const SHORT = { ...VIDEO, videoId: "a_b-C1d2E3f", isShort: true };
const PLAYLIST_ID = "PLFgquLnL59alCl_2TQvOiD5Vgm1hCaGSI";

describe("youTubeEmbedOf", () => {
  it.each([
    ["https://www.youtube.com/watch?v=dQw4w9WgXcQ", VIDEO],
    ["https://m.youtube.com/watch?feature=share&v=dQw4w9WgXcQ", VIDEO],
    ["http://WWW.YouTube.com/watch?v=dQw4w9WgXcQ", VIDEO],
    ["https://youtu.be/dQw4w9WgXcQ?si=abc", VIDEO],
    ["https://music.youtube.com/watch?v=dQw4w9WgXcQ", VIDEO],
    ["https://www.youtube.com/live/dQw4w9WgXcQ?si=abc", VIDEO],
    ["https://www.youtube.com/embed/dQw4w9WgXcQ", VIDEO],
    ["https://www.youtube.com/v/dQw4w9WgXcQ", VIDEO],
    ["https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ", VIDEO],
    ["https://youtube-nocookie.com/embed/dQw4w9WgXcQ", VIDEO],
    ["https://www.youtube.com/shorts/a_b-C1d2E3f", SHORT],
    ["https://youtube.com/shorts/a_b-C1d2E3f?si=Xy-9_aBcDeFgHiJk", SHORT],
    ["https://m.youtube.com/shorts/a_b-C1d2E3f", SHORT],
  ])("reads %s", (link, expected) => {
    expect(youTubeEmbedOf(link)).toEqual(expected);
  });

  it.each([
    ["t=90", 90],
    ["t=90s", 90],
    ["t=2m", 120],
    ["t=1h", 3600],
    ["t=1h2m3s", 3723],
    ["t=2m5", 125],
    ["start=42", 42],
    ["t=10&start=42", 10],
  ])("starts at %s", (query, seconds) => {
    expect(youTubeEmbedOf(`https://www.youtube.com/watch?v=dQw4w9WgXcQ&${query}`)?.startSeconds).toBe(seconds);
    expect(youTubeEmbedOf(`https://youtu.be/dQw4w9WgXcQ?${query}`)?.startSeconds).toBe(seconds);
  });

  it.each(["t=", "t=0", "t=0s", "t=abc", "t=1x", "t=-5", "t=s", "t=1m2h", "t=12345678", "start=4.5", "start=1e3", "start="])(
    "starts at the beginning with %s",
    (query) => {
      expect(youTubeEmbedOf(`https://www.youtube.com/watch?v=dQw4w9WgXcQ&${query}`)).toEqual(VIDEO);
    },
  );

  it("keeps a public playlist and drops the ones the embedded player cannot show", () => {
    expect(youTubeEmbedOf(`https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=${PLAYLIST_ID}`)?.playlistId).toBe(PLAYLIST_ID);
    expect(youTubeEmbedOf(`https://youtu.be/dQw4w9WgXcQ?list=${PLAYLIST_ID}&t=5`)).toEqual({
      ...VIDEO,
      playlistId: PLAYLIST_ID,
      startSeconds: 5,
    });
    for (const list of ["RD", "RDdQw4w9WgXcQ", "LL", "WL", "PL<script>xxxxxxx", `${PLAYLIST_ID}&`, "x".repeat(65)]) {
      expect(youTubeEmbedOf(`https://music.youtube.com/watch?v=dQw4w9WgXcQ&list=${encodeURIComponent(list)}`)).toEqual(VIDEO);
    }
  });

  it("refuses ids of the wrong length or with other characters", () => {
    expect(youTubeEmbedOf("https://www.youtube.com/watch?v=dQw4w9WgXc")).toBeNull();
    expect(youTubeEmbedOf("https://www.youtube.com/watch?v=dQw4w9WgXcQQ")).toBeNull();
    expect(youTubeEmbedOf("https://youtu.be/dQw4w9WgX%2F")).toBeNull();
    expect(youTubeEmbedOf("https://www.youtube.com/watch?v=dQw4w9WgX.Q")).toBeNull();
    expect(youTubeEmbedOf("https://www.youtube.com/watch")).toBeNull();
    expect(youTubeEmbedOf("https://www.youtube.com/embed/")).toBeNull();
  });

  it("ignores other YouTube pages and look-alike hosts", () => {
    expect(youTubeEmbedOf("https://www.youtube.com/@channel")).toBeNull();
    expect(youTubeEmbedOf("https://www.youtube.com/playlist?list=PL123")).toBeNull();
    expect(youTubeEmbedOf("https://www.youtube.com/shorts/dQw4w9WgXcQ/extra")).toBeNull();
    expect(youTubeEmbedOf("https://www.youtube.com/channel/UCuAXFkgsw1L7xaCfnd5JJOw")).toBeNull();
    expect(youTubeEmbedOf("https://youtu.be/dQw4w9WgXcQ/extra")).toBeNull();
    expect(youTubeEmbedOf("https://youtu.be/shorts/dQw4w9WgXcQ")).toBeNull();
    expect(youTubeEmbedOf("https://notyoutube.com/watch?v=dQw4w9WgXcQ")).toBeNull();
    expect(youTubeEmbedOf("https://www.youtube.com.evil.example/watch?v=dQw4w9WgXcQ")).toBeNull();
    expect(youTubeEmbedOf("https://youtube-nocookie.com.evil.example/embed/dQw4w9WgXcQ")).toBeNull();
    expect(youTubeEmbedOf("https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC")).toBeNull();
    expect(youTubeEmbedOf("javascript:alert(1)//youtu.be/dQw4w9WgXcQ")).toBeNull();
    expect(youTubeEmbedOf("https://user@youtu.be/dQw4w9WgXcQ")).toBeNull();
  });
});

describe("linkPreviewLayout", () => {
  it("shows a wide picture large and a small or square one as a thumbnail", () => {
    expect(linkPreviewLayout({ ...ARTICLE, image: { width: 800, height: 420 } }, null)).toBe("large-picture");
    expect(linkPreviewLayout({ ...ARTICLE, image: { width: 800, height: 600 } }, null)).toBe("thumbnail");
    expect(linkPreviewLayout({ ...ARTICLE, image: { width: 200, height: 100 } }, null)).toBe("thumbnail");
    expect(linkPreviewLayout({ ...ARTICLE, image: { width: 300, height: 400 } }, null)).toBe("thumbnail");
  });

  it("shows a link that is itself a picture as the picture alone", () => {
    expect(linkPreviewLayout({ id: "1", url: "https://example.org/cat.png", image: { width: 64, height: 64 } }, null)).toBe(
      "picture-only",
    );
  });

  it("keeps a card without picture, and drops a preview with nothing to show", () => {
    expect(linkPreviewLayout(ARTICLE, null)).toBe("text-only");
    expect(linkPreviewLayout({ id: "1", url: "https://example.org/", description: "Only words" }, null)).toBe("text-only");
    expect(linkPreviewLayout({ id: "1", url: "https://example.org/", site_name: "Example" }, null)).toBe("none");
    expect(linkPreviewLayout({ ...ARTICLE, image: { width: 0, height: 10 } }, null)).toBe("text-only");
  });

  it("gives every YouTube video its player, with or without a picture", () => {
    expect(linkPreviewLayout({ ...ARTICLE, image: { width: 480, height: 360 } }, VIDEO)).toBe("youtube");
    expect(linkPreviewLayout(ARTICLE, VIDEO)).toBe("youtube");
  });

  it("never links anywhere but http(s)", () => {
    expect(linkPreviewLayout({ ...ARTICLE, url: "javascript:alert(1)" }, null)).toBe("none");
  });

  it("shows the plain card for an embed kind from a newer server (ADR-0044)", () => {
    const preview: LinkPreview = JSON.parse(
      '{"id":"1","url":"https://example.org/article","title":"An article","embed":{"kind":"hologram","depth":3}}',
    );
    expect(linkPreviewLayout(preview, null)).toBe("text-only");
  });
});

describe("knownLinkEmbedOf (ADR-0044)", () => {
  const withEmbed = (embed: string): LinkPreview => JSON.parse(`{"id":"1","url":"https://example.org/","embed":${embed}}`);

  it("reads the kinds and values this client knows", () => {
    expect(knownLinkEmbedOf(withEmbed('{"kind":"twitch","item":"clip","id":"Slug"}'))).toEqual({
      kind: "twitch",
      item: "clip",
      id: "Slug",
    });
    expect(knownLinkEmbedOf(ARTICLE)).toBeNull();
  });

  it.each([
    '{"kind":"hologram","depth":3}',
    '{"kind":"toString"}',
    '{"kind":"music","provider":"tidal","item":"track","id":"1"}',
    '{"kind":"music","provider":"spotify","item":"audiobook","id":"1"}',
    '{"kind":"github_pull_request","owner":"a","repository":"b","number":1,"state":"queued","author":"c","comments":0}',
    '{"kind":"twitch","item":"stream","id":"a"}',
  ])("leaves out %s from a newer server", (embed) => {
    expect(knownLinkEmbedOf(withEmbed(embed))).toBeNull();
  });
});

describe("youTubeEmbedOf on the instance's canonical links (ADR-0044)", () => {
  it.each([
    [`https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=${PLAYLIST_ID}&t=90`, { ...VIDEO, playlistId: PLAYLIST_ID, startSeconds: 90 }],
    ["https://www.youtube.com/shorts/a_b-C1d2E3f?t=5", { ...SHORT, startSeconds: 5 }],
    ["https://music.youtube.com/watch?v=dQw4w9WgXcQ", VIDEO],
  ])("still plays %s", (link, expected) => {
    expect(youTubeEmbedOf(link)).toEqual(expected);
  });
});

describe("linkPreviewImageUrl", () => {
  it("points at the instance's stored picture", () => {
    expect(linkPreviewImageUrl("30", "800", "900")).toBe("/api/v1/channels/30/messages/800/link-previews/900/image");
  });
});

describe("youTubeEmbedUrl", () => {
  it("uses the privacy-enhanced player and starts it", () => {
    expect(youTubeEmbedUrl(VIDEO)).toBe("https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1");
  });

  it("carries the start time and the playlist", () => {
    expect(youTubeEmbedUrl({ ...VIDEO, startSeconds: 3723, playlistId: PLAYLIST_ID })).toBe(
      `https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1&start=3723&list=${PLAYLIST_ID}`,
    );
  });

  it("plays a Shorts link in the same player", () => {
    const short = youTubeEmbedOf("https://youtube.com/shorts/a_b-C1d2E3f?si=Xy-9_aBcDeFgHiJk");
    expect(short && youTubeEmbedUrl(short)).toBe("https://www.youtube-nocookie.com/embed/a_b-C1d2E3f?autoplay=1");
  });
});

describe("linkPreviewBarColor", () => {
  it("writes the page's colour as hex, padded", () => {
    expect(linkPreviewBarColor(3385975)).toBe("#33aa77");
    expect(linkPreviewBarColor(0x0000ff)).toBe("#0000ff");
    expect(linkPreviewBarColor(0)).toBe("#000000");
  });

  it("falls back to the token for a missing or impossible colour", () => {
    expect(linkPreviewBarColor(undefined)).toBeNull();
    expect(linkPreviewBarColor(-1)).toBeNull();
    expect(linkPreviewBarColor(0x1000000)).toBeNull();
    expect(linkPreviewBarColor(1.5)).toBeNull();
  });
});

describe("linkHostName", () => {
  it("names the site of a link", () => {
    expect(linkHostName("https://www.rust-lang.org/learn")).toBe("www.rust-lang.org");
    expect(linkHostName("not a link")).toBe("not a link");
  });
});

describe("thumbnailWidth", () => {
  it("fits the picture in the thumbnail square without upscaling", () => {
    expect(thumbnailWidth({ width: 64, height: 32 })).toBe(
      "min(var(--mw-component-link-preview-thumbnail-size), calc(var(--mw-component-link-preview-thumbnail-size) * 64 / 32), 64px)",
    );
  });
});
