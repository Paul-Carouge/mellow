// Pure rules of the channel settings (DESIGN.md §5.8, docs/design/web-channel-settings.md): which
// sections a member sees, the three-way Deny / Inherit / Allow state of each channel permission,
// overwrite drafts, and which targets the member may add, change or remove. The instance decides
// every action (ADR-0007); these rules only hide or lock what it would refuse (ADR-0014).

import {
  CHANNEL_SCOPED_PERMISSIONS,
  type Channel,
  type Guild,
  MAXIMUM_VOICE_USER_LIMIT,
  MINIMUM_VOICE_USER_LIMIT,
  type Member,
  Permission,
  type PermissionBits,
  type PermissionOverwrite,
  type Role,
  type Snowflake,
} from "../lib/protocol";
import { PERMISSION_GROUPS, type RoleManager, matchesMemberSearch } from "./server-settings";

export type ChannelSettingsSection = "overview" | "permissions";

/** Holds every bit of `permission`; administrators hold them all. */
function holdsInChannel(channelPermissions: PermissionBits | null, permission: bigint): boolean {
  if (channelPermissions === null) {
    return false;
  }
  const held = BigInt(channelPermissions);
  return (held & Permission.ADMINISTRATOR) !== 0n || (held & permission) === permission;
}

/**
 * Overview with `MANAGE_CHANNELS`, Permissions with `MANAGE_ROLES`, both as held in this channel
 * (its overwrites included), as the instance checks them. Categories have no settings page yet.
 */
export function visibleChannelSections(
  channel: Pick<Channel, "kind">,
  channelPermissions: PermissionBits | null,
): ChannelSettingsSection[] {
  if (channel.kind === "category") {
    return [];
  }
  return [
    ...(holdsInChannel(channelPermissions, Permission.MANAGE_CHANNELS) ? (["overview"] as const) : []),
    ...(holdsInChannel(channelPermissions, Permission.MANAGE_ROLES) ? (["permissions"] as const) : []),
  ];
}

/** Rename, delete, or move to another category: `MANAGE_CHANNELS` held in the channel itself. */
export function canManageChannel(channelPermissions: PermissionBits | null): boolean {
  return holdsInChannel(channelPermissions, Permission.MANAGE_CHANNELS);
}

/** The name as the instance stores it: as typed, trimmed (`normalize_channel_name`). */
export function channelNameAsSaved(name: string): string {
  return name.trim();
}

/** Overview fields typed by the member; `null` for a field left as stored. */
export interface ChannelOverviewDrafts {
  name: string | null;
  topic: string | null;
  /** A voice channel's limit as chosen; absent or null when left as stored. */
  userLimit?: VoiceUserLimitChoice | null;
}

/** What the user limit select offers: "unlimited", or 1 to 25 people. */
export type VoiceUserLimitChoice = number | "unlimited";

export const VOICE_USER_LIMIT_CHOICES: readonly VoiceUserLimitChoice[] = [
  "unlimited",
  ...Array.from(
    { length: MAXIMUM_VOICE_USER_LIMIT - MINIMUM_VOICE_USER_LIMIT + 1 },
    (_, index) => MINIMUM_VOICE_USER_LIMIT + index,
  ),
];

/** The choice showing a channel's stored limit. */
export function userLimitChoiceOf(channel: Pick<Channel, "user_limit">): VoiceUserLimitChoice {
  return channel.user_limit ?? "unlimited";
}

/**
 * The fields to send: only those typed and different from the stored channel, as the instance
 * would store them. A field someone else changed meanwhile is never sent back with its old value.
 */
export function channelOverviewChanges(
  stored: Pick<Channel, "name" | "topic" | "user_limit">,
  drafts: ChannelOverviewDrafts,
): { name?: string; topic?: string; user_limit?: number | null } {
  const name = drafts.name === null ? null : channelNameAsSaved(drafts.name);
  const topic = drafts.topic === null ? null : drafts.topic.trim();
  const userLimit = drafts.userLimit ?? null;
  return {
    ...(name !== null && name !== stored.name ? { name } : {}),
    ...(topic !== null && topic !== stored.topic ? { topic } : {}),
    ...(userLimit !== null && userLimit !== userLimitChoiceOf(stored)
      ? { user_limit: userLimit === "unlimited" ? null : userLimit }
      : {}),
  };
}

// --- Overwrites -----------------------------------------------------------------------------------

export type OverwriteState = "deny" | "inherit" | "allow";
/** Discord's order, so Allow sits where its users look for it. */
export const OVERWRITE_STATES: readonly OverwriteState[] = ["deny", "inherit", "allow"];

export interface OverwriteBits {
  allow: PermissionBits;
  deny: PermissionBits;
}

const NO_OVERWRITE: OverwriteBits = { allow: "0", deny: "0" };

export function overwriteStateOf(bits: OverwriteBits, permission: bigint): OverwriteState {
  if ((BigInt(bits.allow) & permission) !== 0n) {
    return "allow";
  }
  return (BigInt(bits.deny) & permission) !== 0n ? "deny" : "inherit";
}

/** A bit is never allowed and denied at once (the instance refuses it). */
export function withOverwriteState(bits: OverwriteBits, permission: bigint, state: OverwriteState): OverwriteBits {
  const allow = BigInt(bits.allow) & ~permission;
  const deny = BigInt(bits.deny) & ~permission;
  return {
    allow: String(state === "allow" ? allow | permission : allow),
    deny: String(state === "deny" ? deny | permission : deny),
  };
}

/** The channel-scoped permissions of the role editor's groups, in the same order. */
export const CHANNEL_PERMISSION_GROUPS = PERMISSION_GROUPS.map((group) => ({
  ...group,
  permissions: group.permissions.filter((permission) => (permission.bit & CHANNEL_SCOPED_PERMISSIONS) !== 0n),
})).filter((group) => group.permissions.length > 0);

/** As on Discord, a channel's editor leaves out the other kind's permissions: no "Speak" in a text channel. */
export function channelPermissionGroupsFor(kind: Channel["kind"]): typeof CHANNEL_PERMISSION_GROUPS {
  const otherKindGroup = kind === "voice" ? "textChannels" : "voiceChannels";
  return CHANNEL_PERMISSION_GROUPS.filter((group) => group.group !== otherKindGroup);
}

export interface OverwriteTarget {
  kind: PermissionOverwrite["kind"];
  id: Snowflake;
}

/**
 * The list on the leading side: roles with an overwrite, highest first, then `@everyone`, which is
 * always there (as on Discord), then members with an overwrite in the order they were added.
 */
export function overwriteTargets(guild: Guild, channel: Channel): OverwriteTarget[] {
  const roleIds = new Set(
    channel.permission_overwrites.filter((overwrite) => overwrite.kind === "role").map((overwrite) => overwrite.id),
  );
  const roles = guild.roles
    .filter((role) => role.id !== guild.id && roleIds.has(role.id))
    .sort((left, right) => right.position - left.position)
    .map((role): OverwriteTarget => ({ kind: "role", id: role.id }));
  const members = channel.permission_overwrites
    .filter((overwrite) => overwrite.kind === "member")
    .map((overwrite): OverwriteTarget => ({ kind: "member", id: overwrite.id }));
  return [...roles, { kind: "role", id: guild.id }, ...members];
}

/** No overwrite reads as all Inherit. */
export function storedOverwriteOf(channel: Channel, targetId: Snowflake): OverwriteBits {
  const overwrite = channel.permission_overwrites.find((candidate) => candidate.id === targetId);
  return overwrite ? { allow: overwrite.allow, deny: overwrite.deny } : NO_OVERWRITE;
}

/** What the user changed for one target, on top of its overwrite as it was when they started. */
export interface OverwriteEdit {
  target: OverwriteTarget;
  base: OverwriteBits;
  draft: OverwriteBits;
}

/**
 * The same changes on top of `stored`, or null once they no longer differ from it. As role drafts
 * do (`rebaseRoleEdit`), only the bits the user changed take the wanted value, so a change made
 * meanwhile by someone else is kept, and our own save echoed by the gateway ends the edit.
 */
export function rebaseOverwriteEdit(edit: OverwriteEdit, stored: OverwriteBits): OverwriteEdit | null {
  const changed =
    (BigInt(edit.draft.allow) ^ BigInt(edit.base.allow)) | (BigInt(edit.draft.deny) ^ BigInt(edit.base.deny));
  const take = (wanted: PermissionBits, current: PermissionBits) =>
    String((BigInt(current) & ~changed) | (BigInt(wanted) & changed));
  const draft = { allow: take(edit.draft.allow, stored.allow), deny: take(edit.draft.deny, stored.deny) };
  const isSameAsStored = BigInt(draft.allow) === BigInt(stored.allow) && BigInt(draft.deny) === BigInt(stored.deny);
  return isSameAsStored ? null : { target: edit.target, base: stored, draft };
}

/** `@everyone`, or a role below the manager's highest (the owner targets any): `can_target_role_in_overwrites`. */
export function canTargetRole(manager: RoleManager, role: Role, guildId: Snowflake): boolean {
  return manager.isOwner || role.id === guildId || role.position < manager.highestRolePosition;
}

/** Nobody allows, denies or resets a permission they do not hold in this channel. */
export function canChangeChannelPermission(channelPermissions: PermissionBits | null, permission: bigint): boolean {
  return holdsInChannel(channelPermissions, permission);
}

/** Whether the target's overwrite can be changed at all: a role must be one the manager may target. */
export function canEditTarget(guild: Guild, manager: RoleManager, target: OverwriteTarget): boolean {
  if (target.kind === "member") {
    return true;
  }
  const role = guild.roles.find((candidate) => candidate.id === target.id);
  return role !== undefined && canTargetRole(manager, role, guild.id);
}

/** `@everyone` stays; removing needs every permission the overwrite allows or denies. */
export function canRemoveOverwrite(
  guild: Guild,
  manager: RoleManager,
  channelPermissions: PermissionBits | null,
  channel: Channel,
  target: OverwriteTarget,
): boolean {
  const stored = channel.permission_overwrites.find((overwrite) => overwrite.id === target.id);
  return (
    stored !== undefined &&
    target.id !== guild.id &&
    canEditTarget(guild, manager, target) &&
    canChangeChannelPermission(channelPermissions, BigInt(stored.allow) | BigInt(stored.deny))
  );
}

/** Roles that can be added: not in the list yet, below the manager's highest, highest first. */
export function addableRoles(guild: Guild, channel: Channel, manager: RoleManager, query: string): Role[] {
  const needle = query.trim().toLocaleLowerCase();
  return guild.roles
    .filter(
      (role) =>
        role.id !== guild.id &&
        !channel.permission_overwrites.some((overwrite) => overwrite.id === role.id) &&
        canTargetRole(manager, role, guild.id) &&
        role.name.toLocaleLowerCase().includes(needle),
    )
    .sort((left, right) => right.position - left.position);
}

/** Members that can be added: anyone without an overwrite here, whatever their rank (as on Discord). */
export function addableMembers(channel: Channel, members: readonly Member[], query: string): Member[] {
  return members.filter(
    (member) =>
      !channel.permission_overwrites.some((overwrite) => overwrite.id === member.user.id) &&
      matchesMemberSearch(member, query),
  );
}
