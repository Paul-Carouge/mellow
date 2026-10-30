// Tells the reader about new messages (docs/design/web-notifications.md): browser notifications
// while they are away, Melli's chime and the sound while they are elsewhere in Mellow, the count
// in the tab's title and a dot on the favicon. The rules are in `notifications.ts`.

import { strings } from "../i18n";
import type { MellowClient } from "../lib/client.svelte";
import type { Message, Snowflake } from "../lib/protocol";
import { playChimeSound, prepareChimeSound } from "./chime-sound";
import { displayName } from "./format";
import { conversationDisplayName, isGroupConversation } from "./group-conversations";
import {
  isMuted,
  noteStoredNotificationSettingsChanged,
  notificationSettings,
  refreshNotificationPreferences,
} from "./notification-preferences.svelte";
import {
  type NotificationPlace,
  alertDeliveryFor,
  messagePreviewText,
  notificationContent,
  shouldAlertAbout,
  tabTitle,
} from "./notifications";
import { colorTokenValue } from "./tokens";

/** The browser's answer, or "unsupported" when it has no notifications Mellow can show. */
export type NotificationSupport = NotificationPermission | "unsupported";

const APP_NAME = "Mellow";
const NOTIFICATION_ICON_URL = "/icons/mellow-192.png";
const FAVICON_URL = "/favicon.svg";
/** Events that count as a gesture for audio, listened to in the capture phase. */
const AUDIO_UNLOCKING_GESTURES = ["pointerdown", "pointerup", "keydown"] as const;
/** A burst rings once: the chime does not play again within this time. */
const MINIMUM_MILLISECONDS_BETWEEN_SOUNDS = 2_000;

const notifierState = $state({ support: readSupport(), chimeCount: 0 });

function readSupport(): NotificationSupport {
  // shortcut: Chrome on Android has the API but refuses `new Notification` without a service
  // worker, which Mellow keeps out; Android is treated as unsupported so the bar never asks for
  // nothing. Revisit with push notifications (a worker and an ADR).
  const isAndroid = typeof navigator !== "undefined" && /Android/iu.test(navigator.userAgent);
  return typeof Notification === "undefined" || isAndroid ? "unsupported" : Notification.permission;
}

export function notificationSupport(): NotificationSupport {
  return notifierState.support;
}

/** Increases with each alert; Melli plays its chime once per increase. */
export function melliChimeCount(): number {
  return notifierState.chimeCount;
}

/** Asks the browser; call it only from a gesture (the settings switch, the permission bar). */
export async function askNotificationPermission(): Promise<NotificationSupport> {
  if (typeof Notification === "undefined") {
    return "unsupported";
  }
  try {
    notifierState.support = await Notification.requestPermission();
  } catch (error) {
    console.warn("couldn't ask for notification permission", error);
    notifierState.support = readSupport();
  }
  return notifierState.support;
}

/** Alerts per channel since the reader last looked at it, with its browser notification. */
const alertsByChannel = new Map<Snowflake, { count: number; notification: Notification | null }>();
let lastSoundAt = 0;

/** The reader looked at `channelId`: its notification goes and its count starts again. */
export function noteChannelSeen(channelId: Snowflake): void {
  alertsByChannel.get(channelId)?.notification?.close();
  alertsByChannel.delete(channelId);
}

function placeOf(client: MellowClient, channelId: Snowflake): NotificationPlace | null {
  if (client.conversations.some((conversation) => conversation.id === channelId)) {
    return { guildId: null, channelId };
  }
  const guild = client.guilds.find((candidate) => candidate.channels.some((channel) => channel.id === channelId));
  return guild ? { guildId: guild.id, channelId } : null;
}

function placeTitleOf(client: MellowClient, place: NotificationPlace): { title: string; isOneToOne: boolean } {
  if (place.guildId === null) {
    const conversation = client.conversations.find((candidate) => candidate.id === place.channelId);
    return conversation
      ? { title: conversationDisplayName(conversation, client.user?.id ?? null), isOneToOne: !isGroupConversation(conversation) }
      : { title: APP_NAME, isOneToOne: false };
  }
  const guild = client.guilds.find((candidate) => candidate.id === place.guildId);
  const channel = guild?.channels.find((candidate) => candidate.id === place.channelId);
  return {
    title: guild && channel ? strings.notifications.channelInServer(channel.name, guild.name) : APP_NAME,
    isOneToOne: false,
  };
}

function isReaderLookingAtPage(): { isPageVisible: boolean; hasWindowFocus: boolean } {
  return { isPageVisible: document.visibilityState === "visible", hasWindowFocus: document.hasFocus() };
}

function alertAbout(client: MellowClient, message: Message): void {
  const place = placeOf(client, message.channel_id);
  const settings = notificationSettings();
  if (
    place === null ||
    !shouldAlertAbout({
      message,
      currentUserId: client.user?.id ?? null,
      place,
      isAuthorBlocked: client.isBlocked(message.author.id),
      chosenStatus: client.user?.status,
      settings,
      now: Date.now(),
    })
  ) {
    return;
  }
  const delivery = alertDeliveryFor(message.channel_id, {
    ...isReaderLookingAtPage(),
    openChannelId: client.selectedChannelId,
  });
  if (delivery === "none") {
    return;
  }
  notifierState.chimeCount += 1;
  const now = Date.now();
  if (settings.isSoundEnabled && now - lastSoundAt >= MINIMUM_MILLISECONDS_BETWEEN_SOUNDS) {
    lastSoundAt = now;
    playChimeSound();
  }
  if (delivery === "browser" && settings.areDesktopNotificationsEnabled && notifierState.support === "granted") {
    showBrowserNotification(client, message, place);
  }
}

function showBrowserNotification(client: MellowClient, message: Message, place: NotificationPlace): void {
  const previous = alertsByChannel.get(message.channel_id);
  const count = (previous?.count ?? 0) + 1;
  const { title: placeTitle, isOneToOne } = placeTitleOf(client, place);
  const { title, body } = notificationContent({
    authorName: displayName(message.author),
    placeTitle,
    isOneToOneConversation: isOneToOne,
    preview: messagePreviewText(message),
    messageCount: count,
    shouldShowPreview: notificationSettings().shouldShowPreviews,
  });
  let notification: Notification;
  try {
    // One per channel: the tag replaces the channel's previous one, silently (the chime is ours).
    notification = new Notification(title, { body, tag: `mellow-channel-${message.channel_id}`, icon: NOTIFICATION_ICON_URL, silent: true });
  } catch (error) {
    // Chrome on Android only shows notifications through a service worker, which Mellow keeps out.
    console.warn("couldn't show a notification", error);
    notifierState.support = "unsupported";
    return;
  }
  const entry = { count, notification };
  alertsByChannel.set(message.channel_id, entry);
  notification.onclick = () => {
    window.focus();
    client.selectGuild(place.guildId, place.channelId);
    notification.close();
  };
  notification.onclose = () => {
    if (alertsByChannel.get(message.channel_id) === entry) {
      alertsByChannel.delete(message.channel_id);
    }
  };
}

/** Listens to `client`'s new messages until the returned function is called (sign-out, unmount). */
export function startNotifier(client: MellowClient): () => void {
  client.onIncomingMessage = (message) => alertAbout(client, message);
  // Audio may only start from a gesture: the first click, tap or key prepares the chime. A touch's
  // `pointerdown` is not a user activation, its `pointerup` is.
  const prepareSound = () => {
    if (notificationSettings().isSoundEnabled) {
      void prepareChimeSound();
    }
  };
  // The permission may change in the browser's own settings, and mutes may have ended while the
  // computer slept; the open channel is seen on return.
  const noteReturn = () => {
    refreshNotificationPreferences();
    if (notifierState.support !== "unsupported") {
      notifierState.support = readSupport();
    }
    const { isPageVisible, hasWindowFocus } = isReaderLookingAtPage();
    if (isPageVisible && hasWindowFocus && client.selectedChannelId !== null) {
      noteChannelSeen(client.selectedChannelId);
    }
  };
  // Another tab changed this user's choices: read them again before this tab writes its own.
  const noteStorage = (event: StorageEvent) => noteStoredNotificationSettingsChanged(event.key);
  for (const gesture of AUDIO_UNLOCKING_GESTURES) {
    window.addEventListener(gesture, prepareSound, { capture: true });
  }
  window.addEventListener("focus", noteReturn);
  window.addEventListener("storage", noteStorage);
  document.addEventListener("visibilitychange", noteReturn);
  return () => {
    client.onIncomingMessage = null;
    for (const gesture of AUDIO_UNLOCKING_GESTURES) {
      window.removeEventListener(gesture, prepareSound, { capture: true });
    }
    window.removeEventListener("focus", noteReturn);
    window.removeEventListener("storage", noteStorage);
    document.removeEventListener("visibilitychange", noteReturn);
    for (const { notification } of alertsByChannel.values()) {
      notification?.close();
    }
    alertsByChannel.clear();
    showAttentionCount(0);
  };
}

/** Mention badges and the badges of conversations not muted, added up. */
export function attentionCount(client: MellowClient): number {
  const inServers = client.guilds.reduce((sum, guild) => sum + client.guildMentionCount(guild), 0);
  const inConversations = client.conversations.reduce(
    (sum, conversation) =>
      isMuted({ guildId: null, channelId: conversation.id }) ? sum : sum + client.mentionCount(conversation.id),
    0,
  );
  return inServers + inConversations;
}

// --- Title and favicon -----------------------------------------------------------------------

let plainFaviconSource: Promise<string | null> | null = null;
let isFaviconDotShown = false;

/** "(3) Mellow" and a red dot on the favicon while the count is above zero. */
export function showAttentionCount(count: number): void {
  document.title = tabTitle(count, APP_NAME);
  const shouldShowDot = count > 0;
  if (shouldShowDot !== isFaviconDotShown) {
    isFaviconDotShown = shouldShowDot;
    void setFaviconDot(shouldShowDot);
  }
}

async function setFaviconDot(shouldShowDot: boolean): Promise<void> {
  const link = document.querySelector<HTMLLinkElement>('link[rel="icon"]');
  if (!link) {
    return;
  }
  if (!shouldShowDot) {
    link.href = FAVICON_URL;
    return;
  }
  plainFaviconSource ??= fetch(FAVICON_URL)
    .then((response) => (response.ok ? response.text() : null))
    .catch((error: unknown) => {
      console.warn("couldn't load the favicon", error);
      return null;
    });
  const source = await plainFaviconSource;
  // The count may have dropped to zero while the favicon loaded.
  if (source === null || !isFaviconDotShown) {
    return;
  }
  // The badge colour on a ring of the tile's darkest dusk, in the tile's bottom trailing corner.
  const dot = `<circle cx="196" cy="196" r="54" fill="${colorTokenValue("component-badge-background")}" stroke="${colorTokenValue("color-mascot-tile-bottom")}" stroke-width="14"/>`;
  link.href = `data:image/svg+xml,${encodeURIComponent(source.replace("</svg>", `${dot}</svg>`))}`;
}
