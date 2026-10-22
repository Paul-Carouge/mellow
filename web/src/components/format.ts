// Text shown in the shell, built the way the desktop client builds it (mellow-desktop live_rows.rs).

import { formattingLocale, strings, wordedFormattingLocale } from "../i18n";
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
export function timeOfDayLabel(unixMilliseconds: number, locale: string = formattingLocale): string {
  return new Date(unixMilliseconds).toLocaleTimeString(locale, { hour: "2-digit", minute: "2-digit" });
}

/** "Today at 14:05", "Yesterday at 09:12", or the date and time for older messages (DESIGN.md §5.4). */
export function messageTimestampLabel(unixMilliseconds: number, now: Date, locale: string = formattingLocale): string {
  const sent = new Date(unixMilliseconds);
  const yesterday = new Date(now.getFullYear(), now.getMonth(), now.getDate() - 1);
  const timeOfDay = timeOfDayLabel(unixMilliseconds, locale);
  if (isSameLocalDay(sent, now)) {
    return strings.time.todayAt(timeOfDay);
  }
  if (isSameLocalDay(sent, yesterday)) {
    return strings.time.yesterdayAt(timeOfDay);
  }
  return sent.toLocaleString(locale, { dateStyle: "short", timeStyle: "short" });
}

/** Full date and time, for tooltips and screen readers. */
export function fullTimestampLabel(unixMilliseconds: number, locale: string = wordedFormattingLocale): string {
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

/** `color.avatar.tint-1` to `tint-8` in design/tokens.json. */
const AVATAR_TINT_COUNT = 8;
const FNV_OFFSET_BASIS = 0x811c9dc5;
const FNV_PRIME = 0x01000193;

/**
 * The default avatar's tint (DESIGN.md §2.3): the same person keeps the same colour everywhere.
 * FNV-1a over the id's digits, so neighbouring Snowflakes still spread over the palette.
 */
export function avatarTintToken(userId: Snowflake): string {
  let hash = FNV_OFFSET_BASIS;
  for (const character of userId) {
    hash = Math.imul(hash ^ character.charCodeAt(0), FNV_PRIME) >>> 0;
  }
  return `--mw-color-avatar-tint-${(hash % AVATAR_TINT_COUNT) + 1}`;
}

export function displayName(user: Pick<PublicUser, "display_name" | "username">): string {
  return user.display_name.trim() || user.username;
}

/** "Ana is typing…" up to "Several people are typing…" (DESIGN.md §5.5); empty for nobody. */
/** The typing line in pieces, so the names can be set in bold (as in the Dusk mockup). */
export function typingSegments(names: readonly string[]): { text: string; isName: boolean }[] {
  const [first, second] = names;
  const name = (text: string) => ({ text, isName: true });
  const words = (text: string) => ({ text, isName: false });
  if (first === undefined) {
    return [];
  }
  if (names.length > MAXIMUM_TYPING_PEOPLE_COUNTED) {
    return [words(strings.typing.several)];
  }
  if (second === undefined) {
    return [name(first), words(strings.typing.isTyping)];
  }
  if (names.length === MAXIMUM_TYPING_NAMES_LISTED) {
    return [name(first), words(strings.typing.and), name(second), words(strings.typing.areTyping)];
  }
  const others = names.length - MAXIMUM_TYPING_NAMES_LISTED;
  return [name(first), words(strings.typing.listSeparator), name(second), words(strings.typing.andOthersAreTyping(others))];
}

export function typingLabel(names: readonly string[]): string {
  return typingSegments(names)
    .map((segment) => segment.text)
    .join("");
}

/** Binary steps with Discord's labels: the instance's "25 MB" limit is 25 × 1024 × 1024 bytes. */
const BYTES_PER_UNIT_STEP = 1024;
const FILE_SIZE_UNITS = ["KB", "MB", "GB"] as const;
const FILE_SIZE_MAXIMUM_FRACTION_DIGITS = 1;

/** "512 bytes", "1 KB", "2.4 MB", in the reader's locale. */
export function humanFileSize(bytes: number, locale: string = formattingLocale): string {
  if (bytes < BYTES_PER_UNIT_STEP) {
    return strings.fileSize.bytes(bytes);
  }
  let value = bytes;
  let unit: (typeof FILE_SIZE_UNITS)[number] = FILE_SIZE_UNITS[0];
  for (const candidate of FILE_SIZE_UNITS) {
    value /= BYTES_PER_UNIT_STEP;
    unit = candidate;
    // Rounding 1023.96 KB up must read "1 MB", not "1,024 KB".
    if (Math.round(value * 10) / 10 < BYTES_PER_UNIT_STEP) {
      break;
    }
  }
  const number = value.toLocaleString(locale, { maximumFractionDigits: FILE_SIZE_MAXIMUM_FRACTION_DIGITS });
  return strings.fileSize.inUnit(number, unit);
}

const TRUNCATION_ELLIPSIS = "…";
/** Longer endings are not extensions people recognise; they are cut like the rest of the name. */
const MAXIMUM_KEPT_EXTENSION_LENGTH = 8;

/**
 * Shortens a file name in the middle, keeping its end and extension readable
 * ("holiday-photos-from…ummer.jpg"); names that fit are returned unchanged.
 */
export function middleTruncate(fileName: string, maximumCharacters: number): string {
  const characters = Array.from(fileName);
  if (characters.length <= maximumCharacters) {
    return fileName;
  }
  const extensionStart = fileName.lastIndexOf(".");
  const extensionLength =
    extensionStart > 0 ? Array.from(fileName.slice(extensionStart)).length : 0;
  const keptExtensionLength = extensionLength <= MAXIMUM_KEPT_EXTENSION_LENGTH ? extensionLength : 0;
  const available = Math.max(maximumCharacters - TRUNCATION_ELLIPSIS.length, 2);
  // The end keeps the extension plus a few characters before it, so "…(2).jpg" stays distinguishable.
  const endLength = Math.min(Math.max(Math.floor(available / 3), keptExtensionLength + 2), available - 1);
  const startLength = available - endLength;
  return characters.slice(0, startLength).join("") + TRUNCATION_ELLIPSIS + characters.slice(-endLength).join("");
}

/** "1 attachment", "2 attachments" (DESIGN.md §7.3); empty for none. */
export function attachmentCountLabel(count: number): string {
  if (count === 0) {
    return "";
  }
  return strings.messages.attachmentCount(count);
}
