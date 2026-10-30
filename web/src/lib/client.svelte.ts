// What the web client knows and does: the same model as `mellow-client-core` (state reducer,
// outbox, history pages), kept in Svelte runes so components re-render from it.

import { SvelteMap, SvelteSet } from "svelte/reactivity";

import { strings } from "../i18n";
import {
  ApiError,
  HISTORY_PAGE_SIZE,
  type IconCrop,
  type InviteLimits,
  type ProfileImageKind,
  RestClient,
  generateMessageNonce,
} from "./api";
import { GatewayConnection, type GatewayUpdate } from "./gateway";
import { channelSections } from "./channels";
import { reactionKey, withReactionAdded, withReactionRemoved, withoutEmoji } from "./reactions";
import {
  type Attachment,
  type Channel,
  type ChannelKind,
  type ChannelPlacement,
  type ChosenStatus,
  type CurrentUser,
  type CustomEmoji,
  type DirectMessageChannel,
  type Guild,
  type InitialCategory,
  type GatewayEvent,
  type Invite,
  type Member,
  type Message,
  type PermissionBits,
  type PermissionOverwrite,
  type Presence,
  type PresenceStatus,
  type PublicUser,
  type ReactionSummary,
  type Ready,
  type Relationship,
  type RelationshipKind,
  type Role,
  type Snowflake,
  DIRECT_MESSAGE_PERMISSIONS,
  MAXIMUM_COUNTED_UNREAD_MENTIONS,
  compareSnowflakes,
  computeChannelPermissions,
} from "./protocol";

const MAXIMUM_CACHED_MESSAGES_PER_CHANNEL = 500;
const TYPING_INDICATOR_LIFETIME_MILLISECONDS = 8_000;
const TYPING_SEND_INTERVAL_MILLISECONDS = 5_000;

/** `unreachable`: the instance did not answer at launch, so whether a session exists is unknown. */
export type ConnectionStatus = "signed-out" | "unreachable" | "connecting" | "connected" | "reconnecting";

/** Refusals a retry would meet again: such a message stays failed instead of blocking the others. */
const REFUSED_FOR_GOOD: ReadonlySet<ApiError["code"]> = new Set([
  "VALIDATION_FAILED",
  "NOT_FOUND",
  "MISSING_PERMISSIONS",
  "LIMIT_REACHED",
]);

export interface PendingMessage {
  nonce: string;
  channelId: Snowflake;
  content: string;
  replyToId: Snowflake | null;
  /** Uploaded already; they stay sendable for an hour (ADR-0020). */
  attachments: Attachment[];
  status: "sending" | "failed";
  /** Why the last attempt failed, shown under "Not sent."; absent while sending. */
  failureReason?: string;
}

/** A channel's pinned messages, newest pin first, as far as the client has asked for them. */
export type PinnedMessages =
  | { status: "loading" }
  | { status: "failed" }
  | { status: "loaded"; messages: Message[] };

/** What a channel's visit counts as new: after the last read message, up to the newest at opening. */
export interface UnreadMarker {
  lastReadMessageId: Snowflake;
  newestAtOpenMessageId: Snowflake;
}

/** Text shown to people: built from the error code, never from the instance's own message. */
export function describeError(error: unknown, summary: string): string {
  return strings.errors.withReason(summary, errorReason(error));
}

/** Why a request failed, as a sentence end ("the instance refused what was entered."). */
export function errorReason(error: unknown): string {
  if (!(error instanceof ApiError)) {
    return strings.errors.unknownReason;
  }
  const reasons: Partial<Record<ApiError["code"], string>> = strings.errors.reasons;
  return reasons[error.code] ?? strings.errors.unknownReason;
}

export class MellowClient {
  status = $state<ConnectionStatus>("signed-out");
  user = $state<CurrentUser | null>(null);
  guilds = $state<Guild[]>([]);
  /** Most recent activity first. */
  conversations = $state<DirectMessageChannel[]>([]);
  messagesByChannel = new SvelteMap<Snowflake, Message[]>();
  /** Channels whose first message is cached: nothing older to fetch. */
  reachedStartChannelIds = new SvelteSet<Snowflake>();
  membersByGuild = new SvelteMap<Snowflake, Member[]>();
  /** What others see of each user online now; absent users are offline. */
  presenceByUserId = new SvelteMap<Snowflake, Presence>();
  /** Friends, requests and blocks by the other person's id (ADR-0025). */
  relationshipByUserId = new SvelteMap<Snowflake, Relationship>();
  lastReadByChannel = new SvelteMap<Snowflake, Snowflake>();
  /**
   * Each channel's newest message as far as this session knows (`Ready.channel_unreads`, then
   * `MESSAGE_CREATE`), so a channel never opened still shows unread. Kept apart from the guild
   * snapshots, which `GUILD_CREATE` and `CHANNEL_UPDATE` replace.
   */
  private newestMessageIdByChannel = new SvelteMap<Snowflake, Snowflake>();
  /**
   * Unread messages that mention the user (in a conversation: unread messages from others), from
   * `Ready.channel_unreads`, then counted live; absent means none.
   */
  private mentionCountByChannel = new SvelteMap<Snowflake, number>();
  /**
   * Each channel's newest message when the server built `Ready`. The gateway replays what was
   * committed while it did, and `Ready` already counted those: a replayed message at or below
   * this id is cached but not counted again. Not reactive: it never drives the interface.
   */
  private newestCountedByReady = new Map<Snowflake, Snowflake>();
  /** `Ready` carried `channel_unreads`: the server fills `Message.mention_ids`, absent when empty. */
  doesServerFindMentions = $state(false);
  /**
   * The last message read before the open channel was opened, while that visit shows the "New"
   * divider and the jump bar (DESIGN.md §5.4). Opening the channel still marks it read at once.
   */
  unreadMarkers = new SvelteMap<Snowflake, UnreadMarker>();
  /** Set when a channel is opened; the first acknowledgement with messages decides its marker. */
  private isUnreadMarkerPending = false;
  /** `${channelId}:${userId}` → when the indicator expires. */
  typingUntil = new SvelteMap<string, number>();
  pendingMessages = $state<PendingMessage[]>([]);
  /** `null` is home (direct messages). */
  selectedGuildId = $state<Snowflake | null>(null);
  selectedChannelId = $state<Snowflake | null>(null);
  isLoadingOlder = $state(false);
  /** Their last history request failed: no automatic retry until "Try again" or the next `Ready`. */
  failedHistoryChannelIds = new SvelteSet<Snowflake>();
  failedMemberGuildIds = new SvelteSet<Snowflake>();
  /** Empty hides it; built with `describeError`. */
  signOutError = $state("");
  /**
   * The open conversation that closed under the user: they left it, or its owner removed them
   * (ADR-0026). The shell announces it once Friends shows; cleared at sign-out.
   */
  closedConversation = $state<{ conversation: DirectMessageChannel; reason: "left" | "removed" } | null>(null);
  /** Groups the user asked to leave: the gateway's delete may arrive before the answer. */
  private channelIdsBeingLeft = new Set<Snowflake>();
  /** Loaded when the pins panel opens; kept up to date by `MESSAGE_UPDATE` and deletions. */
  pinsByChannel = new SvelteMap<Snowflake, PinnedMessages>();
  /** `${messageId}:${emoji key}` of the user's reaction clicks still on their way. */
  private reactionsInFlight = new Set<string>();
  /** Channels whose pins changed while their list was loading: asked again once it arrives. */
  private pinsChangedWhileLoading = new Set<Snowflake>();
  /**
   * Messages deleted this session: a history page or a send answer that left before the deletion
   * must not bring them back. Ids are never reused, so they are kept until sign-out.
   */
  private deletedMessageIds = new Set<Snowflake>();
  /** Bumped when the cache is emptied (`Ready`, sign-out): answers to older requests are dropped. */
  private historyGeneration = 0;

  private gateway: GatewayConnection | null = null;
  /** Before the first `Ready`, no selection exists yet: home then means "not chosen". */
  private hasReceivedReady = $state(false);
  /**
   * Called once per message new to this session (not a replay `Ready` already counted, not one
   * already cached), after the client state took it in: the notifier decides whether to tell.
   */
  onIncomingMessage: ((message: Message) => void) | null = null;
  /** Whether this user may create servers (`Ready.may_create_guilds`); `true` with servers that predate it. */
  mayCreateGuilds = $state(true);

  /** `rest` is injected by tests; settings pages read lists (bans, invites, the log) through it. */
  constructor(readonly rest: RestClient = new RestClient()) {}
  private lastTypingSentAt = 0;
  private requestedHistory = new Set<Snowflake>();
  private requestedMembers = new Set<Snowflake>();

  get selectedGuild(): Guild | null {
    return this.guilds.find((guild) => guild.id === this.selectedGuildId) ?? null;
  }

  get selectedChannel(): Channel | null {
    return this.selectedGuild?.channels.find((channel) => channel.id === this.selectedChannelId) ?? null;
  }

  get selectedConversation(): DirectMessageChannel | null {
    if (this.selectedGuildId !== null) {
      return null;
    }
    return this.conversations.find((channel) => channel.id === this.selectedChannelId) ?? null;
  }

  /** Home with no conversation open shows the Friends space, as on Discord (ADR-0025). */
  get isFriendsSpaceOpen(): boolean {
    return this.hasReceivedReady && this.selectedGuildId === null && this.selectedConversation === null;
  }

  /** What the signed-in user may do in a channel, only to adapt the interface (ADR-0014); `null`
   * for an unknown channel. */
  myChannelPermissions(channelId: Snowflake): PermissionBits | null {
    if (this.conversations.some((channel) => channel.id === channelId)) {
      return DIRECT_MESSAGE_PERMISSIONS;
    }
    const userId = this.user?.id;
    for (const guild of this.guilds) {
      const channel = guild.channels.find((candidate) => candidate.id === channelId);
      if (channel && userId) {
        return computeChannelPermissions(
          guild.my_permissions,
          guild.id,
          userId,
          guild.my_role_ids,
          channel.permission_overwrites,
        );
      }
    }
    return null;
  }

  messages(channelId: Snowflake): Message[] {
    return this.messagesByChannel.get(channelId) ?? [];
  }

  hasUnread(channelId: Snowflake): boolean {
    const cached = this.messages(channelId).at(-1)?.id ?? null;
    const known = this.conversations.find((channel) => channel.id === channelId)?.last_message_id ?? null;
    const reported = this.newestMessageIdByChannel.get(channelId) ?? null;
    const newest = [cached, known, reported]
      .filter((id): id is Snowflake => id !== null)
      .sort(compareSnowflakes)
      .at(-1);
    if (!newest) {
      return false;
    }
    const lastRead = this.lastReadByChannel.get(channelId);
    return !lastRead || compareSnowflakes(newest, lastRead) > 0;
  }

  mentionCount(channelId: Snowflake): number {
    return this.mentionCountByChannel.get(channelId) ?? 0;
  }

  /** The red count on a server: the sum of its visible channels. */
  guildMentionCount(guild: Guild): number {
    return guild.channels.reduce((sum, channel) => sum + this.mentionCount(channel.id), 0);
  }

  /** The red count on Home: the sum of the conversations. */
  conversationsMentionCount(): number {
    return this.conversations.reduce((sum, conversation) => sum + this.mentionCount(conversation.id), 0);
  }

  typingUserIds(channelId: Snowflake, now: number): Snowflake[] {
    const prefix = `${channelId}:`;
    return [...this.typingUntil.entries()]
      .filter(([key, until]) => key.startsWith(prefix) && until > now)
      .map(([key]) => key.slice(prefix.length))
      .filter((userId) => userId !== this.user?.id);
  }

  // --- Session -------------------------------------------------------------------------------

  /** Resumes the session cookie's session, if the instance still accepts it. */
  async resumeStoredSession(): Promise<void> {
    try {
      this.user = await this.rest.currentUser();
    } catch (error) {
      // No session, or an expired one: the sign-in screen stays. An instance that did not answer
      // may still hold a session, so signing in again would only add one.
      this.status = error instanceof ApiError && error.needsNewLogin ? "signed-out" : "unreachable";
      return;
    }
    this.connect();
  }

  async signIn(login: string, password: string): Promise<void> {
    this.user = (await this.rest.login(login, password)).user;
    this.connect();
  }

  async register(username: string, email: string, password: string): Promise<void> {
    this.user = (await this.rest.register(username, email, password)).user;
    this.connect();
  }

  /** The session cookie can only be revoked by the instance: if it cannot be reached, the session
   * stays, or the next page load would sign back in on a shared computer. */
  async signOut(): Promise<void> {
    this.signOutError = "";
    try {
      await this.rest.logout();
    } catch (error) {
      if (!(error instanceof ApiError && error.needsNewLogin)) {
        this.signOutError = describeError(error, strings.settings.signOutFailed);
        return;
      }
    }
    this.endSession();
  }

  private connect(): void {
    this.gateway?.close();
    this.status = "connecting";
    this.gateway = new GatewayConnection(
      "",
      (update) => this.applyGatewayUpdate(update),
      () => this.isTokenRefused(),
    );
    this.gateway.open();
  }

  private async isTokenRefused(): Promise<boolean> {
    try {
      await this.rest.currentUser();
      return false;
    } catch (error) {
      return error instanceof ApiError && error.needsNewLogin;
    }
  }

  private endSession(): void {
    this.gateway?.close();
    this.gateway = null;
    this.hasReceivedReady = false;
    this.signOutError = "";
    this.closedConversation = null;
    this.requestedHistory.clear();
    this.requestedMembers.clear();
    this.failedHistoryChannelIds.clear();
    this.failedMemberGuildIds.clear();
    this.status = "signed-out";
    this.user = null;
    this.guilds = [];
    this.conversations = [];
    this.pendingMessages = [];
    this.messagesByChannel.clear();
    this.reachedStartChannelIds.clear();
    this.membersByGuild.clear();
    this.presenceByUserId.clear();
    this.relationshipByUserId.clear();
    this.lastReadByChannel.clear();
    this.newestMessageIdByChannel.clear();
    this.mentionCountByChannel.clear();
    this.doesServerFindMentions = false;
    this.unreadMarkers.clear();
    this.typingUntil.clear();
    this.pinsByChannel.clear();
    this.deletedMessageIds.clear();
    this.historyGeneration += 1;
    this.selectedGuildId = null;
    this.selectedChannelId = null;
  }

  // --- Gateway -------------------------------------------------------------------------------

  private applyGatewayUpdate(update: GatewayUpdate): void {
    switch (update.kind) {
      case "ready":
        this.applyReady(update.ready);
        this.status = "connected";
        void this.resendFailedMessages();
        break;
      case "event":
        this.applyEvent(update.event, Date.now());
        break;
      case "resumed":
        this.status = "connected";
        this.retryFailedLoads();
        void this.resendFailedMessages();
        break;
      case "disconnected":
        this.status = "reconnecting";
        break;
      case "signed-out":
        this.endSession();
        break;
      default: {
        const unknown: never = update;
        throw new Error(`unhandled gateway update ${JSON.stringify(unknown)}`);
      }
    }
  }

  applyReady(ready: Ready): void {
    const wasHome = this.hasReceivedReady && this.selectedGuildId === null;
    this.hasReceivedReady = true;
    this.user = ready.user;
    this.mayCreateGuilds = ready.may_create_guilds ?? true;
    this.guilds = ready.guilds;
    const conversations = ready.direct_message_channels ?? [];
    // Kept while its first message is still unsent: `Ready` lists only conversations with one.
    for (const previous of this.conversations) {
      const hasPending = this.pendingMessages.some((pending) => pending.channelId === previous.id);
      if (hasPending && !conversations.some((channel) => channel.id === previous.id)) {
        conversations.push(previous);
      }
    }
    this.conversations = sortByLastActivity(conversations);
    this.messagesByChannel.clear();
    this.historyGeneration += 1;
    this.reachedStartChannelIds.clear();
    this.membersByGuild.clear();
    this.requestedHistory.clear();
    this.requestedMembers.clear();
    this.failedHistoryChannelIds.clear();
    this.failedMemberGuildIds.clear();
    this.presenceByUserId.clear();
    this.relationshipByUserId.clear();
    for (const relationship of ready.relationships ?? []) {
      this.relationshipByUserId.set(relationship.user.id, relationship);
    }
    const presences =
      ready.presences ?? ready.online_user_ids.map((userId): Presence => ({ user_id: userId, status: "online" }));
    for (const presence of presences) {
      this.presenceByUserId.set(presence.user_id, presence);
    }
    // Pins may have changed while the link was down; the panel asks again when it opens.
    this.pinsByChannel.clear();
    this.lastReadByChannel.clear();
    for (const state of ready.read_states) {
      this.lastReadByChannel.set(state.channel_id, state.last_read_message_id);
    }
    this.doesServerFindMentions = ready.channel_unreads !== undefined;
    this.newestMessageIdByChannel.clear();
    this.mentionCountByChannel.clear();
    this.newestCountedByReady.clear();
    for (const unread of ready.channel_unreads ?? []) {
      this.newestMessageIdByChannel.set(unread.channel_id, unread.last_message_id);
      this.newestCountedByReady.set(unread.channel_id, unread.last_message_id);
      if (unread.mention_count > 0) {
        this.mentionCountByChannel.set(unread.channel_id, unread.mention_count);
      }
    }
    this.pendingMessages = this.pendingMessages.filter((pending) => this.isKnownChannel(pending.channelId));
    const guildId = this.guilds.some((guild) => guild.id === this.selectedGuildId)
      ? this.selectedGuildId
      : wasHome
        ? null
        : (this.guilds[0]?.id ?? null);
    this.selectGuild(guildId, this.selectedChannelId);
  }

  applyEvent(event: GatewayEvent, now: number): void {
    switch (event.t) {
      case "MESSAGE_CREATE": {
        const message = event.data;
        if (!this.isKnownChannel(message.channel_id)) {
          return;
        }
        this.typingUntil.delete(`${message.channel_id}:${message.author.id}`);
        this.noteConversationActivity(message.channel_id, message.id);
        const previous = this.cachedMessage(message.channel_id, message.id);
        const newest = newestOf(this.newestMessageIdByChannel.get(message.channel_id) ?? null, message.id);
        this.newestMessageIdByChannel.set(message.channel_id, newest ?? message.id);
        this.insertMessages(message.channel_id, [message]);
        if (message.channel_id === this.selectedChannelId) {
          // Moves the read position while the user looks, so the message is not counted below.
          void this.acknowledgeSelectedChannel();
        }
        const countedByReady = this.newestCountedByReady.get(message.channel_id);
        if (countedByReady === undefined || compareSnowflakes(message.id, countedByReady) > 0) {
          this.adjustMentionCount(message.channel_id, previous, message);
          if (previous === undefined) {
            this.onIncomingMessage?.(message);
          }
        }
        return;
      }
      case "MESSAGE_UPDATE": {
        if (!this.isKnownChannel(event.data.channel_id)) {
          return;
        }
        this.applyPinnedState(event.data);
        const messages = this.messagesByChannel.get(event.data.channel_id);
        if (!messages) {
          return;
        }
        // A late REST answer must not undo a newer edit the gateway already brought. Events never
        // carry reactions (`me` differs per viewer), so the cached ones stay.
        const updated = event.data;
        const previous = messages.find((message) => message.id === updated.id);
        if (previous && (previous.edited_at_unix_milliseconds ?? 0) <= (updated.edited_at_unix_milliseconds ?? 0)) {
          this.adjustMentionCount(updated.channel_id, previous, updated);
        }
        this.messagesByChannel.set(
          updated.channel_id,
          messages.map((message) =>
            message.id === updated.id &&
            (message.edited_at_unix_milliseconds ?? 0) <= (updated.edited_at_unix_milliseconds ?? 0)
              ? { ...updated, reactions: message.reactions }
              : message,
          ),
        );
        return;
      }
      case "MESSAGE_DELETE": {
        this.deletedMessageIds.add(event.data.id);
        this.dropPin(event.data.channel_id, event.data.id);
        const messages = this.messagesByChannel.get(event.data.channel_id);
        if (messages && this.isKnownChannel(event.data.channel_id)) {
          const deleted = this.cachedMessage(event.data.channel_id, event.data.id);
          this.adjustMentionCount(event.data.channel_id, deleted, undefined);
          this.messagesByChannel.set(
            event.data.channel_id,
            messages.filter((message) => message.id !== event.data.id),
          );
        }
        return;
      }
      case "CHANNEL_UPDATE":
        // An update racing the snapshot that hid its channel must not bring it back (ADR-0015).
        if (!this.isKnownChannel(event.data.id)) {
          return;
        }
        this.updateGuild(event.data.guild_id, (guild) => ({
          ...guild,
          channels: [...guild.channels.filter((channel) => channel.id !== event.data.id), event.data],
        }));
        return;
      case "CHANNEL_CREATE":
        this.updateGuild(event.data.guild_id, (guild) => ({
          ...guild,
          channels: [...guild.channels.filter((channel) => channel.id !== event.data.id), event.data],
        }));
        return;
      case "CHANNEL_DELETE":
        this.updateGuild(event.data.guild_id, (guild) => ({
          ...guild,
          channels: guild.channels
            .filter((channel) => channel.id !== event.data.id)
            .map((channel) => (channel.parent_id === event.data.id ? { ...channel, parent_id: null } : channel)),
        }));
        this.forgetChannel(event.data.id);
        if (this.selectedChannelId === event.data.id) {
          this.selectGuild(this.selectedGuildId, null);
        }
        return;
      case "GUILD_CREATE":
        this.upsertGuild(event.data);
        return;
      case "GUILD_UPDATE":
        this.updateGuild(event.data.id, (guild) => ({
          ...guild,
          name: event.data.name,
          icon: event.data.icon ?? null,
        }));
        return;
      case "GUILD_DELETE":
        this.removeGuild(event.data.id);
        return;
      case "GUILD_MEMBER_ADD": {
        const members = this.membersByGuild.get(event.data.guild_id);
        if (members && !members.some((member) => member.user.id === event.data.member.user.id)) {
          this.membersByGuild.set(event.data.guild_id, [...members, event.data.member]);
        }
        return;
      }
      case "GUILD_MEMBER_REMOVE": {
        const members = this.membersByGuild.get(event.data.guild_id);
        if (members) {
          this.membersByGuild.set(
            event.data.guild_id,
            members.filter((member) => member.user.id !== event.data.user_id),
          );
        }
        return;
      }
      case "GUILD_MEMBER_UPDATE": {
        const members = this.membersByGuild.get(event.data.guild_id);
        if (members) {
          this.membersByGuild.set(
            event.data.guild_id,
            members.map((member) =>
              member.user.id === event.data.user_id ? { ...member, role_ids: event.data.role_ids } : member,
            ),
          );
        }
        return;
      }
      case "GUILD_ROLES_UPDATE": {
        // A deleted role leaves its members without a `GUILD_MEMBER_UPDATE`: drop it here.
        const roleIds = new Set(event.data.roles.map((role) => role.id));
        this.updateGuild(event.data.guild_id, (guild) => ({
          ...guild,
          roles: event.data.roles,
          my_role_ids: guild.my_role_ids.filter((id) => roleIds.has(id)),
        }));
        const members = this.membersByGuild.get(event.data.guild_id);
        if (members?.some((member) => member.role_ids.some((id) => !roleIds.has(id)))) {
          this.membersByGuild.set(
            event.data.guild_id,
            members.map((member) => ({ ...member, role_ids: member.role_ids.filter((id) => roleIds.has(id)) })),
          );
        }
        return;
      }
      case "GUILD_EMOJIS_UPDATE":
        this.updateGuild(event.data.guild_id, (guild) => ({ ...guild, emojis: event.data.emojis }));
        return;
      case "TYPING_START":
        for (const [key, until] of this.typingUntil) {
          if (until <= now) {
            this.typingUntil.delete(key);
          }
        }
        if (this.isKnownChannel(event.data.channel_id)) {
          this.typingUntil.set(
            `${event.data.channel_id}:${event.data.user_id}`,
            now + TYPING_INDICATOR_LIFETIME_MILLISECONDS,
          );
        }
        return;
      case "PRESENCE_UPDATE":
        if (event.data.status === "offline") {
          this.presenceByUserId.delete(event.data.user_id);
        } else {
          this.presenceByUserId.set(event.data.user_id, event.data);
        }
        this.applyOwnPresenceFromAnotherDevice(event.data);
        return;
      case "USER_UPDATE":
        this.applyUserUpdate(event.data);
        return;
      case "DIRECT_MESSAGE_CHANNEL_CREATE":
      case "DIRECT_MESSAGE_CHANNEL_UPDATE":
        this.upsertConversation(event.data);
        return;
      case "DIRECT_MESSAGE_CHANNEL_DELETE":
        this.dropConversation(event.data.id);
        return;
      case "RELATIONSHIP_ADD":
        this.relationshipByUserId.set(event.data.user.id, event.data);
        return;
      case "RELATIONSHIP_REMOVE":
        this.relationshipByUserId.delete(event.data.user_id);
        return;
      case "MESSAGE_REACTION_ADD": {
        const { channel_id, message_id, emoji, user_id } = event.data;
        const isMine = user_id === this.user?.id;
        this.updateReactions(channel_id, message_id, (reactions) => withReactionAdded(reactions, emoji, isMine));
        return;
      }
      case "MESSAGE_REACTION_REMOVE": {
        const { channel_id, message_id, emoji, user_id } = event.data;
        const isMine = user_id === this.user?.id;
        this.updateReactions(channel_id, message_id, (reactions) => withReactionRemoved(reactions, emoji, isMine));
        return;
      }
      case "MESSAGE_REACTION_REMOVE_EMOJI": {
        const { channel_id, message_id, emoji } = event.data;
        this.updateReactions(channel_id, message_id, (reactions) => withoutEmoji(reactions, emoji));
        return;
      }
      case "MESSAGE_REACTION_REMOVE_ALL":
        this.updateReactions(event.data.channel_id, event.data.message_id, () => []);
        return;
      default: {
        // An event this client does not know yet: skipped, as the native clients do.
        const unknown: never = event;
        console.warn("skipping an unknown gateway event", unknown);
      }
    }
  }

  private isKnownChannel(channelId: Snowflake): boolean {
    return (
      this.conversations.some((channel) => channel.id === channelId) ||
      this.guilds.some((guild) => guild.channels.some((channel) => channel.id === channelId))
    );
  }

  /** Applies `change` to a message's reactions, in the history and in the pins alike. */
  private updateReactions(
    channelId: Snowflake,
    messageId: Snowflake,
    change: (reactions: readonly ReactionSummary[]) => ReactionSummary[],
  ): void {
    if (!this.isKnownChannel(channelId)) {
      return;
    }
    const withChangedReactions = (message: Message) =>
      message.id === messageId ? { ...message, reactions: change(message.reactions ?? []) } : message;
    const messages = this.messagesByChannel.get(channelId);
    if (messages) {
      this.messagesByChannel.set(channelId, messages.map(withChangedReactions));
    }
    const pins = this.pinsByChannel.get(channelId);
    if (pins?.status === "loaded") {
      this.pinsByChannel.set(channelId, { status: "loaded", messages: pins.messages.map(withChangedReactions) });
    }
    this.notePinsChangedWhileLoading(channelId);
  }

  /** A change the list on its way may not hold yet. */
  private notePinsChangedWhileLoading(channelId: Snowflake): void {
    if (this.pinsByChannel.get(channelId)?.status === "loading") {
      this.pinsChangedWhileLoading.add(channelId);
    }
  }

  /** A message pinned, unpinned or edited: the loaded pins follow, newest pin first. */
  private applyPinnedState(message: Message): void {
    if (this.deletedMessageIds.has(message.id)) {
      return;
    }
    this.notePinsChangedWhileLoading(message.channel_id);
    const pins = this.pinsByChannel.get(message.channel_id);
    if (pins?.status !== "loaded") {
      return;
    }
    const existing = pins.messages.find((candidate) => candidate.id === message.id);
    const others = pins.messages.filter((candidate) => candidate.id !== message.id);
    if (!message.pinned) {
      this.pinsByChannel.set(message.channel_id, { status: "loaded", messages: others });
      return;
    }
    const pinnedMessageWithReactions = {
      ...message,
      reactions: existing?.reactions ?? this.cachedReactions(message),
    };
    this.pinsByChannel.set(message.channel_id, {
      status: "loaded",
      messages: existing
        ? pins.messages.map((candidate) => (candidate.id === message.id ? pinnedMessageWithReactions : candidate))
        : [pinnedMessageWithReactions, ...others],
    });
  }

  private cachedReactions(message: Message): ReactionSummary[] {
    return this.messages(message.channel_id).find((candidate) => candidate.id === message.id)?.reactions ?? [];
  }

  private dropPin(channelId: Snowflake, messageId: Snowflake): void {
    this.notePinsChangedWhileLoading(channelId);
    const pins = this.pinsByChannel.get(channelId);
    if (pins?.status === "loaded") {
      this.pinsByChannel.set(channelId, {
        status: "loaded",
        messages: pins.messages.filter((message) => message.id !== messageId),
      });
    }
  }

  private updateGuild(guildId: Snowflake, change: (guild: Guild) => Guild): void {
    this.guilds = this.guilds.map((guild) => (guild.id === guildId ? change(guild) : guild));
  }

  /** A channel gone or hidden from us: its cache, its loads and its unsent messages go too. */
  private forgetChannel(channelId: Snowflake): void {
    this.messagesByChannel.delete(channelId);
    this.newestMessageIdByChannel.delete(channelId);
    this.mentionCountByChannel.delete(channelId);
    this.pinsByChannel.delete(channelId);
    this.reachedStartChannelIds.delete(channelId);
    this.requestedHistory.delete(channelId);
    this.failedHistoryChannelIds.delete(channelId);
    this.pendingMessages = this.pendingMessages.filter((pending) => pending.channelId !== channelId);
    for (const key of this.typingUntil.keys()) {
      if (key.startsWith(`${channelId}:`)) {
        this.typingUntil.delete(key);
      }
    }
  }

  private upsertGuild(guild: Guild): void {
    const previous = this.guilds.find((existing) => existing.id === guild.id);
    for (const channel of previous?.channels ?? []) {
      if (!guild.channels.some((candidate) => candidate.id === channel.id)) {
        this.forgetChannel(channel.id);
      }
    }
    const exists = previous !== undefined;
    this.guilds = exists
      ? this.guilds.map((existing) => (existing.id === guild.id ? guild : existing))
      : [...this.guilds, guild];
    // A fresh snapshot follows permission changes: channels it no longer lists were hidden.
    if (this.selectedGuildId === guild.id && !guild.channels.some((channel) => channel.id === this.selectedChannelId)) {
      this.selectGuild(guild.id, null);
    }
  }

  private removeGuild(guildId: Snowflake): void {
    const guild = this.guilds.find((candidate) => candidate.id === guildId);
    for (const channel of guild?.channels ?? []) {
      this.forgetChannel(channel.id);
    }
    this.guilds = this.guilds.filter((candidate) => candidate.id !== guildId);
    this.membersByGuild.delete(guildId);
    this.requestedMembers.delete(guildId);
    this.failedMemberGuildIds.delete(guildId);
    if (this.selectedGuildId === guildId) {
      this.selectGuild(this.guilds[0]?.id ?? null, null);
    }
  }

  upsertConversation(conversation: DirectMessageChannel): void {
    const existing = this.conversations.find((channel) => channel.id === conversation.id);
    const merged = existing
      ? {
          ...conversation,
          last_message_id: newestOf(existing.last_message_id, conversation.last_message_id),
        }
      : conversation;
    this.conversations = sortByLastActivity([
      ...this.conversations.filter((channel) => channel.id !== conversation.id),
      merged,
    ]);
  }

  private noteConversationActivity(channelId: Snowflake, messageId: Snowflake): void {
    if (!this.conversations.some((channel) => channel.id === channelId)) {
      return;
    }
    this.conversations = sortByLastActivity(
      this.conversations.map((channel) =>
        channel.id === channelId
          ? { ...channel, last_message_id: newestOf(channel.last_message_id, messageId) }
          : channel,
      ),
    );
  }

  /** Names and pictures wherever the user appears; an absent avatar or banner was removed. */
  private applyUserUpdate(updated: PublicUser): void {
    const changes = {
      username: updated.username,
      display_name: updated.display_name,
      avatar: updated.avatar ?? null,
      banner: updated.banner ?? null,
    };
    const userId = updated.id;
    const updateIfThisUser = <T extends PublicUser>(user: T): T => (user.id === userId ? { ...user, ...changes } : user);
    if (this.user?.id === userId) {
      this.user = updateIfThisUser(this.user);
    }
    for (const [guildId, members] of this.membersByGuild) {
      this.membersByGuild.set(
        guildId,
        members.map((member) => ({ ...member, user: updateIfThisUser(member.user) })),
      );
    }
    for (const [channelId, messages] of this.messagesByChannel) {
      this.messagesByChannel.set(
        channelId,
        messages.map((message) => ({ ...message, author: updateIfThisUser(message.author) })),
      );
    }
    this.conversations = this.conversations.map((channel) => ({
      ...channel,
      recipients: channel.recipients.map(updateIfThisUser),
    }));
    const relationship = this.relationshipByUserId.get(userId);
    if (relationship) {
      this.relationshipByUserId.set(userId, { ...relationship, user: updateIfThisUser(relationship.user) });
    }
  }

  /** Merges by id, oldest first; outside the open channel the cache keeps the newest messages. */
  private insertMessages(channelId: Snowflake, incoming: Message[]): void {
    const byId = new Map(this.messages(channelId).map((message) => [message.id, message]));
    for (const message of incoming) {
      if (!this.deletedMessageIds.has(message.id)) {
        byId.set(message.id, message);
      }
    }
    let merged = [...byId.values()].sort((left, right) => compareSnowflakes(left.id, right.id));
    if (channelId !== this.selectedChannelId && merged.length > MAXIMUM_CACHED_MESSAGES_PER_CHANNEL) {
      merged = merged.slice(-MAXIMUM_CACHED_MESSAGES_PER_CHANNEL);
      this.reachedStartChannelIds.delete(channelId);
    }
    this.messagesByChannel.set(channelId, merged);
  }

  // --- Navigation ----------------------------------------------------------------------------

  /** `guildId` null is home: it opens the given conversation, or the Friends space without one. */
  selectGuild(guildId: Snowflake | null, preferredChannelId: Snowflake | null): void {
    this.selectedGuildId = guildId;
    const guild = this.selectedGuild;
    let channelId: Snowflake | null;
    if (guild) {
      const textChannels = channelSections(guild).flatMap((section) => section.channels);
      channelId =
        textChannels.find((channel) => channel.id === preferredChannelId)?.id ?? textChannels[0]?.id ?? null;
      void this.ensureMembersLoaded(guild.id);
    } else {
      channelId = this.conversations.find((channel) => channel.id === preferredChannelId)?.id ?? null;
    }
    this.selectChannel(channelId);
  }

  selectChannel(channelId: Snowflake | null): void {
    const previousChannelId = this.selectedChannelId;
    if (previousChannelId && previousChannelId !== channelId) {
      this.trimHistory(previousChannelId);
      // Leaving a channel ends its visit: the divider does not come back with it.
      this.unreadMarkers.delete(previousChannelId);
    }
    if (channelId !== previousChannelId) {
      this.isUnreadMarkerPending = true;
    }
    this.selectedChannelId = channelId;
    if (channelId) {
      void this.ensureHistoryLoaded(channelId);
      void this.acknowledgeSelectedChannel();
    }
  }

  /** A channel left keeps only its newest messages, as `trim_history` does. */
  private trimHistory(channelId: Snowflake): void {
    const messages = this.messagesByChannel.get(channelId);
    if (messages && messages.length > MAXIMUM_CACHED_MESSAGES_PER_CHANNEL) {
      this.messagesByChannel.set(channelId, messages.slice(-MAXIMUM_CACHED_MESSAGES_PER_CHANNEL));
      this.reachedStartChannelIds.delete(channelId);
    }
  }

  private async ensureHistoryLoaded(channelId: Snowflake): Promise<void> {
    if (this.requestedHistory.has(channelId) || this.failedHistoryChannelIds.has(channelId)) {
      return;
    }
    this.requestedHistory.add(channelId);
    const generation = this.historyGeneration;
    try {
      const page = await this.rest.listMessages(channelId);
      if (generation !== this.historyGeneration) {
        return;
      }
      this.insertMessages(channelId, page);
      if (page.length < HISTORY_PAGE_SIZE) {
        this.reachedStartChannelIds.add(channelId);
      }
      if (channelId === this.selectedChannelId) {
        void this.acknowledgeSelectedChannel();
      }
    } catch (error) {
      if (generation !== this.historyGeneration) {
        return;
      }
      this.requestedHistory.delete(channelId);
      this.failedHistoryChannelIds.add(channelId);
      this.reportError(error, "load messages");
    }
  }

  /** "Try again" on a failed history: the first page if none loaded, else the older one. */
  retryLoadingHistory(): void {
    const channelId = this.selectedChannelId;
    if (!channelId) {
      return;
    }
    this.failedHistoryChannelIds.delete(channelId);
    if (this.messages(channelId).length === 0) {
      void this.ensureHistoryLoaded(channelId);
    } else {
      void this.loadOlderMessages();
    }
  }

  retryLoadingMembers(): void {
    const guildId = this.selectedGuildId;
    if (guildId) {
      this.failedMemberGuildIds.delete(guildId);
      void this.ensureMembersLoaded(guildId);
    }
  }

  /** After a resume, what failed while the link was down is asked again. */
  private retryFailedLoads(): void {
    this.failedHistoryChannelIds.clear();
    this.failedMemberGuildIds.clear();
    if (this.selectedChannelId) {
      void this.ensureHistoryLoaded(this.selectedChannelId);
    }
    if (this.selectedGuildId) {
      void this.ensureMembersLoaded(this.selectedGuildId);
    }
  }

  async loadOlderMessages(): Promise<void> {
    const channelId = this.selectedChannelId;
    if (
      !channelId ||
      this.isLoadingOlder ||
      this.reachedStartChannelIds.has(channelId) ||
      this.failedHistoryChannelIds.has(channelId)
    ) {
      return;
    }
    const oldest = this.messages(channelId)[0];
    if (!oldest) {
      return;
    }
    this.isLoadingOlder = true;
    const generation = this.historyGeneration;
    try {
      const page = await this.rest.listMessages(channelId, oldest.id);
      // Only next to the cursor: the cache was emptied or trimmed meanwhile, and a page inserted
      // elsewhere would leave a gap no later page fills.
      if (generation !== this.historyGeneration || this.messages(channelId)[0]?.id !== oldest.id) {
        return;
      }
      this.insertMessages(channelId, page);
      if (page.length < HISTORY_PAGE_SIZE) {
        this.reachedStartChannelIds.add(channelId);
      }
    } catch (error) {
      if (generation !== this.historyGeneration) {
        return;
      }
      // No automatic retry: scrolling at the top would otherwise ask again on every frame.
      this.failedHistoryChannelIds.add(channelId);
      this.reportError(error, "load older messages");
    } finally {
      this.isLoadingOlder = false;
    }
  }

  private async ensureMembersLoaded(guildId: Snowflake): Promise<void> {
    if (this.requestedMembers.has(guildId)) {
      return;
    }
    this.requestedMembers.add(guildId);
    try {
      this.membersByGuild.set(guildId, await this.rest.listMembers(guildId));
    } catch (error) {
      this.requestedMembers.delete(guildId);
      this.failedMemberGuildIds.add(guildId);
      this.reportError(error, "load members");
    }
  }

  /**
   * "Mark as read" on the jump bar, or the unread messages seen down to the latest: the divider
   * and the bar go; the channel is already read.
   */
  dismissUnreadMarker(channelId: Snowflake): void {
    this.unreadMarkers.delete(channelId);
  }

  /** Messages that arrived while the tab was hidden are read once it shows again. */
  noteTabVisible(): void {
    void this.acknowledgeSelectedChannel();
  }

  private async acknowledgeSelectedChannel(): Promise<void> {
    const channelId = this.selectedChannelId;
    const newest = channelId ? this.messages(channelId).at(-1) : undefined;
    // Unread for the server until someone sees it (tests run without a document).
    const isTabVisible = typeof document === "undefined" || document.visibilityState === "visible";
    if (!channelId || !newest || !isTabVisible) {
      return;
    }
    // The user is looking at the channel: its mentions are read.
    this.mentionCountByChannel.delete(channelId);
    const lastRead = this.lastReadByChannel.get(channelId);
    if (this.isUnreadMarkerPending) {
      this.isUnreadMarkerPending = false;
      // Only what others wrote since the last visit; a channel never read has no line to draw.
      const hasUnreadFromOthers =
        lastRead !== undefined &&
        this.messages(channelId).some(
          (message) => compareSnowflakes(message.id, lastRead) > 0 && message.author.id !== this.user?.id,
        );
      if (hasUnreadFromOthers) {
        this.unreadMarkers.set(channelId, { lastReadMessageId: lastRead, newestAtOpenMessageId: newest.id });
      }
    }
    if (lastRead && compareSnowflakes(newest.id, lastRead) <= 0) {
      return;
    }
    this.lastReadByChannel.set(channelId, newest.id);
    try {
      await this.rest.acknowledge(channelId, newest.id);
    } catch (error) {
      this.reportError(error, "acknowledge");
    }
  }

  private cachedMessage(channelId: Snowflake, messageId: Snowflake): Message | undefined {
    return this.messages(channelId).find((message) => message.id === messageId);
  }

  /**
   * Whether the message counts in its channel's red badge: after the read position, from someone
   * else, and mentioning the user, or anything in a conversation.
   */
  private isUnreadMentionOfCurrentUser(message: Message): boolean {
    const userId = this.user?.id;
    // The server leaves blocked authors out of its counts too (ADR-0025).
    if (!userId || message.author.id === userId || this.isBlocked(message.author.id)) {
      return false;
    }
    const lastRead = this.lastReadByChannel.get(message.channel_id);
    if (lastRead && compareSnowflakes(message.id, lastRead) <= 0) {
      return false;
    }
    const isConversation = this.conversations.some((channel) => channel.id === message.channel_id);
    return isConversation || (message.mention_ids ?? []).includes(userId);
  }

  /**
   * Follows a message from `before` to `after` (`undefined`: not known here, or deleted).
   * shortcut: a message this session never cached (counted in `Ready` only) cannot be followed,
   * so its edits and deletion leave the count as it is until the next `Ready`; upgrade if the
   * gateway ever sends mention counts.
   */
  private adjustMentionCount(channelId: Snowflake, before: Message | undefined, after: Message | undefined): void {
    // An older server sends no counts in `Ready`, so live ones would vanish at the next one.
    if (!this.doesServerFindMentions) {
      return;
    }
    const counts = (message: Message | undefined) => (message && this.isUnreadMentionOfCurrentUser(message) ? 1 : 0);
    const change = counts(after) - counts(before);
    // The server stops counting at the maximum: "at least 100" minus one is not 99.
    const isSaturated = this.mentionCount(channelId) >= MAXIMUM_COUNTED_UNREAD_MENTIONS;
    if (change === 0 || (change < 0 && isSaturated)) {
      return;
    }
    const next = Math.max(0, this.mentionCount(channelId) + change);
    if (next === 0) {
      this.mentionCountByChannel.delete(channelId);
    } else {
      this.mentionCountByChannel.set(channelId, next);
    }
  }

  // --- Messages (outbox) ---------------------------------------------------------------------

  pendingMessagesIn(channelId: Snowflake): PendingMessage[] {
    return this.pendingMessages.filter((pending) => pending.channelId === channelId);
  }

  /** Uploads one file to the open channel, for the next message sent there. */
  uploadAttachment(file: File): Promise<Attachment> {
    const channelId = this.selectedChannelId;
    if (!channelId) {
      return Promise.reject(new ApiError(404, "NOT_FOUND"));
    }
    return this.rest.uploadAttachment(channelId, file);
  }

  async sendMessage(
    content: string,
    replyToId: Snowflake | null = null,
    attachments: readonly Attachment[] = [],
  ): Promise<void> {
    const channelId = this.selectedChannelId;
    const text = content.trim();
    if (!channelId || (!text && attachments.length === 0)) {
      return;
    }
    const pending: PendingMessage = {
      nonce: generateMessageNonce(),
      channelId,
      content: text,
      replyToId,
      attachments: [...attachments],
      status: "sending",
    };
    this.pendingMessages = [...this.pendingMessages, pending];
    await this.deliver(pending);
  }

  async retryMessage(nonce: string): Promise<void> {
    const pending = this.pendingMessages.find((candidate) => candidate.nonce === nonce);
    if (pending && pending.status === "failed") {
      this.setPendingStatus(nonce, "sending");
      await this.deliver(pending);
    }
  }

  /** Only a failed message: one being sent could still arrive. */
  discardMessage(nonce: string): void {
    this.pendingMessages = this.pendingMessages.filter(
      (pending) => pending.nonce !== nonce || pending.status !== "failed",
    );
  }

  /** `refused` when a retry would be refused again. */
  private async deliver(pending: PendingMessage): Promise<"sent" | "refused" | "failed"> {
    try {
      // The same nonce on every attempt: a retry of a send that did arrive returns that message.
      const message = await this.rest.sendMessage(
        pending.channelId,
        pending.content,
        pending.nonce,
        pending.replyToId,
        pending.attachments.map((attachment) => attachment.id),
      );
      this.pendingMessages = this.pendingMessages.filter((candidate) => candidate.nonce !== pending.nonce);
      this.noteConversationActivity(message.channel_id, message.id);
      this.insertMessages(message.channel_id, [{ ...message, nonce: null }]);
      return "sent";
    } catch (error) {
      this.setPendingStatus(pending.nonce, "failed", errorReason(error));
      this.reportError(error, "send message");
      return error instanceof ApiError && REFUSED_FOR_GOOD.has(error.code) ? "refused" : "failed";
    }
  }

  private setPendingStatus(nonce: string, status: PendingMessage["status"], failureReason?: string): void {
    this.pendingMessages = this.pendingMessages.map((pending) =>
      pending.nonce === nonce ? { ...pending, status, failureReason } : pending,
    );
  }

  /** One after another, so failed messages keep their order: the whole batch is marked sending
   * first (so none can be discarded mid-way), and a failure that may pass stops it until the next
   * reconnect or a manual retry. A new session ends the batch. */
  private async resendFailedMessages(): Promise<void> {
    const gateway = this.gateway;
    const batch = this.pendingMessages.filter((candidate) => candidate.status === "failed");
    const nonces = new Set(batch.map((pending) => pending.nonce));
    this.pendingMessages = this.pendingMessages.map((pending) =>
      nonces.has(pending.nonce) ? { ...pending, status: "sending" } : pending,
    );
    for (const [index, pending] of batch.entries()) {
      if (this.gateway !== gateway || !this.pendingMessages.some((candidate) => candidate.nonce === pending.nonce)) {
        return;
      }
      if ((await this.deliver(pending)) === "failed") {
        for (const remaining of batch.slice(index + 1)) {
          this.setPendingStatus(remaining.nonce, "failed");
        }
        return;
      }
    }
  }

  /** In the message's own channel: the open channel may have changed while the editor was open. */
  async editMessage(message: Message, content: string): Promise<void> {
    const updated = await this.rest.editMessage(message.channel_id, message.id, content);
    this.applyEvent({ t: "MESSAGE_UPDATE", data: updated }, Date.now());
  }

  /** A message already gone (deleted elsewhere, its event not here yet) counts as deleted. */
  async deleteMessage(message: Message): Promise<void> {
    try {
      await this.rest.deleteMessage(message.channel_id, message.id);
    } catch (error) {
      if (!(error instanceof ApiError && error.code === "NOT_FOUND")) {
        throw error;
      }
    }
    this.applyEvent({ t: "MESSAGE_DELETE", data: { channel_id: message.channel_id, id: message.id } }, Date.now());
  }

  // --- Reactions and pins --------------------------------------------------------------------

  /**
   * Adds the user's reaction, or takes it back if they already reacted with it. Shown at once and
   * undone if the instance refuses; the error is thrown for the caller to tell. A second click
   * while the first is on its way is ignored, so the two cannot arrive out of order.
   */
  async toggleReaction(channelId: Snowflake, messageId: Snowflake, emoji: string): Promise<void> {
    const key = reactionKey(emoji);
    const flightKey = `${messageId}:${key}`;
    if (this.reactionsInFlight.has(flightKey)) {
      return;
    }
    const message =
      this.messages(channelId).find((candidate) => candidate.id === messageId) ??
      this.pinnedMessages(channelId).find((candidate) => candidate.id === messageId);
    const isRemoving = message?.reactions?.some((reaction) => reaction.emoji === key && reaction.me) ?? false;
    const applyOwnReactionLocally = (isAdding: boolean) =>
      this.updateReactions(channelId, messageId, (reactions) =>
        isAdding ? withReactionAdded(reactions, key, true) : withReactionRemoved(reactions, key, true),
      );
    this.reactionsInFlight.add(flightKey);
    applyOwnReactionLocally(!isRemoving);
    try {
      if (isRemoving) {
        await this.rest.removeOwnReaction(channelId, messageId, emoji);
      } else {
        await this.rest.addReaction(channelId, messageId, emoji);
      }
    } catch (error) {
      applyOwnReactionLocally(isRemoving);
      this.reportError(error, "react");
      throw error;
    } finally {
      this.reactionsInFlight.delete(flightKey);
    }
  }

  listReactingUsers(channelId: Snowflake, messageId: Snowflake, emoji: string): Promise<PublicUser[]> {
    return this.rest.listReactingUsers(channelId, messageId, emoji);
  }

  /** A moderator's removal: the instance's `MESSAGE_REACTION_REMOVE` updates the count. */
  removeUserReaction(channelId: Snowflake, messageId: Snowflake, emoji: string, userId: Snowflake): Promise<void> {
    return this.rest.removeUserReaction(channelId, messageId, emoji, userId);
  }

  async removeAllReactions(channelId: Snowflake, messageId: Snowflake): Promise<void> {
    await this.rest.removeAllReactions(channelId, messageId);
    this.updateReactions(channelId, messageId, () => []);
  }

  pinnedMessages(channelId: Snowflake): Message[] {
    const pins = this.pinsByChannel.get(channelId);
    return pins?.status === "loaded" ? pins.messages : [];
  }

  /** Asks the instance again unless the pins are loaded or on their way (`force` for Try again). */
  async loadPins(channelId: Snowflake, force = false): Promise<void> {
    const current = this.pinsByChannel.get(channelId);
    if (!force && (current?.status === "loaded" || current?.status === "loading")) {
      return;
    }
    this.pinsByChannel.set(channelId, { status: "loading" });
    this.pinsChangedWhileLoading.delete(channelId);
    try {
      const messages = await this.rest.listPins(channelId);
      if (!this.isKnownChannel(channelId)) {
        return;
      }
      if (this.pinsChangedWhileLoading.delete(channelId)) {
        // The answer may predate a pin, an unpin or a reaction: ask for a fresh one.
        await this.loadPins(channelId, true);
        return;
      }
      this.pinsByChannel.set(channelId, { status: "loaded", messages });
    } catch (error) {
      if (this.isKnownChannel(channelId)) {
        this.pinsByChannel.set(channelId, { status: "failed" });
      }
      this.reportError(error, "load pins");
    }
  }

  /** The instance answers with no message: the change is applied here, and its event repeats it. */
  async setPinned(message: Message, isPinned: boolean): Promise<void> {
    if (isPinned) {
      await this.rest.pinMessage(message.channel_id, message.id);
    } else {
      await this.rest.unpinMessage(message.channel_id, message.id);
    }
    const cached = this.messages(message.channel_id).find((candidate) => candidate.id === message.id) ?? message;
    const changed = { ...cached, pinned: isPinned };
    const messages = this.messagesByChannel.get(message.channel_id);
    if (messages) {
      this.messagesByChannel.set(
        message.channel_id,
        messages.map((candidate) => (candidate.id === message.id ? changed : candidate)),
      );
    }
    this.applyPinnedState(changed);
  }

  /** At most one indicator every few seconds while typing. */
  noteTyping(): void {
    const channelId = this.selectedChannelId;
    const now = Date.now();
    if (!channelId || now - this.lastTypingSentAt < TYPING_SEND_INTERVAL_MILLISECONDS) {
      return;
    }
    this.lastTypingSentAt = now;
    this.rest.startTyping(channelId).catch((error: unknown) => this.reportError(error, "typing"));
  }

  // --- Account -------------------------------------------------------------------------------

  async updateDisplayName(displayName: string): Promise<void> {
    this.applyCurrentUser(await this.rest.updateCurrentUser({ display_name: displayName.trim() }));
  }

  async updateUsername(username: string, currentPassword: string): Promise<void> {
    this.applyCurrentUser(
      await this.rest.updateCurrentUser({ username: username.trim(), current_password: currentPassword }),
    );
  }

  /**
   * Keeps this tab's own pick in step with the others, so a status chosen here never sends back a
   * custom status another device already changed. While this session is live, the user can only
   * look offline by going invisible; the custom status then stays as this tab knew it.
   */
  private applyOwnPresenceFromAnotherDevice(presence: Presence): void {
    if (!this.user || presence.user_id !== this.user.id) {
      return;
    }
    this.user =
      presence.status === "offline"
        ? { ...this.user, status: "invisible" }
        : { ...this.user, status: presence.status, custom_status: presence.custom_status ?? null };
  }

  // --- Friends (ADR-0025) ---------------------------------------------------------------------

  /** The relationships of `kind`, by name. */
  relationshipsOfKind(kind: RelationshipKind): Relationship[] {
    return [...this.relationshipByUserId.values()]
      .filter((relationship) => relationship.kind === kind)
      .sort((a, b) =>
        (a.user.display_name || a.user.username).localeCompare(b.user.display_name || b.user.username),
      );
  }

  /** Requests waiting for the user's answer: the Friends entry's badge. */
  incomingRequestCount(): number {
    return this.relationshipsOfKind("incoming_request").length;
  }

  isBlocked(userId: Snowflake): boolean {
    return this.relationshipByUserId.get(userId)?.kind === "blocked";
  }

  /**
   * Asks `username` to be friends, or accepts their request. The answer is applied at once; the
   * gateway's `RELATIONSHIP_ADD` brings the same.
   */
  async sendFriendRequest(username: string): Promise<Relationship> {
    const relationship = await this.rest.sendFriendRequest(username.trim());
    this.relationshipByUserId.set(relationship.user.id, relationship);
    return relationship;
  }

  async acceptFriendRequest(userId: Snowflake): Promise<void> {
    const relationship = await this.rest.setRelationship(userId, "friend");
    this.relationshipByUserId.set(relationship.user.id, relationship);
  }

  async blockUser(userId: Snowflake): Promise<void> {
    const relationship = await this.rest.setRelationship(userId, "blocked");
    this.relationshipByUserId.set(relationship.user.id, relationship);
  }

  /**
   * Removes a friend, refuses or cancels a request, or unblocks. Already gone (the other person
   * removed it, or another session did) counts as done: the wanted state is reached.
   */
  async removeRelationship(userId: Snowflake): Promise<void> {
    try {
      await this.rest.removeRelationship(userId);
    } catch (error) {
      if (!(error instanceof ApiError && error.code === "NOT_FOUND")) {
        throw error;
      }
    }
    this.relationshipByUserId.delete(userId);
  }

  /** A blank custom status clears it. */
  async updatePresence(status: ChosenStatus, customStatus: string | null): Promise<void> {
    this.applyCurrentUser(await this.rest.updatePresence(status, customStatus?.trim() || null));
  }

  /**
   * What this client shows for `userId`. The signed-in user sees their own pick, invisible as
   * offline, even before the instance echoes it.
   */
  presenceOf(userId: Snowflake): PresenceStatus {
    if (userId === this.user?.id) {
      const chosen = this.user.status ?? "online";
      return chosen === "invisible" ? "offline" : chosen;
    }
    return this.presenceByUserId.get(userId)?.status ?? "offline";
  }

  /** `null` for offline and invisible users, and for the signed-in user without one. */
  customStatusOf(userId: Snowflake): string | null {
    if (userId === this.user?.id) {
      return this.user.status === "invisible" ? null : (this.user.custom_status ?? null);
    }
    return this.presenceByUserId.get(userId)?.custom_status ?? null;
  }

  /** Signs out the other devices; this session stays. */
  changePassword(currentPassword: string, newPassword: string): Promise<void> {
    return this.rest.changePassword(currentPassword, newPassword);
  }

  /** Resized and compressed by the instance first: a large animated GIF takes a second or two. */
  async setProfileImage(kind: ProfileImageKind, file: File): Promise<void> {
    this.applyCurrentUser(await this.rest.setProfileImage(kind, file));
  }

  async removeProfileImage(kind: ProfileImageKind): Promise<void> {
    this.applyCurrentUser(await this.rest.removeProfileImage(kind));
  }

  /** The gateway sends the same `USER_UPDATE`; applying the answer shows the change at once. */
  private applyCurrentUser(user: CurrentUser): void {
    this.user = user;
    this.applyUserUpdate(user);
  }

  // --- Servers and conversations -------------------------------------------------------------

  /** Returns the new server, so its icon can follow. */
  async createServer(name: string, initialCategories?: readonly InitialCategory[]): Promise<Guild> {
    const guild = await this.rest.createGuild(name, initialCategories);
    this.upsertGuild(guild);
    this.selectGuild(guild.id, null);
    return guild;
  }

  async joinServer(code: string): Promise<void> {
    const guild = await this.rest.joinWithInvite(code);
    this.upsertGuild(guild);
    this.selectGuild(guild.id, null);
  }

  /** For any server, open or not: the strip's menu invites without opening it. */
  async createInvite(guildId: Snowflake, limits: InviteLimits): Promise<Invite> {
    return await this.rest.createInvite(guildId, limits);
  }

  /** A text channel opens once created; a category does not. */
  async createChannel(
    kind: ChannelKind,
    name: string,
    topic = "",
    categoryId: Snowflake | null = null,
  ): Promise<void> {
    const guildId = this.selectedGuildId;
    if (!guildId) {
      return;
    }
    const channel = await this.rest.createChannel(guildId, kind, name, topic, categoryId);
    this.applyEvent({ t: "CHANNEL_CREATE", data: channel }, Date.now());
    if (kind === "text" && this.selectedGuildId === guildId) {
      this.selectChannel(channel.id);
    }
  }

  /** Removed at once: a removal cannot bring older state back. */
  async deleteChannel(guildId: Snowflake, channelId: Snowflake): Promise<void> {
    await this.rest.deleteChannel(channelId);
    this.applyEvent({ t: "CHANNEL_DELETE", data: { guild_id: guildId, id: channelId } }, Date.now());
  }

  /**
   * The moves show when the gateway's `GUILD_CREATE` arrives (one snapshot per member rather than
   * an event per moved channel), as other server settings do.
   */
  async reorderChannels(guildId: Snowflake, placements: readonly ChannelPlacement[]): Promise<void> {
    await this.rest.reorderChannels(guildId, placements);
  }

  async leaveServer(guildId: Snowflake): Promise<void> {
    await this.rest.leaveGuild(guildId);
    this.removeGuild(guildId);
  }

  // --- Server settings ----------------------------------------------------------------------
  // Edits show when the gateway brings them, never from the REST answer: an answer arriving after
  // a newer event would put older state back. Only removals (which cannot bring anything back)
  // and a created role (which the page selects at once) are applied from the answer.

  async renameServer(guildId: Snowflake, name: string): Promise<void> {
    await this.rest.renameGuild(guildId, name);
  }

  /** Cropped and compressed by the instance first: a large animated GIF takes a second or two. */
  async setServerIcon(guildId: Snowflake, file: File, crop: IconCrop | null): Promise<void> {
    await this.rest.setGuildIcon(guildId, file, crop);
  }

  async removeServerIcon(guildId: Snowflake): Promise<void> {
    await this.rest.removeGuildIcon(guildId);
  }

  // --- Channel settings ---------------------------------------------------------------------

  async updateChannel(channelId: Snowflake, changes: { name?: string; topic?: string }): Promise<void> {
    await this.rest.updateChannel(channelId, changes);
  }

  async setChannelIcon(channelId: Snowflake, file: File, crop: IconCrop | null): Promise<void> {
    await this.rest.setChannelIcon(channelId, file, crop);
  }

  async removeChannelIcon(channelId: Snowflake): Promise<void> {
    await this.rest.removeChannelIcon(channelId);
  }

  async setPermissionOverwrite(channelId: Snowflake, overwrite: PermissionOverwrite): Promise<void> {
    await this.rest.setPermissionOverwrite(channelId, overwrite.id, {
      kind: overwrite.kind,
      allow: overwrite.allow,
      deny: overwrite.deny,
    });
  }

  async deletePermissionOverwrite(channelId: Snowflake, targetId: Snowflake): Promise<void> {
    await this.rest.deletePermissionOverwrite(channelId, targetId);
  }

  async deleteServer(guildId: Snowflake): Promise<void> {
    await this.rest.deleteGuild(guildId);
    this.removeGuild(guildId);
  }

  // --- Custom emoji (ADR-0028) --------------------------------------------------------------

  /**
   * Every custom emoji of the user's guilds by id: the pictures this user may load, so the ones a
   * message or reaction can show. Others read as their name.
   */
  readonly knownCustomEmojis: ReadonlyMap<Snowflake, CustomEmoji> = $derived.by(() => {
    const lists = this.guilds.map((guild) => guild.emojis);
    // Channel and role events replace the guild but keep its list: the same map then spares every
    // message row a new parse.
    const isUnchanged =
      lists.length === this.knownCustomEmojiLists.length &&
      lists.every((list, index) => list === this.knownCustomEmojiLists[index]);
    if (!isUnchanged) {
      this.knownCustomEmojiLists = lists;
      this.knownCustomEmojiMap = new Map(lists.flatMap((list) => (list ?? []).map((emoji) => [emoji.id, emoji] as const)));
    }
    return this.knownCustomEmojiMap;
  });
  /** The lists `knownCustomEmojis` was last built from; never reactive. */
  private knownCustomEmojiLists: readonly (CustomEmoji[] | undefined)[] = [];
  private knownCustomEmojiMap: ReadonlyMap<Snowflake, CustomEmoji> = new Map();

  /** The guild's custom emoji, oldest first; none from an older instance. */
  emojisOf(guildId: Snowflake): CustomEmoji[] {
    return this.guilds.find((guild) => guild.id === guildId)?.emojis ?? [];
  }

  private applyEmojis(guildId: Snowflake, emojis: CustomEmoji[]): void {
    this.applyEvent({ t: "GUILD_EMOJIS_UPDATE", data: { guild_id: guildId, emojis } }, Date.now());
  }

  /** Processed by the instance first: an animated GIF takes a second or two. */
  async createEmoji(guildId: Snowflake, name: string, file: File): Promise<CustomEmoji> {
    const created = await this.rest.createEmoji(guildId, name, file);
    const emojis = this.emojisOf(guildId);
    // The gateway's list may have come first.
    if (!emojis.some((emoji) => emoji.id === created.id)) {
      this.applyEmojis(guildId, [...emojis, created]);
    }
    return created;
  }

  /**
   * The new name shows with `GUILD_EMOJIS_UPDATE`, not with the answer: an answer arriving after
   * a later rename's event would put the older name back.
   */
  async renameEmoji(guildId: Snowflake, emojiId: Snowflake, name: string): Promise<void> {
    await this.rest.renameEmoji(guildId, emojiId, name);
  }

  async deleteEmoji(guildId: Snowflake, emojiId: Snowflake): Promise<void> {
    await this.rest.deleteEmoji(guildId, emojiId);
    this.applyEmojis(
      guildId,
      this.emojisOf(guildId).filter((emoji) => emoji.id !== emojiId),
    );
  }

  private applyRoles(guildId: Snowflake, roles: Role[]): void {
    this.applyEvent({ t: "GUILD_ROLES_UPDATE", data: { guild_id: guildId, roles } }, Date.now());
  }

  private rolesOf(guildId: Snowflake): Role[] {
    return this.guilds.find((guild) => guild.id === guildId)?.roles ?? [];
  }

  /** The new role sits just above `@everyone`, pushing the others up one, as the server does. */
  async createRole(guildId: Snowflake, name: string): Promise<Role> {
    const created = await this.rest.createRole(guildId, name);
    const roles = this.rolesOf(guildId);
    // The gateway's list may have come first; it already holds the new order then.
    if (!roles.some((role) => role.id === created.id)) {
      this.applyRoles(guildId, [
        ...roles.map((role) => (role.id === guildId ? role : { ...role, position: role.position + 1 })),
        created,
      ]);
    }
    return created;
  }

  async updateRole(
    guildId: Snowflake,
    roleId: Snowflake,
    changes: { name?: string; color?: number; permissions?: PermissionBits },
  ): Promise<void> {
    await this.rest.updateRole(guildId, roleId, changes);
  }

  async deleteRole(guildId: Snowflake, roleId: Snowflake): Promise<void> {
    await this.rest.deleteRole(guildId, roleId);
    this.applyRoles(
      guildId,
      this.rolesOf(guildId).filter((role) => role.id !== roleId),
    );
  }

  async reorderRoles(guildId: Snowflake, roleIds: readonly Snowflake[]): Promise<void> {
    await this.rest.reorderRoles(guildId, roleIds);
  }

  async setMemberRole(guildId: Snowflake, userId: Snowflake, roleId: Snowflake, isGiven: boolean): Promise<void> {
    await (isGiven
      ? this.rest.addMemberRole(guildId, userId, roleId)
      : this.rest.removeMemberRole(guildId, userId, roleId));
  }

  /** `banReason` is null for a kick, the (possibly empty) reason of a ban otherwise. */
  async removeMember(guildId: Snowflake, userId: Snowflake, banReason: string | null): Promise<void> {
    await (banReason === null
      ? this.rest.kickMember(guildId, userId)
      : this.rest.banMember(guildId, userId, banReason));
    this.applyEvent({ t: "GUILD_MEMBER_REMOVE", data: { guild_id: guildId, user_id: userId } }, Date.now());
  }

  async openDirectMessage(userId: Snowflake): Promise<void> {
    const conversation = await this.rest.openDirectMessage(userId);
    this.upsertConversation(conversation);
    this.selectGuild(null, conversation.id);
  }

  /**
   * The "New message" picker: one person opens the one-to-one conversation, several start a new
   * group (ADR-0026).
   */
  async startConversation(userIds: Snowflake[], groupName = ""): Promise<void> {
    const [onlyUserId] = userIds;
    if (userIds.length === 1 && onlyUserId !== undefined) {
      await this.openDirectMessage(onlyUserId);
      return;
    }
    const group = await this.rest.createGroupConversation(userIds, groupName.trim());
    this.upsertConversation(group);
    this.selectGuild(null, group.id);
  }

  async renameGroupConversation(channelId: Snowflake, name: string): Promise<void> {
    this.applyGroupAnswer(await this.rest.renameGroupConversation(channelId, name.trim()));
  }

  async addGroupParticipant(channelId: Snowflake, userId: Snowflake): Promise<void> {
    this.applyGroupAnswer(await this.rest.addGroupParticipant(channelId, userId));
  }

  /** The instance's answer about a group, unless the group left meanwhile (a delete event came first). */
  private applyGroupAnswer(group: DirectMessageChannel): void {
    if (this.conversations.some((channel) => channel.id === group.id)) {
      this.upsertConversation(group);
    }
  }

  /** The owner removes someone; removing oneself leaves (see `leaveGroupConversation`). */
  async removeGroupParticipant(channelId: Snowflake, userId: Snowflake): Promise<void> {
    await this.rest.removeGroupParticipant(channelId, userId);
    // At once, not on the gateway's update: the sheet moves focus to the next row when this one goes.
    const group = this.conversations.find((channel) => channel.id === channelId);
    if (group) {
      this.upsertConversation({ ...group, recipients: group.recipients.filter((recipient) => recipient.id !== userId) });
    }
  }

  async leaveGroupConversation(channelId: Snowflake): Promise<void> {
    const userId = this.user?.id;
    if (!userId) {
      return;
    }
    this.channelIdsBeingLeft.add(channelId);
    try {
      await this.rest.removeGroupParticipant(channelId, userId);
      this.dropConversation(channelId);
    } finally {
      this.channelIdsBeingLeft.delete(channelId);
    }
  }

  /** A conversation the user can no longer read: gone from the list, and left if open. */
  private dropConversation(channelId: Snowflake): void {
    const conversation = this.conversations.find((channel) => channel.id === channelId);
    this.conversations = this.conversations.filter((channel) => channel.id !== channelId);
    this.forgetChannel(channelId);
    if (this.selectedChannelId === channelId && this.selectedGuildId === null) {
      this.selectGuild(null, null);
      if (conversation) {
        this.closedConversation = {
          conversation,
          reason: this.channelIdsBeingLeft.has(channelId) ? "left" : "removed",
        };
      }
    }
  }

  private reportError(error: unknown, action: string): void {
    if (error instanceof ApiError && error.needsNewLogin) {
      this.endSession();
      return;
    }
    console.warn(`${action} failed`, error);
  }
}

function newestOf(left: Snowflake | null, right: Snowflake | null): Snowflake | null {
  if (!left || !right) {
    return left ?? right;
  }
  return compareSnowflakes(left, right) >= 0 ? left : right;
}

function sortByLastActivity(conversations: DirectMessageChannel[]): DirectMessageChannel[] {
  return [...conversations].sort((left, right) =>
    compareSnowflakes(right.last_message_id ?? right.id, left.last_message_id ?? left.id),
  );
}
