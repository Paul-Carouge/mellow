// What the web client knows and does: the same model as `mellow-client-core` (state reducer,
// outbox, history pages), kept in Svelte runes so components re-render from it.

import { SvelteMap, SvelteSet } from "svelte/reactivity";

import { ApiError, HISTORY_PAGE_SIZE, RestClient, generateMessageNonce } from "./api";
import { GatewayConnection, type GatewayUpdate } from "./gateway";
import { channelSections } from "./channels";
import {
  type Channel,
  type CurrentUser,
  type DirectMessageChannel,
  type Guild,
  type GatewayEvent,
  type Member,
  type Message,
  type PermissionBits,
  type Ready,
  type Snowflake,
  DIRECT_MESSAGE_PERMISSIONS,
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
  status: "sending" | "failed";
}

/** Text shown to people: built from the error code, never from the instance's own message. */
export function describeError(error: unknown, summary: string): string {
  if (!(error instanceof ApiError)) {
    return `${summary}: something went wrong. Try again.`;
  }
  const reasons: Partial<Record<ApiError["code"], string>> = {
    NETWORK: "the instance could not be reached. Try again.",
    INVALID_CREDENTIALS: "the login or the password is wrong.",
    USERNAME_TAKEN: "this username is taken.",
    EMAIL_TAKEN: "this email already has an account.",
    REGISTRATION_CLOSED: "this instance does not accept new accounts.",
    VALIDATION_FAILED: "the instance refused what was entered.",
    RATE_LIMITED: "too many attempts. Wait a moment and try again.",
    MISSING_PERMISSIONS: "you don't have the permission to do that here.",
    NOT_FOUND: "it no longer exists, or you can't see it.",
    INVALID_INVITE: "this invite is unknown, expired or used up.",
    BANNED: "you are banned from this server.",
    OWNER_CANNOT_LEAVE: "the owner cannot leave their own server.",
    LIMIT_REACHED: "a limit of this instance is reached.",
  };
  return `${summary}: ${reasons[error.code] ?? "something went wrong. Try again."}`;
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
  onlineUserIds = new SvelteSet<Snowflake>();
  lastReadByChannel = new SvelteMap<Snowflake, Snowflake>();
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

  private gateway: GatewayConnection | null = null;
  /** Before the first `Ready`, no selection exists yet: home then means "not chosen". */
  private hasReceivedReady = false;

  /** `rest` is injected by tests. */
  constructor(private readonly rest: RestClient = new RestClient()) {}
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
    const newest = [cached, known]
      .filter((id): id is Snowflake => id !== null)
      .sort(compareSnowflakes)
      .at(-1);
    if (!newest) {
      return false;
    }
    const lastRead = this.lastReadByChannel.get(channelId);
    return !lastRead || compareSnowflakes(newest, lastRead) > 0;
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
        this.signOutError = describeError(error, "Couldn't sign out");
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
    this.onlineUserIds.clear();
    this.lastReadByChannel.clear();
    this.typingUntil.clear();
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
    this.reachedStartChannelIds.clear();
    this.membersByGuild.clear();
    this.requestedHistory.clear();
    this.requestedMembers.clear();
    this.failedHistoryChannelIds.clear();
    this.failedMemberGuildIds.clear();
    this.onlineUserIds.clear();
    for (const userId of ready.online_user_ids) {
      this.onlineUserIds.add(userId);
    }
    this.lastReadByChannel.clear();
    for (const state of ready.read_states) {
      this.lastReadByChannel.set(state.channel_id, state.last_read_message_id);
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
        this.insertMessages(message.channel_id, [message]);
        if (message.channel_id === this.selectedChannelId) {
          void this.acknowledgeSelectedChannel();
        }
        return;
      }
      case "MESSAGE_UPDATE": {
        const messages = this.messagesByChannel.get(event.data.channel_id);
        if (!messages || !this.isKnownChannel(event.data.channel_id)) {
          return;
        }
        // A late REST answer must not undo a newer edit the gateway already brought.
        const updated = event.data;
        this.messagesByChannel.set(
          updated.channel_id,
          messages.map((message) =>
            message.id === updated.id &&
            (message.edited_at_unix_milliseconds ?? 0) <= (updated.edited_at_unix_milliseconds ?? 0)
              ? updated
              : message,
          ),
        );
        return;
      }
      case "MESSAGE_DELETE": {
        const messages = this.messagesByChannel.get(event.data.channel_id);
        if (messages && this.isKnownChannel(event.data.channel_id)) {
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
      case "GUILD_ROLES_UPDATE":
        this.updateGuild(event.data.guild_id, (guild) => ({ ...guild, roles: event.data.roles }));
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
        if (event.data.status === "online") {
          this.onlineUserIds.add(event.data.user_id);
        } else {
          this.onlineUserIds.delete(event.data.user_id);
        }
        return;
      case "USER_UPDATE":
        this.renameUser(event.data.id, event.data.username, event.data.display_name);
        return;
      case "DIRECT_MESSAGE_CHANNEL_CREATE":
        this.upsertConversation(event.data);
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

  private updateGuild(guildId: Snowflake, change: (guild: Guild) => Guild): void {
    this.guilds = this.guilds.map((guild) => (guild.id === guildId ? change(guild) : guild));
  }

  /** A channel gone or hidden from us: its cache, its loads and its unsent messages go too. */
  private forgetChannel(channelId: Snowflake): void {
    this.messagesByChannel.delete(channelId);
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

  private renameUser(userId: Snowflake, username: string, displayName: string): void {
    const rename = <T extends { id: Snowflake; username: string; display_name: string }>(user: T): T =>
      user.id === userId ? { ...user, username, display_name: displayName } : user;
    if (this.user?.id === userId) {
      this.user = rename(this.user);
    }
    for (const [guildId, members] of this.membersByGuild) {
      this.membersByGuild.set(
        guildId,
        members.map((member) => ({ ...member, user: rename(member.user) })),
      );
    }
    for (const [channelId, messages] of this.messagesByChannel) {
      this.messagesByChannel.set(
        channelId,
        messages.map((message) => ({ ...message, author: rename(message.author) })),
      );
    }
    this.conversations = this.conversations.map((channel) => ({
      ...channel,
      recipients: channel.recipients.map(rename),
    }));
  }

  /** Merges by id, oldest first; outside the open channel the cache keeps the newest messages. */
  private insertMessages(channelId: Snowflake, incoming: Message[]): void {
    const byId = new Map(this.messages(channelId).map((message) => [message.id, message]));
    for (const message of incoming) {
      byId.set(message.id, message);
    }
    let merged = [...byId.values()].sort((left, right) => compareSnowflakes(left.id, right.id));
    if (channelId !== this.selectedChannelId && merged.length > MAXIMUM_CACHED_MESSAGES_PER_CHANNEL) {
      merged = merged.slice(-MAXIMUM_CACHED_MESSAGES_PER_CHANNEL);
      this.reachedStartChannelIds.delete(channelId);
    }
    this.messagesByChannel.set(channelId, merged);
  }

  // --- Navigation ----------------------------------------------------------------------------

  /** `guildId` null is home: it opens the given conversation, or the most recent one. */
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
      channelId =
        this.conversations.find((channel) => channel.id === preferredChannelId)?.id ??
        this.conversations[0]?.id ??
        null;
    }
    this.selectChannel(channelId);
  }

  selectChannel(channelId: Snowflake | null): void {
    const previousChannelId = this.selectedChannelId;
    if (previousChannelId && previousChannelId !== channelId) {
      this.trimHistory(previousChannelId);
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
    try {
      const page = await this.rest.listMessages(channelId);
      this.insertMessages(channelId, page);
      if (page.length < HISTORY_PAGE_SIZE) {
        this.reachedStartChannelIds.add(channelId);
      }
      if (channelId === this.selectedChannelId) {
        void this.acknowledgeSelectedChannel();
      }
    } catch (error) {
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
    try {
      const page = await this.rest.listMessages(channelId, oldest.id);
      this.insertMessages(channelId, page);
      if (page.length < HISTORY_PAGE_SIZE) {
        this.reachedStartChannelIds.add(channelId);
      }
    } catch (error) {
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
    const lastRead = this.lastReadByChannel.get(channelId);
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

  // --- Messages (outbox) ---------------------------------------------------------------------

  pendingMessagesIn(channelId: Snowflake): PendingMessage[] {
    return this.pendingMessages.filter((pending) => pending.channelId === channelId);
  }

  async sendMessage(content: string, replyToId: Snowflake | null = null): Promise<void> {
    const channelId = this.selectedChannelId;
    const text = content.trim();
    if (!channelId || !text) {
      return;
    }
    const pending: PendingMessage = {
      nonce: generateMessageNonce(),
      channelId,
      content: text,
      replyToId,
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
      const message = await this.rest.sendMessage(pending.channelId, pending.content, pending.nonce, pending.replyToId);
      this.pendingMessages = this.pendingMessages.filter((candidate) => candidate.nonce !== pending.nonce);
      this.noteConversationActivity(message.channel_id, message.id);
      this.insertMessages(message.channel_id, [{ ...message, nonce: null }]);
      return "sent";
    } catch (error) {
      this.setPendingStatus(pending.nonce, "failed");
      this.reportError(error, "send message");
      return error instanceof ApiError && REFUSED_FOR_GOOD.has(error.code) ? "refused" : "failed";
    }
  }

  private setPendingStatus(nonce: string, status: PendingMessage["status"]): void {
    this.pendingMessages = this.pendingMessages.map((pending) =>
      pending.nonce === nonce ? { ...pending, status } : pending,
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

  async editMessage(messageId: Snowflake, content: string): Promise<void> {
    const channelId = this.selectedChannelId;
    if (!channelId) {
      return;
    }
    const updated = await this.rest.editMessage(channelId, messageId, content);
    this.applyEvent({ t: "MESSAGE_UPDATE", data: updated }, Date.now());
  }

  async deleteMessage(messageId: Snowflake): Promise<void> {
    const channelId = this.selectedChannelId;
    if (!channelId) {
      return;
    }
    await this.rest.deleteMessage(channelId, messageId);
    this.applyEvent({ t: "MESSAGE_DELETE", data: { channel_id: channelId, id: messageId } }, Date.now());
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

  /** Signs out the other devices; this session stays. */
  changePassword(currentPassword: string, newPassword: string): Promise<void> {
    return this.rest.changePassword(currentPassword, newPassword);
  }

  /** The gateway sends the same `USER_UPDATE`; applying the answer shows the change at once. */
  private applyCurrentUser(user: CurrentUser): void {
    this.user = user;
    this.renameUser(user.id, user.username, user.display_name);
  }

  // --- Servers and conversations -------------------------------------------------------------

  async createServer(name: string): Promise<void> {
    const guild = await this.rest.createGuild(name);
    this.upsertGuild(guild);
    this.selectGuild(guild.id, null);
  }

  async joinServer(code: string): Promise<void> {
    const guild = await this.rest.joinWithInvite(code);
    this.upsertGuild(guild);
    this.selectGuild(guild.id, null);
  }

  async createInvite(): Promise<string> {
    const guildId = this.selectedGuildId;
    if (!guildId) {
      throw new ApiError(404, "NOT_FOUND");
    }
    return (await this.rest.createInvite(guildId)).code;
  }

  async createChannel(name: string): Promise<void> {
    const guildId = this.selectedGuildId;
    if (!guildId) {
      return;
    }
    const channel = await this.rest.createTextChannel(guildId, name);
    this.applyEvent({ t: "CHANNEL_CREATE", data: channel }, Date.now());
    if (this.selectedGuildId === guildId) {
      this.selectChannel(channel.id);
    }
  }

  async leaveServer(): Promise<void> {
    const guildId = this.selectedGuildId;
    if (!guildId) {
      return;
    }
    await this.rest.leaveGuild(guildId);
    this.removeGuild(guildId);
  }

  async openDirectMessage(userId: Snowflake): Promise<void> {
    const conversation = await this.rest.openDirectMessage(userId);
    this.upsertConversation(conversation);
    this.selectGuild(null, conversation.id);
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
