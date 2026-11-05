// GIFs from Klipy, asked for and loaded by this browser (ADR-0042): the request addresses, the
// checks on every answer, and the rule that turns a message holding only a Klipy media address
// into the GIF itself. Klipy's answers are untrusted: a field that is missing or of another shape
// skips its item, and only media from Klipy's own hosts is ever shown.

import type { GifSettings } from "./protocol";

const KLIPY_API_ORIGIN = "https://api.klipy.com";
/** The hosts the page's Content-Security-Policy opens when GIFs are on (ADR-0042). */
const KLIPY_MEDIA_HOSTS: ReadonlySet<string> = new Set(["static.klipy.com", "static1.klipy.com", "static2.klipy.com"]);
/** One page of the picker: Klipy's own default, three or four rows of tiles. */
export const KLIPY_PAGE_SIZE = 24;
/** Klipy's slugs, as the instance accepts them for favourites (`domain::gifs::validate_slug`). */
const KLIPY_SLUG_PATTERN = /^[A-Za-z0-9_-]{1,64}$/u;
/** A blurred placeholder, safe inside a CSS `url()`: base64 characters only. */
const BLUR_PREVIEW_PATTERN = /^data:image\/(?:jpeg|png|webp|gif);base64,[A-Za-z0-9+/]+={0,2}$/u;
/** Longer than any real one (a few hundred bytes), so a broken answer cannot bloat the page. */
const MAXIMUM_BLUR_PREVIEW_LENGTH = 8_192;
const MAXIMUM_TITLE_LENGTH = 200;

/** A picture or clip on Klipy's hosts, with its size when Klipy gave one. */
export interface KlipyMedia {
  readonly url: string;
  readonly width: number;
  readonly height: number;
  readonly isVideo: boolean;
}

export interface KlipyGif {
  readonly kind: "gif";
  readonly slug: string;
  /** Klipy's title, or empty: the picker names untitled GIFs itself. */
  readonly title: string;
  /** The picker's tile: small animated WebP, or MP4, or GIF. */
  readonly tile: KlipyMedia;
  /** The small JPEG, shown while animated pictures are kept still. */
  readonly still: KlipyMedia | null;
  /** What a message carries (ADR-0042): the medium WebP, else the medium GIF, else a small one. */
  readonly messageUrl: string;
  readonly blurPreview: string | null;
}

/**
 * An advertisement Klipy may slip in when ads are on for the key (they are off on Mellow's).
 * Klipy's ad is HTML for a web view, which this page never renders: it keeps its place as a tile.
 */
export interface KlipyAd {
  readonly kind: "ad";
  /** Its place in the answer, for a stable key. */
  readonly position: number;
  readonly width: number | null;
  readonly height: number | null;
}

export type KlipyItem = KlipyGif | KlipyAd;

export interface KlipyPage {
  /** In Klipy's order, never filtered or reordered beyond dropping what cannot be shown. */
  readonly items: readonly KlipyItem[];
  readonly hasNext: boolean;
}

export type KlipyFailure = "offline" | "rate-limited" | "refused" | "failed";

export class KlipyError extends Error {
  constructor(readonly failure: KlipyFailure) {
    super(`klipy request failed: ${failure}`);
    this.name = "KlipyError";
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function positiveInteger(value: unknown): number | null {
  return typeof value === "number" && Number.isInteger(value) && value > 0 ? value : null;
}

/** The address itself when it is `https` on one of Klipy's media hosts, without credentials or port. */
export function klipyMediaUrl(candidate: unknown): string | null {
  if (typeof candidate !== "string") {
    return null;
  }
  let url: URL;
  try {
    url = new URL(candidate);
  } catch {
    // Not an absolute address: nothing to show.
    return null;
  }
  const isKlipyMedia =
    url.protocol === "https:" && KLIPY_MEDIA_HOSTS.has(url.hostname) && url.port === "" && url.username === "" && url.password === "";
  return isKlipyMedia ? url.href : null;
}

const VIDEO_EXTENSIONS: readonly string[] = [".mp4", ".webm"];
const PICTURE_EXTENSIONS: readonly string[] = [".webp", ".gif", ".jpg", ".jpeg", ".png"];

function extensionOf(href: string): string {
  const path = new URL(href).pathname.toLowerCase();
  const dot = path.lastIndexOf(".");
  return dot < 0 ? "" : path.slice(dot);
}

function mediaOf(format: unknown, isVideo: boolean): KlipyMedia | null {
  if (!isRecord(format)) {
    return null;
  }
  const url = klipyMediaUrl(format.url);
  const width = positiveInteger(format.width);
  const height = positiveInteger(format.height);
  return url && width && height ? { url, width, height, isVideo } : null;
}

type KlipySize = "hd" | "md" | "sm" | "xs";
type KlipyFormat = "gif" | "webp" | "mp4" | "jpg";

function formatOf(file: Record<string, unknown>, size: KlipySize, format: KlipyFormat): KlipyMedia | null {
  const sized = file[size];
  return isRecord(sized) ? mediaOf(sized[format], format === "mp4") : null;
}

/** The first of `choices` the file has, as `[size, format]` pairs in order of preference. */
function firstFormat(file: Record<string, unknown>, choices: readonly (readonly [KlipySize, KlipyFormat])[]): KlipyMedia | null {
  for (const [size, format] of choices) {
    const media = formatOf(file, size, format);
    if (media) {
      return media;
    }
  }
  return null;
}

// The picker's tiles are about 100 px high: the small WebP is sharp on dense screens and an
// eighth of the medium one's weight; the extra small one only when nothing else is there.
const TILE_CHOICES = [
  ["sm", "webp"],
  ["sm", "mp4"],
  ["sm", "gif"],
  ["xs", "webp"],
  ["md", "webp"],
] as const;
const STILL_CHOICES = [
  ["sm", "jpg"],
  ["xs", "jpg"],
  ["md", "jpg"],
] as const;
const MESSAGE_CHOICES = [
  ["md", "webp"],
  ["md", "gif"],
  ["sm", "webp"],
  ["sm", "gif"],
] as const;

function parseGif(item: Record<string, unknown>): KlipyGif | null {
  const slug = typeof item.slug === "string" && KLIPY_SLUG_PATTERN.test(item.slug) ? item.slug : null;
  if (!slug || !isRecord(item.file)) {
    return null;
  }
  const tile = firstFormat(item.file, TILE_CHOICES);
  const message = firstFormat(item.file, MESSAGE_CHOICES);
  if (!tile || !message) {
    return null;
  }
  const blurPreview =
    typeof item.blur_preview === "string" &&
    item.blur_preview.length <= MAXIMUM_BLUR_PREVIEW_LENGTH &&
    BLUR_PREVIEW_PATTERN.test(item.blur_preview)
      ? item.blur_preview
      : null;
  return {
    kind: "gif",
    slug,
    title: typeof item.title === "string" ? item.title.trim().slice(0, MAXIMUM_TITLE_LENGTH) : "",
    tile,
    still: firstFormat(item.file, STILL_CHOICES),
    messageUrl: message.url,
    blurPreview,
  };
}

/** Each item checked on its own: one broken item never hides the others. */
export function parseKlipyItems(items: unknown): KlipyItem[] {
  if (!Array.isArray(items)) {
    return [];
  }
  return items.flatMap((item: unknown, position): KlipyItem[] => {
    if (!isRecord(item)) {
      return [];
    }
    if (item.type === "ad") {
      return [{ kind: "ad", position, width: positiveInteger(item.width), height: positiveInteger(item.height) }];
    }
    // Absent on some answers (items by slug): a GIF endpoint answers GIFs.
    if (item.type !== undefined && item.type !== "gif") {
      return [];
    }
    const gif = parseGif(item);
    return gif ? [gif] : [];
  });
}

/** `{ result: true, data: { data: [...], has_next } }`, or null for any other shape. */
export function parseKlipyPage(body: unknown): KlipyPage | null {
  if (!isRecord(body) || body.result !== true || !isRecord(body.data) || !Array.isArray(body.data.data)) {
    return null;
  }
  return { items: parseKlipyItems(body.data.data), hasNext: body.data.has_next === true };
}

/**
 * Klipy's `locale` is a country code ("fr", "us", "gb"): the region of the reader's formatting
 * locale when it has one, else their language.
 */
export function klipyLocaleOf(languageTag: string): string {
  const [language = "", region] = languageTag.split("-");
  const code = region && /^[A-Za-z]{2}$/u.test(region) ? region : language;
  return /^[A-Za-z]{2}$/u.test(code) ? code.toLowerCase() : "us";
}

function apiUrl(settings: GifSettings, path: string, query: Record<string, string>): string {
  const url = new URL(`/api/v1/${encodeURIComponent(settings.klipy_app_key)}/gifs/${path}`, KLIPY_API_ORIGIN);
  for (const [name, value] of Object.entries(query)) {
    url.searchParams.set(name, value);
  }
  return url.href;
}

/** Trending, or a search when `query` is not blank; `page` counts from 1. */
export function klipyListUrl(settings: GifSettings, query: string, page: number, locale: string): string {
  const common = {
    page: String(page),
    per_page: String(KLIPY_PAGE_SIZE),
    customer_id: settings.customer_id,
    locale,
  };
  const trimmed = query.trim();
  return trimmed === "" ? apiUrl(settings, "trending", common) : apiUrl(settings, "search", { ...common, q: trimmed });
}

/** Klipy's items endpoint takes slugs as one comma-separated parameter. */
export function klipyItemsUrl(settings: GifSettings, slugs: readonly string[]): string {
  return apiUrl(settings, "items", { slugs: slugs.join(",") });
}

export function klipyShareUrl(settings: GifSettings, slug: string): string {
  return apiUrl(settings, `share/${encodeURIComponent(slug)}`, {});
}

/** Klipy's requests carry no cookie and no page address: Klipy learns nothing it does not need. */
const KLIPY_REQUEST_DEFAULTS: RequestInit = { credentials: "omit", referrerPolicy: "no-referrer", mode: "cors" };
const KLIPY_REQUEST_TIMEOUT_MILLISECONDS = 15_000;

function failureOfStatus(status: number): KlipyFailure {
  if (status === 429) {
    return "rate-limited";
  }
  return status === 401 || status === 403 ? "refused" : "failed";
}

/** One page from Klipy; throws `KlipyError`, `AbortError` when `signal` aborts. */
export async function fetchKlipyPage(url: string, signal: AbortSignal): Promise<KlipyPage> {
  // `AbortSignal.any` is too recent for the browsers Vite builds for: the caller's abort and the
  // timeout are joined by hand.
  const request = new AbortController();
  const abortRequest = () => request.abort();
  signal.addEventListener("abort", abortRequest, { once: true });
  const timeout = setTimeout(abortRequest, KLIPY_REQUEST_TIMEOUT_MILLISECONDS);
  try {
    let response: Response;
    try {
      response = await fetch(url, { ...KLIPY_REQUEST_DEFAULTS, signal: request.signal });
    } catch (error) {
      if (signal.aborted) {
        throw error;
      }
      throw new KlipyError(navigator.onLine === false ? "offline" : "failed");
    }
    if (!response.ok) {
      throw new KlipyError(failureOfStatus(response.status));
    }
    // A body that is not JSON is a broken answer, said as such just below.
    const page = parseKlipyPage(await response.json().catch(() => null));
    if (!page) {
      throw new KlipyError("failed");
    }
    return page;
  } finally {
    clearTimeout(timeout);
    signal.removeEventListener("abort", abortRequest);
  }
}

/**
 * Tells Klipy a GIF was sent, as their API asks (`q` is the search that found it, empty from
 * trending or favourites). Fire and forget: a failure changes nothing for the sender.
 */
export function triggerKlipyShare(settings: GifSettings, slug: string, query: string): void {
  void fetch(klipyShareUrl(settings, slug), {
    ...KLIPY_REQUEST_DEFAULTS,
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ customer_id: settings.customer_id, q: query.trim() }),
    keepalive: true,
  }).catch((error: unknown) => console.debug("klipy share trigger failed", error));
}

/** A message shown as a GIF: its whole text is one address of Klipy's media (ADR-0042). */
export interface MessageGifMedia {
  readonly url: string;
  readonly isVideo: boolean;
}

/**
 * The GIF a message is, or null: its trimmed text must be a single `https://static*.klipy.com/…`
 * address of a picture or clip. Anything more (words, a second link, `<…>`) keeps the link.
 */
export function messageGifMediaOf(content: string): MessageGifMedia | null {
  const text = content.trim();
  // Cheap first: almost no message starts so.
  if (!text.startsWith("https://static") || /\s/u.test(text)) {
    return null;
  }
  const url = klipyMediaUrl(text);
  if (!url) {
    return null;
  }
  const extension = extensionOf(url);
  if (VIDEO_EXTENSIONS.includes(extension)) {
    return { url, isVideo: true };
  }
  return PICTURE_EXTENSIONS.includes(extension) ? { url, isVideo: false } : null;
}
