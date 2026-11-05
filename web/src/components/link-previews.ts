import type { LinkPreview, Snowflake } from "../lib/protocol";
import { safeHttpUrl } from "./markdown";

/**
 * How a link's preview shows under its message (docs/design/web-link-previews.md):
 * - `picture-only`: the link is itself a picture, shown alone like an attached one;
 * - `youtube`: the card with a 16:9 picture (9:16 for Shorts) that becomes the player on a click;
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

export function linkPreviewLayout(preview: LinkPreview, youTubeEmbed: YouTubeEmbed | null): LinkPreviewLayout {
  if (safeHttpUrl(preview.url) === null) {
    return "none";
  }
  if (youTubeEmbed !== null) {
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
/**
 * A public playlist (`PL…`, `OLAK5uy_…`, `UU…`). Shorter ids are a member's own lists (`LL`, `WL`)
 * and `RD…` are mixes made for one viewer: the embedded player cannot show either.
 */
const YOUTUBE_PLAYLIST_ID = /^(?!RD)[A-Za-z0-9_-]{10,64}$/;
/** `t=` as YouTube's share button writes it: `90`, `90s`, `2m`, `1h2m3s`, each part bounded. */
const YOUTUBE_TIME = /^(?:(\d{1,3})h)?(?:(\d{1,5})m)?(?:(\d{1,7})s?)?$/;
/** `start=` as embed links write it: whole seconds. */
const YOUTUBE_START_SECONDS = /^\d{1,7}$/;
const YOUTUBE_WATCH_HOSTS: readonly string[] = [
  "youtube.com",
  "www.youtube.com",
  "m.youtube.com",
  "music.youtube.com",
  "youtube-nocookie.com",
  "www.youtube-nocookie.com",
];
const YOUTUBE_SHORT_LINK_HOSTS: readonly string[] = ["youtu.be", "www.youtu.be"];
/** Paths `/<kind>/<video id>`. */
const YOUTUBE_VIDEO_PATH_KINDS: readonly string[] = ["shorts", "live", "embed", "v"];
const SECONDS_PER_MINUTE = 60;
const SECONDS_PER_HOUR = 3600;

/** What the player needs from a YouTube link, every part checked strictly: it goes into the player's address. */
export interface YouTubeEmbed {
  videoId: string;
  /** A public playlist the video plays in, or null. */
  playlistId: string | null;
  /** Where the video starts, in whole seconds above zero, or null. */
  startSeconds: number | null;
  /** A Shorts link: the video is vertical and plays in a 9:16 frame. */
  isShort: boolean;
}

/**
 * The video of a YouTube watch, short, Shorts, live, embed or `/v/` link (YouTube Music's watch
 * links and `youtube-nocookie.com` too), with its playlist and start time. Null for anything else
 * (channels, playlists alone, look-alike hosts).
 */
export function youTubeEmbedOf(link: string): YouTubeEmbed | null {
  const normalised = safeHttpUrl(link);
  if (normalised === null) {
    return null;
  }
  const url = new URL(normalised);
  const host = url.hostname.toLowerCase();
  const pathParts = url.pathname.split("/").filter((part) => part !== "");
  const [kind, idInPath] = pathParts;
  const isShortLinkHost = YOUTUBE_SHORT_LINK_HOSTS.includes(host);
  let videoId: string | null = null;
  if (isShortLinkHost) {
    videoId = pathParts.length === 1 ? (kind ?? null) : null;
  } else if (YOUTUBE_WATCH_HOSTS.includes(host)) {
    if (pathParts.length === 1 && kind === "watch") {
      videoId = url.searchParams.get("v");
    } else if (pathParts.length === 2 && kind !== undefined && YOUTUBE_VIDEO_PATH_KINDS.includes(kind)) {
      videoId = idInPath ?? null;
    }
  }
  if (videoId === null || !YOUTUBE_VIDEO_ID.test(videoId)) {
    return null;
  }
  const playlistId = url.searchParams.get("list");
  return {
    videoId,
    playlistId: playlistId !== null && YOUTUBE_PLAYLIST_ID.test(playlistId) ? playlistId : null,
    startSeconds: youTubeStartSecondsOf(url.searchParams),
    isShort: !isShortLinkHost && pathParts.length === 2 && kind === "shorts",
  };
}

/** `t=`, else `start=`; null when absent, zero or malformed. */
function youTubeStartSecondsOf(searchParams: URLSearchParams): number | null {
  const time = searchParams.get("t");
  const start = searchParams.get("start");
  let seconds: number | null = null;
  if (time !== null && time !== "") {
    const match = YOUTUBE_TIME.exec(time);
    if (match !== null) {
      const [, hours = "0", minutes = "0", remainingSeconds = "0"] = match;
      seconds = Number(hours) * SECONDS_PER_HOUR + Number(minutes) * SECONDS_PER_MINUTE + Number(remainingSeconds);
    }
  } else if (start !== null && YOUTUBE_START_SECONDS.test(start)) {
    seconds = Number(start);
  }
  return seconds !== null && seconds > 0 ? seconds : null;
}

/** The privacy-enhanced player (ADR-0041), allowed by the page's `frame-src`; only built after a click. */
export function youTubeEmbedUrl(embed: YouTubeEmbed): string {
  const parameters = new URLSearchParams({ autoplay: "1" });
  if (embed.startSeconds !== null) {
    parameters.set("start", String(embed.startSeconds));
  }
  if (embed.playlistId !== null) {
    parameters.set("list", embed.playlistId);
  }
  return `https://www.youtube-nocookie.com/embed/${encodeURIComponent(embed.videoId)}?${parameters.toString()}`;
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
