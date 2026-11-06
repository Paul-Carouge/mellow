//! Who is told about a message, how, and with what preview (`web/src/components/notifications.ts`
//! and `web/src/lib/reader-mentions.ts`, docs/design/web-notifications.md). Pure: the notifier
//! gathers the facts and does the telling; the catalogue words titles and bodies.

use std::collections::HashMap;

use mellow_protocol::{ChosenStatus, Message, Role, Snowflake};

use super::markdown::{
    BlockNode, InlineNode, MarkdownSources, RoleMentionContext, parse_message_markdown,
    with_custom_emoji_as_names,
};
use super::mentions::MentionablePeople;
use super::notification_settings::{
    MuteDuration, MuteEnd, NotificationSettings, NotifyLevel, PlaceNotificationSettings,
    is_mute_active, mute_end_for,
};
use super::role_mentions::role_mention_label;
use super::unicode_classes::{is_js_whitespace, js_trim, js_trim_end};

/// A message's place: a server's channel, or a conversation (no server).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationPlace {
    pub guild_id: Option<Snowflake>,
    pub channel_id: Snowflake,
}

/// What a mute or a "Notify me about" choice applies to (the menus and their dialog).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationTarget {
    Server {
        guild_id: Snowflake,
    },
    Channel {
        guild_id: Snowflake,
        channel_id: Snowflake,
    },
    Conversation {
        channel_id: Snowflake,
    },
}

/// The key its choices are stored under.
#[must_use]
pub fn notification_target_id(target: NotificationTarget) -> Snowflake {
    match target {
        NotificationTarget::Server { guild_id } => guild_id,
        NotificationTarget::Channel { channel_id, .. }
        | NotificationTarget::Conversation { channel_id } => channel_id,
    }
}

/// As on Discord: a server tells about mentions only, a conversation about every message.
#[must_use]
pub fn default_notify_level(guild_id: Option<Snowflake>) -> NotifyLevel {
    if guild_id.is_none() {
        NotifyLevel::All
    } else {
        NotifyLevel::Mentions
    }
}

/// What a server's channels follow when they have no choice of their own.
#[must_use]
pub fn server_notify_level(settings: &NotificationSettings, guild_id: Snowflake) -> NotifyLevel {
    settings
        .places
        .get(&guild_id)
        .and_then(|place| place.level)
        .unwrap_or_else(|| default_notify_level(Some(guild_id)))
}

/// The channel's own choice, else its server's, else the default.
#[must_use]
pub fn notify_level_of(settings: &NotificationSettings, place: NotificationPlace) -> NotifyLevel {
    if let Some(own) = settings
        .places
        .get(&place.channel_id)
        .and_then(|own| own.level)
    {
        return own;
    }
    place.guild_id.map_or_else(
        || default_notify_level(None),
        |guild_id| server_notify_level(settings, guild_id),
    )
}

// --- The dialog's choices --------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MuteChoice {
    NotMuted,
    /// The timed mute running now, kept as it is.
    Current,
    For(MuteDuration),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LevelChoice {
    /// "Same as the server", for a channel.
    SameAsServer,
    Level(NotifyLevel),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationTargetKind {
    Server,
    Channel,
    Conversation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationDialogChoices {
    pub target_kind: NotificationTargetKind,
    pub chosen_mute: MuteChoice,
    pub chosen_level: LevelChoice,
    /// The mute running when the dialog opened, if any.
    pub running_mute_end: Option<MuteEnd>,
    /// What applies without a choice of its own: the server's for a channel, else the default.
    pub fallback_level: NotifyLevel,
    pub now: u64,
}

/// What Save stores for the place. Keeping a timed mute keeps its end (it does not restart), and
/// a level equal to the default stays implicit, so a later change of default or of the server's
/// choice still applies; "Same as the server" stores no level. A channel's explicit level is kept
/// even when it matches the server's, since the reader chose it over following the server.
#[must_use]
pub fn place_settings_from_dialog(choices: NotificationDialogChoices) -> PlaceNotificationSettings {
    let muted_until = match choices.chosen_mute {
        MuteChoice::NotMuted => None,
        MuteChoice::Current => choices.running_mute_end,
        MuteChoice::For(duration) => Some(mute_end_for(duration, choices.now)),
    };
    let picked_level = match choices.chosen_level {
        LevelChoice::SameAsServer => None,
        LevelChoice::Level(level) => Some(level),
    };
    let is_channel = choices.target_kind == NotificationTargetKind::Channel;
    let level = if is_channel || picked_level != Some(choices.fallback_level) {
        picked_level
    } else {
        None
    };
    PlaceNotificationSettings { level, muted_until }
}

/// Muted itself, or in a muted server.
#[must_use]
pub fn is_place_muted(settings: &NotificationSettings, place: NotificationPlace, now: u64) -> bool {
    let muted_until = |id: Snowflake| settings.places.get(&id).and_then(|place| place.muted_until);
    is_mute_active(muted_until(place.channel_id), now)
        || place
            .guild_id
            .is_some_and(|guild_id| is_mute_active(muted_until(guild_id), now))
}

/// Who reads, and where: `guild_id` is `None` and `role_ids` empty in a direct conversation.
#[derive(Debug, Clone, Copy)]
pub struct MentionReader<'a> {
    pub user_id: Snowflake,
    pub guild_id: Option<Snowflake>,
    pub role_ids: &'a [Snowflake],
}

/// The server decided who a message mentions: `mention_ids` names people, `mention_role_ids` the
/// roles the author could mention and the guild's id for `@everyone`. The reader is mentioned by
/// name, by a role they hold now, or by `@everyone` in their guild; never by their own message.
#[must_use]
pub fn does_message_mention_reader(message: &Message, reader: MentionReader<'_>) -> bool {
    if message.author.id == reader.user_id {
        return false;
    }
    if message.mention_ids.contains(&reader.user_id) {
        return true;
    }
    let role_ids = &message.mention_role_ids;
    reader.guild_id.is_some_and(|guild_id| {
        !role_ids.is_empty()
            && (role_ids.contains(&guild_id)
                || reader
                    .role_ids
                    .iter()
                    .any(|role_id| role_ids.contains(role_id)))
    })
}

#[derive(Debug, Clone, Copy)]
pub struct IncomingMessageFacts<'a> {
    pub message: &'a Message,
    pub current_user_id: Option<Snowflake>,
    /// The reader's roles in the place's server: a role or `@everyone` mention counts (ADR-0040).
    pub reader_role_ids: &'a [Snowflake],
    pub place: NotificationPlace,
    pub is_author_blocked: bool,
    pub chosen_status: Option<ChosenStatus>,
    pub settings: &'a NotificationSettings,
    pub now: u64,
}

/// Whether a new message should reach the reader at all: someone else's, not blocked, not in
/// Do not disturb, and allowed by the place's level; a muted place lets only mentions through.
#[must_use]
pub fn should_alert_about(facts: IncomingMessageFacts<'_>) -> bool {
    let Some(current_user_id) = facts.current_user_id else {
        return false;
    };
    if facts.message.author.id == current_user_id
        || facts.is_author_blocked
        || facts.chosen_status == Some(ChosenStatus::DoNotDisturb)
    {
        return false;
    }
    let mentions_reader = does_message_mention_reader(
        facts.message,
        MentionReader {
            user_id: current_user_id,
            guild_id: facts.place.guild_id,
            role_ids: facts.reader_role_ids,
        },
    );
    match notify_level_of(facts.settings, facts.place) {
        NotifyLevel::Nothing => false,
        NotifyLevel::Mentions => mentions_reader,
        // "Direct mentions are never muted" (DESIGN.md §5.2).
        NotifyLevel::All => {
            mentions_reader || !is_place_muted(facts.settings, facts.place, facts.now)
        }
    }
}

/// Where the reader's eyes are, as far as the app can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReaderAttention {
    pub is_window_visible: bool,
    pub has_window_focus: bool,
    pub open_channel_id: Option<Snowflake>,
}

/// How an alert reaches the reader: not at all when they are looking at that channel, in the
/// app (Melli and the sound) while they use Mellow elsewhere, by the system when they are away.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertDelivery {
    None,
    InApp,
    System,
}

#[must_use]
pub fn alert_delivery_for(channel_id: Snowflake, reader: ReaderAttention) -> AlertDelivery {
    if !reader.is_window_visible || !reader.has_window_focus {
        return AlertDelivery::System;
    }
    if reader.open_channel_id == Some(channel_id) {
        AlertDelivery::None
    } else {
        AlertDelivery::InApp
    }
}

// --- Previews --------------------------------------------------------------------------------

/// A preview never runs longer than this many characters, ellipsis included.
pub const MAXIMUM_PREVIEW_CHARACTERS: usize = 120;
const ELLIPSIS: char = '…';

/// The catalogue's words a preview holds.
#[derive(Debug, Clone, Copy)]
pub struct PreviewWords<'a> {
    /// What a spoiler reads as: hidden in the chat until clicked, never on the desktop.
    pub spoiler: &'a str,
    /// A role this client does not know, « rôle-supprimé ».
    pub deleted_role_name: &'a str,
}

/// A message's content as plain text: no Markdown marks, spoilers hidden, emoji as `:name:`,
/// roles as `@Name` among `roles`. Polls and trackers have their own summary instead.
// The roles feed `RoleMentionContext`, which holds the standard map the client's state keeps.
#[allow(clippy::implicit_hasher)]
#[must_use]
pub fn markdown_as_plain_text(
    content: &str,
    roles: &HashMap<Snowflake, Role>,
    words: PreviewWords<'_>,
) -> String {
    let people = MentionablePeople::new();
    let custom_emojis = HashMap::new();
    let sources = MarkdownSources {
        people: &people,
        custom_emojis: &custom_emojis,
        role_mentions: RoleMentionContext {
            roles,
            is_everyone_mentioned: false,
        },
    };
    blocks_as_text(&parse_message_markdown(content, sources), words)
}

/// What a notification or a list shows for a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessagePreview {
    /// One line, at most [`MAXIMUM_PREVIEW_CHARACTERS`].
    Text(String),
    /// No text but files: "Sent 2 files".
    SentFiles(usize),
    Empty,
}

/// `text` (from [`markdown_as_plain_text`], or a poll's or tracker's summary) as one line, cut
/// with an ellipsis past [`MAXIMUM_PREVIEW_CHARACTERS`]; the files when there is no text.
///
/// shortcut: the web counts graphemes (`Intl.Segmenter`); this counts characters, so an emoji
/// sequence may be cut inside. Count graphemes once a segmenter is in the dependency tree.
#[must_use]
pub fn message_preview(text: &str, attachment_count: usize) -> MessagePreview {
    let one_line = text
        .split(is_js_whitespace)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let one_line = js_trim(&one_line);
    if one_line.is_empty() {
        return if attachment_count > 0 {
            MessagePreview::SentFiles(attachment_count)
        } else {
            MessagePreview::Empty
        };
    }
    if one_line.chars().count() <= MAXIMUM_PREVIEW_CHARACTERS {
        return MessagePreview::Text(one_line.to_owned());
    }
    let kept: String = one_line
        .chars()
        .take(MAXIMUM_PREVIEW_CHARACTERS - 1)
        .collect();
    MessagePreview::Text(format!("{}{ELLIPSIS}", js_trim_end(&kept)))
}

fn blocks_as_text(blocks: &[BlockNode], words: PreviewWords<'_>) -> String {
    blocks
        .iter()
        .map(|block| match block {
            BlockNode::Paragraph(children)
            | BlockNode::Heading { children, .. }
            | BlockNode::Subtext(children) => inlines_as_text(children, words),
            BlockNode::Quote(children) => blocks_as_text(children, words),
            BlockNode::List(list) => list_as_text(list, words),
            BlockNode::CodeBlock { text, .. } => text.clone(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn list_as_text(list: &super::markdown::MarkdownList, words: PreviewWords<'_>) -> String {
    list.items
        .iter()
        .map(|item| {
            let sublists: Vec<String> = item
                .sublists
                .iter()
                .map(|sublist| list_as_text(sublist, words))
                .collect();
            format!(
                "{} {}",
                inlines_as_text(&item.children, words),
                sublists.join(" ")
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn inlines_as_text(nodes: &[InlineNode], words: PreviewWords<'_>) -> String {
    nodes
        .iter()
        .map(|node| match node {
            InlineNode::Text(text)
            | InlineNode::Mention { text, .. }
            | InlineNode::EveryoneMention(text)
            | InlineNode::Match(text) => with_custom_emoji_as_names(text),
            InlineNode::RoleMention(role) => {
                role_mention_label(role.as_ref(), words.deleted_role_name)
            }
            InlineNode::Code(text) => text.clone(),
            InlineNode::CustomEmoji(emoji) => format!(":{}:", emoji.name),
            InlineNode::Spoiler { .. } => words.spoiler.to_owned(),
            InlineNode::Strong(children)
            | InlineNode::Emphasis(children)
            | InlineNode::Underline(children)
            | InlineNode::Strikethrough(children)
            | InlineNode::Link { children, .. } => inlines_as_text(children, words),
        })
        .collect()
}

// --- What the notification says --------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationTitle {
    /// The person for a one-to-one conversation, the group's name, or "#general · Rust Lyon".
    Place,
    /// "Ana, 3 new messages": alerts in this channel since the reader last looked.
    PlaceWithCount(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationBody {
    /// "New message from Ana": previews hidden, or nothing to preview.
    NewMessageFrom,
    /// The preview alone, in a one-to-one conversation.
    Preview,
    /// "Ana: see you at 7?"
    AuthorSaid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationContentFacts {
    pub is_one_to_one_conversation: bool,
    pub has_preview: bool,
    /// Alerts in this channel since the reader last looked, this one included.
    pub message_count: u32,
    pub should_show_preview: bool,
}

#[must_use]
pub fn notification_content(
    facts: NotificationContentFacts,
) -> (NotificationTitle, NotificationBody) {
    let title = if facts.message_count > 1 {
        NotificationTitle::PlaceWithCount(facts.message_count)
    } else {
        NotificationTitle::Place
    };
    let body = if !facts.should_show_preview || !facts.has_preview {
        NotificationBody::NewMessageFrom
    } else if facts.is_one_to_one_conversation {
        NotificationBody::Preview
    } else {
        NotificationBody::AuthorSaid
    };
    (title, body)
}

/// Beyond this, the title says "99+".
const MAXIMUM_SHOWN_COUNT: u32 = 99;

/// The window's title: "(3) Mellow" while something waits for the reader.
#[must_use]
pub fn window_title(attention_count: u32, app_name: &str) -> String {
    match attention_count {
        0 => app_name.to_owned(),
        count if count > MAXIMUM_SHOWN_COUNT => format!("({MAXIMUM_SHOWN_COUNT}+) {app_name}"),
        count => format!("({count}) {app_name}"),
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::Permissions;
    use serde_json::json;

    use super::super::notification_settings::NotificationSettings;
    use super::*;

    const ME: u64 = 1;
    const ANA: u64 = 2;
    const SERVER: u64 = 100;
    const GENERAL: u64 = 101;
    const CONVERSATION: u64 = 200;
    const NOW: u64 = 1_800_000_000_000;
    const WORDS: PreviewWords<'static> = PreviewWords {
        spoiler: "spoiler",
        deleted_role_name: "deleted-role",
    };

    fn id(raw: u64) -> Snowflake {
        Snowflake::from_raw(raw)
    }

    fn message(author_id: u64, mention_ids: &[u64], mention_role_ids: &[u64]) -> Message {
        let ids = |list: &[u64]| list.iter().map(ToString::to_string).collect::<Vec<_>>();
        serde_json::from_value(json!({
            "id": "5000", "channel_id": GENERAL.to_string(), "content": "hello", "reply_to_id": null,
            "edited_at_unix_milliseconds": null,
            "author": { "id": author_id.to_string(), "username": "ana", "display_name": "Ana" },
            "mention_ids": ids(mention_ids), "mention_role_ids": ids(mention_role_ids),
        }))
        .expect("a valid message")
    }

    fn settings_with(places: &[(u64, PlaceNotificationSettings)]) -> NotificationSettings {
        NotificationSettings {
            places: places
                .iter()
                .map(|(place, settings)| (id(*place), *settings))
                .collect(),
            ..NotificationSettings::default()
        }
    }

    fn place(
        level: Option<NotifyLevel>,
        muted_until: Option<MuteEnd>,
    ) -> PlaceNotificationSettings {
        PlaceNotificationSettings { level, muted_until }
    }

    const IN_SERVER: NotificationPlace = NotificationPlace {
        guild_id: Some(Snowflake::from_raw(SERVER)),
        channel_id: Snowflake::from_raw(GENERAL),
    };
    const IN_CONVERSATION: NotificationPlace = NotificationPlace {
        guild_id: None,
        channel_id: Snowflake::from_raw(CONVERSATION),
    };

    /// Ana's message in the server's general channel, read by me, online, with default settings.
    fn alert(message: &Message, place: NotificationPlace, settings: &NotificationSettings) -> bool {
        alert_with(
            message,
            place,
            settings,
            &[],
            Some(ChosenStatus::Online),
            false,
        )
    }

    fn alert_with(
        message: &Message,
        place: NotificationPlace,
        settings: &NotificationSettings,
        reader_role_ids: &[Snowflake],
        chosen_status: Option<ChosenStatus>,
        is_author_blocked: bool,
    ) -> bool {
        should_alert_about(IncomingMessageFacts {
            message,
            current_user_id: Some(id(ME)),
            reader_role_ids,
            place,
            is_author_blocked,
            chosen_status,
            settings,
            now: NOW,
        })
    }

    #[test]
    fn tells_about_mentions_in_a_server_and_every_message_in_a_conversation_by_default() {
        let defaults = NotificationSettings::default();
        assert!(alert(&message(ANA, &[ME], &[]), IN_SERVER, &defaults));
        assert!(!alert(&message(ANA, &[], &[]), IN_SERVER, &defaults));
        assert!(alert(&message(ANA, &[], &[]), IN_CONVERSATION, &defaults));
    }

    #[test]
    fn never_tells_about_own_messages_blocked_authors_or_anything_in_do_not_disturb() {
        let defaults = NotificationSettings::default();
        assert!(!alert(&message(ME, &[ME], &[]), IN_CONVERSATION, &defaults));
        assert!(!alert_with(
            &message(ANA, &[], &[]),
            IN_CONVERSATION,
            &defaults,
            &[],
            None,
            true
        ));
        assert!(!alert_with(
            &message(ANA, &[ME], &[]),
            IN_CONVERSATION,
            &defaults,
            &[],
            Some(ChosenStatus::DoNotDisturb),
            false
        ));
        let signed_out = IncomingMessageFacts {
            message: &message(ANA, &[], &[]),
            current_user_id: None,
            reader_role_ids: &[],
            place: IN_CONVERSATION,
            is_author_blocked: false,
            chosen_status: None,
            settings: &defaults,
            now: NOW,
        };
        assert!(!should_alert_about(signed_out));
    }

    #[test]
    fn counts_everyone_and_a_role_the_reader_holds_as_mentions() {
        let defaults = NotificationSettings::default();
        assert!(alert(&message(ANA, &[], &[SERVER]), IN_SERVER, &defaults));
        assert!(alert_with(
            &message(ANA, &[], &[300]),
            IN_SERVER,
            &defaults,
            &[id(300)],
            None,
            false
        ));
        assert!(!alert_with(
            &message(ANA, &[], &[300]),
            IN_SERVER,
            &defaults,
            &[id(301)],
            None,
            false
        ));
        let muted_channel = settings_with(&[(
            GENERAL,
            place(Some(NotifyLevel::All), Some(MuteEnd::At(NOW + 1))),
        )]);
        assert!(alert(
            &message(ANA, &[], &[SERVER]),
            IN_SERVER,
            &muted_channel
        ));
    }

    #[test]
    fn follows_the_channels_level_else_the_servers() {
        let all_in_server = settings_with(&[(SERVER, place(Some(NotifyLevel::All), None))]);
        assert!(alert(&message(ANA, &[], &[]), IN_SERVER, &all_in_server));
        let nothing_in_channel = settings_with(&[
            (SERVER, place(Some(NotifyLevel::All), None)),
            (GENERAL, place(Some(NotifyLevel::Nothing), None)),
        ]);
        assert!(!alert(
            &message(ANA, &[ME], &[]),
            IN_SERVER,
            &nothing_in_channel
        ));
        assert_eq!(
            notify_level_of(
                &nothing_in_channel,
                NotificationPlace {
                    guild_id: Some(id(SERVER)),
                    channel_id: id(999)
                }
            ),
            NotifyLevel::All
        );
        let mentions_in_conversation =
            settings_with(&[(CONVERSATION, place(Some(NotifyLevel::Mentions), None))]);
        assert!(!alert(
            &message(ANA, &[], &[]),
            IN_CONVERSATION,
            &mentions_in_conversation
        ));
    }

    #[test]
    fn lets_only_mentions_through_a_running_mute_of_the_channel_or_its_server() {
        let muted_channel = settings_with(&[(
            GENERAL,
            place(Some(NotifyLevel::All), Some(MuteEnd::At(NOW + 1))),
        )]);
        assert!(!alert(&message(ANA, &[], &[]), IN_SERVER, &muted_channel));
        assert!(alert(&message(ANA, &[ME], &[]), IN_SERVER, &muted_channel));
        let muted_server =
            settings_with(&[(SERVER, place(Some(NotifyLevel::All), Some(MuteEnd::Never)))]);
        assert!(!alert(&message(ANA, &[], &[]), IN_SERVER, &muted_server));
        assert!(alert(&message(ANA, &[ME], &[]), IN_SERVER, &muted_server));
        let ended = settings_with(&[(
            GENERAL,
            place(Some(NotifyLevel::All), Some(MuteEnd::At(NOW))),
        )]);
        assert!(alert(&message(ANA, &[], &[]), IN_SERVER, &ended));
        let muted_conversation =
            settings_with(&[(CONVERSATION, place(None, Some(MuteEnd::Never)))]);
        assert!(!alert(
            &message(ANA, &[], &[]),
            IN_CONVERSATION,
            &muted_conversation
        ));
    }

    #[test]
    fn delivers_nowhere_on_the_open_channel_in_the_app_elsewhere_and_by_the_system_when_away() {
        let attention = |is_window_visible, has_window_focus, open| ReaderAttention {
            is_window_visible,
            has_window_focus,
            open_channel_id: Some(id(open)),
        };
        assert_eq!(
            alert_delivery_for(id(GENERAL), attention(true, true, GENERAL)),
            AlertDelivery::None
        );
        assert_eq!(
            alert_delivery_for(id(GENERAL), attention(true, true, CONVERSATION)),
            AlertDelivery::InApp
        );
        assert_eq!(
            alert_delivery_for(id(GENERAL), attention(false, false, GENERAL)),
            AlertDelivery::System
        );
        assert_eq!(
            alert_delivery_for(id(GENERAL), attention(true, false, GENERAL)),
            AlertDelivery::System
        );
    }

    fn preview(content: &str, roles: &HashMap<Snowflake, Role>) -> MessagePreview {
        message_preview(&markdown_as_plain_text(content, roles, WORDS), 0)
    }

    fn text_preview(value: &str) -> MessagePreview {
        MessagePreview::Text(value.to_owned())
    }

    #[test]
    fn previews_plain_text_without_markdown_and_never_a_spoiler() {
        let no_roles = HashMap::new();
        assert_eq!(
            preview("**see** you at\n\n||the secret place|| at `7`", &no_roles),
            text_preview("see you at spoiler at 7")
        );
        assert_eq!(
            preview("look <:party:123> > here", &no_roles),
            text_preview("look :party: > here")
        );
        assert_eq!(
            preview("3. pain\n  - complet\n4. lait\n-# merci", &no_roles),
            text_preview("pain complet lait merci")
        );
    }

    #[test]
    fn names_mentioned_roles_never_their_markup() {
        let moderators = Role {
            id: id(300),
            name: "Moderators".into(),
            color: 0,
            position: 1,
            permissions: Permissions::empty(),
            is_displayed_separately: false,
            is_mentionable: false,
            icon: None,
            unicode_emoji: None,
        };
        let roles = HashMap::from([(moderators.id, moderators)]);
        assert_eq!(
            preview("<@&300> and <@&301>, @everyone", &roles),
            text_preview("@Moderators and @deleted-role, @everyone")
        );
    }

    #[test]
    fn says_files_were_sent_when_there_is_no_text_and_cuts_long_text() {
        assert_eq!(message_preview("", 1), MessagePreview::SentFiles(1));
        assert_eq!(message_preview(" ", 2), MessagePreview::SentFiles(2));
        assert_eq!(message_preview("", 0), MessagePreview::Empty);
        let MessagePreview::Text(long) = message_preview(&"é".repeat(500), 0) else {
            panic!("expected text");
        };
        assert_eq!(long.chars().count(), MAXIMUM_PREVIEW_CHARACTERS);
        assert!(long.ends_with('…'));
    }

    #[test]
    fn names_the_author_except_in_a_one_to_one_conversation_and_counts_a_burst() {
        let base = NotificationContentFacts {
            is_one_to_one_conversation: false,
            has_preview: true,
            message_count: 1,
            should_show_preview: true,
        };
        assert_eq!(
            notification_content(base),
            (NotificationTitle::Place, NotificationBody::AuthorSaid)
        );
        assert_eq!(
            notification_content(NotificationContentFacts {
                is_one_to_one_conversation: true,
                message_count: 3,
                ..base
            }),
            (
                NotificationTitle::PlaceWithCount(3),
                NotificationBody::Preview
            )
        );
        assert_eq!(
            notification_content(NotificationContentFacts {
                should_show_preview: false,
                ..base
            })
            .1,
            NotificationBody::NewMessageFrom
        );
    }

    #[test]
    fn puts_the_count_in_the_title_capped_at_99_plus() {
        assert_eq!(window_title(0, "Mellow"), "Mellow");
        assert_eq!(window_title(3, "Mellow"), "(3) Mellow");
        assert_eq!(window_title(250, "Mellow"), "(99+) Mellow");
    }

    fn choices(
        target_kind: NotificationTargetKind,
        chosen_level: LevelChoice,
        fallback_level: NotifyLevel,
    ) -> NotificationDialogChoices {
        NotificationDialogChoices {
            target_kind,
            chosen_mute: MuteChoice::NotMuted,
            chosen_level,
            running_mute_end: None,
            fallback_level,
            now: NOW,
        }
    }

    #[test]
    fn keeps_a_level_equal_to_the_default_implicit_and_stores_any_other() {
        let server = |level| {
            choices(
                NotificationTargetKind::Server,
                LevelChoice::Level(level),
                NotifyLevel::Mentions,
            )
        };
        assert_eq!(
            place_settings_from_dialog(server(NotifyLevel::Mentions)),
            PlaceNotificationSettings::default()
        );
        assert_eq!(
            place_settings_from_dialog(server(NotifyLevel::All)).level,
            Some(NotifyLevel::All)
        );
        let conversation = |level| {
            choices(
                NotificationTargetKind::Conversation,
                LevelChoice::Level(level),
                NotifyLevel::All,
            )
        };
        assert_eq!(
            place_settings_from_dialog(conversation(NotifyLevel::All)).level,
            None
        );
        assert_eq!(
            place_settings_from_dialog(conversation(NotifyLevel::Nothing)).level,
            Some(NotifyLevel::Nothing)
        );
    }

    #[test]
    fn stores_no_level_for_same_as_the_server_and_a_channels_own_pick_even_when_it_matches() {
        let channel = |level| {
            choices(
                NotificationTargetKind::Channel,
                level,
                NotifyLevel::Mentions,
            )
        };
        assert_eq!(
            place_settings_from_dialog(channel(LevelChoice::SameAsServer)).level,
            None
        );
        assert_eq!(
            place_settings_from_dialog(channel(LevelChoice::Level(NotifyLevel::Mentions))).level,
            Some(NotifyLevel::Mentions)
        );
    }

    #[test]
    fn keeps_a_running_timed_mutes_end_instead_of_restarting_it() {
        let running = |chosen_mute| NotificationDialogChoices {
            chosen_mute,
            running_mute_end: Some(MuteEnd::At(NOW + 600_000)),
            now: NOW + 300_000,
            ..choices(
                NotificationTargetKind::Server,
                LevelChoice::Level(NotifyLevel::Mentions),
                NotifyLevel::Mentions,
            )
        };
        assert_eq!(
            place_settings_from_dialog(running(MuteChoice::Current)).muted_until,
            Some(MuteEnd::At(NOW + 600_000))
        );
        assert_eq!(
            place_settings_from_dialog(running(MuteChoice::For(MuteDuration::OneHour))).muted_until,
            Some(MuteEnd::At(NOW + 300_000 + 3_600_000))
        );
        assert_eq!(
            place_settings_from_dialog(running(MuteChoice::NotMuted)).muted_until,
            None
        );
        assert_eq!(
            place_settings_from_dialog(running(MuteChoice::For(MuteDuration::UntilTurnedBackOn)))
                .muted_until,
            Some(MuteEnd::Never)
        );
    }

    #[test]
    fn stores_choices_under_the_targets_own_id() {
        assert_eq!(
            notification_target_id(NotificationTarget::Server {
                guild_id: id(SERVER)
            }),
            id(SERVER)
        );
        assert_eq!(
            notification_target_id(NotificationTarget::Channel {
                guild_id: id(SERVER),
                channel_id: id(GENERAL)
            }),
            id(GENERAL)
        );
    }
}
