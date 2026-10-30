import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { notificationSettingsStorageKey, withPlaceSettings } from "../lib/notification-settings";
import {
  isTargetMuted,
  loadNotificationPreferences,
  noteStoredNotificationSettingsChanged,
  notificationSettings,
  refreshNotificationPreferences,
  updateNotificationSettings,
} from "./notification-preferences.svelte";

const CHANNEL = { kind: "conversation", channelId: "500" } as const;
const MINUTE = 60_000;

/** The browser's `localStorage`, shared by "tabs" that are just calls here. */
function memoryStorage(): Storage {
  const items = new Map<string, string>();
  return {
    get length() {
      return items.size;
    },
    clear: () => items.clear(),
    getItem: (key) => items.get(key) ?? null,
    key: (index) => [...items.keys()][index] ?? null,
    removeItem: (key) => void items.delete(key),
    setItem: (key, value) => void items.set(key, value),
  };
}

let userNumber = 0;

beforeEach(() => {
  vi.useFakeTimers();
  vi.stubGlobal("localStorage", memoryStorage());
  // A new user each time, so the module's state starts from this test's storage.
  userNumber += 1;
  loadNotificationPreferences(String(userNumber));
});

afterEach(() => {
  loadNotificationPreferences(null);
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

describe("mute ends", () => {
  it("ends a timed mute by itself when its time comes, and forgets it", () => {
    const end = Date.now() + 15 * MINUTE;
    updateNotificationSettings((settings) => withPlaceSettings(settings, CHANNEL.channelId, { mutedUntil: end }));
    expect(isTargetMuted(CHANNEL)).toBe(true);
    vi.advanceTimersByTime(15 * MINUTE - 1);
    expect(isTargetMuted(CHANNEL)).toBe(true);
    vi.advanceTimersByTime(2);
    expect(isTargetMuted(CHANNEL)).toBe(false);
    expect(notificationSettings().places).toEqual({});
  });

  it("judges mutes again on refresh, as after the computer slept past the timer", () => {
    const end = Date.now() + MINUTE;
    updateNotificationSettings((settings) => withPlaceSettings(settings, CHANNEL.channelId, { mutedUntil: end }));
    // The clock jumps without the timer firing.
    vi.setSystemTime(end + MINUTE);
    expect(isTargetMuted(CHANNEL)).toBe(true);
    refreshNotificationPreferences();
    expect(isTargetMuted(CHANNEL)).toBe(false);
  });

  it("never ends a mute until turned back on", () => {
    updateNotificationSettings((settings) => withPlaceSettings(settings, CHANNEL.channelId, { mutedUntil: "never" }));
    vi.advanceTimersByTime(365 * 24 * 60 * MINUTE);
    refreshNotificationPreferences();
    expect(isTargetMuted(CHANNEL)).toBe(true);
  });
});

describe("other tabs", () => {
  it("reads again what another tab stored, so the next write keeps its mute", () => {
    const key = notificationSettingsStorageKey(String(userNumber));
    // Another tab mutes the conversation and stores its choices.
    localStorage.setItem(key, JSON.stringify({ places: { [CHANNEL.channelId]: { mutedUntil: "never" } } }));
    noteStoredNotificationSettingsChanged("someone-else's-key");
    expect(isTargetMuted(CHANNEL)).toBe(false);
    noteStoredNotificationSettingsChanged(key);
    expect(isTargetMuted(CHANNEL)).toBe(true);
    // This tab now changes something else: the other tab's mute survives.
    updateNotificationSettings((settings) => ({ ...settings, isSoundEnabled: false }));
    expect(JSON.parse(localStorage.getItem(key) ?? "{}").places).toEqual({ [CHANNEL.channelId]: { mutedUntil: "never" } });
  });
});
