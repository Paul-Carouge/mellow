import { describe, expect, it } from "vitest";

import {
  DEFAULT_NOTIFICATION_SETTINGS,
  type NotificationSettings,
  isMuteActive,
  muteEndFor,
  nextMuteEnd,
  notificationSettingsStorageKey,
  parseNotificationSettings,
  readNotificationSettings,
  withPlaceSettings,
  withoutEndedMutes,
  writeNotificationSettings,
} from "./notification-settings";

const NOW = 1_800_000_000_000;
const HOUR = 3_600_000;

function memoryStorage(): Pick<Storage, "getItem" | "setItem"> & { items: Map<string, string> } {
  const items = new Map<string, string>();
  return { items, getItem: (key) => items.get(key) ?? null, setItem: (key, value) => void items.set(key, value) };
}

describe("mute ends", () => {
  it("adds each offered length to now, and never ends the last one", () => {
    expect(muteEndFor("15-minutes", NOW)).toBe(NOW + HOUR / 4);
    expect(muteEndFor("1-hour", NOW)).toBe(NOW + HOUR);
    expect(muteEndFor("8-hours", NOW)).toBe(NOW + 8 * HOUR);
    expect(muteEndFor("24-hours", NOW)).toBe(NOW + 24 * HOUR);
    expect(muteEndFor("until-turned-back-on", NOW)).toBe("never");
  });

  it("is active until its end, not at it", () => {
    expect(isMuteActive(NOW + 1, NOW)).toBe(true);
    expect(isMuteActive(NOW, NOW)).toBe(false);
    expect(isMuteActive("never", NOW)).toBe(true);
    expect(isMuteActive(undefined, NOW)).toBe(false);
  });

  it("wakes up at the earliest end still to come", () => {
    const settings: NotificationSettings = {
      ...DEFAULT_NOTIFICATION_SETTINGS,
      places: { "1": { mutedUntil: NOW + 2 * HOUR }, "2": { mutedUntil: NOW + HOUR }, "3": { mutedUntil: "never" }, "4": { mutedUntil: NOW - 1 } },
    };
    expect(nextMuteEnd(settings, NOW)).toBe(NOW + HOUR);
    expect(nextMuteEnd(DEFAULT_NOTIFICATION_SETTINGS, NOW)).toBeNull();
  });

  it("forgets ended mutes but keeps the level beside them", () => {
    const settings: NotificationSettings = {
      ...DEFAULT_NOTIFICATION_SETTINGS,
      places: { "1": { mutedUntil: NOW - 1, level: "all" }, "2": { mutedUntil: NOW - 1 }, "3": { mutedUntil: NOW + 1 } },
    };
    expect(withoutEndedMutes(settings, NOW).places).toEqual({ "1": { level: "all" }, "3": { mutedUntil: NOW + 1 } });
    expect(withoutEndedMutes(DEFAULT_NOTIFICATION_SETTINGS, NOW)).toBe(DEFAULT_NOTIFICATION_SETTINGS);
  });
});

describe("place settings", () => {
  it("drops an entry left with nothing, so storage holds only real choices", () => {
    const muted = withPlaceSettings(DEFAULT_NOTIFICATION_SETTINGS, "7", { mutedUntil: "never" });
    expect(muted.places).toEqual({ "7": { mutedUntil: "never" } });
    expect(withPlaceSettings(muted, "7", { mutedUntil: undefined }).places).toEqual({});
  });
});

describe("parsing what storage held", () => {
  it("falls back to the defaults for anything but an object", () => {
    for (const stored of [null, 3, "x", [], undefined]) {
      expect(parseNotificationSettings(stored)).toEqual(DEFAULT_NOTIFICATION_SETTINGS);
    }
  });

  it("keeps well-formed fields and drops the rest", () => {
    const parsed = parseNotificationSettings({
      areDesktopNotificationsEnabled: true,
      shouldShowPreviews: "yes",
      isSoundEnabled: false,
      places: {
        "10": { level: "nothing", mutedUntil: NOW },
        "11": { level: "loud", mutedUntil: "forever" },
        "12": { mutedUntil: "never" },
        "not-an-id": { level: "all" },
        "13": { mutedUntil: Number.POSITIVE_INFINITY },
        "14": "all",
      },
    });
    expect(parsed).toEqual({
      areDesktopNotificationsEnabled: true,
      shouldShowPreviews: true,
      isSoundEnabled: false,
      hasAnsweredPermissionBar: false,
      places: { "10": { level: "nothing", mutedUntil: NOW }, "12": { mutedUntil: "never" } },
    });
  });
});

describe("storage per user", () => {
  it("keeps each user's choices apart", () => {
    const storage = memoryStorage();
    const settings = withPlaceSettings({ ...DEFAULT_NOTIFICATION_SETTINGS, isSoundEnabled: false }, "9", { level: "all" });
    writeNotificationSettings(storage, "1", settings);
    expect(storage.items.has(notificationSettingsStorageKey("1"))).toBe(true);
    expect(readNotificationSettings(storage, "1")).toEqual(settings);
    expect(readNotificationSettings(storage, "2")).toEqual(DEFAULT_NOTIFICATION_SETTINGS);
  });

  it("survives a blocked or corrupted storage", () => {
    const throwing = {
      getItem: () => {
        throw new Error("blocked");
      },
      setItem: () => {
        throw new Error("full");
      },
    };
    expect(readNotificationSettings(throwing, "1")).toEqual(DEFAULT_NOTIFICATION_SETTINGS);
    expect(() => writeNotificationSettings(throwing, "1", DEFAULT_NOTIFICATION_SETTINGS)).not.toThrow();
    const corrupted = memoryStorage();
    corrupted.setItem(notificationSettingsStorageKey("1"), "{not json");
    expect(readNotificationSettings(corrupted, "1")).toEqual(DEFAULT_NOTIFICATION_SETTINGS);
    expect(readNotificationSettings(null, "1")).toEqual(DEFAULT_NOTIFICATION_SETTINGS);
  });
});
