use serde::{Deserialize, Serialize};

use crate::{PublicUser, Snowflake};

/// Body of `POST /api/v1/channels/{channel_id}/messages`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendMessageRequest {
    pub content: String,
    /// The message this one answers; must be in the same channel.
    pub reply_to_id: Option<Snowflake>,
    /// Client-generated, echoed back so an offline outbox can match its pending copy
    /// (ARCHITECTURE.md §6). Not stored.
    pub nonce: Option<String>,
    /// Files uploaded to this channel beforehand by the sender (ADR-0020), at most 10. Left out of
    /// the body when empty, so older servers still accept text-only messages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachment_ids: Vec<Snowflake>,
    /// Forwards this message (ADR-0039): the server copies its content and files, so `content`
    /// and `attachment_ids` stay empty and `reply_to_id` absent. Left out of the body when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forwarded_from: Option<ForwardOrigin>,
    /// Sends a poll (ADR-0043): `content`, `attachment_ids`, `reply_to_id` and `forwarded_from`
    /// stay empty. Left out of the body when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll: Option<NewPoll>,
    /// Sends a checklist (ADR-0047), with nothing else of its own, as a poll. Left out of the
    /// body when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checklist: Option<NewChecklist>,
    /// Sends a progress bar (ADR-0047), with nothing else of its own, as a poll. Left out of the
    /// body when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_bar: Option<NewProgressBar>,
}

/// The checklist a message carries (ADR-0047), as its author writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewChecklist {
    /// 1 to `MAXIMUM_CHECKLIST_TITLE_LENGTH` characters.
    pub title: String,
    /// 1 to `MAXIMUM_CHECKLIST_ITEMS`, each 1 to `MAXIMUM_CHECKLIST_ITEM_LENGTH` characters.
    pub items: Vec<String>,
}

/// Body of `POST …/checklist/items` and `PATCH …/checklist/items/{item_id}` (ADR-0047).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChecklistItemRequest {
    /// 1 to `MAXIMUM_CHECKLIST_ITEM_LENGTH` characters.
    pub text: String,
}

pub const MAXIMUM_CHECKLIST_TITLE_LENGTH: usize = 300;
pub const MAXIMUM_CHECKLIST_ITEMS: usize = 25;
pub const MAXIMUM_CHECKLIST_ITEM_LENGTH: usize = 200;

/// A message's checklist (ADR-0047).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checklist {
    pub title: String,
    /// In the order they were added; an item keeps its id for life, and ids are never reused.
    pub items: Vec<ChecklistItem>,
    /// Grows with every change to the checklist: clients keep the highest they saw, since a
    /// `MESSAGE_UPDATE` or a history page may have read it before a later change.
    pub revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub id: u32,
    pub text: String,
    /// Who ticked it and when; absent while it is not ticked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked: Option<ChecklistTick>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecklistTick {
    pub user_id: Snowflake,
    pub at_unix_milliseconds: u64,
}

/// A progress bar a message carries (ADR-0047), as its author writes it or updates it (the body
/// of `PATCH …/progress-bar`): `value` out of `maximum`, shown as a percentage when `maximum` is
/// 100.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewProgressBar {
    /// 1 to `MAXIMUM_PROGRESS_BAR_LABEL_LENGTH` characters.
    pub label: String,
    /// 0 to `maximum`.
    pub value: u32,
    /// 1 to `MAXIMUM_PROGRESS_BAR_MAXIMUM`.
    pub maximum: u32,
}

pub const MAXIMUM_PROGRESS_BAR_LABEL_LENGTH: usize = 300;
pub const MAXIMUM_PROGRESS_BAR_MAXIMUM: u32 = 1_000_000;

/// A message's progress bar (ADR-0047).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgressBar {
    pub label: String,
    pub value: u32,
    pub maximum: u32,
    /// Grows with every update, as `Checklist::revision`.
    pub revision: u64,
}

/// The poll a message asks (ADR-0043), as its author writes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewPoll {
    /// 1 to `MAXIMUM_POLL_QUESTION_LENGTH` characters.
    pub question: String,
    /// `MINIMUM_POLL_ANSWERS` to `MAXIMUM_POLL_ANSWERS`, each 1 to `MAXIMUM_POLL_ANSWER_LENGTH`
    /// characters.
    pub answers: Vec<String>,
    #[serde(default)]
    pub allows_multiple_answers: bool,
    /// One of `POLL_DURATIONS_IN_HOURS`.
    pub duration_hours: u32,
}

pub const MAXIMUM_POLL_QUESTION_LENGTH: usize = 300;
pub const MINIMUM_POLL_ANSWERS: usize = 2;
pub const MAXIMUM_POLL_ANSWERS: usize = 10;
pub const MAXIMUM_POLL_ANSWER_LENGTH: usize = 55;
/// What a poll may last, as on Discord.
pub const POLL_DURATIONS_IN_HOURS: [u32; 7] = [1, 4, 8, 24, 72, 168, 336];

/// A message's poll as the reader sees it (ADR-0043).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Poll {
    pub question: String,
    /// In the author's order; ids are 1, 2, … in that order.
    pub answers: Vec<PollAnswer>,
    pub allows_multiple_answers: bool,
    /// Votes are refused from then on.
    pub expires_at_unix_milliseconds: u64,
    /// Set when the author ended the poll early.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at_unix_milliseconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PollAnswer {
    pub id: u32,
    pub text: String,
    pub vote_count: u32,
    /// The reader voted for it. Filled in REST answers only, like `ReactionSummary::me`; always
    /// false in events, which clients follow with `MESSAGE_POLL_VOTE_ADD` and `_REMOVE`.
    #[serde(default)]
    pub me_voted: bool,
}

/// Where a forwarded message was first written (ADR-0039). Its date is the message id's.
/// Unknown fields are allowed: it is also part of `Message`, which later servers may extend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardOrigin {
    pub channel_id: Snowflake,
    pub message_id: Snowflake,
}

/// A file shared in a message (ADR-0020), downloaded at
/// `GET /api/v1/attachments/{id}/{file_name}`. Also the answer to an upload,
/// `POST /api/v1/channels/{channel_id}/attachments?file_name=…`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: Snowflake,
    pub file_name: String,
    /// `image/png`, `image/jpeg`, `image/gif`, `image/webp` or `image/avif` when the bytes are one,
    /// which clients may preview; `application/octet-stream` for anything else. Servers may
    /// replace a picture with a smaller AVIF or WebP after upload (ADR-0033).
    pub content_type: String,
    pub size_bytes: u64,
    /// The picture's size in pixels as it shows, EXIF orientation applied, so clients can hold
    /// its place before it loads. Absent for anything else, for uploads made before servers
    /// recorded it, and from older servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
}

/// A preview of a link in a message, fetched by the server after the message was sent
/// (ADR-0041). Every text is plain: clients never read it as HTML or Markdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPreview {
    pub id: Snowflake,
    /// The link as fetched, without its fragment; it may differ from the message's text.
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site_name: Option<String>,
    /// The page's colour, `0xRRGGBB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_color: Option<u32>,
    /// Served by
    /// `GET /api/v1/channels/{channel_id}/messages/{message_id}/link-previews/{id}/image`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<LinkPreviewImage>,
    /// What a known provider says about the link, beyond the card (ADR-0044). The card's `title`,
    /// `description` and `image` still carry the title, the summary and the one picture (cover,
    /// avatar, poster or thumbnail). Absent for any other link and from older servers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embed: Option<LinkEmbed>,
}

/// A provider's facts about a link (ADR-0044), told apart by `kind`. Every id is the provider's,
/// checked by the server; clients check it again before building a player's address from it. A
/// kind this client does not know reads as `Unknown`, which clients ignore (they show the card).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum LinkEmbed {
    /// A YouTube video or Short.
    #[serde(rename = "youtube_video")]
    YouTubeVideo(YouTubeVideoEmbed),
    /// A YouTube Music track: `title` is the track's, `image` its square cover.
    #[serde(rename = "youtube_music_track")]
    YouTubeMusicTrack(YouTubeMusicTrackEmbed),
    /// A GitHub repository: `description` is the repository's, `image` the owner's avatar.
    #[serde(rename = "github_repository")]
    GitHubRepository(GitHubRepositoryEmbed),
    /// A GitHub issue: `title` is the issue's.
    #[serde(rename = "github_issue")]
    GitHubIssue(GitHubIssueEmbed),
    /// A GitHub pull request: `title` is the pull request's.
    #[serde(rename = "github_pull_request")]
    GitHubPullRequest(GitHubIssueEmbed),
    /// A track, album, playlist, episode or show of a music service: `image` is its cover.
    #[serde(rename = "music")]
    Music(MusicEmbed),
    #[serde(rename = "vimeo_video")]
    VimeoVideo(VimeoVideoEmbed),
    /// A Twitch channel, video or clip.
    #[serde(rename = "twitch")]
    Twitch(TwitchEmbed),
    /// A Wikipedia article: `description` is its summary, `image` its thumbnail.
    #[serde(rename = "wikipedia_article")]
    WikipediaArticle(WikipediaArticleEmbed),
    /// A kind from a newer server.
    #[serde(other, rename = "unknown")]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YouTubeVideoEmbed {
    pub video_id: String,
    /// A Short plays in a 9:16 frame.
    #[serde(default)]
    pub is_short: bool,
    /// Where the link starts the video, in whole seconds above zero.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_seconds: Option<u32>,
    /// A public playlist the link plays the video in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub playlist_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct YouTubeMusicTrackEmbed {
    /// Plays in the visible YouTube player (YouTube's terms forbid audio only).
    pub video_id: String,
    pub artist: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubRepositoryEmbed {
    pub owner: String,
    pub name: String,
    #[serde(default)]
    pub stars: u64,
    #[serde(default)]
    pub forks: u64,
    /// The main language, as GitHub names it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// The language's colour, `0xRRGGBB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language_color: Option<u32>,
    #[serde(default)]
    pub topics: Vec<String>,
    /// An SPDX licence id, such as `MIT` or `AGPL-3.0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub licence_spdx: Option<String>,
    #[serde(default)]
    pub is_archived: bool,
}

/// An issue or a pull request (the variant says which).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitHubIssueEmbed {
    pub owner: String,
    pub repository: String,
    pub number: u64,
    pub state: GitHubIssueState,
    /// The author's login.
    pub author: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opened_at_unix_milliseconds: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_at_unix_milliseconds: Option<u64>,
    #[serde(default)]
    pub comments: u32,
}

/// `Draft` and `Merged` for pull requests, `Completed` and `NotPlanned` for closed issues.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitHubIssueState {
    Open,
    Draft,
    Merged,
    Completed,
    NotPlanned,
    Closed,
    /// A state from a newer server.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MusicEmbed {
    pub provider: MusicProvider,
    pub item: MusicItem,
    /// The provider's id of the item, which its player's address is built from (Bandcamp's is
    /// numeric).
    pub id: String,
    /// Apple Music's storefront, two lowercase letters (`fr`), which its player's address needs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub storefront: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artist: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_seconds: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MusicProvider {
    Spotify,
    Deezer,
    #[serde(rename = "soundcloud")]
    SoundCloud,
    AppleMusic,
    Bandcamp,
    /// A provider from a newer server.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MusicItem {
    Track,
    Album,
    Playlist,
    Episode,
    Show,
    Artist,
    /// An item from a newer server.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VimeoVideoEmbed {
    pub video_id: String,
    /// The privacy hash of an unlisted video (`h=`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TwitchEmbed {
    pub item: TwitchItem,
    /// The channel's login, the video's number or the clip's slug.
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TwitchItem {
    Channel,
    Video,
    Clip,
    /// An item from a newer server.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WikipediaArticleEmbed {
    /// The wiki's language code, `fr` for `fr.wikipedia.org`.
    pub language: String,
    #[serde(default)]
    pub is_disambiguation: bool,
}

/// A link preview's picture, as stored: its size in pixels, so clients hold its place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkPreviewImage {
    pub width: u32,
    pub height: u32,
}

/// Body of `PATCH /api/v1/channels/{channel_id}/messages/{message_id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditMessageRequest {
    pub content: String,
}

/// Query of `GET /api/v1/channels/{channel_id}/messages`. With neither cursor, the newest
/// messages are returned. Results are always in chronological order (oldest first).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageHistoryQuery {
    /// Messages strictly older than this id.
    pub before: Option<Snowflake>,
    /// Messages strictly newer than this id.
    pub after: Option<Snowflake>,
    /// 1 to 100; defaults to 50.
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// Also encodes the creation time (Snowflake, ADR-0004).
    pub id: Snowflake,
    pub channel_id: Snowflake,
    pub author: PublicUser,
    pub content: String,
    /// May point to a message that has since been deleted ("Original message was deleted").
    pub reply_to_id: Option<Snowflake>,
    pub edited_at_unix_milliseconds: Option<u64>,
    /// Echo of the sender's nonce, only to the sender: in the response to their send and in the
    /// `MESSAGE_CREATE` sent to their own sessions, never to anyone else. Absent from older
    /// servers' events.
    #[serde(default)]
    pub nonce: Option<String>,
    /// Absent from older servers' payloads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    /// In the order they were first added. Filled in REST answers, where `me` is the requester;
    /// always empty in gateway events (`MESSAGE_UPDATE` included), since `me` differs per
    /// viewer: clients keep the reactions they hold and follow `MESSAGE_REACTION_ADD` and
    /// `MESSAGE_REACTION_REMOVE`. Absent when empty and from older servers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<ReactionSummary>,
    /// Pinned to its channel (`GET /api/v1/channels/{channel_id}/pins`). Absent from older
    /// servers' payloads.
    #[serde(default)]
    pub pinned: bool,
    /// The people the server found mentioned (`@username`, or a one-word display name, outside
    /// code) who can view the channel; never the author. Absent when empty and from older
    /// servers' payloads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mention_ids: Vec<Snowflake>,
    /// The roles the message mentions (`<@&id>`), and the guild's id for `@everyone`, when the
    /// author may mention them (ADR-0040); each viewer holding one is mentioned. Never in
    /// direct messages or forwards. Absent when empty and from older servers' payloads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mention_role_ids: Vec<Snowflake>,
    /// Previews of the message's links, in their order, once the server fetched them
    /// (ADR-0041); `MESSAGE_LINK_PREVIEWS_UPDATE` brings them to open clients. Absent when empty
    /// and from older servers' payloads.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub link_previews: Vec<LinkPreview>,
    /// The message's poll (ADR-0043). Absent otherwise and from older servers' payloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll: Option<Poll>,
    /// The message's checklist (ADR-0047). Absent otherwise and from older servers' payloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checklist: Option<Checklist>,
    /// The message's progress bar (ADR-0047). Absent otherwise and from older servers' payloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress_bar: Option<ProgressBar>,
    /// Set on a forward (ADR-0039): its content and files are a copy of that message's, taken
    /// when it was forwarded. Absent otherwise and from older servers' payloads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forwarded_from: Option<ForwardOrigin>,
}

/// One emoji's reactions on a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReactionSummary {
    /// The stored key: a Unicode emoji without variation selectors (`"👍"`, `"❤"`), or a custom
    /// emoji's id after a colon (`":501"`, ADR-0028).
    pub emoji: String,
    pub count: u32,
    /// Whether the requesting user is among them.
    pub me: bool,
}

/// Query of `GET /api/v1/channels/{channel_id}/messages/{message_id}/reactions/{emoji}`: who
/// reacted, by ascending user id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReactionUsersQuery {
    /// Users whose id is strictly greater than this one.
    pub after: Option<Snowflake>,
    /// 1 to 100; defaults to 25.
    pub limit: Option<u32>,
}

/// Body of `PUT /api/v1/channels/{channel_id}/read-state`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeRequest {
    pub last_read_message_id: Snowflake,
}

/// Entry of `GET /api/v1/users/@me/read-states`, for unread markers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadState {
    pub channel_id: Snowflake,
    pub last_read_message_id: Snowflake,
}

/// Where a channel the user can view stands, in `Ready`: its newest message and how many unread
/// messages mention the user (in a direct-message conversation: how many unread messages from
/// others), counted up to `MAXIMUM_COUNTED_UNREAD_MENTIONS`. Only for channels with a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelUnread {
    pub channel_id: Snowflake,
    pub last_message_id: Snowflake,
    pub mention_count: u32,
}

/// Counting stops here, so a long unread history costs a bounded read; clients show "99+".
pub const MAXIMUM_COUNTED_UNREAD_MENTIONS: u32 = 100;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_query_and_message_wire_format() {
        let query: MessageHistoryQuery =
            serde_json::from_str(r#"{"before":"42","limit":20}"#).unwrap();
        assert_eq!(query.before, Some(Snowflake::from_raw(42)));
        assert_eq!(query.after, None);

        let message = Message {
            id: Snowflake::from_raw(9),
            channel_id: Snowflake::from_raw(3),
            author: PublicUser {
                id: Snowflake::from_raw(1),
                username: "aurore".into(),
                display_name: "Aurore".into(),
                avatar: None,
                banner: None,
            },
            content: "hello".into(),
            reply_to_id: None,
            edited_at_unix_milliseconds: None,
            nonce: Some("abc".into()),
            attachments: Vec::new(),
            reactions: vec![ReactionSummary {
                emoji: "👍".into(),
                count: 2,
                me: true,
            }],
            pinned: true,
            mention_ids: Vec::new(),
            mention_role_ids: Vec::new(),
            link_previews: Vec::new(),
            poll: None,
            checklist: None,
            progress_bar: None,
            forwarded_from: Some(ForwardOrigin {
                channel_id: Snowflake::from_raw(7),
                message_id: Snowflake::from_raw(8),
            }),
        };
        let json = serde_json::to_value(&message).unwrap();
        assert_eq!(json["id"], "9");
        assert_eq!(json["author"]["username"], "aurore");
        assert_eq!(json["nonce"], "abc");
        assert_eq!(
            json["reactions"],
            serde_json::json!([{"emoji": "👍", "count": 2, "me": true}])
        );
        assert_eq!(json["pinned"], true);
        assert_eq!(
            json["forwarded_from"],
            serde_json::json!({"channel_id": "7", "message_id": "8"})
        );
    }

    #[test]
    fn messages_from_older_servers_have_no_reactions_and_are_not_pinned() {
        let message: Message = serde_json::from_str(
            r#"{"id":"9","channel_id":"3","author":{"id":"1","username":"a","display_name":"A"},"content":"hi","reply_to_id":null,"edited_at_unix_milliseconds":null,"nonce":null}"#,
        )
        .unwrap();
        assert_eq!(message.reactions, Vec::new());
        assert!(!message.pinned);
        assert_eq!(message.forwarded_from, None);
    }

    #[test]
    fn attachment_dimensions_are_optional_both_ways() {
        let picture = Attachment {
            id: Snowflake::from_raw(5),
            file_name: "photo.jpg".into(),
            content_type: "image/jpeg".into(),
            size_bytes: 1_234,
            width: Some(3_000),
            height: Some(4_000),
        };
        let json = serde_json::to_value(&picture).unwrap();
        assert_eq!(json["width"], 3_000);
        assert_eq!(json["height"], 4_000);
        assert_eq!(serde_json::from_value::<Attachment>(json).unwrap(), picture);

        let file = Attachment {
            content_type: "application/octet-stream".into(),
            width: None,
            height: None,
            ..picture
        };
        let json = serde_json::to_value(&file).unwrap();
        assert!(json.get("width").is_none() && json.get("height").is_none());
        let from_older_server: Attachment = serde_json::from_str(
            r#"{"id":"5","file_name":"photo.jpg","content_type":"application/octet-stream","size_bytes":1234}"#,
        )
        .unwrap();
        assert_eq!(from_older_server, file);
    }

    #[test]
    fn embeds_are_tagged_by_kind_and_round_trip() {
        let embed = LinkEmbed::YouTubeVideo(YouTubeVideoEmbed {
            video_id: "dQw4w9WgXcQ".into(),
            is_short: false,
            start_seconds: Some(90),
            playlist_id: None,
            duration_seconds: None,
        });
        let json = serde_json::to_value(&embed).unwrap();
        assert_eq!(
            json,
            serde_json::json!({"kind": "youtube_video", "video_id": "dQw4w9WgXcQ",
                "is_short": false, "start_seconds": 90})
        );
        assert_eq!(serde_json::from_value::<LinkEmbed>(json).unwrap(), embed);

        let pull_request = LinkEmbed::GitHubPullRequest(GitHubIssueEmbed {
            owner: "rust-lang".into(),
            repository: "rust".into(),
            number: 12,
            state: GitHubIssueState::NotPlanned,
            author: "ada".into(),
            opened_at_unix_milliseconds: Some(1),
            merged_at_unix_milliseconds: None,
            comments: 3,
        });
        let json = serde_json::to_value(&pull_request).unwrap();
        assert_eq!(json["kind"], "github_pull_request");
        assert_eq!(json["state"], "not_planned");
        assert_eq!(
            serde_json::from_value::<LinkEmbed>(json).unwrap(),
            pull_request
        );
        let music = serde_json::to_value(LinkEmbed::Music(MusicEmbed {
            provider: MusicProvider::SoundCloud,
            item: MusicItem::Track,
            id: "1".into(),
            storefront: None,
            artist: None,
            track_count: None,
            duration_seconds: None,
        }))
        .unwrap();
        assert_eq!(music["provider"], "soundcloud");
    }

    #[test]
    fn kinds_and_values_from_a_newer_server_do_not_break_the_preview() {
        let preview: LinkPreview = serde_json::from_str(
            r#"{"id":"1","url":"https://example.org/","embed":{"kind":"hologram","depth":3}}"#,
        )
        .unwrap();
        assert_eq!(preview.embed, Some(LinkEmbed::Unknown));

        let music: LinkEmbed = serde_json::from_str(
            r#"{"kind":"music","provider":"tidal","item":"mixtape","id":"9","extra":true}"#,
        )
        .unwrap();
        let LinkEmbed::Music(music) = music else {
            panic!("{music:?}");
        };
        assert_eq!(
            (music.provider, music.item),
            (MusicProvider::Unknown, MusicItem::Unknown)
        );

        let older: LinkPreview =
            serde_json::from_str(r#"{"id":"1","url":"https://example.org/"}"#).unwrap();
        assert_eq!(older.embed, None);
        assert!(serde_json::to_value(&older).unwrap().get("embed").is_none());
    }
}
