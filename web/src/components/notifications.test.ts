import { describe, expect, it } from "vitest";

import { DEFAULT_NOTIFICATION_SETTINGS, type NotificationSettings } from "../lib/notification-settings";
import {
  type IncomingMessageFacts,
  MAXIMUM_PREVIEW_CHARACTERS,
  alertDeliveryFor,
  messagePreviewText,
  notificationContent,
  CURRENT_MUTE_CHOICE,
  NOT_MUTED_CHOICE,
  SERVER_LEVEL_CHOICE,
  notifyLevelOf,
  placeSettingsFromDialog,
  shouldAlertAbout,
  tabTitle,
} from "./notifications";

const ME = "1";
const ANA = "2";
const SERVER = "100";
const GENERAL = "101";
const CONVERSATION = "200";
const NOW = 1_800_000_000_000;

function settingsWith(places: NotificationSettings["places"]): NotificationSettings {
  return { ...DEFAULT_NOTIFICATION_SETTINGS, places };
}

function facts(overrides: Partial<IncomingMessageFacts> & { mentionsMe?: boolean } = {}): IncomingMessageFacts {
  const { mentionsMe = false, ...rest } = overrides;
  return {
    message: { author: { id: ANA, username: "ana", display_name: "Ana" }, mention_ids: mentionsMe ? [ME] : [] },
    currentUserId: ME,
    place: { guildId: SERVER, channelId: GENERAL },
    isAuthorBlocked: false,
    chosenStatus: "online",
    settings: DEFAULT_NOTIFICATION_SETTINGS,
    now: NOW,
    ...rest,
  };
}

const IN_CONVERSATION = { guildId: null, channelId: CONVERSATION };

describe("who is told", () => {
  it("tells about mentions in a server and every message in a conversation, by default", () => {
    expect(shouldAlertAbout(facts({ mentionsMe: true }))).toBe(true);
    expect(shouldAlertAbout(facts())).toBe(false);
    expect(shouldAlertAbout(facts({ place: IN_CONVERSATION }))).toBe(true);
  });

  it("never tells about the reader's own messages, blocked authors, or anything in Do not disturb", () => {
    const own = facts({ place: IN_CONVERSATION, mentionsMe: true });
    expect(shouldAlertAbout({ ...own, message: { ...own.message, author: { id: ME, username: "me", display_name: "Me" } } })).toBe(false);
    expect(shouldAlertAbout(facts({ place: IN_CONVERSATION, isAuthorBlocked: true }))).toBe(false);
    expect(shouldAlertAbout(facts({ place: IN_CONVERSATION, chosenStatus: "do_not_disturb", mentionsMe: true }))).toBe(false);
    expect(shouldAlertAbout(facts({ place: IN_CONVERSATION, currentUserId: null }))).toBe(false);
  });

  it("follows the channel's level, else the server's", () => {
    const allInServer = settingsWith({ [SERVER]: { level: "all" } });
    expect(shouldAlertAbout(facts({ settings: allInServer }))).toBe(true);
    const nothingInChannel = settingsWith({ [SERVER]: { level: "all" }, [GENERAL]: { level: "nothing" } });
    expect(shouldAlertAbout(facts({ settings: nothingInChannel, mentionsMe: true }))).toBe(false);
    expect(notifyLevelOf(nothingInChannel, { guildId: SERVER, channelId: "999" })).toBe("all");
    const mentionsInConversation = settingsWith({ [CONVERSATION]: { level: "mentions" } });
    expect(shouldAlertAbout(facts({ place: IN_CONVERSATION, settings: mentionsInConversation }))).toBe(false);
  });

  it("lets only mentions through a mute, of the channel or its server, while it runs", () => {
    const mutedChannel = settingsWith({ [GENERAL]: { level: "all", mutedUntil: NOW + 1 } });
    expect(shouldAlertAbout(facts({ settings: mutedChannel }))).toBe(false);
    expect(shouldAlertAbout(facts({ settings: mutedChannel, mentionsMe: true }))).toBe(true);
    const mutedServer = settingsWith({ [SERVER]: { level: "all", mutedUntil: "never" } });
    expect(shouldAlertAbout(facts({ settings: mutedServer }))).toBe(false);
    expect(shouldAlertAbout(facts({ settings: mutedServer, now: NOW, mentionsMe: true }))).toBe(true);
    const ended = settingsWith({ [GENERAL]: { level: "all", mutedUntil: NOW } });
    expect(shouldAlertAbout(facts({ settings: ended }))).toBe(true);
    const mutedConversation = settingsWith({ [CONVERSATION]: { mutedUntil: "never" } });
    expect(shouldAlertAbout(facts({ place: IN_CONVERSATION, settings: mutedConversation }))).toBe(false);
  });
});

describe("how they are told", () => {
  it("stays silent on the channel the reader is looking at", () => {
    expect(alertDeliveryFor(GENERAL, { isPageVisible: true, hasWindowFocus: true, openChannelId: GENERAL })).toBe("none");
  });

  it("tells in the app while the reader is elsewhere in Mellow", () => {
    expect(alertDeliveryFor(GENERAL, { isPageVisible: true, hasWindowFocus: true, openChannelId: CONVERSATION })).toBe("in-app");
  });

  it("uses the browser while the tab is hidden or the window has lost focus, even on the open channel", () => {
    expect(alertDeliveryFor(GENERAL, { isPageVisible: false, hasWindowFocus: false, openChannelId: GENERAL })).toBe("browser");
    expect(alertDeliveryFor(GENERAL, { isPageVisible: true, hasWindowFocus: false, openChannelId: GENERAL })).toBe("browser");
  });
});

describe("words", () => {
  it("previews plain text without Markdown and never a spoiler", () => {
    expect(messagePreviewText({ content: "**see** you at\n\n||the secret place|| at `7`" })).toBe(
      "see you at spoiler at 7",
    );
    expect(messagePreviewText({ content: "look <:party:123> > here" })).toBe("look :party: > here");
  });

  it("says files were sent when there is no text", () => {
    const file = { id: "1", file_name: "a.png", content_type: "image/png", size_bytes: 1 };
    expect(messagePreviewText({ content: "", attachments: [file] })).toBe("Sent a file");
    expect(messagePreviewText({ content: " ", attachments: [file, file] })).toBe("Sent 2 files");
    expect(messagePreviewText({ content: "" })).toBe("");
  });

  it("cuts long messages on a character boundary, with an ellipsis", () => {
    const preview = messagePreviewText({ content: "é".repeat(500) });
    expect(Array.from(preview)).toHaveLength(MAXIMUM_PREVIEW_CHARACTERS);
    expect(preview.endsWith("…")).toBe(true);
  });

  it("names the author in the body, except in a one-to-one conversation, and counts a burst", () => {
    const base = {
      authorName: "Ana",
      placeTitle: "#general · Rust Lyon",
      isOneToOneConversation: false,
      preview: "see you at 7?",
      messageCount: 1,
      shouldShowPreview: true,
    };
    expect(notificationContent(base)).toEqual({ title: "#general · Rust Lyon", body: "Ana: see you at 7?" });
    expect(notificationContent({ ...base, placeTitle: "Ana", isOneToOneConversation: true, messageCount: 3 })).toEqual({
      title: "Ana, 3 new messages",
      body: "see you at 7?",
    });
  });

  it("hides the preview when asked, and says who wrote", () => {
    expect(
      notificationContent({
        authorName: "Ana",
        placeTitle: "Ana",
        isOneToOneConversation: true,
        preview: "secret",
        messageCount: 1,
        shouldShowPreview: false,
      }).body,
    ).toBe("New message from Ana");
  });

  it("puts the count in the tab's title, capped at 99+", () => {
    expect(tabTitle(0, "Mellow")).toBe("Mellow");
    expect(tabTitle(3, "Mellow")).toBe("(3) Mellow");
    expect(tabTitle(250, "Mellow")).toBe("(99+) Mellow");
  });
});

describe("what the dialog saves", () => {
  const base = {
    targetKind: "server" as const,
    chosenMute: NOT_MUTED_CHOICE,
    chosenLevel: "mentions",
    runningMuteEnd: undefined,
    fallbackLevel: "mentions" as const,
    now: NOW,
  };

  it("keeps a level equal to the default implicit, and stores any other", () => {
    expect(placeSettingsFromDialog(base)).toEqual({ mutedUntil: undefined, level: undefined });
    expect(placeSettingsFromDialog({ ...base, chosenLevel: "all" }).level).toBe("all");
    const conversation = { ...base, targetKind: "conversation" as const, fallbackLevel: "all" as const };
    expect(placeSettingsFromDialog({ ...conversation, chosenLevel: "all" }).level).toBeUndefined();
    expect(placeSettingsFromDialog({ ...conversation, chosenLevel: "nothing" }).level).toBe("nothing");
  });

  it('stores no level for "Same as the server", and a channel\'s own pick even when it matches', () => {
    const channel = { ...base, targetKind: "channel" as const };
    expect(placeSettingsFromDialog({ ...channel, chosenLevel: SERVER_LEVEL_CHOICE }).level).toBeUndefined();
    expect(placeSettingsFromDialog({ ...channel, chosenLevel: "mentions" }).level).toBe("mentions");
  });

  it("keeps a running timed mute's end instead of restarting it", () => {
    const running = { ...base, runningMuteEnd: NOW + 600_000, now: NOW + 300_000 };
    expect(placeSettingsFromDialog({ ...running, chosenMute: CURRENT_MUTE_CHOICE }).mutedUntil).toBe(NOW + 600_000);
    expect(placeSettingsFromDialog({ ...running, chosenMute: "1-hour" }).mutedUntil).toBe(NOW + 300_000 + 3_600_000);
    expect(placeSettingsFromDialog({ ...running, chosenMute: NOT_MUTED_CHOICE }).mutedUntil).toBeUndefined();
    expect(placeSettingsFromDialog({ ...base, chosenMute: "until-turned-back-on" }).mutedUntil).toBe("never");
  });
});
