// The reader's notification choices (docs/design/web-notifications.md): what this browser
// remembers, checked on the way in, and when mutes end. Pure: storage is passed in.
//
// shortcut: kept per browser in `localStorage`, so another device keeps its own choices; move
// them to a server setting when settings sync across devices.

import type { Snowflake } from "./protocol";

/** What a place notifies about ("Notify me about"). */
export type NotifyLevel = "all" | "mentions" | "nothing";
export const NOTIFY_LEVELS: readonly NotifyLevel[] = ["all", "mentions", "nothing"];

/** The mute lengths offered in the dialog, as on Discord. */
export type MuteDuration = "15-minutes" | "1-hour" | "8-hours" | "24-hours" | "until-turned-back-on";
export const MUTE_DURATIONS: readonly MuteDuration[] = [
  "15-minutes",
  "1-hour",
  "8-hours",
  "24-hours",
  "until-turned-back-on",
];
const MINUTE_MILLISECONDS = 60_000;
const MUTE_DURATION_MILLISECONDS: Record<Exclude<MuteDuration, "until-turned-back-on">, number> = {
  "15-minutes": 15 * MINUTE_MILLISECONDS,
  "1-hour": 60 * MINUTE_MILLISECONDS,
  "8-hours": 8 * 60 * MINUTE_MILLISECONDS,
  "24-hours": 24 * 60 * MINUTE_MILLISECONDS,
};

/** When a mute ends: a Unix time in milliseconds, or never (until the reader turns it back on). */
export type MuteEnd = number | "never";

/** A server's, a channel's or a conversation's own choices; absent fields follow the default. */
export interface PlaceNotificationSettings {
  readonly level?: NotifyLevel;
  readonly mutedUntil?: MuteEnd;
}

export interface NotificationSettings {
  /** The reader turned desktop notifications on (the browser's permission is checked apart). */
  readonly areDesktopNotificationsEnabled: boolean;
  readonly shouldShowPreviews: boolean;
  readonly isSoundEnabled: boolean;
  /** The one-time permission bar was answered or dismissed. */
  readonly hasAnsweredPermissionBar: boolean;
  /** By server, channel or conversation id: Snowflakes never collide across kinds. */
  readonly places: Readonly<Record<Snowflake, PlaceNotificationSettings>>;
}

export const DEFAULT_NOTIFICATION_SETTINGS: NotificationSettings = {
  areDesktopNotificationsEnabled: false,
  shouldShowPreviews: true,
  isSoundEnabled: true,
  hasAnsweredPermissionBar: false,
  places: {},
};

/** At most this many places are kept, so a hostile or corrupted entry cannot grow without bound. */
const MAXIMUM_STORED_PLACES = 2_000;
const SNOWFLAKE = /^\d{1,20}$/u;

export function notificationSettingsStorageKey(userId: Snowflake): string {
  return `mellow.notifications.${userId}`;
}

export function muteEndFor(duration: MuteDuration, now: number): MuteEnd {
  return duration === "until-turned-back-on" ? "never" : now + MUTE_DURATION_MILLISECONDS[duration];
}

export function isMuteActive(mutedUntil: MuteEnd | undefined, now: number): boolean {
  return mutedUntil === "never" || (mutedUntil !== undefined && mutedUntil > now);
}

/** The earliest time a mute still running ends, to wake up then; null when none will. */
export function nextMuteEnd(settings: NotificationSettings, now: number): number | null {
  let earliest: number | null = null;
  for (const place of Object.values(settings.places)) {
    const end = place.mutedUntil;
    if (typeof end === "number" && end > now && (earliest === null || end < earliest)) {
      earliest = end;
    }
  }
  return earliest;
}

/** `settings` with `placeId`'s choices replaced; an entry left with nothing is dropped. */
export function withPlaceSettings(
  settings: NotificationSettings,
  placeId: Snowflake,
  place: PlaceNotificationSettings,
): NotificationSettings {
  const { [placeId]: _replaced, ...others } = settings.places;
  const kept = compactPlace(place);
  return { ...settings, places: kept === null ? others : { ...others, [placeId]: kept } };
}

function compactPlace(place: PlaceNotificationSettings): PlaceNotificationSettings | null {
  const compact: { level?: NotifyLevel; mutedUntil?: MuteEnd } = {};
  if (place.level !== undefined) {
    compact.level = place.level;
  }
  if (place.mutedUntil !== undefined) {
    compact.mutedUntil = place.mutedUntil;
  }
  return Object.keys(compact).length === 0 ? null : compact;
}

/** `settings` without mutes that ended, so storage does not keep them forever. */
export function withoutEndedMutes(settings: NotificationSettings, now: number): NotificationSettings {
  let result = settings;
  for (const [placeId, place] of Object.entries(settings.places)) {
    if (place.mutedUntil !== undefined && !isMuteActive(place.mutedUntil, now)) {
      result = withPlaceSettings(result, placeId, { ...place, mutedUntil: undefined });
    }
  }
  return result;
}

/** What storage held, kept only where it has the expected shape; anything else is the default. */
export function parseNotificationSettings(stored: unknown): NotificationSettings {
  if (typeof stored !== "object" || stored === null) {
    return DEFAULT_NOTIFICATION_SETTINGS;
  }
  const record = stored as Record<string, unknown>;
  const booleanOr = (key: keyof NotificationSettings, fallback: boolean) =>
    typeof record[key] === "boolean" ? record[key] : fallback;
  return {
    areDesktopNotificationsEnabled: booleanOr(
      "areDesktopNotificationsEnabled",
      DEFAULT_NOTIFICATION_SETTINGS.areDesktopNotificationsEnabled,
    ),
    shouldShowPreviews: booleanOr("shouldShowPreviews", DEFAULT_NOTIFICATION_SETTINGS.shouldShowPreviews),
    isSoundEnabled: booleanOr("isSoundEnabled", DEFAULT_NOTIFICATION_SETTINGS.isSoundEnabled),
    hasAnsweredPermissionBar: booleanOr(
      "hasAnsweredPermissionBar",
      DEFAULT_NOTIFICATION_SETTINGS.hasAnsweredPermissionBar,
    ),
    places: parsePlaces(record.places),
  };
}

function parsePlaces(stored: unknown): Record<Snowflake, PlaceNotificationSettings> {
  if (typeof stored !== "object" || stored === null || Array.isArray(stored)) {
    return {};
  }
  const places: Record<Snowflake, PlaceNotificationSettings> = {};
  for (const [placeId, value] of Object.entries(stored).slice(0, MAXIMUM_STORED_PLACES)) {
    if (!SNOWFLAKE.test(placeId) || typeof value !== "object" || value === null) {
      continue;
    }
    const { level, mutedUntil } = value as Record<string, unknown>;
    const isMuteEndTime = typeof mutedUntil === "number" && Number.isFinite(mutedUntil) && mutedUntil > 0;
    const place = compactPlace({
      level: NOTIFY_LEVELS.find((candidate) => candidate === level),
      mutedUntil: mutedUntil === "never" ? "never" : isMuteEndTime ? mutedUntil : undefined,
    });
    if (place !== null) {
      places[placeId] = place;
    }
  }
  return places;
}

export function readNotificationSettings(
  storage: Pick<Storage, "getItem"> | null,
  userId: Snowflake,
): NotificationSettings {
  try {
    const stored = storage?.getItem(notificationSettingsStorageKey(userId));
    return stored ? parseNotificationSettings(JSON.parse(stored)) : DEFAULT_NOTIFICATION_SETTINGS;
  } catch (error) {
    console.warn("unreadable notification settings", error);
    return DEFAULT_NOTIFICATION_SETTINGS;
  }
}

/** Stores `settings` when the browser lets us; a full or blocked storage keeps them for this session only. */
export function writeNotificationSettings(
  storage: Pick<Storage, "setItem"> | null,
  userId: Snowflake,
  settings: NotificationSettings,
): void {
  try {
    storage?.setItem(notificationSettingsStorageKey(userId), JSON.stringify(settings));
  } catch (error) {
    console.warn("couldn't store the notification settings", error);
  }
}
