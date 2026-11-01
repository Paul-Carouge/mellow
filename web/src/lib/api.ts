// REST client for `/api/v1` on the origin that served this page (ADR-0019). The session is an
// HttpOnly cookie the instance sets at sign-in: page scripts never see the token.

import {
  AUDIT_LOG_PAGE_SIZE,
  type ApiErrorCode,
  type Attachment,
  type AuditLogEntry,
  type Ban,
  type Channel,
  type ChannelKind,
  type ChannelPlacement,
  type InitialCategory,
  type ChosenStatus,
  type CurrentUser,
  type CustomEmoji,
  type DirectMessageChannel,
  type Guild,
  type GuildInvite,
  type Invite,
  type Member,
  type Message,
  type MessageSearchQuery,
  type MessageSearchResults,
  type PermissionBits,
  type PermissionOverwrite,
  type PublicUser,
  type Relationship,
  type Role,
  type SessionCreated,
  type Snowflake,
  type SoundboardSound,
  reactionsPath,
} from "./protocol";

export const HISTORY_PAGE_SIZE = 50;
/** The largest history page the instance gives (`MAXIMUM_HISTORY_PAGE_SIZE` on the server). */
export const MAXIMUM_HISTORY_PAGE_SIZE = 100;
/** One page of "who reacted": the most the instance gives at once. */
export const REACTING_USERS_PAGE_SIZE = 100;
/** The path segment of `/users/@me/{kind}` (ADR-0021). */
export type ProfileImageKind = "avatar" | "banner";

/** What `POST /guilds/{id}/invites` accepts: up to 10 000 uses and 30 days, `null` for no limit. */
export interface InviteLimits {
  readonly maxUses: number | null;
  readonly expiresInSeconds: number | null;
}

const DEVICE_NAME = "Web browser";
/** JSON requests only: uploads may take longer on a slow link. */
const REQUEST_TIMEOUT_MILLISECONDS = 30_000;
const SESSION_TRANSPORT_HEADER = "X-Mellow-Session-Transport";

/** A refusal from the instance. Its `message` is the instance's own text: never shown as is. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly code: ApiErrorCode | "NETWORK" | "FILE_TOO_LARGE",
  ) {
    super(`request failed: ${code}`);
    this.name = "ApiError";
  }

  get needsNewLogin(): boolean {
    return this.code === "UNAUTHORIZED";
  }
}

export class RestClient {
  private async request<T>(method: string, path: string, body?: unknown): Promise<T> {
    const headers: Record<string, string> = { [SESSION_TRANSPORT_HEADER]: "cookie" };
    if (body !== undefined) {
      headers["Content-Type"] = "application/json";
    }
    let response: Response;
    try {
      response = await fetch(`/api/v1/${path}`, {
        method,
        headers,
        body: body === undefined ? undefined : JSON.stringify(body),
        credentials: "same-origin",
        // A silently dead connection would otherwise leave a send "Sending…" for minutes, with no
        // way to retry; sends carry a nonce, so retrying after a timeout never duplicates them.
        signal: AbortSignal.timeout(REQUEST_TIMEOUT_MILLISECONDS),
      });
    } catch {
      throw new ApiError(0, "NETWORK");
    }
    if (!response.ok) {
      const code = await response
        .json()
        .then((error: { code?: ApiErrorCode }) => error.code ?? "INTERNAL")
        .catch(() => "INTERNAL" as const);
      throw new ApiError(response.status, code);
    }
    if (response.status === 204) {
      return undefined as T;
    }
    return (await response.json()) as T;
  }

  login(login: string, password: string): Promise<SessionCreated> {
    return this.request<SessionCreated>("POST", "auth/login", {
      login,
      password,
      device_name: DEVICE_NAME,
    });
  }

  register(username: string, email: string, password: string): Promise<SessionCreated> {
    return this.request<SessionCreated>("POST", "auth/register", {
      username,
      email,
      password,
      device_name: DEVICE_NAME,
    });
  }

  logout(): Promise<void> {
    return this.request<void>("POST", "auth/logout");
  }

  currentUser(): Promise<CurrentUser> {
    return this.request("GET", "users/@me");
  }

  /** Absent fields stay unchanged; a new username needs the current password. */
  updateCurrentUser(changes: {
    display_name?: string;
    username?: string;
    current_password?: string;
  }): Promise<CurrentUser> {
    return this.request("PATCH", "users/@me", changes);
  }

  /** Replaces both; a blank or absent custom status clears it. */
  updatePresence(status: ChosenStatus, customStatus: string | null): Promise<CurrentUser> {
    return this.request("PUT", "users/@me/presence", { status, custom_status: customStatus });
  }

  /** Signs out the user's other sessions; this one stays. */
  changePassword(currentPassword: string, newPassword: string): Promise<void> {
    return this.request("POST", "users/@me/password", {
      current_password: currentPassword,
      new_password: newPassword,
    });
  }

  /** Without `initialCategories`, the instance adds an English "Text channels" with "general". */
  createGuild(name: string, initialCategories?: readonly InitialCategory[]): Promise<Guild> {
    return this.request("POST", "guilds", { name, initial_categories: initialCategories });
  }

  /** The instance reads `ae8c 78a9` as `AE8C-78A9`; spaces are dropped here, dashes kept. */
  joinWithInvite(typedCode: string): Promise<Guild> {
    const code = typedCode.replace(/\s+/g, "");
    // The code becomes a path segment: refuse anything that is not a plain code.
    if (!/^[A-Za-z0-9-]+$/.test(code)) {
      return Promise.reject(new ApiError(404, "INVALID_INVITE"));
    }
    return this.request("POST", `invites/${code}`);
  }

  /** `null` limits mean unlimited uses and an invite that never expires. */
  createInvite(guildId: Snowflake, limits: InviteLimits): Promise<Invite> {
    return this.request("POST", `guilds/${guildId}/invites`, {
      max_uses: limits.maxUses,
      expires_in_seconds: limits.expiresInSeconds,
    });
  }

  /** `categoryId` puts a text channel in that category; null leaves it uncategorised. */
  createChannel(
    guildId: Snowflake,
    kind: ChannelKind,
    name: string,
    topic = "",
    categoryId: Snowflake | null = null,
    userLimit: number | null = null,
  ): Promise<Channel> {
    return this.request("POST", `guilds/${guildId}/channels`, {
      kind,
      name,
      topic,
      parent_id: categoryId,
      ...(kind === "voice" && userLimit !== null ? { user_limit: userLimit } : {}),
    });
  }

  /** A deleted category's channels stay, uncategorised. */
  deleteChannel(channelId: Snowflake): Promise<void> {
    return this.request("DELETE", `channels/${channelId}`);
  }

  /** Every channel the user can view, once each, in their new order; answers the new list. */
  reorderChannels(guildId: Snowflake, placements: readonly ChannelPlacement[]): Promise<Channel[]> {
    return this.request("PUT", `guilds/${guildId}/channels/order`, { channels: placements });
  }

  // --- Server settings (DESIGN.md §5.8) -----------------------------------------------------

  renameGuild(guildId: Snowflake, name: string): Promise<Guild> {
    return this.request("PATCH", `guilds/${guildId}`, { name });
  }

  /** The picture as the raw body, never re-encoded here: the instance crops and compresses it (animated GIFs stay animated). */
  setGuildIcon(guildId: Snowflake, file: File, crop: IconCrop | null): Promise<Guild> {
    return this.sendFile("PUT", `guilds/${guildId}/icon${iconCropQuery(crop)}`, file);
  }

  removeGuildIcon(guildId: Snowflake): Promise<Guild> {
    return this.request("DELETE", `guilds/${guildId}/icon`);
  }

  /** Text channels only; always stored still. */
  setChannelIcon(channelId: Snowflake, file: File, crop: IconCrop | null): Promise<Channel> {
    return this.sendFile("PUT", `channels/${channelId}/icon${iconCropQuery(crop)}`, file);
  }

  removeChannelIcon(channelId: Snowflake): Promise<Channel> {
    return this.request("DELETE", `channels/${channelId}/icon`);
  }

  // --- Custom emoji (ADR-0028) --------------------------------------------------------------

  listEmojis(guildId: Snowflake): Promise<CustomEmoji[]> {
    return this.request("GET", `guilds/${guildId}/emojis`);
  }

  /** The picture as the raw body (at most `MAXIMUM_EMOJI_UPLOAD_BYTES`); the instance keeps its centred square. */
  createEmoji(guildId: Snowflake, name: string, file: File): Promise<CustomEmoji> {
    return this.sendFile("POST", `guilds/${guildId}/emojis?name=${encodeURIComponent(name)}`, file);
  }

  renameEmoji(guildId: Snowflake, emojiId: Snowflake, name: string): Promise<CustomEmoji> {
    return this.request("PATCH", `guilds/${guildId}/emojis/${emojiId}`, { name });
  }

  deleteEmoji(guildId: Snowflake, emojiId: Snowflake): Promise<void> {
    return this.request("DELETE", `guilds/${guildId}/emojis/${emojiId}`);
  }

  // --- Soundboard (ADR-0032) -----------------------------------------------------------------

  /** The canonical WAV as the raw body (`encodeWav`); `emoji` is a reaction key, or `null` for none. */
  createSound(guildId: Snowflake, name: string, emoji: string | null, wav: Blob): Promise<SoundboardSound> {
    const query = new URLSearchParams({ name, ...(emoji === null ? {} : { emoji }) });
    return this.sendFile("POST", `guilds/${guildId}/sounds?${query.toString()}`, wav, { "Content-Type": "audio/wav" });
  }

  /** Replaces both: `emoji: null` removes the sound's emoji. */
  updateSound(guildId: Snowflake, soundId: Snowflake, name: string, emoji: string | null): Promise<SoundboardSound> {
    return this.request("PATCH", `guilds/${guildId}/sounds/${soundId}`, { name, emoji });
  }

  deleteSound(guildId: Snowflake, soundId: Snowflake): Promise<void> {
    return this.request("DELETE", `guilds/${guildId}/sounds/${soundId}`);
  }

  /** `null` clears it; a sound of another server is `NOT_FOUND`. */
  setJoinSound(guildId: Snowflake, soundId: Snowflake | null): Promise<void> {
    return this.request("PATCH", `guilds/${guildId}/members/@me/join-sound`, { sound_id: soundId });
  }

  /** Absent fields stay unchanged. */
  /** `user_limit: null` clears a voice channel's limit. */
  updateChannel(channelId: Snowflake, changes: { name?: string; topic?: string; user_limit?: number | null }): Promise<Channel> {
    return this.request("PATCH", `channels/${channelId}`, changes);
  }

  setPermissionOverwrite(
    channelId: Snowflake,
    targetId: Snowflake,
    overwrite: { kind: PermissionOverwrite["kind"]; allow: PermissionBits; deny: PermissionBits },
  ): Promise<Channel> {
    return this.request("PUT", `channels/${channelId}/permissions/${targetId}`, overwrite);
  }

  deletePermissionOverwrite(channelId: Snowflake, targetId: Snowflake): Promise<Channel> {
    return this.request("DELETE", `channels/${channelId}/permissions/${targetId}`);
  }

  /** The owner only. */
  deleteGuild(guildId: Snowflake): Promise<void> {
    return this.request("DELETE", `guilds/${guildId}`);
  }

  createRole(guildId: Snowflake, name: string): Promise<Role> {
    return this.request("POST", `guilds/${guildId}/roles`, { name });
  }

  /** Absent fields stay unchanged; `@everyone` takes permissions only. */
  updateRole(
    guildId: Snowflake,
    roleId: Snowflake,
    changes: { name?: string; color?: number; permissions?: PermissionBits },
  ): Promise<Role> {
    return this.request("PATCH", `guilds/${guildId}/roles/${roleId}`, changes);
  }

  deleteRole(guildId: Snowflake, roleId: Snowflake): Promise<void> {
    return this.request("DELETE", `guilds/${guildId}/roles/${roleId}`);
  }

  /** Every role but `@everyone`, lowest to highest; answers the guild's new role list. */
  reorderRoles(guildId: Snowflake, roleIds: readonly Snowflake[]): Promise<Role[]> {
    return this.request("PUT", `guilds/${guildId}/roles/order`, { role_ids: roleIds });
  }

  addMemberRole(guildId: Snowflake, userId: Snowflake, roleId: Snowflake): Promise<void> {
    return this.request("PUT", `guilds/${guildId}/members/${userId}/roles/${roleId}`);
  }

  removeMemberRole(guildId: Snowflake, userId: Snowflake, roleId: Snowflake): Promise<void> {
    return this.request("DELETE", `guilds/${guildId}/members/${userId}/roles/${roleId}`);
  }

  kickMember(guildId: Snowflake, userId: Snowflake): Promise<void> {
    return this.request("DELETE", `guilds/${guildId}/members/${userId}`);
  }

  banMember(guildId: Snowflake, userId: Snowflake, reason: string): Promise<void> {
    return this.request("PUT", `guilds/${guildId}/bans/${userId}`, { reason });
  }

  unbanMember(guildId: Snowflake, userId: Snowflake): Promise<void> {
    return this.request("DELETE", `guilds/${guildId}/bans/${userId}`);
  }

  listBans(guildId: Snowflake): Promise<Ban[]> {
    return this.request("GET", `guilds/${guildId}/bans`);
  }

  listInvites(guildId: Snowflake): Promise<GuildInvite[]> {
    return this.request("GET", `guilds/${guildId}/invites`);
  }

  revokeInvite(code: string): Promise<void> {
    // The code becomes a path segment: refuse anything that is not a plain code.
    if (!/^[A-Za-z0-9]+$/.test(code)) {
      return Promise.reject(new ApiError(404, "NOT_FOUND"));
    }
    return this.request("DELETE", `invites/${code}`);
  }

  /** Newest first; `before` pages back from an entry id. */
  listAuditLog(guildId: Snowflake, before?: Snowflake): Promise<AuditLogEntry[]> {
    const query = new URLSearchParams({ limit: String(AUDIT_LOG_PAGE_SIZE) });
    if (before) {
      query.set("before", before);
    }
    return this.request("GET", `guilds/${guildId}/audit-log?${query}`);
  }

  leaveGuild(guildId: Snowflake): Promise<void> {
    return this.request("DELETE", `guilds/${guildId}/members/@me`);
  }

  listMembers(guildId: Snowflake): Promise<Member[]> {
    return this.request("GET", `guilds/${guildId}/members`);
  }

  listMessages(channelId: Snowflake, before?: Snowflake, limit = HISTORY_PAGE_SIZE): Promise<Message[]> {
    const query = new URLSearchParams({ limit: String(limit) });
    if (before) {
      query.set("before", before);
    }
    return this.request("GET", `channels/${channelId}/messages?${query}`);
  }

  sendMessage(
    channelId: Snowflake,
    content: string,
    nonce: string,
    replyToId: Snowflake | null = null,
    attachmentIds: readonly Snowflake[] = [],
  ): Promise<Message> {
    return this.request("POST", `channels/${channelId}/messages`, {
      content,
      reply_to_id: replyToId,
      nonce,
      ...(attachmentIds.length > 0 ? { attachment_ids: attachmentIds } : {}),
    });
  }

  /** One file, sent as the raw body; its id goes into a message within the hour (ADR-0020). */
  uploadAttachment(channelId: Snowflake, file: File): Promise<Attachment> {
    // The name travels in a header, never in the URL: proxy logs must not hold file names.
    return this.sendFile("POST", `channels/${channelId}/attachments`, file, {
      "X-Mellow-File-Name": encodeURIComponent(file.name),
    });
  }

  /** The picture as the raw body; the instance resizes and compresses it before answering (ADR-0021). */
  setProfileImage(kind: ProfileImageKind, file: File): Promise<CurrentUser> {
    return this.sendFile("PUT", `users/@me/${kind}`, file);
  }

  removeProfileImage(kind: ProfileImageKind): Promise<CurrentUser> {
    return this.request("DELETE", `users/@me/${kind}`);
  }

  /** `extraHeaders` may replace the generic `Content-Type`. */
  private async sendFile<T>(
    method: string,
    path: string,
    file: Blob,
    extraHeaders: Record<string, string> = {},
  ): Promise<T> {
    let response: Response;
    try {
      response = await fetch(`/api/v1/${path}`, {
        method,
        headers: {
          [SESSION_TRANSPORT_HEADER]: "cookie",
          "Content-Type": "application/octet-stream",
          ...extraHeaders,
        },
        body: file,
        credentials: "same-origin",
      });
    } catch {
      throw new ApiError(0, "NETWORK");
    }
    if (response.status === 413) {
      throw new ApiError(413, "FILE_TOO_LARGE");
    }
    if (!response.ok) {
      const code = await response
        .json()
        .then((error: { code?: ApiErrorCode }) => error.code ?? "INTERNAL")
        .catch(() => "INTERNAL" as const);
      throw new ApiError(response.status, code);
    }
    return (await response.json()) as T;
  }

  editMessage(channelId: Snowflake, messageId: Snowflake, content: string): Promise<Message> {
    return this.request("PATCH", `channels/${channelId}/messages/${messageId}`, { content });
  }

  deleteMessage(channelId: Snowflake, messageId: Snowflake): Promise<void> {
    return this.request("DELETE", `channels/${channelId}/messages/${messageId}`);
  }

  /** Idempotent: reacting twice with one emoji changes nothing. */
  addReaction(channelId: Snowflake, messageId: Snowflake, emoji: string): Promise<void> {
    return this.request("PUT", `${reactionsPath(channelId, messageId, emoji)}/@me`);
  }

  removeOwnReaction(channelId: Snowflake, messageId: Snowflake, emoji: string): Promise<void> {
    return this.request("DELETE", `${reactionsPath(channelId, messageId, emoji)}/@me`);
  }

  /** A moderator removes someone else's reaction. */
  removeUserReaction(channelId: Snowflake, messageId: Snowflake, emoji: string, userId: Snowflake): Promise<void> {
    return this.request("DELETE", `${reactionsPath(channelId, messageId, emoji)}/${userId}`);
  }

  /** A moderator removes every reaction of the message. */
  removeAllReactions(channelId: Snowflake, messageId: Snowflake): Promise<void> {
    return this.request("DELETE", reactionsPath(channelId, messageId));
  }

  /** Who reacted with this emoji, by ascending id: the first page only. */
  listReactingUsers(channelId: Snowflake, messageId: Snowflake, emoji: string): Promise<PublicUser[]> {
    const query = new URLSearchParams({ limit: String(REACTING_USERS_PAGE_SIZE) });
    return this.request("GET", `${reactionsPath(channelId, messageId, emoji)}?${query}`);
  }

  /** Searches a server's channels the user can read, or with `guildId` null their conversations. */
  searchMessages(guildId: Snowflake | null, query: MessageSearchQuery): Promise<MessageSearchResults> {
    // A POST, so the query text never shows in URLs and access logs.
    const path = guildId === null ? "users/@me/messages/search" : `guilds/${guildId}/messages/search`;
    return this.request("POST", path, query);
  }

  /** The most recently pinned first. */
  listPins(channelId: Snowflake): Promise<Message[]> {
    return this.request("GET", `channels/${channelId}/pins`);
  }

  pinMessage(channelId: Snowflake, messageId: Snowflake): Promise<void> {
    return this.request("PUT", `channels/${channelId}/pins/${messageId}`);
  }

  unpinMessage(channelId: Snowflake, messageId: Snowflake): Promise<void> {
    return this.request("DELETE", `channels/${channelId}/pins/${messageId}`);
  }

  acknowledge(channelId: Snowflake, messageId: Snowflake): Promise<void> {
    return this.request("PUT", `channels/${channelId}/read-state`, {
      last_read_message_id: messageId,
    });
  }

  startTyping(channelId: Snowflake): Promise<void> {
    return this.request("POST", `channels/${channelId}/typing`);
  }

  /** A new group with these friends of the caller (ADR-0026). */
  createGroupConversation(recipientIds: Snowflake[], name: string): Promise<DirectMessageChannel> {
    return this.request("POST", "group-conversations", { recipient_ids: recipientIds, name });
  }

  /** An empty name clears it. */
  renameGroupConversation(channelId: Snowflake, name: string): Promise<DirectMessageChannel> {
    return this.request("PATCH", `group-conversations/${channelId}`, { name });
  }

  /** Adds a friend of the caller's. */
  addGroupParticipant(channelId: Snowflake, userId: Snowflake): Promise<DirectMessageChannel> {
    return this.request("PUT", `group-conversations/${channelId}/recipients/${userId}`);
  }

  /** Removing oneself leaves the group; only its owner removes others. */
  removeGroupParticipant(channelId: Snowflake, userId: Snowflake): Promise<void> {
    return this.request("DELETE", `group-conversations/${channelId}/recipients/${userId}`);
  }

  listRelationships(): Promise<Relationship[]> {
    return this.request("GET", "users/@me/relationships");
  }

  /** Asks them to be friends, or accepts their request; a missing account or a block is `NOT_FOUND`. */
  sendFriendRequest(username: string): Promise<Relationship> {
    return this.request("POST", "users/@me/relationships", { username });
  }

  /** `friend` accepts their request; `blocked` blocks them. */
  setRelationship(userId: Snowflake, kind: "friend" | "blocked"): Promise<Relationship> {
    return this.request("PUT", `users/@me/relationships/${userId}`, { kind });
  }

  /** Removes a friend, refuses or cancels a request, or unblocks. */
  removeRelationship(userId: Snowflake): Promise<void> {
    return this.request("DELETE", `users/@me/relationships/${userId}`);
  }

  openDirectMessage(recipientId: Snowflake): Promise<DirectMessageChannel> {
    return this.request("POST", "users/@me/channels", { recipient_id: recipientId });
  }
}

/** The square kept from an icon, in whole pixels of the picture as the browser shows it. */
export interface IconCrop {
  x: number;
  y: number;
  size: number;
}

/** No crop lets the instance keep the centred square. */
export function iconCropQuery(crop: IconCrop | null): string {
  if (crop === null) {
    return "";
  }
  const query = new URLSearchParams({
    crop_x: String(crop.x),
    crop_y: String(crop.y),
    crop_size: String(crop.size),
  });
  return `?${query}`;
}

/** 32 hex characters, like the client core's `generate_message_nonce`. */
export function generateMessageNonce(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}
