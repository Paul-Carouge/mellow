// Which items each context menu shows (DESIGN.md §6.33, docs/design/web-context-menus.md): only
// actions the web client can already do, each only when the reader may do it. Items come in
// sections separated by a line; danger items are always the last section.

import { strings } from "../i18n";
import { type Guild, Permission, type RelationshipKind, type Snowflake, hasPermission } from "../lib/protocol";
import type { ContextMenuEntry, ContextMenuOutcome } from "./ContextMenu.svelte";
import type { IconName } from "./Icon.svelte";
import { canOpenServerSettings } from "./server-settings";

export type MenuAction =
  | MessageMenuAction
  | PersonMenuAction
  | ChannelMenuAction
  | CategoryMenuAction
  | ServerMenuAction
  | GroupMenuAction;

export type MenuLine<Action extends MenuAction> = Action | "separator";

/** The non-empty sections, with a separator between each. */
function joinSections<Action extends MenuAction>(sections: readonly (readonly (Action | false)[])[]): MenuLine<Action>[] {
  const lines: MenuLine<Action>[] = [];
  for (const section of sections) {
    const actions = section.filter((action): action is Action => action !== false);
    if (actions.length === 0) {
      continue;
    }
    if (lines.length > 0) {
      lines.push("separator");
    }
    lines.push(...actions);
  }
  return lines;
}

// --- Notifications (shared by servers, channels and conversations) ------------------------------

/** "Mute…" or "Unmute", then "Notification settings…" (docs/design/web-notifications.md). */
export type NotificationMenuAction = "mute" | "unmute" | "notification-settings";

function notificationSection(isMuted: boolean): NotificationMenuAction[] {
  return [isMuted ? "unmute" : "mute", "notification-settings"];
}

// --- A message ----------------------------------------------------------------------------------

export type MessageMenuAction =
  | "add-reaction"
  | "reply"
  | "edit"
  | "copy-text"
  | "pin"
  | "unpin"
  | "view-reactions"
  | "remove-all-reactions"
  | "delete";

export interface MessageMenuFacts {
  readonly isOwn: boolean;
  readonly hasText: boolean;
  readonly hasReactions: boolean;
  readonly isPinned: boolean;
  readonly canSendMessages: boolean;
  /** Delete others' messages: `MANAGE_MESSAGES`. */
  readonly canManageMessages: boolean;
  readonly canAddReactions: boolean;
  readonly canJoinReactions: boolean;
  readonly canRemoveReactions: boolean;
  readonly canManagePins: boolean;
}

/** The More button and a right-click open the same menu, as on Discord (DESIGN.md §5.4). */
export function messageMenuActions(facts: MessageMenuFacts): MenuLine<MessageMenuAction>[] {
  return joinSections<MessageMenuAction>([
    [facts.canAddReactions && "add-reaction"],
    [
      facts.canSendMessages && "reply",
      facts.isOwn && facts.canSendMessages && "edit",
      facts.hasText && "copy-text",
      facts.canManagePins && (facts.isPinned ? "unpin" : "pin"),
      facts.hasReactions && facts.canJoinReactions && "view-reactions",
    ],
    [
      facts.hasReactions && facts.canRemoveReactions && "remove-all-reactions",
      (facts.isOwn || facts.canManageMessages) && "delete",
    ],
  ]);
}

// --- A person (a member, a message's author, a friend, a one-to-one conversation) ---------------

export type PersonMenuAction =
  | "profile"
  | "message"
  | "add-friend"
  | "accept-request"
  | "ignore-request"
  | "cancel-request"
  | "unblock"
  | "copy-user-id"
  | "copy-channel-id"
  | "remove-friend"
  | "block"
  | NotificationMenuAction;

export interface PersonMenuFacts {
  readonly isSelf: boolean;
  readonly relationship: RelationshipKind | null;
  /** The member list shows profiles; elsewhere there is none to show. */
  readonly canShowProfile: boolean;
  /** Not from the conversation itself, which is already open. */
  readonly canOpenConversation: boolean;
  /** A one-to-one conversation's row also copies the conversation's ID and mutes it. */
  readonly conversationChannel: boolean;
  /** That conversation is muted (read only with `conversationChannel`). */
  readonly isConversationMuted: boolean;
}

export function personMenuActions(facts: PersonMenuFacts): MenuLine<PersonMenuAction>[] {
  const { isSelf, relationship } = facts;
  const isBlocked = relationship === "blocked";
  return joinSections<PersonMenuAction>([
    [facts.canShowProfile && "profile", !isSelf && !isBlocked && facts.canOpenConversation && "message"],
    isSelf
      ? []
      : [
          relationship === null && "add-friend",
          relationship === "incoming_request" && "accept-request",
          relationship === "incoming_request" && "ignore-request",
          relationship === "outgoing_request" && "cancel-request",
          isBlocked && "unblock",
        ],
    facts.conversationChannel ? notificationSection(facts.isConversationMuted) : [],
    ["copy-user-id", facts.conversationChannel && "copy-channel-id"],
    [relationship === "friend" && "remove-friend", !isSelf && !isBlocked && "block"],
  ]);
}

// --- A channel row ------------------------------------------------------------------------------

export type ChannelMenuAction =
  | "join-voice"
  | "leave-voice"
  | "edit-channel"
  | "move-up"
  | "move-down"
  | "move-to-category"
  | "copy-channel-id"
  | "delete-channel"
  | NotificationMenuAction;

export interface ChannelMenuFacts {
  /** The reader sees at least one page of its settings. */
  readonly canEditChannel: boolean;
  /** Muted itself (a muted server's channels are quiet too, but unmute from the server). */
  readonly isMuted: boolean;
  /** `MANAGE_CHANNELS` on the server, the row not first or last of its section, no move in flight. */
  readonly canMoveUp: boolean;
  readonly canMoveDown: boolean;
  /** `MANAGE_CHANNELS` in the channel too, and somewhere else to go. */
  readonly canMoveToCategory: boolean;
  /** `MANAGE_CHANNELS` in the channel. */
  readonly canDelete: boolean;
  /** A voice channel: Join, or Leave when you are in it. Voice channels hold no messages to mute. */
  readonly voice?: "join" | "leave";
}

/** Move up and Move down are the single-pointer and keyboard path for dragging (WCAG 2.2 2.5.7). */
export function channelMenuActions(facts: ChannelMenuFacts): MenuLine<ChannelMenuAction>[] {
  return joinSections<ChannelMenuAction>([
    [facts.voice === "join" && "join-voice", facts.voice === "leave" && "leave-voice"],
    [facts.canEditChannel && "edit-channel"],
    facts.voice === undefined ? notificationSection(facts.isMuted) : [],
    [facts.canMoveUp && "move-up", facts.canMoveDown && "move-down", facts.canMoveToCategory && "move-to-category"],
    ["copy-channel-id"],
    [facts.canDelete && "delete-channel"],
  ]);
}

// --- A category header --------------------------------------------------------------------------

export type CategoryMenuAction = "rename-category" | "move-up" | "move-down" | "copy-category-id" | "delete-category";

export interface CategoryMenuFacts {
  /** Rename and delete: `MANAGE_CHANNELS` in the category. */
  readonly canManage: boolean;
  readonly canMoveUp: boolean;
  readonly canMoveDown: boolean;
}

/** Categories have no settings page: their menu renames them (DESIGN.md §5.3). */
export function categoryMenuActions(facts: CategoryMenuFacts): MenuLine<CategoryMenuAction>[] {
  return joinSections<CategoryMenuAction>([
    [facts.canManage && "rename-category"],
    [facts.canMoveUp && "move-up", facts.canMoveDown && "move-down"],
    ["copy-category-id"],
    [facts.canManage && "delete-category"],
  ]);
}

// --- A server (its icon in the strip, or the header's menu) -------------------------------------

export type ServerMenuAction =
  | "invite-people"
  | "server-settings"
  | "create-channel"
  | "create-category"
  | "copy-server-id"
  | "leave-server"
  | NotificationMenuAction;

export interface ServerMenuFacts {
  readonly canInvitePeople: boolean;
  readonly canOpenSettings: boolean;
  readonly canCreateChannels: boolean;
  /** Everyone but the owner. */
  readonly canLeave: boolean;
  readonly isMuted: boolean;
}

/** What the reader may do with `guild`, from its permission snapshot. */
export function serverMenuFactsOf(guild: Guild, userId: Snowflake | undefined, isMuted: boolean): ServerMenuFacts {
  return {
    canInvitePeople: hasPermission(guild.my_permissions, Permission.CREATE_INVITE),
    canOpenSettings: canOpenServerSettings(guild, userId),
    canCreateChannels: hasPermission(guild.my_permissions, Permission.MANAGE_CHANNELS),
    canLeave: guild.owner_id !== userId,
    isMuted,
  };
}

/**
 * The header's menu works on the open server, as it always did; the strip's icon menu works on
 * any server, without opening it, and copies its ID as Discord's does.
 */
export function serverMenuActions(facts: ServerMenuFacts, place: "header" | "strip"): MenuLine<ServerMenuAction>[] {
  return place === "header"
    ? joinSections<ServerMenuAction>([
        [
          facts.canInvitePeople && "invite-people",
          facts.canOpenSettings && "server-settings",
          facts.canCreateChannels && "create-channel",
          facts.canCreateChannels && "create-category",
        ],
        notificationSection(facts.isMuted),
        [facts.canLeave && "leave-server"],
      ])
    : joinSections<ServerMenuAction>([
        [facts.canInvitePeople && "invite-people", facts.canOpenSettings && "server-settings"],
        notificationSection(facts.isMuted),
        ["copy-server-id"],
        [facts.canLeave && "leave-server"],
      ]);
}

// --- A group conversation's row -----------------------------------------------------------------

export type GroupMenuAction = "rename-group" | "copy-channel-id" | "leave-group" | NotificationMenuAction;

/** Anyone in a group may rename it or leave it (ADR-0026). */
export function groupMenuActions(isMuted: boolean): MenuLine<GroupMenuAction>[] {
  return joinSections<GroupMenuAction>([
    ["rename-group"],
    notificationSection(isMuted),
    ["copy-channel-id"],
    ["leave-group"],
  ]);
}

// --- How each item reads -------------------------------------------------------------------------

export interface MenuItemLook {
  readonly label: string;
  readonly icon: IconName;
  readonly tone: "normal" | "accent" | "danger";
}

const LOOKS: Record<MenuAction, MenuItemLook> = {
  "add-reaction": { label: strings.reactions.addReaction, icon: "smiley", tone: "normal" },
  reply: { label: strings.messages.reply, icon: "arrow-bend-up-left", tone: "normal" },
  edit: { label: strings.messages.editMessage, icon: "pencil-simple", tone: "normal" },
  "copy-text": { label: strings.messages.copyText, icon: "copy", tone: "normal" },
  pin: { label: strings.pins.pin, icon: "push-pin", tone: "normal" },
  unpin: { label: strings.pins.unpin, icon: "push-pin-slash", tone: "normal" },
  "view-reactions": { label: strings.reactions.reactions, icon: "users", tone: "normal" },
  "remove-all-reactions": { label: strings.reactions.removeAll, icon: "x-circle", tone: "danger" },
  delete: { label: strings.contextMenu.deleteMessage, icon: "trash", tone: "danger" },
  profile: { label: strings.members.profile, icon: "at", tone: "normal" },
  message: { label: strings.contextMenu.message, icon: "chat-circle", tone: "normal" },
  "add-friend": { label: strings.members.addFriend, icon: "user-plus", tone: "normal" },
  "accept-request": { label: strings.members.acceptRequest, icon: "check", tone: "normal" },
  "ignore-request": { label: strings.contextMenu.ignoreRequest, icon: "x", tone: "normal" },
  "cancel-request": { label: strings.contextMenu.cancelRequest, icon: "x", tone: "normal" },
  unblock: { label: strings.friends.unblock, icon: "prohibit", tone: "normal" },
  "copy-user-id": { label: strings.userCard.copyUserId, icon: "copy", tone: "normal" },
  "copy-channel-id": { label: strings.contextMenu.copyChannelId, icon: "copy", tone: "normal" },
  "remove-friend": { label: strings.friends.removeFriend, icon: "user-minus", tone: "danger" },
  block: { label: strings.friends.block, icon: "prohibit", tone: "danger" },
  "join-voice": { label: strings.contextMenu.joinVoice, icon: "speaker-high", tone: "normal" },
  "leave-voice": { label: strings.contextMenu.leaveVoice, icon: "phone-disconnect", tone: "danger" },
  "edit-channel": { label: strings.contextMenu.editChannel, icon: "gear-six", tone: "normal" },
  "move-up": { label: strings.contextMenu.moveUp, icon: "caret-up", tone: "normal" },
  "move-down": { label: strings.contextMenu.moveDown, icon: "caret-down", tone: "normal" },
  "move-to-category": { label: strings.contextMenu.moveToCategory, icon: "list", tone: "normal" },
  "copy-category-id": { label: strings.contextMenu.copyCategoryId, icon: "copy", tone: "normal" },
  "delete-channel": { label: strings.channelRemoval.deleteChannel, icon: "trash", tone: "danger" },
  "rename-category": { label: strings.channelRemoval.renameCategory, icon: "pencil-simple", tone: "normal" },
  "delete-category": { label: strings.channelRemoval.deleteCategory, icon: "trash", tone: "danger" },
  "invite-people": { label: strings.sidebar.invitePeople, icon: "user-plus", tone: "accent" },
  "server-settings": { label: strings.sidebar.serverSettings, icon: "gear-six", tone: "normal" },
  "create-channel": { label: strings.sidebar.createChannel, icon: "plus-circle", tone: "normal" },
  "create-category": { label: strings.sidebar.createCategory, icon: "list", tone: "normal" },
  "copy-server-id": { label: strings.contextMenu.copyServerId, icon: "copy", tone: "normal" },
  "leave-server": { label: strings.sidebar.leaveServer, icon: "sign-out", tone: "danger" },
  "rename-group": { label: strings.groups.rename, icon: "pencil-simple", tone: "normal" },
  "leave-group": { label: strings.groups.leave, icon: "sign-out", tone: "danger" },
  mute: { label: strings.notifications.muteMenu, icon: "bell-slash", tone: "normal" },
  unmute: { label: strings.notifications.unmute, icon: "bell", tone: "normal" },
  "notification-settings": { label: strings.notifications.settingsMenu, icon: "bell", tone: "normal" },
};

export function menuItemLook(action: MenuAction): MenuItemLook {
  return LOOKS[action];
}

// --- From actions to the menu's entries ---------------------------------------------------------

export type MenuItemHandler =
  | { readonly onselect: () => void }
  | { readonly onselectAndWait: () => Promise<ContextMenuOutcome> };

/** The entries of `ContextMenu`, each action read as it looks and acting through `handlerOf`. */
export function menuEntries<Action extends MenuAction>(
  lines: readonly MenuLine<Action>[],
  handlerOf: (action: Action) => MenuItemHandler,
): ContextMenuEntry[] {
  return lines.map((line) => (line === "separator" ? line : { ...menuItemLook(line), ...handlerOf(line) }));
}

/** Copy user, channel or server ID: the result is read out; a refusal stays in the menu. */
export async function copyIdentifier(id: string, announcement: string): Promise<ContextMenuOutcome> {
  try {
    await navigator.clipboard.writeText(id);
    return { announcement };
  } catch (error) {
    console.warn("copying an ID failed", error);
    return { error: strings.contextMenu.copyIdFailed };
  }
}
