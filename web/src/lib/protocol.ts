// Wire types of `mellow-protocol` (REST `/api/v1`, gateway `/gateway?v=1`).
// Written by hand from the Rust crate; protocol-wire-samples.generated.ts checks them against it
// (ADR-0019): after changing a wire type, regenerate the samples and fix what `npm run check` reports.

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

/**
 * A guild's custom emoji (ADR-0028). Messages write it `<:name:id>` (`<a:name:id>` when animated),
 * reactions key it `:id`; members load its picture with `emojiUrl`.
 */
export interface CustomEmoji {
  id: Snowflake;
  /** 2 to 32 ASCII letters, digits and `_`, unique in its guild whatever the case. */
  name: string;
  is_animated: boolean;
}

/** Mirrors the server's rules (mellow-server domain/guild_rules.rs); the server still decides. */
export const MAXIMUM_EMOJIS_PER_GUILD_AND_KIND = 50;
export const MAXIMUM_EMOJI_UPLOAD_BYTES = 256 * 1024;
export const MINIMUM_EMOJI_NAME_LENGTH = 2;
export const MAXIMUM_EMOJI_NAME_LENGTH = 32;
const EMOJI_NAME = new RegExp(`^[A-Za-z0-9_]{${MINIMUM_EMOJI_NAME_LENGTH},${MAXIMUM_EMOJI_NAME_LENGTH}}$`);
/** A reaction key `:id`; the id without leading zero, as the instance writes it. */
const CUSTOM_EMOJI_REACTION_KEY = /^:(0|[1-9][0-9]{0,19})$/;

export function isValidEmojiName(name: string): boolean {
  return EMOJI_NAME.test(name);
}

/** A custom emoji's picture, served like an icon: animated only when asked for and there is one. */
export function emojiUrl(emoji: CustomEmoji, isAnimationWanted: boolean): string {
  return iconUrl(emoji, isAnimationWanted);
}

/** What `PUT …/reactions/{emoji}/@me` takes to react with a custom emoji. */
export function customEmojiReactionKey(emoji: CustomEmoji): string {
  return `:${emoji.id}`;
}

/**
 * The emoji id of a custom reaction key, `null` for a Unicode emoji. Its name and picture come
 * from `guild.emojis`; an id the guild no longer has is a deleted emoji (show a generic chip).
 */
export function customEmojiOfReactionKey(key: string): Snowflake | null {
  return CUSTOM_EMOJI_REACTION_KEY.exec(key)?.[1] ?? null;
}

/** How a message's text carries a custom emoji (Discord's syntax). */
export function customEmojiMarkup(emoji: CustomEmoji): string {
  return `<${emoji.is_animated ? "a" : ""}:${emoji.name}:${emoji.id}>`;
}

/**
 * A guild's soundboard sound (ADR-0032). Its samples, a canonical PCM WAV (mono, 48 kHz, 16-bit),
 * are served at `soundUrl` to the guild's members; every client plays it on its own output.
 */
export interface SoundboardSound {
  id: Snowflake;
  /** 2 to 32 characters once trimmed, not unique. */
  name: string;
  /** A reaction key (ADR-0028): a Unicode emoji, or `:id` of one of the guild's custom emoji. */
  emoji?: string | null;
  duration_milliseconds: number;
}

/** Mirrors the server's rules (ADR-0032); the server still decides. */
export const MAXIMUM_SOUNDS_PER_GUILD = 48;
export const MINIMUM_SOUND_NAME_LENGTH = 2;
export const MAXIMUM_SOUND_NAME_LENGTH = 32;
/** The only format the instance stores: mono 16-bit PCM at this rate, at most this many samples (5.2 s). */
export const SOUND_SAMPLE_RATE = 48_000;
export const MAXIMUM_SOUND_SAMPLES = 249_600;
/** The shortest time between two sounds played by one member (`MINIMUM_MILLISECONDS_BETWEEN_SOUNDS`). */
export const MINIMUM_MILLISECONDS_BETWEEN_SOUNDS = 2_000;

/** A sound's WAV, for members of its server only. */
export function soundUrl(soundId: Snowflake): string {
  return `/api/v1/sounds/${soundId}`;
}

/** Mirrors the server's rules (mellow-server domain/guild_rules.rs); the server still decides. */
export const MAXIMUM_CHANNEL_NAME_LENGTH = 100;
export const MAXIMUM_CHANNEL_TOPIC_LENGTH = 1024;

export interface SessionCreated {
  token: string;
  user: CurrentUser;
}

/** A voice channel is joined, never opened: it has no messages (ADR-0030). */
export type ChannelKind = "category" | "text" | "voice";

/** A voice channel's optional user limit, and the hard cap per channel (ADR-0030). */
export const MINIMUM_VOICE_USER_LIMIT = 1;
export const MAXIMUM_VOICE_USER_LIMIT = 25;
/** Receive m-lines each client offers: the server forwards at most this many voices at once. */
export const VOICE_RECEIVE_SLOTS = 16;
/** Video receive m-lines a client able to show video offers: the most cameras it watches (ADR-0035). */
export const VIDEO_RECEIVE_SLOTS = 9;
/** Audio receive m-lines after the video ones: the most shared screens heard at once (ADR-0037). */
export const SCREEN_SOUND_RECEIVE_SLOTS = 3;

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
  /** Voice channels only: at most this many people at once; absent means unlimited. */
  user_limit?: number | null;
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
  /** Its members are listed under its name, above everyone else, in the member list. */
  is_displayed_separately: boolean;
  /**
   * Anyone may mention it (`<@&id>`); otherwise only holders of `MENTION_EVERYONE` in the channel
   * (ADR-0040). Never set on `@everyone`. Absent from older servers.
   */
  is_mentionable?: boolean;
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
  /** Oldest first. Absent from older servers. */
  emojis?: CustomEmoji[];
  /** Soundboard sounds, oldest first (ADR-0032). Absent from older servers. */
  sounds?: SoundboardSound[];
  /** The sound this member plays on joining a voice channel here. Absent when none. */
  my_join_sound_id?: Snowflake | null;
}

export interface Member {
  user: PublicUser;
  role_ids: Snowflake[];
  joined_at_unix_milliseconds: number;
}

export interface Attachment {
  id: Snowflake;
  file_name: string;
  /** One of the previewable image or playable media types, or `application/octet-stream`. */
  content_type: string;
  size_bytes: number;
  /** As the picture shows (EXIF orientation applied), to hold its place while it loads. Absent for other files, older uploads and older servers. */
  width?: number;
  height?: number;
}

/** Pictures the instance reads: avatars, banners, icons and emoji must be one of these. */
export const PICTURE_UPLOAD_TYPES: readonly string[] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/** Shown inline: the instance serves only these as images (ADR-0020); it stores most compressed pictures as AVIF (ADR-0033). */
export const PREVIEWABLE_IMAGE_TYPES: readonly string[] = [...PICTURE_UPLOAD_TYPES, "image/avif"];

/** Played inline (ADR-0038): the instance serves only these, sniffed from their bytes, as media with byte ranges. */
export const PLAYABLE_VIDEO_TYPES: readonly string[] = ["video/mp4", "video/quicktime", "video/webm"];
export const PLAYABLE_AUDIO_TYPES: readonly string[] = ["audio/mpeg", "audio/aac", "audio/mp4", "audio/ogg", "audio/wav", "audio/flac"];

/**
 * Why a chosen file cannot become an avatar or banner, checked before sending it; null lets the
 * instance decide. An empty type (unknown to the browser) is left to the instance too.
 */
export function profileImageFileProblem(file: Pick<Blob, "size" | "type">): "not-a-picture" | "too-large" | null {
  if (file.type !== "" && !PICTURE_UPLOAD_TYPES.includes(file.type)) {
    return "not-a-picture";
  }
  return file.size > MAXIMUM_PROFILE_IMAGE_BYTES ? "too-large" : null;
}

/** The name is not in the URL (the instance sends it in `Content-Disposition`). */
export function attachmentUrl(attachment: Attachment): string {
  return `/api/v1/attachments/${attachment.id}`;
}

/**
 * The smaller copy of a picture for the message list, sized for its box; the instance answers
 * with the file itself when the picture needs none.
 */
export function attachmentPreviewUrl(attachment: Attachment): string {
  return `${attachmentUrl(attachment)}/preview`;
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
  /**
   * The roles mentioned (`<@&id>`), and the guild's id for `@everyone`, as far as the author may
   * mention them (ADR-0040); a viewer holding one is mentioned. Absent when empty and from older
   * servers.
   */
  mention_role_ids?: Snowflake[];
  /**
   * Previews of the message's links, in order, once the server fetched them (ADR-0041);
   * `MESSAGE_LINK_PREVIEWS_UPDATE` brings them later. Absent when empty and from older servers.
   */
  link_previews?: LinkPreview[];
  /** Set on a forward (ADR-0039): its content and files copy that message's. */
  forwarded_from?: ForwardOrigin;
  /** The message's poll (ADR-0043); its content is then empty. */
  poll?: Poll;
}

/** A message's poll (ADR-0043). Ended once `expires_at_unix_milliseconds` passed or `ended_at` is set. */
export interface Poll {
  question: string;
  /** In the author's order; ids 1, 2, … */
  answers: PollAnswer[];
  allows_multiple_answers: boolean;
  expires_at_unix_milliseconds: number;
  ended_at_unix_milliseconds?: number;
}

export interface PollAnswer {
  id: number;
  text: string;
  vote_count: number;
  /** In REST answers only; events always say false (follow the vote events). */
  me_voted: boolean;
}

/** What a poll is sent with (`SendMessageRequest.poll`). */
export interface NewPoll {
  question: string;
  answers: string[];
  allows_multiple_answers: boolean;
  /** One of `POLL_DURATIONS_IN_HOURS`. */
  duration_hours: number;
}

export const MAXIMUM_POLL_QUESTION_LENGTH = 300;
export const MINIMUM_POLL_ANSWERS = 2;
export const MAXIMUM_POLL_ANSWERS = 10;
export const MAXIMUM_POLL_ANSWER_LENGTH = 55;
export const POLL_DURATIONS_IN_HOURS = [1, 4, 8, 24, 72, 168, 336] as const;

/** `MESSAGE_POLL_VOTE_ADD` and `MESSAGE_POLL_VOTE_REMOVE`. */
export interface PollVote {
  guild_id?: Snowflake;
  channel_id: Snowflake;
  message_id: Snowflake;
  answer_id: number;
  user_id: Snowflake;
}

/**
 * `GET /api/v1/gifs/settings` (ADR-0042), `404` when the instance has no GIFs: what the browser
 * needs to call Klipy itself.
 */
export interface GifSettings {
  klipy_app_key: string;
  /** Opaque and stable per member; never their id. */
  customer_id: string;
}

/** A member keeps at most this many starred GIFs (`/api/v1/users/@me/gif-favourites`). */
export const MAXIMUM_GIF_FAVOURITES = 200;

/**
 * A link's preview, fetched by the server (ADR-0041). Every text is plain: never render it as
 * HTML or Markdown. The picture is served at
 * `/api/v1/channels/{channel_id}/messages/{message_id}/link-previews/{id}/image`.
 */
export interface LinkPreview {
  id: Snowflake;
  /** The link as fetched, without its fragment. */
  url: string;
  title?: string;
  description?: string;
  site_name?: string;
  /** `0xRRGGBB`. */
  theme_color?: number;
  /** Its size in pixels as stored. */
  image?: { width: number; height: number };
  /**
   * What a known provider says about the link (ADR-0044); `title`, `description` and `image` keep
   * the title, the summary and the one picture. Absent for other links and from older servers.
   */
  embed?: LinkEmbed | UnknownLinkEmbed;
}

/**
 * An embed from a newer server, of a kind this client does not know: never narrowed to a
 * `LinkEmbed` by its `kind`, so readers go through `knownLinkEmbedOf` and show the plain card.
 */
export interface UnknownLinkEmbed {
  kind: string & {};
}

/**
 * A provider's facts about a link (ADR-0044), told apart by `kind`. A newer server may send kinds
 * and values this list lacks: read `embed` only through `knownLinkEmbedOf`. Re-check every id
 * before building a player's address from it.
 */
export type LinkEmbed =
  | ({ kind: "youtube_video" } & YouTubeVideoEmbed)
  | ({ kind: "youtube_music_track" } & YouTubeMusicTrackEmbed)
  | ({ kind: "github_repository" } & GitHubRepositoryEmbed)
  | ({ kind: "github_issue" } & GitHubIssueEmbed)
  | ({ kind: "github_pull_request" } & GitHubIssueEmbed)
  | ({ kind: "music" } & MusicEmbed)
  | ({ kind: "vimeo_video" } & VimeoVideoEmbed)
  | ({ kind: "twitch" } & TwitchEmbed)
  | ({ kind: "wikipedia_article" } & WikipediaArticleEmbed);

export interface YouTubeVideoEmbed {
  video_id: string;
  /** Plays in a 9:16 frame. */
  is_short: boolean;
  /** Whole seconds above zero. */
  start_seconds?: number;
  playlist_id?: string;
  duration_seconds?: number;
}

/** `title` is the track's, `image` its square cover; plays in the visible YouTube player. */
export interface YouTubeMusicTrackEmbed {
  video_id: string;
  artist: string;
  album?: string;
  duration_seconds?: number;
}

/** `description` is the repository's, `image` the owner's avatar. */
export interface GitHubRepositoryEmbed {
  owner: string;
  name: string;
  stars: number;
  forks: number;
  language?: string;
  /** `0xRRGGBB`. */
  language_color?: number;
  topics: string[];
  /** An SPDX id, such as `MIT`. */
  licence_spdx?: string;
  is_archived: boolean;
}

/** An issue or a pull request; `title` is its title. */
export interface GitHubIssueEmbed {
  owner: string;
  repository: string;
  number: number;
  state: GitHubIssueState;
  /** The author's login. */
  author: string;
  opened_at_unix_milliseconds?: number;
  merged_at_unix_milliseconds?: number;
  comments: number;
}

/** `draft` and `merged` for pull requests, `completed` and `not_planned` for closed issues. */
export type GitHubIssueState = "open" | "draft" | "merged" | "completed" | "not_planned" | "closed";

/** `image` is the cover. */
export interface MusicEmbed {
  provider: MusicProvider;
  item: MusicItem;
  /** The provider's id of the item (Bandcamp's is numeric). */
  id: string;
  /** Apple Music's storefront, two lowercase letters (`fr`), which its player's address needs. */
  storefront?: string;
  artist?: string;
  track_count?: number;
  duration_seconds?: number;
}

export type MusicProvider = "spotify" | "deezer" | "soundcloud" | "apple_music" | "bandcamp";

export type MusicItem = "track" | "album" | "playlist" | "episode" | "show" | "artist";

export interface VimeoVideoEmbed {
  video_id: string;
  /** The privacy hash of an unlisted video. */
  hash?: string;
}

export interface TwitchEmbed {
  item: TwitchItem;
  /** The channel's login, the video's number or the clip's slug. */
  id: string;
}

export type TwitchItem = "channel" | "video" | "clip";

/** `description` is the summary, `image` the thumbnail. */
export interface WikipediaArticleEmbed {
  /** `fr` for `fr.wikipedia.org`. */
  language: string;
  is_disambiguation: boolean;
}

/** Where a forwarded message was first written (ADR-0039); its date is the message id's. */
export interface ForwardOrigin {
  channel_id: Snowflake;
  message_id: Snowflake;
}

/** Body of `POST /channels/{id}/messages`. */
export interface SendMessageRequest {
  content: string;
  reply_to_id?: Snowflake | null;
  nonce?: string | null;
  /** Left out when empty, so older servers still accept text-only messages. */
  attachment_ids?: Snowflake[];
  /**
   * Forwards that message (ADR-0039): the instance copies its content and files, so `content`
   * stays empty, without `attachment_ids` or `reply_to_id`.
   */
  forwarded_from?: ForwardOrigin;
  /** Sends a poll (ADR-0043), with empty `content` and nothing else. */
  poll?: NewPoll;
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

/** Results per page at most, and by default (ADR-0029). */
export const MAXIMUM_SEARCH_RESULTS_PER_PAGE = 25;
/** Pages reach this many results at most (`offset + limit`): past it, refine the query. */
export const MAXIMUM_REACHABLE_SEARCH_RESULTS = 1_000;
/**
 * Longer queries are refused (`VALIDATION_FAILED`), as are more than 16 words and filters. Each user
 * runs one search at a time: a second one before the first answers gets `RATE_LIMITED`.
 */
export const MAXIMUM_SEARCH_QUERY_LENGTH = 256;
export const MAXIMUM_SEARCH_QUERY_PARTS = 16;

/** `relevance` (the default) ranks best matches first; `newest` ignores relevance. */
export type MessageSearchSort = "relevance" | "newest";

/**
 * Body of `POST /guilds/{id}/messages/search` and `POST /users/@me/messages/search`. `q` is the
 * query language: words (all must match, each also as a word prefix), `"exact phrase"`,
 * `-excluded`, `prefix*`, and the filters `from:username`, `mentions:username`, `in:channel` (an
 * id, or in a server a name with or without `#`), `has:file|image|link|pinned`, and
 * `before:`/`after:`/`during:YYYY-MM-DD`. Unknown filters are searched as words; case and
 * accents are ignored.
 */
export interface MessageSearchQuery {
  q: string;
  sort?: MessageSearchSort;
  offset?: number;
  limit?: number;
  /** Minutes east of UTC, so day filters are the user's days. */
  utc_offset_minutes?: number;
}

/** A span of `Message.content` in UTF-16 code units: `content.slice(start, end)`. */
export interface HighlightRange {
  start: number;
  end: number;
}

export interface MessageSearchHit {
  /** As in history pages; blocked authors' messages are included, to fold like in the channel. */
  message: Message;
  /** Matched words, in order and never overlapping; empty for a query of filters only. */
  highlights: HighlightRange[];
}

export interface MessageSearchResults {
  hits: MessageSearchHit[];
  total_count: number;
  total_count_is_lower_bound: boolean;
  /** Matches per channel or conversation, largest first, at most 10. */
  channel_facets: { channel_id: Snowflake; count: number }[];
  /** Matches per author, largest first, at most 10. */
  author_facets: { author: PublicUser; count: number }[];
}

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
  /** `target_id` is the emoji; its name is the `name` change's new value. */
  | "emoji_create"
  /** A rename: the `name` change. */
  | "emoji_update"
  /** `target_id` is the emoji; its name is the `name` change's old value. */
  | "emoji_delete"
  /** `target_id` is the sound; its name is the `name` change's new value. */
  | "sound_create"
  /** `name` and `emoji` changes (the emoji's reaction key, empty for none). */
  | "sound_update"
  /** `target_id` is the sound; its name is the `name` change's old value. */
  | "sound_delete"
  | "unknown";

export interface AuditChange {
  key:
    | "name"
    | "topic"
    | "color"
    | "permissions"
    | "allow"
    | "deny"
    | "code"
    | "icon"
    | "parent"
    | "user_limit"
    /** A sound's emoji: its reaction key, empty for none. */
    | "emoji"
    /** A role's "display separately": `true` or `false`. */
    | "displayed_separately"
    /** A role's "allow anyone to mention": `true` or `false`. */
    | "mentionable"
    | "unknown";
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
  /** Who is connected to which voice channel, in every guild the user is in. Absent from older servers. */
  voice_states?: VoiceState[];
  /** Voice is on and its UDP port is open on this instance; absent (false) on older servers. */
  voice_enabled?: boolean;
  /** The largest attachment the instance accepts, in bytes; absent from older servers (added after v0.45.0). */
  maximum_attachment_bytes?: number;
}

/** One person connected to a voice channel; a user has at most one (ADR-0030). */
export interface VoiceState {
  user_id: Snowflake;
  guild_id: Snowflake;
  channel_id: Snowflake;
  self_mute: boolean;
  self_deaf: boolean;
  /** Without `SPEAK` in the channel: the server does not forward their voice. */
  suppressed: boolean;
  /**
   * The sound clients connected here play instead of the join chime when the person joins or moves
   * in (ADR-0032). Absent when none, and from older servers.
   */
  join_sound_id?: Snowflake | null;
  /** Their camera is on and forwarded (ADR-0035). Absent from older servers: off. */
  self_video?: boolean;
  /** They share a screen, relayed like a camera (ADR-0037); never with `self_video`. Absent from older servers. */
  self_stream?: boolean;
}

/** Why the server refused to turn this session's camera on. A newer server may send another reason. */
export type VoiceVideoRefusal = "missing_permissions" | "too_many_cameras" | "video_unavailable";

/** How large a camera's tile is shown, which decides the layer it receives (ADR-0035). */
export type VideoTileSize = "large" | "medium" | "small";

/** A camera's simulcast layer, by its RID: 1280 × 720, 640 × 360, 320 × 180. */
export type SimulcastLayer = "h" | "l" | "q";

/** Why the server refused a `voice_join`. A newer server may send another reason. */
export type VoiceJoinRefusal =
  | "missing_permissions"
  | "channel_full"
  | "instance_full"
  | "invalid_offer"
  | "voice_unavailable"
  | "rate_limited";

/**
 * Why the server ended this session's voice connection. `moved_elsewhere`: the same user joined
 * from another tab or device.
 */
export type VoiceDisconnectReason =
  | "moved_elsewhere"
  | "permissions_changed"
  | "channel_deleted"
  | "removed_from_guild"
  | "media_timeout";

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
  | {
      t: "MESSAGE_LINK_PREVIEWS_UPDATE";
      data: {
        guild_id?: Snowflake;
        channel_id: Snowflake;
        message_id: Snowflake;
        link_previews: LinkPreview[];
      };
    }
  | { t: "MESSAGE_POLL_VOTE_ADD"; data: PollVote }
  | { t: "MESSAGE_POLL_VOTE_REMOVE"; data: PollVote }
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
  /** The guild's whole emoji list after any was added, renamed or deleted. */
  | { t: "GUILD_EMOJIS_UPDATE"; data: { guild_id: Snowflake; emojis: CustomEmoji[] } }
  /** The guild's whole sound list after any was added, edited or deleted. */
  | { t: "GUILD_SOUNDS_UPDATE"; data: { guild_id: Snowflake; sounds: SoundboardSound[] } }
  /** Only to the user's own sessions: their join sound in that guild changed, or was deleted. */
  | { t: "MY_JOIN_SOUND_UPDATE"; data: { guild_id: Snowflake; sound_id: Snowflake | null } }
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
  | { t: "RELATIONSHIP_REMOVE"; data: { user_id: Snowflake } }
  /** Someone joined, moved, or changed their mute or deafen. */
  | { t: "VOICE_STATE_UPDATE"; data: { voice_state: VoiceState } }
  | { t: "VOICE_STATE_DELETE"; data: { guild_id: Snowflake; channel_id: Snowflake; user_id: Snowflake } }
  /** Only to the session that sent the `voice_join`. */
  | { t: "VOICE_SESSION_DESCRIPTION"; data: { channel_id: Snowflake; offer_id: number; sdp_answer: string } }
  | { t: "VOICE_JOIN_REFUSED"; data: { channel_id: Snowflake; offer_id: number; reason: VoiceJoinRefusal } }
  | { t: "VOICE_DISCONNECTED"; data: { channel_id: Snowflake; reason: VoiceDisconnectReason } }
  /** Only to that session: its camera stays off (ADR-0035). */
  | { t: "VOICE_VIDEO_REFUSED"; data: { channel_id: Snowflake; reason: VoiceVideoRefusal } }
  /**
   * Only to the camera's session: the layers someone watches, lowest first; pause the others.
   * Always holds "q" while the camera is on; empty once the server turned it off.
   */
  | { t: "VOICE_VIDEO_LAYERS"; data: { channel_id: Snowflake; active: SimulcastLayer[] } }
  /**
   * Only to the watcher's session: `user_id`'s camera is paused for this connection, too slow
   * for its lowest layer, or flows again (ADR-0035). A tile starts as not paused.
   */
  | { t: "VOICE_VIDEO_PAUSED"; data: { channel_id: Snowflake; user_id: Snowflake; is_paused: boolean } }
  /**
   * Only to the listener's session: the receive slot whose transceiver has `mid` now carries
   * `user_id`'s voice, or camera for a video transceiver, until another event for the same `mid`.
   * Valid for the current answer only.
   */
  | { t: "VOICE_SLOT_ASSIGNED"; data: { channel_id: Snowflake; mid: string; user_id: Snowflake } }
  /** Edges only, never for muted or suppressed people. */
  | {
      t: "SPEAKING";
      data: { guild_id: Snowflake; channel_id: Snowflake; user_id: Snowflake; speaking: boolean };
    }
  /** To every session connected to the channel, the player's own included: each plays it locally. */
  | { t: "VOICE_SOUND_PLAYED"; data: { channel_id: Snowflake; user_id: Snowflake; sound_id: Snowflake } };

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
  | { op: "heartbeat" }
  /**
   * Join, move, or reconnect with a fresh offer: the server swaps the media session. `offer_id`
   * (0 to 2^32 - 1, chosen by the client) comes back with the answer or the refusal.
   */
  | {
      op: "voice_join";
      d: { channel_id: Snowflake; offer_id: number; sdp_offer: string; self_mute: boolean; self_deaf: boolean };
    }
  | { op: "voice_leave" }
  /** `self_stream` (a screen, ADR-0037) wins over `self_video`. */
  | { op: "voice_state"; d: { self_mute: boolean; self_deaf: boolean; self_video?: boolean; self_stream?: boolean } }
  /**
   * The cameras shown and how large, at most `VIDEO_RECEIVE_SLOTS`; replaces the previous list.
   * A `voice_join` forgets it: send it again after `VOICE_SESSION_DESCRIPTION`.
   */
  | { op: "voice_video_watch"; d: { cameras: { user_id: Snowflake; size: VideoTileSize }[] } }
  /** Ignored by the server unless allowed (ADR-0032): no answer but `VOICE_SOUND_PLAYED`. */
  | { op: "voice_play_sound"; d: { sound_id: Snowflake } };

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
  /** Reserved for nicknames, which no client offers yet; roles may already hold them. */
  CHANGE_NICKNAME: 1n << 13n,
  MANAGE_NICKNAMES: 1n << 14n,
  VIEW_AUDIT_LOG: 1n << 15n,
  ADD_REACTIONS: 1n << 16n,
  /** Add, edit and delete the server's custom emoji and soundboard sounds (named `MANAGE_EMOJIS` before sounds). */
  MANAGE_EXPRESSIONS: 1n << 17n,
  /** Join a voice channel (`VIEW_CHANNEL` too). */
  CONNECT: 1n << 18n,
  /** Be heard in a voice channel. */
  SPEAK: 1n << 19n,
  /** Play the server's sounds in a voice channel, and choose a join sound (ADR-0032). */
  USE_SOUNDBOARD: 1n << 20n,
  /** Turn a camera on in a voice channel, later share a screen; "Video" in the interface (ADR-0035). */
  STREAM: 1n << 21n,
  /** Send a poll, with `SEND_MESSAGES`; "Create polls" in the interface (ADR-0043). */
  SEND_POLLS: 1n << 22n,
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
  Permission.ADD_REACTIONS |
  Permission.CONNECT |
  Permission.SPEAK |
  Permission.USE_SOUNDBOARD |
  Permission.STREAM |
  Permission.SEND_POLLS;

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
