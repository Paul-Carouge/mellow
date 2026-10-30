// The signed-in reader's notification choices as state, shared by the sidebar, the rail, the
// dialogs, the settings page and the notifier. Mutes end on time through one timer set to the
// earliest end, so nothing polls.

import { strings } from "../i18n";
import {
  DEFAULT_NOTIFICATION_SETTINGS,
  type NotificationSettings,
  isMuteActive,
  nextMuteEnd,
  notificationSettingsStorageKey,
  readNotificationSettings,
  withPlaceSettings,
  withoutEndedMutes,
  writeNotificationSettings,
} from "../lib/notification-settings";
import type { Snowflake } from "../lib/protocol";
import { browserStorage } from "../lib/reactions";
import type { MenuItemHandler, NotificationMenuAction } from "./context-menus";
import { type NotificationPlace, type NotificationTarget, isPlaceMuted, notificationTargetId } from "./notifications";

const preferences = $state<{ userId: Snowflake | null; settings: NotificationSettings; now: number }>({
  userId: null,
  settings: DEFAULT_NOTIFICATION_SETTINGS,
  now: Date.now(),
});
let muteEndTimer: ReturnType<typeof setTimeout> | null = null;

/** Reads the choices of `userId` (null at sign-out forgets them); does nothing for the same user. */
export function loadNotificationPreferences(userId: Snowflake | null): void {
  if (userId === preferences.userId) {
    return;
  }
  preferences.userId = userId;
  preferences.settings = userId === null ? DEFAULT_NOTIFICATION_SETTINGS : readNotificationSettings(browserStorage(), userId);
  refreshClock();
}

export function notificationSettings(): NotificationSettings {
  return preferences.settings;
}

/**
 * Another tab stored this user's choices (the window's `storage` event, `null` when storage was
 * cleared): they are read again, so this tab's next write does not erase that tab's mute.
 */
export function noteStoredNotificationSettingsChanged(storageKey: string | null): void {
  const userId = preferences.userId;
  if (userId === null || (storageKey !== null && storageKey !== notificationSettingsStorageKey(userId))) {
    return;
  }
  preferences.settings = readNotificationSettings(browserStorage(), userId);
  refreshClock();
}

/** Judges mutes again now: after the computer slept, a timer may have fired late or not yet. */
export function refreshNotificationPreferences(): void {
  refreshClock();
}

/** Applies `change` and stores the result for this user. */
export function updateNotificationSettings(change: (settings: NotificationSettings) => NotificationSettings): void {
  preferences.settings = change(preferences.settings);
  if (preferences.userId !== null) {
    writeNotificationSettings(browserStorage(), preferences.userId, preferences.settings);
  }
  refreshClock();
}

/** Muted itself or in a muted server. */
export function isMuted(place: NotificationPlace): boolean {
  return isPlaceMuted(preferences.settings, place, preferences.now);
}

/** The target's own mute, as its menu shows it (a channel of a muted server is not "muted" there). */
export function isTargetMuted(target: NotificationTarget): boolean {
  return isMuteActive(preferences.settings.places[notificationTargetId(target)]?.mutedUntil, preferences.now);
}

/** "Unmute" in a menu: at once, keeping the "Notify me about" choice. */
export function unmute(target: NotificationTarget): void {
  const targetId = notificationTargetId(target);
  updateNotificationSettings((settings) =>
    withPlaceSettings(settings, targetId, { ...settings.places[targetId], mutedUntil: undefined }),
  );
}

// --- The dialog, opened from any menu and shown by the shell ---------------------------------

export interface NotificationDialogRequest {
  readonly target: NotificationTarget;
  readonly placeName: string;
  readonly initialGroup: "mute" | "level";
}

const dialog = $state<{ request: NotificationDialogRequest | null }>({ request: null });

export function openNotificationDialog(request: NotificationDialogRequest): void {
  dialog.request = request;
}

export function notificationDialogRequest(): NotificationDialogRequest | null {
  return dialog.request;
}

export function closeNotificationDialog(): void {
  dialog.request = null;
}

/** What "Mute…", "Unmute" and "Notification settings…" do in any menu. */
export function notificationMenuHandler(
  action: NotificationMenuAction,
  target: NotificationTarget,
  placeName: string,
): MenuItemHandler {
  if (action === "unmute") {
    return {
      onselectAndWait: async () => {
        unmute(target);
        return { announcement: strings.notifications.unmuted(placeName) };
      },
    };
  }
  return {
    onselect: () => openNotificationDialog({ target, placeName, initialGroup: action === "mute" ? "mute" : "level" }),
  };
}

function refreshClock(): void {
  const now = Date.now();
  preferences.now = now;
  const cleaned = withoutEndedMutes(preferences.settings, now);
  if (cleaned !== preferences.settings) {
    preferences.settings = cleaned;
    if (preferences.userId !== null) {
      writeNotificationSettings(browserStorage(), preferences.userId, cleaned);
    }
  }
  if (muteEndTimer !== null) {
    clearTimeout(muteEndTimer);
    muteEndTimer = null;
  }
  const end = nextMuteEnd(cleaned, now);
  if (end !== null) {
    // Browsers cap timeouts at about 24.8 days; a longer mute just wakes up early and sets another.
    const MAXIMUM_TIMEOUT_MILLISECONDS = 2_147_483_647;
    muteEndTimer = setTimeout(refreshClock, Math.min(end - now + 1, MAXIMUM_TIMEOUT_MILLISECONDS));
  }
}
