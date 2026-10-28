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
  | "block";

export interface PersonMenuFacts {
  readonly isSelf: boolean;
  readonly relationship: RelationshipKind | null;
  /** The member list shows profiles; elsewhere there is none to show. */
  readonly canShowProfile: boolean;
  /** Not from the conversation itself, which is already open. */
  readonly canOpenConversation: boolean;
  /** A one-to-one conversation's row also copies the conversation's ID. */
  readonly conversationChannel: boolean;
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
    ["copy-user-id", facts.conversationChannel && "copy-channel-id"],
    [relationship === "friend" && "remove-friend", !isSelf && !isBlocked && "block"],
  ]);
}

// --- A channel row ------------------------------------------------------------------------------

export type ChannelMenuAction = "edit-channel" | "copy-channel-id";

/** `canEditChannel`: the reader sees at least one page of its settings. */
export function channelMenuActions(facts: { readonly canEditChannel: boolean }): MenuLine<ChannelMenuAction>[] {
  return joinSections<ChannelMenuAction>([[facts.canEditChannel && "edit-channel"], ["copy-channel-id"]]);
}

// --- A server (its icon in the strip, or the header's menu) -------------------------------------

export type ServerMenuAction = "invite-people" | "server-settings" | "create-channel" | "copy-server-id" | "leave-server";

export interface ServerMenuFacts {
  readonly canInvitePeople: boolean;
  readonly canOpenSettings: boolean;
  readonly canCreateChannels: boolean;
  /** Everyone but the owner. */
  readonly canLeave: boolean;
}

/** What the reader may do with `guild`, from its permission snapshot. */
export function serverMenuFactsOf(guild: Guild, userId: Snowflake | undefined): ServerMenuFacts {
  return {
    canInvitePeople: hasPermission(guild.my_permissions, Permission.CREATE_INVITE),
    canOpenSettings: canOpenServerSettings(guild, userId),
    canCreateChannels: hasPermission(guild.my_permissions, Permission.MANAGE_CHANNELS),
    canLeave: guild.owner_id !== userId,
  };
}

/**
 * The header's menu works on the open server, as it always did; the strip's icon menu works on
 * any server, without opening it, and copies its ID as Discord's does.
 */
export function serverMenuActions(facts: ServerMenuFacts, place: "header" | "strip"): MenuLine<ServerMenuAction>[] {
  return place === "header"
    ? joinSections<ServerMenuAction>([
        [facts.canInvitePeople && "invite-people", facts.canOpenSettings && "server-settings", facts.canCreateChannels && "create-channel"],
        [facts.canLeave && "leave-server"],
      ])
    : joinSections<ServerMenuAction>([
        [facts.canInvitePeople && "invite-people", facts.canOpenSettings && "server-settings"],
        ["copy-server-id"],
        [facts.canLeave && "leave-server"],
      ]);
}

// --- A group conversation's row -----------------------------------------------------------------

export type GroupMenuAction = "rename-group" | "copy-channel-id" | "leave-group";

/** Anyone in a group may rename it or leave it (ADR-0026). */
export function groupMenuActions(): MenuLine<GroupMenuAction>[] {
  return joinSections<GroupMenuAction>([["rename-group"], ["copy-channel-id"], ["leave-group"]]);
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
  "edit-channel": { label: strings.contextMenu.editChannel, icon: "gear-six", tone: "normal" },
  "invite-people": { label: strings.sidebar.invitePeople, icon: "user-plus", tone: "accent" },
  "server-settings": { label: strings.sidebar.serverSettings, icon: "gear-six", tone: "normal" },
  "create-channel": { label: strings.sidebar.createChannel, icon: "plus-circle", tone: "normal" },
  "copy-server-id": { label: strings.contextMenu.copyServerId, icon: "copy", tone: "normal" },
  "leave-server": { label: strings.sidebar.leaveServer, icon: "sign-out", tone: "danger" },
  "rename-group": { label: strings.groups.rename, icon: "pencil-simple", tone: "normal" },
  "leave-group": { label: strings.groups.leave, icon: "sign-out", tone: "danger" },
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
