import { describe, expect, it } from "vitest";

import type { LinkPreview } from "../lib/protocol";
import {
  linkHostName,
  linkPreviewBarColor,
  linkPreviewImageUrl,
  linkPreviewLayout,
  thumbnailWidth,
  youTubeEmbedUrl,
  youTubeVideoIdOf,
} from "./link-previews";

const ARTICLE: LinkPreview = { id: "900", url: "https://example.org/article", title: "An article", description: "What it says" };

describe("youTubeVideoIdOf", () => {
  it("reads watch, short, Shorts and YouTube Music links", () => {
    expect(youTubeVideoIdOf("https://www.youtube.com/watch?v=dQw4w9WgXcQ")).toBe("dQw4w9WgXcQ");
    expect(youTubeVideoIdOf("https://youtube.com/watch?v=dQw4w9WgXcQ&t=42s")).toBe("dQw4w9WgXcQ");
    expect(youTubeVideoIdOf("https://m.youtube.com/watch?feature=share&v=dQw4w9WgXcQ")).toBe("dQw4w9WgXcQ");
    expect(youTubeVideoIdOf("https://youtu.be/dQw4w9WgXcQ?si=abc")).toBe("dQw4w9WgXcQ");
    expect(youTubeVideoIdOf("https://www.youtube.com/shorts/a_b-C1d2E3f")).toBe("a_b-C1d2E3f");
    expect(youTubeVideoIdOf("https://music.youtube.com/watch?v=dQw4w9WgXcQ&list=RD")).toBe("dQw4w9WgXcQ");
    expect(youTubeVideoIdOf("http://WWW.YouTube.com/watch?v=dQw4w9WgXcQ")).toBe("dQw4w9WgXcQ");
  });

  it("refuses ids of the wrong length or with other characters", () => {
    expect(youTubeVideoIdOf("https://www.youtube.com/watch?v=dQw4w9WgXc")).toBeNull();
    expect(youTubeVideoIdOf("https://www.youtube.com/watch?v=dQw4w9WgXcQQ")).toBeNull();
    expect(youTubeVideoIdOf("https://youtu.be/dQw4w9WgX%2F")).toBeNull();
    expect(youTubeVideoIdOf("https://www.youtube.com/watch?v=dQw4w9WgX.Q")).toBeNull();
    expect(youTubeVideoIdOf("https://www.youtube.com/watch")).toBeNull();
  });

  it("ignores other YouTube pages and look-alike hosts", () => {
    expect(youTubeVideoIdOf("https://www.youtube.com/@channel")).toBeNull();
    expect(youTubeVideoIdOf("https://www.youtube.com/playlist?list=PL123")).toBeNull();
    expect(youTubeVideoIdOf("https://www.youtube.com/shorts/dQw4w9WgXcQ/extra")).toBeNull();
    expect(youTubeVideoIdOf("https://youtu.be/dQw4w9WgXcQ/extra")).toBeNull();
    expect(youTubeVideoIdOf("https://notyoutube.com/watch?v=dQw4w9WgXcQ")).toBeNull();
    expect(youTubeVideoIdOf("https://www.youtube.com.evil.example/watch?v=dQw4w9WgXcQ")).toBeNull();
    expect(youTubeVideoIdOf("https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC")).toBeNull();
    expect(youTubeVideoIdOf("javascript:alert(1)//youtu.be/dQw4w9WgXcQ")).toBeNull();
    expect(youTubeVideoIdOf("https://user@youtu.be/dQw4w9WgXcQ")).toBeNull();
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
    expect(linkPreviewLayout({ ...ARTICLE, image: { width: 480, height: 360 } }, "dQw4w9WgXcQ")).toBe("youtube");
    expect(linkPreviewLayout(ARTICLE, "dQw4w9WgXcQ")).toBe("youtube");
  });

  it("never links anywhere but http(s)", () => {
    expect(linkPreviewLayout({ ...ARTICLE, url: "javascript:alert(1)" }, null)).toBe("none");
  });
});

describe("linkPreviewImageUrl", () => {
  it("points at the instance's stored picture", () => {
    expect(linkPreviewImageUrl("30", "800", "900")).toBe("/api/v1/channels/30/messages/800/link-previews/900/image");
  });
});

describe("youTubeEmbedUrl", () => {
  it("uses the privacy-enhanced player and starts it", () => {
    expect(youTubeEmbedUrl("dQw4w9WgXcQ")).toBe("https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ?autoplay=1");
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
