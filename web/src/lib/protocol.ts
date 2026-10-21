// Wire types of `mellow-protocol` (REST `/api/v1`, gateway `/gateway?v=1`).
// shortcut: written by hand from the Rust crate; generate them from it (ADR-0019) once the web
// client covers more than chat, before the protocol grows further.

/** 64-bit ids travel as decimal strings: JavaScript numbers lose precision past 2^53. */
export type Snowflake = string;
/** Permission bitfield as a decimal string (u64). */
export type PermissionBits = string;

export interface CurrentUser {
  id: Snowflake;
  username: string;
  display_name: string;
  email: string;
  is_instance_administrator: boolean;
}

export interface PublicUser {
  id: Snowflake;
  username: string;
  display_name: string;
}

export interface SessionCreated {
  token: string;
  user: CurrentUser;
}

export type ChannelKind = "category" | "text";

export interface PermissionOverwrite {
  id: Snowflake;
  kind: "role" | "member";
  allow: PermissionBits;
  deny: PermissionBits;
}

export interface Channel {
  id: Snowflake;
  guild_id: Snowflake;
  kind: ChannelKind;
  name: string;
  topic: string;
  parent_id: Snowflake | null;
  position: number;
  permission_overwrites: PermissionOverwrite[];
}

export interface Role {
  id: Snowflake;
  name: string;
  color: number;
  position: number;
  permissions: PermissionBits;
}

export interface Guild {
  id: Snowflake;
  name: string;
  owner_id: Snowflake;
  my_permissions: PermissionBits;
  my_role_ids: Snowflake[];
  roles: Role[];
  channels: Channel[];
}

export interface Member {
  user: PublicUser;
  role_ids: Snowflake[];
  joined_at_unix_milliseconds: number;
}

export interface Message {
  id: Snowflake;
  channel_id: Snowflake;
  author: PublicUser;
  content: string;
  reply_to_id: Snowflake | null;
  edited_at_unix_milliseconds: number | null;
  nonce?: string | null;
}

export interface ReadState {
  channel_id: Snowflake;
  last_read_message_id: Snowflake;
}

export interface DirectMessageChannel {
  id: Snowflake;
  recipients: PublicUser[];
  last_message_id: Snowflake | null;
}

export interface Invite {
  code: string;
  guild_id: Snowflake;
  uses: number;
  max_uses: number | null;
  expires_at_unix_milliseconds: number | null;
}

export interface Ready {
  session_id: string;
  user: CurrentUser;
  guilds: Guild[];
  read_states: ReadState[];
  online_user_ids: Snowflake[];
  direct_message_channels?: DirectMessageChannel[];
}

export type ApiErrorCode =
  | "VALIDATION_FAILED"
  | "USERNAME_TAKEN"
  | "EMAIL_TAKEN"
  | "REGISTRATION_CLOSED"
  | "INVALID_CREDENTIALS"
  | "UNAUTHORIZED"
  | "ACCOUNT_DISABLED"
  | "RATE_LIMITED"
  | "SERVER_BUSY"
  | "NOT_FOUND"
  | "MISSING_PERMISSIONS"
  | "INVALID_INVITE"
  | "OWNER_CANNOT_LEAVE"
  | "BANNED"
  | "LIMIT_REACHED"
  | "CONFLICT"
  | "INTERNAL";

export type GatewayEvent =
  | { t: "MESSAGE_CREATE"; data: Message }
  | { t: "MESSAGE_UPDATE"; data: Message }
  | { t: "MESSAGE_DELETE"; data: { guild_id?: Snowflake; channel_id: Snowflake; id: Snowflake } }
  | { t: "CHANNEL_CREATE"; data: Channel }
  | { t: "CHANNEL_UPDATE"; data: Channel }
  | { t: "CHANNEL_DELETE"; data: { guild_id: Snowflake; id: Snowflake } }
  | { t: "GUILD_CREATE"; data: Guild }
  | { t: "GUILD_DELETE"; data: { id: Snowflake } }
  | { t: "GUILD_MEMBER_ADD"; data: { guild_id: Snowflake; member: Member } }
  | { t: "GUILD_MEMBER_REMOVE"; data: { guild_id: Snowflake; user_id: Snowflake } }
  | {
      t: "GUILD_MEMBER_UPDATE";
      data: { guild_id: Snowflake; user_id: Snowflake; role_ids: Snowflake[] };
    }
  | { t: "GUILD_ROLES_UPDATE"; data: { guild_id: Snowflake; roles: Role[] } }
  | {
      t: "TYPING_START";
      data: { guild_id?: Snowflake; channel_id: Snowflake; user_id: Snowflake };
    }
  | { t: "PRESENCE_UPDATE"; data: { user_id: Snowflake; status: "online" | "offline" } }
  | { t: "USER_UPDATE"; data: PublicUser }
  | { t: "DIRECT_MESSAGE_CHANNEL_CREATE"; data: DirectMessageChannel };

export type ServerFrame =
  | { op: "hello"; d: { heartbeat_interval_milliseconds: number } }
  | { op: "ready"; d: Ready }
  | { op: "resumed" }
  | { op: "invalid_session" }
  | { op: "heartbeat_ack" }
  | { op: "dispatch"; s: number; d: GatewayEvent };

export type ClientFrame =
  | { op: "identify"; d: { token: string } }
  | { op: "resume"; d: { token: string; session_id: string; last_sequence: number } }
  | { op: "heartbeat" };

/** Bit positions are part of the wire protocol (`Permissions` in mellow-protocol). */
export const Permission = {
  ADMINISTRATOR: 1n << 0n,
  VIEW_CHANNEL: 1n << 1n,
  SEND_MESSAGES: 1n << 2n,
  READ_MESSAGE_HISTORY: 1n << 3n,
  MANAGE_MESSAGES: 1n << 6n,
  MANAGE_CHANNELS: 1n << 7n,
  MANAGE_ROLES: 1n << 8n,
  CREATE_INVITE: 1n << 10n,
} as const;

export function hasPermission(bits: PermissionBits, permission: bigint): boolean {
  const value = BigInt(bits);
  return (value & Permission.ADMINISTRATOR) !== 0n || (value & permission) !== 0n;
}

/** Every bit a participant holds in a direct-message conversation (`direct_message_permissions`). */
export const DIRECT_MESSAGE_PERMISSIONS: PermissionBits = String(
  Permission.VIEW_CHANNEL | Permission.SEND_MESSAGES | Permission.READ_MESSAGE_HISTORY,
);

/**
 * `compute_channel_permissions` of mellow-protocol (ADR-0007 step 4): the guild permissions, then
 * the channel's overwrites for `@everyone` (the role whose id is the guild id), for the member's
 * roles (all denies, then all allows) and for the member. Administrators ignore overwrites. Only
 * to adapt the interface: the instance decides (ADR-0014).
 */
export function computeChannelPermissions(
  guildPermissions: PermissionBits,
  guildId: Snowflake,
  memberId: Snowflake,
  memberRoleIds: readonly Snowflake[],
  overwrites: readonly PermissionOverwrite[],
): PermissionBits {
  let permissions = BigInt(guildPermissions);
  if ((permissions & Permission.ADMINISTRATOR) !== 0n) {
    return guildPermissions;
  }
  const apply = (current: bigint, overwrite: PermissionOverwrite) =>
    (current & ~BigInt(overwrite.deny)) | BigInt(overwrite.allow);
  const everyone = overwrites.find((overwrite) => overwrite.kind === "role" && overwrite.id === guildId);
  if (everyone) {
    permissions = apply(permissions, everyone);
  }
  let roleDeny = 0n;
  let roleAllow = 0n;
  for (const overwrite of overwrites) {
    if (overwrite.kind === "role" && overwrite.id !== guildId && memberRoleIds.includes(overwrite.id)) {
      roleDeny |= BigInt(overwrite.deny);
      roleAllow |= BigInt(overwrite.allow);
    }
  }
  permissions = (permissions & ~roleDeny) | roleAllow;
  const member = overwrites.find((overwrite) => overwrite.kind === "member" && overwrite.id === memberId);
  if (member) {
    permissions = apply(permissions, member);
  }
  return String(permissions);
}

/** Snowflakes compare by value, not as strings ("9" > "10" as text). */
export function compareSnowflakes(left: Snowflake, right: Snowflake): number {
  const difference = BigInt(left) - BigInt(right);
  return difference === 0n ? 0 : difference < 0n ? -1 : 1;
}
