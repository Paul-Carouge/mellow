// Wire types of `mellow-protocol` (REST `/api/v1`, gateway `/gateway?v=1`).
// shortcut: written by hand from the Rust crate; generate them from it (ADR-0019) once the web
// client covers more than chat, before the protocol grows further.

/** 64-bit ids travel as decimal strings: JavaScript numbers lose precision past 2^53. */
export type Snowflake = string;
/** Permission bitfield as a decimal string (u64). */
export type PermissionBits = string;

/** An avatar, a banner, or a guild or channel icon (ADR-0021). Its id changes with the picture, so its URLs are cached forever. */
export interface ProfileImage {
  id: Snowflake;
  is_animated: boolean;
}

export interface CurrentUser {
  id: Snowflake;
  username: string;
  display_name: string;
  email: string;
  is_instance_administrator: boolean;
  /** Absent when the user has none (and from older servers). */
  avatar?: ProfileImage | null;
  banner?: ProfileImage | null;
  /** Absent from older servers, which only knew "online". */
  status?: ChosenStatus;
  custom_status?: string | null;
}

/** The status a user picks for themselves (`PUT /users/@me/presence`). Others see "invisible" as offline. */
export type ChosenStatus = "online" | "idle" | "do_not_disturb" | "invisible";

/** What others see of a user's presence. */
export type PresenceStatus = "online" | "idle" | "do_not_disturb" | "offline";

export interface Presence {
  user_id: Snowflake;
  status: PresenceStatus;
  custom_status?: string | null;
}

/** The longest custom status the instance accepts, in characters. */
export const MAXIMUM_CUSTOM_STATUS_LENGTH = 128;
/** A message's text, as the instance counts it (`validate_message_content`). */
export const MAXIMUM_MESSAGE_LENGTH = 4_000;
export const MAXIMUM_MESSAGE_BYTES = 8_000;

export interface PublicUser {
  id: Snowflake;
  username: string;
  display_name: string;
  /** Absent when the user has none (and from older servers). */
  avatar?: ProfileImage | null;
  banner?: ProfileImage | null;
}

/** Mirrors the server's rule (mellow-server domain/account_rules.rs); the server still decides. */
export const MINIMUM_PASSWORD_LENGTH = 10;

/** Mirrors the server's rules (mellow-server domain/guild_rules.rs); the server still decides. */
export const MAXIMUM_SERVER_NAME_LENGTH = 100;
export const MAXIMUM_ROLE_NAME_LENGTH = 100;
export const MAXIMUM_BAN_REASON_LENGTH = 512;

/** What `PUT /users/@me/avatar` and `/banner` accept (ADR-0021). */
export const MAXIMUM_PROFILE_IMAGE_BYTES = 8 * 1024 * 1024;

/**
 * The animated picture only when asked for and there is one; otherwise the still one, which for a
 * still picture is the picture itself, so it is served from one URL and cached once.
 */
export function profileImageUrl(image: ProfileImage, isAnimationWanted: boolean): string {
  const base = `/api/v1/profile-images/${image.id}`;
  return image.is_animated && !isAnimationWanted ? `${base}/still` : base;
}

/** A guild or channel icon, with the same still-or-animated rule as `profileImageUrl`. */
export function iconUrl(image: ProfileImage, isAnimationWanted: boolean): string {
  const base = `/api/v1/icons/${image.id}`;
  return image.is_animated && !isAnimationWanted ? `${base}/still` : base;
}

/** Mirrors the server's rules (mellow-server domain/guild_rules.rs); the server still decides. */
export const MAXIMUM_CHANNEL_NAME_LENGTH = 100;
export const MAXIMUM_CHANNEL_TOPIC_LENGTH = 1024;

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
  /** Never animated. Absent when the channel has none (and from older servers). */
  icon?: ProfileImage | null;
  permission_overwrites: PermissionOverwrite[];
}

/** Entry of `PUT /guilds/{id}/channels/order`. */
export interface ChannelPlacement {
  id: Snowflake;
  /** `null` for categories and uncategorised channels. */
  parent_id: Snowflake | null;
}

/** A category a new server starts with (`POST /guilds`), and its text channels in order. */
export interface InitialCategory {
  name: string;
  channels: { name: string; topic?: string }[];
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
  /** Absent when the guild has none (and from older servers). */
  icon?: ProfileImage | null;
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

export interface Attachment {
  id: Snowflake;
  file_name: string;
  /** One of the four previewable image types, or `application/octet-stream`. */
  content_type: string;
  size_bytes: number;
}

/** Shown inline: the instance serves only these as images (ADR-0020). */
export const PREVIEWABLE_IMAGE_TYPES: readonly string[] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/**
 * Why a chosen file cannot become an avatar or banner, checked before sending it; null lets the
 * instance decide. An empty type (unknown to the browser) is left to the instance too.
 */
export function profileImageFileProblem(file: Pick<Blob, "size" | "type">): "not-a-picture" | "too-large" | null {
  if (file.type !== "" && !PREVIEWABLE_IMAGE_TYPES.includes(file.type)) {
    return "not-a-picture";
  }
  return file.size > MAXIMUM_PROFILE_IMAGE_BYTES ? "too-large" : null;
}

/** The name is not in the URL (the instance sends it in `Content-Disposition`). */
export function attachmentUrl(attachment: Attachment): string {
  return `/api/v1/attachments/${attachment.id}`;
}

export interface Message {
  id: Snowflake;
  channel_id: Snowflake;
  author: PublicUser;
  content: string;
  reply_to_id: Snowflake | null;
  edited_at_unix_milliseconds: number | null;
  nonce?: string | null;
  /** Absent from older servers' payloads. */
  attachments?: Attachment[];
  /**
   * In the order first added; `me` is the requester. Only in REST answers: gateway events never
   * carry them (keep the cached ones on `MESSAGE_UPDATE`). Absent when empty.
   */
  reactions?: ReactionSummary[];
  /** Absent from older servers' payloads. */
  pinned?: boolean;
  /**
   * Who the server found mentioned (`@username`, or a one-word display name, outside code) among
   * those who can view the channel; never the author. Absent when empty and from older servers.
   */
  mention_ids?: Snowflake[];
}

export interface ReactionSummary {
  emoji: string;
  count: number;
  me: boolean;
}

/** `MESSAGE_REACTION_ADD` and `MESSAGE_REACTION_REMOVE`. */
export interface MessageReaction {
  guild_id?: Snowflake;
  channel_id: Snowflake;
  message_id: Snowflake;
  user_id: Snowflake;
  emoji: string;
}

/** A message's reactions under `/api/v1/`, or one emoji's: the emoji percent-encoded. */
export function reactionsPath(channelId: Snowflake, messageId: Snowflake, emoji?: string): string {
  const base = `channels/${channelId}/messages/${messageId}/reactions`;
  return emoji === undefined ? base : `${base}/${encodeURIComponent(emoji)}`;
}

export interface ReadState {
  channel_id: Snowflake;
  last_read_message_id: Snowflake;
}

/**
 * Where a viewable channel with messages stands in `Ready`: its newest message and its unread
 * mentions (in a conversation: unread messages from others), counted up to
 * `MAXIMUM_COUNTED_UNREAD_MENTIONS`.
 */
export interface ChannelUnread {
  channel_id: Snowflake;
  last_message_id: Snowflake;
  mention_count: number;
}

/** The server stops counting here: show "99+" from 100. */
export const MAXIMUM_COUNTED_UNREAD_MENTIONS = 100;

export interface DirectMessageChannel {
  id: Snowflake;
  recipients: PublicUser[];
  last_message_id: Snowflake | null;
  /** Present for a group (ADR-0026): who may remove people from it. */
  owner_id?: Snowflake;
  /** A group's name; absent or empty for none. */
  name?: string;
}

/** At most this many people in a group, its creator included. */
export const MAXIMUM_GROUP_CONVERSATION_PARTICIPANTS = 10;
/** A group's name is at most this many characters. */
export const MAXIMUM_GROUP_CONVERSATION_NAME_LENGTH = 100;

export interface Invite {
  code: string;
  guild_id: Snowflake;
  uses: number;
  max_uses: number | null;
  expires_at_unix_milliseconds: number | null;
}

/** Entry of `GET /guilds/{id}/invites`: a usable invite, newest first. */
export interface GuildInvite {
  code: string;
  creator: PublicUser;
  uses: number;
  max_uses: number | null;
  expires_at_unix_milliseconds: number | null;
  created_at_unix_milliseconds: number;
}

export interface Ban {
  user: PublicUser;
  /** Shown to moderators only; empty when none was given. */
  reason: string;
  created_at_unix_milliseconds: number;
}

/** A newer server may send an action this client does not know: it reads as "unknown" then. */
export type AuditAction =
  | "channel_create"
  | "channel_update"
  | "channel_delete"
  | "overwrite_set"
  | "overwrite_delete"
  | "role_create"
  | "role_update"
  | "role_delete"
  | "roles_reorder"
  | "channels_reorder"
  | "member_role_add"
  | "member_role_remove"
  | "member_kick"
  | "member_ban_add"
  | "member_ban_remove"
  | "message_delete"
  | "message_pin"
  | "message_unpin"
  | "message_reaction_remove"
  | "guild_update"
  | "invite_create"
  | "invite_delete"
  | "unknown";

export interface AuditChange {
  key: "name" | "topic" | "color" | "permissions" | "allow" | "deny" | "code" | "icon" | "parent" | "unknown";
  /** Permissions and colours in decimal. */
  old_value: string;
  new_value: string;
}

/** Entry of `GET /guilds/{id}/audit-log`, newest first; its id encodes when it happened. */
export interface AuditLogEntry {
  id: Snowflake;
  /** Null once that account no longer exists. */
  actor_id: Snowflake | null;
  action: AuditAction;
  target_id: Snowflake | null;
  context_id: Snowflake | null;
  reason: string;
  changes: AuditChange[];
}

/** What `GET /guilds/{id}/audit-log` answers by default, and the most it answers. */
export const AUDIT_LOG_PAGE_SIZE = 50;

export interface Ready {
  session_id: string;
  user: CurrentUser;
  guilds: Guild[];
  read_states: ReadState[];
  online_user_ids: Snowflake[];
  direct_message_channels?: DirectMessageChannel[];
  /** Added after `v0.10.0`: absent from older servers. */
  channel_unreads?: ChannelUnread[];
  /** Added after `v0.11.0`: absent from older servers, whose online users are all "online". */
  presences?: Presence[];
  /** Friends, requests and blocks (ADR-0025). Added after `v0.12.0`: absent from older servers. */
  relationships?: Relationship[];
  /** Whether the user may create servers on this instance; absent from older servers, which let everyone. */
  may_create_guilds?: boolean;
}

/** `incoming_request`: they asked; `outgoing_request`: you asked; `blocked`: never shown to them. */
export type RelationshipKind = "friend" | "incoming_request" | "outgoing_request" | "blocked";

export interface Relationship {
  user: PublicUser;
  kind: RelationshipKind;
  since_unix_milliseconds: number;
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
  | { t: "MESSAGE_REACTION_ADD"; data: MessageReaction }
  | { t: "MESSAGE_REACTION_REMOVE"; data: MessageReaction }
  | {
      t: "MESSAGE_REACTION_REMOVE_ALL";
      data: { guild_id?: Snowflake; channel_id: Snowflake; message_id: Snowflake };
    }
  | {
      t: "MESSAGE_REACTION_REMOVE_EMOJI";
      data: { guild_id?: Snowflake; channel_id: Snowflake; message_id: Snowflake; emoji: string };
    }
  | { t: "CHANNEL_CREATE"; data: Channel }
  | { t: "CHANNEL_UPDATE"; data: Channel }
  | { t: "CHANNEL_DELETE"; data: { guild_id: Snowflake; id: Snowflake } }
  | { t: "GUILD_CREATE"; data: Guild }
  /** `icon` is the current one: absent means none. */
  | { t: "GUILD_UPDATE"; data: { id: Snowflake; name: string; icon?: ProfileImage | null } }
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
  | { t: "PRESENCE_UPDATE"; data: Presence }
  | { t: "USER_UPDATE"; data: PublicUser }
  | { t: "DIRECT_MESSAGE_CHANNEL_CREATE"; data: DirectMessageChannel }
  | { t: "DIRECT_MESSAGE_CHANNEL_UPDATE"; data: DirectMessageChannel }
  | { t: "DIRECT_MESSAGE_CHANNEL_DELETE"; data: { id: Snowflake } }
  | { t: "RELATIONSHIP_ADD"; data: Relationship }
  | { t: "RELATIONSHIP_REMOVE"; data: { user_id: Snowflake } };

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
  ATTACH_FILES: 1n << 4n,
  MENTION_EVERYONE: 1n << 5n,
  MANAGE_MESSAGES: 1n << 6n,
  MANAGE_CHANNELS: 1n << 7n,
  MANAGE_ROLES: 1n << 8n,
  MANAGE_GUILD: 1n << 9n,
  CREATE_INVITE: 1n << 10n,
  KICK_MEMBERS: 1n << 11n,
  BAN_MEMBERS: 1n << 12n,
  VIEW_AUDIT_LOG: 1n << 15n,
  ADD_REACTIONS: 1n << 16n,
} as const;

/** What a channel overwrite may allow or deny (`Permissions::CHANNEL_SCOPED`). */
export const CHANNEL_SCOPED_PERMISSIONS =
  Permission.VIEW_CHANNEL |
  Permission.SEND_MESSAGES |
  Permission.READ_MESSAGE_HISTORY |
  Permission.ATTACH_FILES |
  Permission.MENTION_EVERYONE |
  Permission.MANAGE_MESSAGES |
  Permission.MANAGE_CHANNELS |
  Permission.MANAGE_ROLES |
  Permission.CREATE_INVITE |
  Permission.ADD_REACTIONS;

export function hasPermission(bits: PermissionBits, permission: bigint): boolean {
  const value = BigInt(bits);
  return (value & Permission.ADMINISTRATOR) !== 0n || (value & permission) !== 0n;
}

/** Every bit a participant holds in a direct-message conversation (`direct_message_permissions`). */
export const DIRECT_MESSAGE_PERMISSIONS: PermissionBits = String(
  Permission.VIEW_CHANNEL |
    Permission.SEND_MESSAGES |
    Permission.READ_MESSAGE_HISTORY |
    Permission.ATTACH_FILES |
    Permission.ADD_REACTIONS,
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
