// Text shown in the shell, built the way the desktop client builds it (mellow-desktop live_rows.rs).

import type { PublicUser, Snowflake } from "../lib/protocol";

/** Unix time of 2026-01-01T00:00:00Z: `MELLOW_EPOCH_UNIX_MILLISECONDS` in mellow-protocol. */
const MELLOW_EPOCH_UNIX_MILLISECONDS = 1_767_225_600_000;
/** Ids keep 42 bits of milliseconds above 22 bits of sequence and reserved bits (ADR-0004). */
const SNOWFLAKE_TIMESTAMP_SHIFT = 22n;
const MAXIMUM_SERVER_INITIALS = 3;
const MAXIMUM_TYPING_NAMES_LISTED = 2;
/** Beyond this many, DESIGN.md §5.5 stops counting: "Several people are typing…". */
const MAXIMUM_TYPING_PEOPLE_COUNTED = 5;

export function unixMillisecondsOfSnowflake(id: Snowflake): number {
  return Number(BigInt(id) >> SNOWFLAKE_TIMESTAMP_SHIFT) + MELLOW_EPOCH_UNIX_MILLISECONDS;
}

function isSameLocalDay(left: Date, right: Date): boolean {
  return (
    left.getFullYear() === right.getFullYear() &&
    left.getMonth() === right.getMonth() &&
    left.getDate() === right.getDate()
  );
}

/** "14:05" in the reader's locale. */
export function timeOfDayLabel(unixMilliseconds: number, locale?: string): string {
  return new Date(unixMilliseconds).toLocaleTimeString(locale, { hour: "2-digit", minute: "2-digit" });
}

/** "Today at 14:05", "Yesterday at 09:12", or the date and time for older messages (DESIGN.md §5.4). */
export function messageTimestampLabel(unixMilliseconds: number, now: Date, locale?: string): string {
  const sent = new Date(unixMilliseconds);
  const yesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1);
  const timeOfDay = timeOfDayLabel(unixMilliseconds, locale);
  if (isSameLocalDay(sent, now)) {
    return `Today at ${timeOfDay}`;
  }
  if (isSameLocalDay(sent, yesterday)) {
    return `Yesterday at ${timeOfDay}`;
  }
  return sent.toLocaleString(locale, { dateStyle: "short", timeStyle: "short" });
}

/** Full date and time, for tooltips and screen readers. */
export function fullTimestampLabel(unixMilliseconds: number, locale?: string): string {
  return new Date(unixMilliseconds).toLocaleString(locale, { dateStyle: "full", timeStyle: "short" });
}

/** First letter of up to three words, as Discord labels servers without an icon. */
export function serverInitials(name: string): string {
  return name
    .trim()
    .split(/\s+/u)
    .filter((word) => word !== "")
    .slice(0, MAXIMUM_SERVER_INITIALS)
    .map((word) => Array.from(word)[0] ?? "")
    .join("");
}

/** First grapheme of the display name, shown on default avatars (DESIGN.md §2.3). */
export function avatarInitial(user: Pick<PublicUser, "display_name" | "username">): string {
  const name = user.display_name.trim() || user.username;
  return (Array.from(name)[0] ?? "").toUpperCase();
}

export function displayName(user: Pick<PublicUser, "display_name" | "username">): string {
  return user.display_name.trim() || user.username;
}

/** "Ana is typing…" up to "Several people are typing…" (DESIGN.md §5.5); empty for nobody. */
export function typingLabel(names: readonly string[]): string {
  const [first, second] = names;
  if (first === undefined) {
    return "";
  }
  if (names.length > MAXIMUM_TYPING_PEOPLE_COUNTED) {
    return "Several people are typing…";
  }
  if (second === undefined) {
    return `${first} is typing…`;
  }
  if (names.length === MAXIMUM_TYPING_NAMES_LISTED) {
    return `${first} and ${second} are typing…`;
  }
  const others = names.length - MAXIMUM_TYPING_NAMES_LISTED;
  return `${first}, ${second} and ${others} ${others === 1 ? "other" : "others"} are typing…`;
}

export type MessageTextSegment = { kind: "text"; text: string } | { kind: "link"; text: string; url: string };

const LINK_CANDIDATE = /https?:\/\/[^\s<>"]+/giu;
/** Punctuation that usually ends the sentence around a link rather than the link itself. */
const TRAILING_PUNCTUATION = /[.,;:!?'")\]}]+$/u;

/** Splits message text into plain text and http(s) links; anything else stays text. */
export function splitMessageText(content: string): MessageTextSegment[] {
  const segments: MessageTextSegment[] = [];
  let textStart = 0;
  for (const match of content.matchAll(LINK_CANDIDATE)) {
    const candidate = match[0].replace(TRAILING_PUNCTUATION, "");
    const url = safeHttpUrl(candidate);
    if (!url) {
      continue;
    }
    if (match.index > textStart) {
      segments.push({ kind: "text", text: content.slice(textStart, match.index) });
    }
    segments.push({ kind: "link", text: candidate, url });
    textStart = match.index + candidate.length;
  }
  if (textStart < content.length) {
    segments.push({ kind: "text", text: content.slice(textStart) });
  }
  return segments;
}

function safeHttpUrl(candidate: string): string | null {
  try {
    const url = new URL(candidate);
    return url.protocol === "http:" || url.protocol === "https:" ? url.href : null;
  } catch {
    // Not a URL after all: it stays plain text.
    return null;
  }
}
