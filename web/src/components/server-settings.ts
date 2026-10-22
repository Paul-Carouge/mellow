// Pure rules of the server settings (DESIGN.md §5.8): which sections a member sees, the role
// hierarchy, role drafts and the moderation log's sentences. The instance decides every action
// (ADR-0007); these rules only hide or lock what it would refuse (ADR-0014).

import { strings } from "../i18n";
import {
  type AuditChange,
  type AuditLogEntry,
  type Guild,
  type Member,
  Permission,
  type PermissionBits,
  type PublicUser,
  type Role,
  type Snowflake,
} from "../lib/protocol";
import { displayName } from "./format";

export type ServerSettingsSection = "overview" | "roles" | "invites" | "members" | "bans" | "moderation-log";

/** Discord's order: the server's own settings, then moderation. */
const SECTION_PERMISSIONS: readonly [ServerSettingsSection, bigint][] = [
  ["overview", Permission.MANAGE_GUILD],
  ["roles", Permission.MANAGE_ROLES],
  ["invites", Permission.MANAGE_GUILD],
  ["members", Permission.KICK_MEMBERS | Permission.BAN_MEMBERS | Permission.MANAGE_ROLES],
  ["bans", Permission.BAN_MEMBERS],
  ["moderation-log", Permission.VIEW_AUDIT_LOG],
];

/** Holds any of `permissions` (the owner and administrators hold them all). */
export function holds(guild: Guild, permissions: bigint): boolean {
  const held = BigInt(guild.my_permissions);
  return (held & Permission.ADMINISTRATOR) !== 0n || (held & permissions) !== 0n;
}

export function visibleSections(guild: Guild): ServerSettingsSection[] {
  return SECTION_PERMISSIONS.filter(([, permissions]) => holds(guild, permissions)).map(([section]) => section);
}

export function isOwner(guild: Guild, userId: Snowflake | undefined): boolean {
  return userId !== undefined && guild.owner_id === userId;
}

/** The owner always can: "Delete server" is theirs alone. */
export function canOpenServerSettings(guild: Guild, userId: Snowflake | undefined): boolean {
  return isOwner(guild, userId) || visibleSections(guild).length > 0;
}

/**
 * Whether leaving the shown page must wait for its unsaved changes. Only the page that holds them
 * keeps the reader: a page that went away (its permission was lost) never traps them.
 */
export function mustStayOnPage(
  unsavedSection: ServerSettingsSection | null,
  shownSection: ServerSettingsSection | null,
): boolean {
  return unsavedSection !== null && unsavedSection === shownSection;
}

/** The control to focus once a row is gone: the next row's, else the previous one's, else none. */
export function focusAfterRemoval(rowIds: readonly string[], removedId: string): string | null {
  const index = rowIds.indexOf(removedId);
  if (index < 0) {
    return null;
  }
  return rowIds[index + 1] ?? rowIds[index - 1] ?? null;
}

// --- Roles ------------------------------------------------------------------------------------

/** Discord's role colour presets: user-chosen data shown on names, not design tokens. */
export const ROLE_COLOR_PRESETS: readonly number[] = [
  0x1abc9c, 0x2ecc71, 0x3498db, 0x9b59b6, 0xe91e63, 0xf1c40f, 0xe67e22, 0xe74c3c, 0x95a5a6, 0x607d8b, 0x11806a,
  0x1f8b4c, 0x206694, 0x71368a, 0xad1457, 0xc27c0e, 0xa84300, 0x992d22, 0x979c9f, 0x546e7a,
];
/** The wire value of "no colour", as on Discord. */
export const NO_ROLE_COLOR = 0;

/** `#RRGGBB` of a role colour (`0xRRGGBB`), as picked: swatches and dots, never names. */
export function roleColorHex(color: number): string {
  return `#${color.toString(16).padStart(6, "0").toUpperCase()}`;
}

/** What the signed-in member may do with roles; mirrors `RoleManager` in the server's domain. */
export interface RoleManager {
  permissions: bigint;
  isOwner: boolean;
  highestRolePosition: number;
}

export function roleManagerOf(guild: Guild, userId: Snowflake | undefined): RoleManager {
  const positions = guild.roles.filter((role) => guild.my_role_ids.includes(role.id)).map((role) => role.position);
  return {
    permissions: BigInt(guild.my_permissions),
    isOwner: isOwner(guild, userId),
    highestRolePosition: Math.max(0, ...positions),
  };
}

function containsAll(held: bigint, needed: bigint): boolean {
  return (held & needed) === needed;
}

/** `@everyone`'s id is the guild's id. */
export function canManageRole(manager: RoleManager, role: Role, guildId: Snowflake): boolean {
  return (
    containsAll(manager.permissions, Permission.MANAGE_ROLES) &&
    (manager.isOwner ||
      ((role.id === guildId || role.position < manager.highestRolePosition) &&
        containsAll(manager.permissions, BigInt(role.permissions))))
  );
}

/** Nobody grants or removes a permission they do not hold. */
export function canTogglePermission(manager: RoleManager, permission: bigint): boolean {
  return containsAll(manager.permissions, permission);
}

/** Highest first, `@everyone` last, as the role list shows them. */
export function rolesHighestFirst(guild: Guild): Role[] {
  return [...guild.roles].sort((left, right) => right.position - left.position);
}

export type MoveDirection = "up" | "down";

/**
 * The `PUT /roles/order` body (named roles, lowest to highest) after swapping `roleId` with its
 * neighbour, or null when the manager may not: both roles change slots, so both must be theirs.
 */
export function roleIdsAfterMove(
  guild: Guild,
  manager: RoleManager,
  roleId: Snowflake,
  direction: MoveDirection,
): Snowflake[] | null {
  const named = guild.roles
    .filter((role) => role.id !== guild.id)
    .sort((left, right) => left.position - right.position || compareIds(left.id, right.id));
  const index = named.findIndex((role) => role.id === roleId);
  const neighbourIndex = direction === "up" ? index + 1 : index - 1;
  const role = named[index];
  const neighbour = named[neighbourIndex];
  if (!role || !neighbour || !canManageRole(manager, role, guild.id) || !canManageRole(manager, neighbour, guild.id)) {
    return null;
  }
  named[index] = neighbour;
  named[neighbourIndex] = role;
  return named.map((candidate) => candidate.id);
}

function compareIds(left: Snowflake, right: Snowflake): number {
  const difference = BigInt(left) - BigInt(right);
  return difference === 0n ? 0 : difference < 0n ? -1 : 1;
}

/** Members holding the role; every member holds `@everyone`. */
export function roleMemberCount(guild: Guild, role: Role, members: readonly Member[]): number {
  return role.id === guild.id ? members.length : members.filter((member) => member.role_ids.includes(role.id)).length;
}

/** The editable part of a role. */
export interface RoleDraft {
  name: string;
  color: number;
  permissions: PermissionBits;
}

export function draftOf(role: Role): RoleDraft {
  return { name: role.name, color: role.color, permissions: role.permissions };
}

/** What the user changed, on top of the role as it was when they started changing it. */
export interface RoleEdit {
  roleId: Snowflake;
  base: RoleDraft;
  draft: RoleDraft;
}

/** The instance trims names, so trailing spaces alone are not a change. */
function isSameAsStored(draft: RoleDraft, stored: RoleDraft): boolean {
  return draft.name.trim() === stored.name && draft.color === stored.color && draft.permissions === stored.permissions;
}

/**
 * The same changes on top of `stored`, or null once they no longer differ from it: a change
 * arriving meanwhile (another manager revoking a permission) is never undone by saving an older
 * copy of the role. Only toggled permission bits move; unknown bits of a newer server stay.
 */
export function rebaseRoleEdit(edit: RoleEdit, stored: Role): RoleEdit | null {
  const base = draftOf(stored);
  const wanted = BigInt(edit.draft.permissions);
  const toggled = wanted ^ BigInt(edit.base.permissions);
  const draft: RoleDraft = {
    name: edit.draft.name === edit.base.name ? base.name : edit.draft.name,
    color: edit.draft.color === edit.base.color ? base.color : edit.draft.color,
    // The toggled bits take the wanted value, so a role that already has it (our own save
    // echoed by the gateway) ends the edit instead of reverting it.
    permissions: String((BigInt(base.permissions) & ~toggled) | (wanted & toggled)),
  };
  return isSameAsStored(draft, base) ? null : { roleId: edit.roleId, base, draft };
}

/**
 * Whether an edit still has something to save on top of the stored role. Once it has not (saved,
 * undone, or the role is gone) the page drops it, so that a later change of the role (someone
 * undoing it) cannot bring a stale edit back.
 */
export function isRoleEditPending(edit: RoleEdit, stored: Role | null): boolean {
  return stored !== null && edit.roleId === stored.id && rebaseRoleEdit(edit, stored) !== null;
}

export function togglePermission(draft: RoleDraft, permission: bigint): RoleDraft {
  return { ...draft, permissions: String(BigInt(draft.permissions) ^ permission) };
}

/** Only the changed fields; `@everyone` takes permissions only. Null for a blank name. */
export function roleUpdateBody(
  stored: Role,
  draft: RoleDraft,
  isEveryone: boolean,
): { name?: string; color?: number; permissions?: PermissionBits } | null {
  const name = draft.name.trim();
  if (!isEveryone && name === "") {
    return null;
  }
  return {
    ...(!isEveryone && name !== stored.name ? { name } : {}),
    ...(!isEveryone && draft.color !== stored.color ? { color: draft.color } : {}),
    ...(draft.permissions !== stored.permissions ? { permissions: draft.permissions } : {}),
  };
}

export type PermissionKey = keyof typeof strings.serverSettings.permissions;
export type PermissionGroupKey = keyof typeof strings.serverSettings.roles.groups;

export interface PermissionDescription {
  key: PermissionKey;
  bit: bigint;
  isSensitive: boolean;
}

/**
 * Every permission the editor shows, by group, in Discord's order. Nicknames have no feature yet,
 * so their bits are not offered; like any bit not listed, they stay as stored.
 */
export const PERMISSION_GROUPS: readonly { group: PermissionGroupKey; permissions: readonly PermissionDescription[] }[] = [
  {
    group: "general",
    permissions: [
      { key: "viewChannels", bit: Permission.VIEW_CHANNEL, isSensitive: false },
      { key: "manageChannels", bit: Permission.MANAGE_CHANNELS, isSensitive: false },
      { key: "manageRoles", bit: Permission.MANAGE_ROLES, isSensitive: true },
      { key: "manageServer", bit: Permission.MANAGE_GUILD, isSensitive: false },
      { key: "viewModerationLog", bit: Permission.VIEW_AUDIT_LOG, isSensitive: false },
    ],
  },
  {
    group: "membership",
    permissions: [{ key: "createInvite", bit: Permission.CREATE_INVITE, isSensitive: false }],
  },
  {
    group: "textChannels",
    permissions: [
      { key: "sendMessages", bit: Permission.SEND_MESSAGES, isSensitive: false },
      { key: "readMessageHistory", bit: Permission.READ_MESSAGE_HISTORY, isSensitive: false },
      { key: "attachFiles", bit: Permission.ATTACH_FILES, isSensitive: false },
      { key: "mentionEveryone", bit: Permission.MENTION_EVERYONE, isSensitive: false },
      { key: "manageMessages", bit: Permission.MANAGE_MESSAGES, isSensitive: false },
    ],
  },
  {
    group: "moderation",
    permissions: [
      { key: "kickMembers", bit: Permission.KICK_MEMBERS, isSensitive: false },
      { key: "banMembers", bit: Permission.BAN_MEMBERS, isSensitive: true },
    ],
  },
  {
    group: "advanced",
    permissions: [{ key: "administrator", bit: Permission.ADMINISTRATOR, isSensitive: true }],
  },
];

// --- Members ----------------------------------------------------------------------------------

/**
 * Whether the manager may kick, ban or (with `MANAGE_ROLES`) act on this member: strictly below
 * their highest role, never the owner or themselves (the server's `outranks`).
 */
export function outranksMember(guild: Guild, manager: RoleManager, member: Member, userId: Snowflake | undefined): boolean {
  if (member.user.id === userId || member.user.id === guild.owner_id) {
    return false;
  }
  if (manager.isOwner) {
    return true;
  }
  const positions = guild.roles.filter((role) => member.role_ids.includes(role.id)).map((role) => role.position);
  return manager.highestRolePosition > Math.max(0, ...positions);
}

/** Case- and accent-insensitive, on the display name and the username. */
export function matchesMemberSearch(member: Member, query: string): boolean {
  const needle = foldForSearch(query.trim());
  return (
    needle === "" ||
    foldForSearch(member.user.display_name).includes(needle) ||
    foldForSearch(member.user.username).includes(needle)
  );
}

function foldForSearch(text: string): string {
  return text.normalize("NFD").replace(/\p{Diacritic}/gu, "").toLocaleLowerCase();
}

// --- Moderation log -----------------------------------------------------------------------------

export interface AuditContext {
  guild: Guild;
  /** Who the log can name: the members, and the banned users when the reader may list them. */
  people: readonly PublicUser[];
}

function personName(context: AuditContext, userId: Snowflake | null): string {
  if (userId === null) {
    return strings.serverSettings.moderationLog.deletedAccount;
  }
  const person = context.people.find((candidate) => candidate.id === userId);
  return person ? displayName(person) : strings.serverSettings.moderationLog.formerMember;
}

function roleName(context: AuditContext, roleId: Snowflake | null): string {
  const role = context.guild.roles.find((candidate) => candidate.id === roleId);
  return role ? role.name : strings.serverSettings.moderationLog.deletedRole;
}

function channelName(context: AuditContext, channelId: Snowflake | null): string {
  const channel = context.guild.channels.find((candidate) => candidate.id === channelId);
  return channel ? channel.name : strings.serverSettings.moderationLog.deletedChannel;
}

/** A role, `@everyone` or a member: the target of a channel overwrite. */
function roleOrPersonName(context: AuditContext, targetId: Snowflake | null): string {
  return context.guild.roles.some((role) => role.id === targetId)
    ? roleName(context, targetId)
    : personName(context, targetId);
}

function changeOf(entry: AuditLogEntry, key: AuditChange["key"]): AuditChange | undefined {
  return entry.changes.find((change) => change.key === key);
}

/** The entry as one sentence in the reader's language: "Ana kicked Paul". */
export function auditEntrySentence(entry: AuditLogEntry, context: AuditContext): string {
  const log = strings.serverSettings.moderationLog;
  const actor = personName(context, entry.actor_id);
  const target = entry.target_id;
  switch (entry.action) {
    case "channel_create":
      return log.channelCreate(actor, channelName(context, target));
    case "channel_update":
      return log.channelUpdate(actor, channelName(context, target));
    case "channel_delete":
      return log.channelDelete(actor);
    case "overwrite_set":
      return log.overwriteSet(actor, roleOrPersonName(context, target), channelName(context, entry.context_id));
    case "overwrite_delete":
      return log.overwriteDelete(actor, roleOrPersonName(context, target), channelName(context, entry.context_id));
    case "role_create":
      return log.roleCreate(actor, roleName(context, target));
    case "role_update":
      return log.roleUpdate(actor, roleName(context, target));
    case "role_delete":
      return log.roleDelete(actor);
    case "roles_reorder":
      return log.rolesReorder(actor);
    case "member_role_add":
      return log.memberRoleAdd(actor, roleName(context, entry.context_id), personName(context, target));
    case "member_role_remove":
      return log.memberRoleRemove(actor, roleName(context, entry.context_id), personName(context, target));
    case "member_kick":
      return log.memberKick(actor, personName(context, target));
    case "member_ban_add":
      return log.memberBanAdd(actor, personName(context, target));
    case "member_ban_remove":
      return log.memberBanRemove(actor, personName(context, target));
    case "message_delete":
      return log.messageDelete(actor, personName(context, target), channelName(context, entry.context_id));
    case "guild_update": {
      const name = changeOf(entry, "name");
      return name ? log.guildRename(actor, name.old_value, name.new_value) : log.guildUpdate(actor);
    }
    case "invite_create":
      return log.inviteCreate(actor, changeOf(entry, "code")?.new_value ?? "");
    case "invite_delete":
      return log.inviteDelete(actor, changeOf(entry, "code")?.old_value ?? "");
    default:
      // "unknown", or an action of a newer server that this client cannot name yet.
      return log.unknown(actor);
  }
}

function permissionNames(bits: bigint): string[] {
  return PERMISSION_GROUPS.flatMap((group) => group.permissions)
    .filter((permission) => (bits & permission.bit) !== 0n)
    .map((permission) => strings.serverSettings.permissions[permission.key].label);
}

/** What an update changed, one line per field, empty when the entry carries no changes. */
export function auditChangeLines(entry: AuditLogEntry): string[] {
  const log = strings.serverSettings.moderationLog;
  return entry.changes.flatMap((change): string[] => {
    switch (change.key) {
      case "name":
        return entry.action === "guild_update" ? [] : [log.nameChange(change.old_value, change.new_value)];
      case "topic":
        return [change.new_value === "" ? log.topicCleared : log.topicChange(change.new_value)];
      case "color":
        return [log.colorChange];
      case "permissions":
      case "allow":
      case "deny": {
        const before = BigInt(change.old_value);
        const after = BigInt(change.new_value);
        const granted = permissionNames(after & ~before);
        const removed = permissionNames(before & ~after);
        const lines: string[] = [];
        const listSeparator = strings.typing.listSeparator;
        if (granted.length > 0) {
          lines.push(log.permissionsAdded(change.key, granted.join(listSeparator)));
        }
        if (removed.length > 0) {
          lines.push(log.permissionsRemoved(change.key, removed.join(listSeparator)));
        }
        return lines;
      }
      default:
        // The invite code is in the sentence; a field of a newer server is left out.
        return [];
    }
  });
}
