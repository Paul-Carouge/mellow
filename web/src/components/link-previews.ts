import type { LinkPreview, Snowflake } from "../lib/protocol";
import { safeHttpUrl } from "./markdown";

/**
 * How a link's preview shows under its message (docs/design/web-link-previews.md):
 * - `picture-only`: the link is itself a picture, shown alone like an attached one;
 * - `youtube`: the card with a 16:9 picture that becomes the player on a click;
 * - `large-picture`: the card with a wide picture under its text;
 * - `thumbnail`: the card with a small or square picture at its trailing top;
 * - `text-only`: the card without picture;
 * - `none`: nothing worth a card (no text, no picture), or a link that is not http(s).
 */
export type LinkPreviewLayout = "picture-only" | "youtube" | "large-picture" | "thumbnail" | "text-only" | "none";

/** A landscape picture at least this much wider than high reads as a banner, shown large. */
export const MINIMUM_LARGE_PICTURE_ASPECT_RATIO = 1.6;

/** Narrower than this, a picture would be stretched past its pixels across the card: a thumbnail. */
export const MINIMUM_LARGE_PICTURE_WIDTH_PIXELS = 300;

export function linkPreviewLayout(preview: LinkPreview, youTubeVideoId: string | null): LinkPreviewLayout {
  if (safeHttpUrl(preview.url) === null) {
    return "none";
  }
  if (youTubeVideoId !== null) {
    return "youtube";
  }
  const hasText = Boolean(preview.title || preview.description);
  const { image } = preview;
  if (!image || !hasUsableSize(image)) {
    return hasText ? "text-only" : "none";
  }
  if (!hasText) {
    return "picture-only";
  }
  const isWide = image.width / image.height >= MINIMUM_LARGE_PICTURE_ASPECT_RATIO;
  return isWide && image.width >= MINIMUM_LARGE_PICTURE_WIDTH_PIXELS ? "large-picture" : "thumbnail";
}

const YOUTUBE_VIDEO_ID = /^[A-Za-z0-9_-]{11}$/;
const YOUTUBE_WATCH_HOSTS: readonly string[] = ["youtube.com", "www.youtube.com", "m.youtube.com", "music.youtube.com"];

/**
 * The video id of a YouTube watch, short or Shorts link (and YouTube Music's watch links), checked
 * strictly: it goes into the player's address. Null for anything else (channels, playlists alone).
 */
export function youTubeVideoIdOf(link: string): string | null {
  const normalised = safeHttpUrl(link);
  if (normalised === null) {
    return null;
  }
  const url = new URL(normalised);
  const host = url.hostname.toLowerCase();
  const pathParts = url.pathname.split("/").filter((part) => part !== "");
  let candidate: string | null = null;
  if (host === "youtu.be" || host === "www.youtu.be") {
    candidate = pathParts.length === 1 ? (pathParts[0] ?? null) : null;
  } else if (YOUTUBE_WATCH_HOSTS.includes(host)) {
    if (pathParts.length === 1 && pathParts[0] === "watch") {
      candidate = url.searchParams.get("v");
    } else if (pathParts.length === 2 && pathParts[0] === "shorts") {
      candidate = pathParts[1] ?? null;
    }
  }
  return candidate !== null && YOUTUBE_VIDEO_ID.test(candidate) ? candidate : null;
}

/** The privacy-enhanced player (ADR-0041), allowed by the page's `frame-src`; only built after a click. */
export function youTubeEmbedUrl(videoId: string): string {
  return `https://www.youtube-nocookie.com/embed/${encodeURIComponent(videoId)}?autoplay=1`;
}

/** The preview's picture as the instance stored it; readers never contact the site. */
export function linkPreviewImageUrl(channelId: Snowflake, messageId: Snowflake, previewId: Snowflake): string {
  return `/api/v1/channels/${channelId}/messages/${messageId}/link-previews/${previewId}/image`;
}

/** `#rrggbb` for the page's `theme-color`, or null (the token's dusk) when absent or out of range. */
export function linkPreviewBarColor(themeColor: number | undefined): string | null {
  if (themeColor === undefined || !Number.isInteger(themeColor) || themeColor < 0 || themeColor > 0xffffff) {
    return null;
  }
  return `#${themeColor.toString(16).padStart(6, "0")}`;
}

/** "example.org" for a link, so a picture or player without a title still has a name. */
export function linkHostName(link: string): string {
  const normalised = safeHttpUrl(link);
  return normalised === null ? link : new URL(normalised).hostname;
}

/**
 * The CSS width of a thumbnail's box before it loads: the picture fitted in the thumbnail square,
 * never above its own pixels, so nothing moves when it arrives.
 */
export function thumbnailWidth(image: { width: number; height: number }): string {
  return `min(var(--mw-component-link-preview-thumbnail-size), calc(var(--mw-component-link-preview-thumbnail-size) * ${image.width} / ${image.height}), ${image.width}px)`;
}

function hasUsableSize(image: { width: number; height: number }): boolean {
  return Number.isInteger(image.width) && Number.isInteger(image.height) && image.width > 0 && image.height > 0;
}
