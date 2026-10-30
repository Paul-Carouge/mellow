// Who is told about a message, how, and in what words (docs/design/web-notifications.md). Pure:
// the notifier (`notifier.svelte.ts`) gathers the facts and does the telling.

import { strings } from "../i18n";
import {
  MUTE_DURATIONS,
  type MuteEnd,
  NOTIFY_LEVELS,
  type NotificationSettings,
  type NotifyLevel,
  type PlaceNotificationSettings,
  isMuteActive,
  muteEndFor,
} from "../lib/notification-settings";
import type { ChosenStatus, Message, Snowflake } from "../lib/protocol";
import { withCustomEmojiAsNames } from "./custom-emoji";
import { type BlockNode, type InlineNode, parseMessageMarkdown } from "./markdown";
import { mentionablePeople } from "./mentions";

/** A message's place: a server's channel, or a conversation (no server). */
export interface NotificationPlace {
  readonly guildId: Snowflake | null;
  readonly channelId: Snowflake;
}

/** What a mute or a "Notify me about" choice applies to (the menus and their dialog). */
export type NotificationTarget =
  | { readonly kind: "server"; readonly guildId: Snowflake }
  | { readonly kind: "channel"; readonly guildId: Snowflake; readonly channelId: Snowflake }
  | { readonly kind: "conversation"; readonly channelId: Snowflake };

/** The key its choices are stored under. */
export function notificationTargetId(target: NotificationTarget): Snowflake {
  return target.kind === "server" ? target.guildId : target.channelId;
}

/** As on Discord: a server tells about mentions only, a conversation about every message. */
export function defaultNotifyLevel(guildId: Snowflake | null): NotifyLevel {
  return guildId === null ? "all" : "mentions";
}

/** What a server's channels follow when they have no choice of their own. */
export function serverNotifyLevel(settings: NotificationSettings, guildId: Snowflake): NotifyLevel {
  return settings.places[guildId]?.level ?? defaultNotifyLevel(guildId);
}

/** The channel's own choice, else its server's, else the default. */
export function notifyLevelOf(settings: NotificationSettings, place: NotificationPlace): NotifyLevel {
  const own = settings.places[place.channelId]?.level;
  if (own !== undefined) {
    return own;
  }
  return place.guildId === null ? defaultNotifyLevel(null) : serverNotifyLevel(settings, place.guildId);
}

// --- The dialog's choices --------------------------------------------------------------------

/** The dialog's radio values beside the durations and the levels. */
export const NOT_MUTED_CHOICE = "off";
/** The timed mute running now, kept as it is. */
export const CURRENT_MUTE_CHOICE = "current";
/** "Same as the server", for a channel. */
export const SERVER_LEVEL_CHOICE = "server";

export interface NotificationDialogChoices {
  readonly targetKind: NotificationTarget["kind"];
  /** `NOT_MUTED_CHOICE`, `CURRENT_MUTE_CHOICE` or a `MuteDuration`. */
  readonly chosenMute: string;
  /** `SERVER_LEVEL_CHOICE` or a `NotifyLevel`. */
  readonly chosenLevel: string;
  /** The mute running when the dialog opened, if any. */
  readonly runningMuteEnd: MuteEnd | undefined;
  /** What applies without a choice of its own: the server's for a channel, else the default. */
  readonly fallbackLevel: NotifyLevel;
  readonly now: number;
}

/**
 * What Save stores for the place. Keeping a timed mute keeps its end (it does not restart), and
 * a level equal to the default stays implicit, so a later change of default or of the server's
 * choice still applies; "Same as the server" stores no level. A channel's explicit level is kept
 * even when it matches the server's, since the reader chose it over following the server.
 */
export function placeSettingsFromDialog(choices: NotificationDialogChoices): PlaceNotificationSettings {
  const duration = MUTE_DURATIONS.find((candidate) => candidate === choices.chosenMute);
  const mutedUntil =
    choices.chosenMute === CURRENT_MUTE_CHOICE
      ? choices.runningMuteEnd
      : duration === undefined
        ? undefined
        : muteEndFor(duration, choices.now);
  const pickedLevel = NOTIFY_LEVELS.find((level) => level === choices.chosenLevel);
  const isChannel = choices.targetKind === "channel";
  return { mutedUntil, level: isChannel || pickedLevel !== choices.fallbackLevel ? pickedLevel : undefined };
}

/** Muted itself, or in a muted server. */
export function isPlaceMuted(settings: NotificationSettings, place: NotificationPlace, now: number): boolean {
  return (
    isMuteActive(settings.places[place.channelId]?.mutedUntil, now) ||
    (place.guildId !== null && isMuteActive(settings.places[place.guildId]?.mutedUntil, now))
  );
}

export interface IncomingMessageFacts {
  readonly message: Pick<Message, "author" | "mention_ids">;
  readonly currentUserId: Snowflake | null;
  readonly place: NotificationPlace;
  readonly isAuthorBlocked: boolean;
  readonly chosenStatus: ChosenStatus | undefined;
  readonly settings: NotificationSettings;
  readonly now: number;
}

/**
 * Whether a new message should reach the reader at all: someone else's, not blocked, not in
 * Do not disturb, and allowed by the place's level; a muted place lets only mentions through.
 */
export function shouldAlertAbout(facts: IncomingMessageFacts): boolean {
  const { message, currentUserId, place, settings } = facts;
  if (currentUserId === null || message.author.id === currentUserId || facts.isAuthorBlocked) {
    return false;
  }
  if (facts.chosenStatus === "do_not_disturb") {
    return false;
  }
  const level = notifyLevelOf(settings, place);
  const mentionsReader = (message.mention_ids ?? []).includes(currentUserId);
  switch (level) {
    case "nothing":
      return false;
    case "mentions":
      return mentionsReader;
    case "all":
      // "Direct mentions are never muted" (DESIGN.md §5.2).
      return mentionsReader || !isPlaceMuted(settings, place, facts.now);
  }
}

/** Where the reader's eyes are, as far as the page can tell. */
export interface ReaderAttention {
  readonly isPageVisible: boolean;
  readonly hasWindowFocus: boolean;
  readonly openChannelId: Snowflake | null;
}

/**
 * How an alert reaches the reader: not at all when they are looking at that channel, in the app
 * (Melli and the sound) while they use Mellow elsewhere, in the browser when they are away.
 */
export type AlertDelivery = "none" | "in-app" | "browser";

export function alertDeliveryFor(channelId: Snowflake, reader: ReaderAttention): AlertDelivery {
  if (!reader.isPageVisible || !reader.hasWindowFocus) {
    return "browser";
  }
  return reader.openChannelId === channelId ? "none" : "in-app";
}

// --- Words -----------------------------------------------------------------------------------

/** A preview never runs longer than this many characters (graphemes), ellipsis included. */
export const MAXIMUM_PREVIEW_CHARACTERS = 120;
const ELLIPSIS = "…";
const NOBODY_TO_MENTION = mentionablePeople([]);
const graphemes = new Intl.Segmenter(undefined, { granularity: "grapheme" });

/** A message as one line of plain text: no Markdown marks, spoilers hidden, emoji as `:name:`. */
export function messagePreviewText(message: Pick<Message, "content" | "attachments">): string {
  const text = blocksAsText(parseMessageMarkdown(message.content, NOBODY_TO_MENTION))
    .replace(/\s+/gu, " ")
    .trim();
  if (text === "") {
    const attachmentCount = message.attachments?.length ?? 0;
    return attachmentCount > 0 ? strings.notifications.sentFiles(attachmentCount) : "";
  }
  const characters = Array.from(graphemes.segment(text), ({ segment }) => segment);
  return characters.length <= MAXIMUM_PREVIEW_CHARACTERS
    ? text
    : `${characters.slice(0, MAXIMUM_PREVIEW_CHARACTERS - 1).join("").trimEnd()}${ELLIPSIS}`;
}

function blocksAsText(blocks: readonly BlockNode[]): string {
  return blocks
    .map((block) => {
      switch (block.kind) {
        case "paragraph":
        case "heading":
          return inlinesAsText(block.children);
        case "quote":
          return blocksAsText(block.children);
        case "list":
          return block.items.map(inlinesAsText).join(" ");
        case "codeBlock":
          return block.text;
      }
    })
    .join(" ");
}

function inlinesAsText(nodes: readonly InlineNode[]): string {
  return nodes
    .map((node) => {
      switch (node.kind) {
        case "text":
        case "mention":
          return withCustomEmojiAsNames(node.text);
        case "code":
          return node.text;
        case "customEmoji":
          return `:${node.emoji.name}:`;
        case "spoiler":
          // Hidden in the chat until clicked: never on the desktop.
          return strings.notifications.spoiler;
        case "strong":
        case "emphasis":
        case "underline":
        case "strikethrough":
        case "link":
          return inlinesAsText(node.children);
      }
    })
    .join("");
}

export interface NotificationContentFacts {
  readonly authorName: string;
  /** The person for a one-to-one conversation, the group's name, or "#general · Rust Lyon". */
  readonly placeTitle: string;
  readonly isOneToOneConversation: boolean;
  readonly preview: string;
  /** Alerts in this channel since the reader last looked, this one included. */
  readonly messageCount: number;
  readonly shouldShowPreview: boolean;
}

export function notificationContent(facts: NotificationContentFacts): { title: string; body: string } {
  const title =
    facts.messageCount > 1 ? strings.notifications.titleWithCount(facts.placeTitle, facts.messageCount) : facts.placeTitle;
  if (!facts.shouldShowPreview || facts.preview === "") {
    return { title, body: strings.notifications.newMessageFrom(facts.authorName) };
  }
  return {
    title,
    body: facts.isOneToOneConversation ? facts.preview : strings.messages.authorSaid(facts.authorName, facts.preview),
  };
}

/** Beyond this, the tab's title says "99+". */
const MAXIMUM_SHOWN_COUNT = 99;

/** The tab's title: "(3) Mellow" while something waits for the reader. */
export function tabTitle(attentionCount: number, appName: string): string {
  if (attentionCount <= 0) {
    return appName;
  }
  return `(${attentionCount > MAXIMUM_SHOWN_COUNT ? `${MAXIMUM_SHOWN_COUNT}+` : attentionCount}) ${appName}`;
}
