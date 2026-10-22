//! Pure conversions from the client state to the rows the Slint shell displays.

use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Local, TimeZone};
use mellow_client_core::state::{ClientState, PendingMessage, PendingMessageStatus};
use mellow_protocol::{
    Channel, ChannelKind, CurrentUser, Guild, Member, Message, Permissions, Snowflake,
};
use slint::{ModelRc, SharedString, VecModel};

use crate::generated_ui::{
    AttachmentRow, ChannelListRow, DirectMessageRow, HistoryStartState, MelliState, MemberListRow,
    MessageDelivery, MessageRow, PresenceStatus, QuickSwitcherKind, RoleColor, ServerItem,
    TypingLine,
};
use crate::mentions::{MentionablePeople, find_mentions};
use crate::message_markup::{styled_text_from_message_body, styled_text_with_mentions};
use crate::person_style::{avatar_tint_index_of, highest_role_color, role_color};
use crate::quick_switcher::Destination;
use crate::shell_rows::{first_letter, saturating_int, server_initials};

/// Discord starts a new author group after this long, even for the same author.
const AUTHOR_GROUP_MAXIMUM_GAP_MILLISECONDS: u64 = 7 * 60 * 1_000;
const REPLY_PREVIEW_MAXIMUM_CHARACTERS: usize = 80;

/// Name and size of each attachment, as the message row lists them (downloads come later).
fn attachment_rows(attachments: &[mellow_protocol::Attachment]) -> ModelRc<AttachmentRow> {
    let rows: Vec<AttachmentRow> = attachments
        .iter()
        .map(|attachment| AttachmentRow {
            name: attachment.file_name.as_str().into(),
            size: human_file_size(attachment.size_bytes).into(),
        })
        .collect();
    ModelRc::new(VecModel::from(rows))
}

/// Decimal units with one decimal, as browsers and Discord show sizes ("2.4 MB").
fn human_file_size(size_bytes: u64) -> String {
    const UNIT: f64 = 1000.0;
    #[allow(clippy::cast_precision_loss)] // A display rounding: the precision lost is invisible.
    let mut size = size_bytes as f64;
    for unit in ["B", "KB", "MB", "GB"] {
        if size < UNIT || unit == "GB" {
            return if unit == "B" {
                format!("{size_bytes} B")
            } else {
                format!("{size:.1} {unit}")
            };
        }
        size /= UNIT;
    }
    format!("{size_bytes} B")
}

pub fn server_items(state: &ClientState) -> Vec<ServerItem> {
    state
        .guilds()
        .iter()
        .map(|guild| ServerItem {
            name: guild.name.as_str().into(),
            initials: server_initials(&guild.name).into(),
            has_unread: guild
                .channels
                .iter()
                .any(|channel| state.has_unread(channel.id)),
            // shortcut: mentions are not tracked yet (no mention parsing in M1).
            mention_count: 0,
        })
        .collect()
}

/// The home column: conversations, most recent activity first, as on Discord.
pub fn direct_message_rows(
    state: &ClientState,
    selected_channel_id: Option<Snowflake>,
) -> Vec<DirectMessageRow> {
    state
        .direct_message_channels()
        .iter()
        .map(|channel| {
            let name = direct_message_name(channel);
            let first_recipient_id = channel.recipients.first().map(|user| user.id);
            DirectMessageRow {
                id: channel.id.to_string().into(),
                initial: first_letter(&name).into(),
                tint: first_recipient_id.map_or(0, avatar_tint_index_of),
                status: if first_recipient_id.is_some_and(|id| state.is_online(id)) {
                    PresenceStatus::Online
                } else {
                    PresenceStatus::Offline
                },
                has_unread: Some(channel.id) != selected_channel_id && state.has_unread(channel.id),
                name: name.into(),
            }
        })
        .collect()
}

pub fn has_unread_direct_messages(state: &ClientState) -> bool {
    state
        .direct_message_channels()
        .iter()
        .any(|channel| state.has_unread(channel.id))
}

/// What the gateway connection is doing, as the connection notice and Melli show it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionStatus {
    /// No session: the sign-in screen shows, nothing to report.
    SignedOut,
    Connecting,
    Connected,
    Reconnecting,
}

/// The Home button's Melli (DESIGN.md §2.10): connection trouble first, then unread direct
/// messages. Same mapping as the web's `homeMelliState`.
pub fn home_melli_state(status: ConnectionStatus, has_unread_direct_messages: bool) -> MelliState {
    match status {
        ConnectionStatus::Reconnecting => MelliState::ConnectionLost,
        ConnectionStatus::Connecting => MelliState::Connecting,
        ConnectionStatus::Connected if has_unread_direct_messages => MelliState::Notification,
        ConnectionStatus::Connected | ConnectionStatus::SignedOut => MelliState::Idle,
    }
}

/// The other participants' display names, as Discord names a conversation until it is renamed.
pub fn direct_message_name(channel: &mellow_protocol::DirectMessageChannel) -> String {
    channel
        .recipients
        .iter()
        .map(|user| user.display_name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Categories in position order; uncategorized channels come first, as on Discord.
pub fn sorted_category_ids(guild: &Guild) -> Vec<Snowflake> {
    let mut categories: Vec<_> = guild
        .channels
        .iter()
        .filter(|channel| channel.kind == ChannelKind::Category)
        .collect();
    categories.sort_by_key(|channel| (channel.position, channel.id));
    categories.into_iter().map(|channel| channel.id).collect()
}

/// The first text channel in display order, opened when a guild is selected.
pub fn first_text_channel_id(guild: &Guild) -> Option<Snowflake> {
    channel_rows(guild, &ClientState::default(), &BTreeSet::new(), None)
        .into_iter()
        .find(|row| !row.is_category)
        .and_then(|row| row.id.parse().ok())
}

/// A collapsed category still lists its selected and unread channels, as on Discord.
pub fn channel_rows(
    guild: &Guild,
    state: &ClientState,
    collapsed_category_ids: &BTreeSet<Snowflake>,
    selected_channel_id: Option<Snowflake>,
) -> Vec<ChannelListRow> {
    let category_ids = sorted_category_ids(guild);
    // A channel whose category is hidden from us is listed with the uncategorised ones.
    let shown_parent_id = |channel: &Channel| {
        channel
            .parent_id
            .filter(|parent_id| category_ids.contains(parent_id))
    };
    let text_channels_under = |parent_id: Option<Snowflake>, is_collapsed: bool| {
        let mut channels: Vec<_> = guild
            .channels
            .iter()
            .filter(|channel| {
                channel.kind == ChannelKind::Text && shown_parent_id(channel) == parent_id
            })
            .filter(|channel| {
                !is_collapsed
                    || state.has_unread(channel.id)
                    || Some(channel.id) == selected_channel_id
            })
            .collect();
        channels.sort_by_key(|channel| (channel.position, channel.id));
        channels.into_iter().map(|channel| ChannelListRow {
            id: channel.id.to_string().into(),
            is_category: false,
            is_collapsed: false,
            category_index: -1,
            name: channel.name.as_str().into(),
            has_unread: state.has_unread(channel.id),
            mention_count: 0,
            is_read_only: !can_send_messages(state, channel.id),
        })
    };

    let uncategorized = text_channels_under(None, false);
    let categorized = category_ids
        .iter()
        .copied()
        .enumerate()
        .filter_map(|(category_index, category_id)| {
            let category = guild
                .channels
                .iter()
                .find(|channel| channel.id == category_id)?;
            let is_collapsed = collapsed_category_ids.contains(&category_id);
            let header = ChannelListRow {
                id: category_id.to_string().into(),
                is_category: true,
                is_collapsed,
                category_index: saturating_int(category_index),
                name: category.name.as_str().into(),
                has_unread: false,
                mention_count: 0,
                is_read_only: false,
            };
            Some(
                std::iter::once(header).chain(text_channels_under(Some(category_id), is_collapsed)),
            )
        })
        .flatten();
    uncategorized.chain(categorized).collect()
}

/// "Today at 14:05", "Yesterday at 09:12", or the full date for older messages.
pub fn timestamp_label<Zone: TimeZone>(unix_milliseconds: u64, now: &DateTime<Zone>) -> String
where
    Zone::Offset: std::fmt::Display,
{
    let Some(sent) = i64::try_from(unix_milliseconds)
        .ok()
        .and_then(|milliseconds| now.timezone().timestamp_millis_opt(milliseconds).single())
    else {
        return String::new();
    };
    let days_ago = now
        .date_naive()
        .signed_duration_since(sent.date_naive())
        .num_days();
    let time_of_day = sent.format("%H:%M");
    match days_ago {
        0 => format!("Today at {time_of_day}"),
        1 => format!("Yesterday at {time_of_day}"),
        _ => sent.format("%Y-%m-%d %H:%M").to_string(),
    }
}

fn short_time<Zone: TimeZone>(unix_milliseconds: u64, zone: &Zone) -> String
where
    Zone::Offset: std::fmt::Display,
{
    i64::try_from(unix_milliseconds)
        .ok()
        .and_then(|milliseconds| zone.timestamp_millis_opt(milliseconds).single())
        .map(|sent| sent.format("%H:%M").to_string())
        .unwrap_or_default()
}

/// What a reply shows above its author: the original's author and the start of its text. The
/// author is empty when the original is gone; everything is empty when the message replies to none.
#[derive(Default)]
struct ReplyReference {
    author: SharedString,
    author_initial: SharedString,
    author_tint: i32,
    text: SharedString,
}

fn reply_reference(reply_to_id: Option<Snowflake>, messages: &[Message]) -> ReplyReference {
    let Some(reply_to_id) = reply_to_id else {
        return ReplyReference::default();
    };
    match messages.iter().find(|message| message.id == reply_to_id) {
        Some(original) => ReplyReference {
            author: original.author.display_name.as_str().into(),
            author_initial: first_letter(&original.author.display_name).into(),
            author_tint: avatar_tint_index_of(original.author.id),
            text: original
                .content
                .chars()
                .take(REPLY_PREVIEW_MAXIMUM_CHARACTERS)
                .collect::<String>()
                .into(),
        },
        // Not in the loaded history, or deleted since.
        None => ReplyReference {
            text: "Original message was deleted or is not loaded".into(),
            ..ReplyReference::default()
        },
    }
}

/// Unknown permissions (before `Ready`) count as allowed: the server refuses anyway.
pub fn can_send_messages(state: &ClientState, channel_id: Snowflake) -> bool {
    state
        .my_channel_permissions(channel_id)
        .is_none_or(|permissions| permissions.contains(Permissions::SEND_MESSAGES))
}

/// Each member's highest role colour in `guild`, for author and member names.
pub fn role_colors_by_user(guild: Option<&Guild>, members: &[Member]) -> HashMap<Snowflake, u32> {
    let Some(guild) = guild else {
        return HashMap::new();
    };
    members
        .iter()
        .filter_map(|member| {
            highest_role_color(guild, &member.role_ids).map(|color| (member.user.id, color))
        })
        .collect()
}

/// Who the open channel's messages can name, and how: role colours by user, the people a
/// mention can name, and the mention colour of the current theme (`#RRGGBB`).
pub struct PeopleContext<'a> {
    pub role_colors: &'a HashMap<Snowflake, u32>,
    pub mentionable: &'a MentionablePeople,
    pub mention_color: &'a str,
}

/// `can_manage_messages`: the signed-in user may delete other people's messages here.
pub fn message_rows(
    messages: &[Message],
    current_user_id: Option<Snowflake>,
    can_manage_messages: bool,
    people: &PeopleContext,
    unread_divider_after_message_id: Option<Snowflake>,
) -> Vec<MessageRow> {
    message_rows_at(
        messages,
        current_user_id,
        can_manage_messages,
        people,
        unread_divider_after_message_id,
        &Local::now(),
    )
}

/// What a channel's visit counts as new (DESIGN.md §5.4): after the last read message, up to the
/// newest message when the channel was opened. Live arrivals are never new.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnreadMarker {
    pub last_read_message_id: Snowflake,
    pub newest_at_open_message_id: Snowflake,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnreadSummary {
    /// Messages from others that were new when the channel was opened, among those loaded.
    pub count: usize,
    /// Older new messages are not loaded yet: the count is a lower bound ("50+").
    pub is_partial: bool,
}

/// What the jump bar counts: never live arrivals, never the user's own messages (as the web).
pub fn unread_summary(
    messages: &[Message],
    marker: &UnreadMarker,
    current_user_id: Option<Snowflake>,
    has_reached_start: bool,
) -> UnreadSummary {
    let count = messages
        .iter()
        .filter(|message| {
            message.id > marker.last_read_message_id
                && message.id <= marker.newest_at_open_message_id
                && Some(message.author.id) != current_user_id
        })
        .count();
    let is_partial = !has_reached_start
        && messages
            .first()
            .is_some_and(|oldest| oldest.id > marker.last_read_message_id);
    UnreadSummary { count, is_partial }
}

/// True once the first new message is loaded, so the "New" divider can sit above it without
/// jumping as older pages arrive.
pub fn is_unread_start_loaded(
    messages: &[Message],
    marker: &UnreadMarker,
    has_reached_start: bool,
) -> bool {
    has_reached_start
        || messages
            .first()
            .is_some_and(|oldest| oldest.id <= marker.last_read_message_id)
}

/// "3 new messages since today at 14:32", dated from the last message read before the visit.
pub fn unread_bar_label(summary: &UnreadSummary, last_read_unix_milliseconds: u64) -> String {
    unread_bar_label_at(summary, last_read_unix_milliseconds, &Local::now())
}

fn unread_bar_label_at<Zone: TimeZone>(
    summary: &UnreadSummary,
    last_read_unix_milliseconds: u64,
    now: &DateTime<Zone>,
) -> String
where
    Zone::Offset: std::fmt::Display,
{
    let since = timestamp_label(last_read_unix_milliseconds, now);
    // "Today at 14:32" inside a sentence.
    let since = match since.strip_prefix("Today") {
        Some(rest) => format!("today{rest}"),
        None => match since.strip_prefix("Yesterday") {
            Some(rest) => format!("yesterday{rest}"),
            None => since,
        },
    };
    let plus = if summary.is_partial { "+" } else { "" };
    let noun = if summary.count == 1 && !summary.is_partial {
        "message"
    } else {
        "messages"
    };
    format!("{}{plus} new {noun} since {since}", summary.count)
}

fn message_rows_at<Zone: TimeZone>(
    messages: &[Message],
    current_user_id: Option<Snowflake>,
    can_manage_messages: bool,
    people: &PeopleContext,
    unread_divider_after_message_id: Option<Snowflake>,
    now: &DateTime<Zone>,
) -> Vec<MessageRow>
where
    Zone::Offset: std::fmt::Display,
{
    // Above the first message someone else wrote after the read position.
    let unread_divider_message_id = unread_divider_after_message_id.and_then(|after| {
        messages
            .iter()
            .find(|message| message.id > after && Some(message.author.id) != current_user_id)
            .map(|message| message.id)
    });
    let mut previous: Option<&Message> = None;
    messages
        .iter()
        .map(|message| {
            let sent_at = message.id.unix_milliseconds();
            let starts_unread = Some(message.id) == unread_divider_message_id;
            // No group runs across the unread divider (DESIGN.md §5.4).
            let starts_author_group = message.reply_to_id.is_some()
                || starts_unread
                || previous.is_none_or(|previous| {
                    previous.author.id != message.author.id
                        || sent_at.saturating_sub(previous.id.unix_milliseconds())
                            > AUTHOR_GROUP_MAXIMUM_GAP_MILLISECONDS
                });
            previous = Some(message);
            let is_own = Some(message.author.id) == current_user_id;
            let reply = reply_reference(message.reply_to_id, messages);
            let mentions = find_mentions(&message.content, people.mentionable);
            MessageRow {
                author: message.author.display_name.as_str().into(),
                author_initial: first_letter(&message.author.display_name).into(),
                author_tint: avatar_tint_index_of(message.author.id),
                author_role_color: role_color(people.role_colors.get(&message.author.id).copied()),
                timestamp: timestamp_label(sent_at, now).into(),
                short_time: short_time(sent_at, &now.timezone()).into(),
                body: styled_text_with_mentions(&message.content, &mentions, people.mention_color),
                body_plain: message.content.as_str().into(),
                attachments: attachment_rows(&message.attachments),
                starts_author_group,
                starts_unread,
                reply_author: reply.author,
                reply_author_initial: reply.author_initial,
                reply_author_tint: reply.author_tint,
                reply_preview: reply.text,
                mentions_current_user: mentions
                    .iter()
                    .any(|mention| Some(mention.user_id) == current_user_id),
                is_own,
                can_delete: is_own || can_manage_messages,
                is_edited: message.edited_at_unix_milliseconds.is_some(),
                delivery: MessageDelivery::Sent,
            }
        })
        .collect()
}

/// Rows of our unconfirmed messages, shown after `confirmed` (the channel's messages) and
/// timestamped now, since the server has not given them an id yet.
pub fn pending_message_rows(
    pending_messages: &[&PendingMessage],
    confirmed: &[Message],
    current_user: &CurrentUser,
    role_colors: &HashMap<Snowflake, u32>,
) -> Vec<MessageRow> {
    pending_message_rows_at(
        pending_messages,
        confirmed,
        current_user,
        role_colors,
        &Local::now(),
    )
}

fn pending_message_rows_at<Zone: TimeZone>(
    pending_messages: &[&PendingMessage],
    confirmed: &[Message],
    current_user: &CurrentUser,
    role_colors: &HashMap<Snowflake, u32>,
    now: &DateTime<Zone>,
) -> Vec<MessageRow>
where
    Zone::Offset: std::fmt::Display,
{
    let now_unix_milliseconds = u64::try_from(now.timestamp_millis()).unwrap_or_default();
    let mut continues_own_group = confirmed.last().is_some_and(|last| {
        last.author.id == current_user.id
            && now_unix_milliseconds.saturating_sub(last.id.unix_milliseconds())
                <= AUTHOR_GROUP_MAXIMUM_GAP_MILLISECONDS
    });
    pending_messages
        .iter()
        .map(|pending| {
            let starts_author_group = pending.reply_to_id.is_some() || !continues_own_group;
            continues_own_group = true;
            let reply = reply_reference(pending.reply_to_id, confirmed);
            MessageRow {
                author: current_user.display_name.as_str().into(),
                author_initial: first_letter(&current_user.display_name).into(),
                author_tint: avatar_tint_index_of(current_user.id),
                author_role_color: role_color(role_colors.get(&current_user.id).copied()),
                timestamp: timestamp_label(now_unix_milliseconds, now).into(),
                short_time: short_time(now_unix_milliseconds, &now.timezone()).into(),
                body: styled_text_from_message_body(&pending.content),
                body_plain: pending.content.as_str().into(),
                // The desktop does not upload files yet.
                attachments: ModelRc::default(),
                starts_author_group,
                starts_unread: false,
                reply_author: reply.author,
                reply_author_initial: reply.author_initial,
                reply_author_tint: reply.author_tint,
                reply_preview: reply.text,
                mentions_current_user: false,
                // Not stored yet: it can be neither edited nor deleted on the server.
                is_own: false,
                can_delete: false,
                is_edited: false,
                delivery: match pending.status {
                    PendingMessageStatus::Sending => MessageDelivery::Sending,
                    PendingMessageStatus::Failed => MessageDelivery::Failed,
                },
            }
        })
        .collect()
}

/// Online members first, then offline, each sorted by display name.
pub fn member_rows(
    members: &[Member],
    state: &ClientState,
    guild: Option<&Guild>,
) -> Vec<MemberListRow> {
    let (mut online, mut offline): (Vec<&Member>, Vec<&Member>) = members
        .iter()
        .partition(|member| state.is_online(member.user.id));
    for group in [&mut online, &mut offline] {
        group.sort_by_cached_key(|member| member.user.display_name.to_lowercase());
    }
    [
        ("Online", online, PresenceStatus::Online),
        ("Offline", offline, PresenceStatus::Offline),
    ]
    .into_iter()
    .filter(|(_, group, _)| !group.is_empty())
    .flat_map(|(label, group, status)| {
        let header = MemberListRow {
            is_group_header: true,
            label: label.into(),
            member_count: saturating_int(group.len()),
            initial: SharedString::default(),
            tint: 0,
            role_color: RoleColor::default(),
            activity: SharedString::default(),
            status: PresenceStatus::Offline,
            user_id: SharedString::default(),
            can_message: false,
        };
        let rows: Vec<_> = group
            .into_iter()
            .map(|member| MemberListRow {
                is_group_header: false,
                label: member.user.display_name.as_str().into(),
                member_count: 0,
                initial: first_letter(&member.user.display_name).into(),
                tint: avatar_tint_index_of(member.user.id),
                role_color: role_color(
                    guild.and_then(|guild| highest_role_color(guild, &member.role_ids)),
                ),
                activity: SharedString::default(),
                status,
                user_id: member.user.id.to_string().into(),
                can_message: state
                    .current_user()
                    .is_some_and(|user| user.id != member.user.id),
            })
            .collect();
        std::iter::once(header).chain(rows)
    })
    .collect()
}

/// Where a conversation sits, in the quick switcher.
const DIRECT_MESSAGE_PLACE: &str = "Direct message";

/// Everywhere the quick switcher can go: conversations, then text channels, then servers, unread
/// ones first within each kind.
pub fn quick_switcher_destinations(state: &ClientState) -> Vec<Destination> {
    let conversations = state
        .direct_message_channels()
        .iter()
        .map(|channel| Destination {
            kind: QuickSwitcherKind::Conversation,
            id: channel.id.to_string(),
            server_id: String::new(),
            name: direct_message_name(channel),
            place: DIRECT_MESSAGE_PLACE.to_owned(),
            has_unread: state.has_unread(channel.id),
        });
    let channels = state.guilds().iter().flat_map(|guild| {
        guild
            .channels
            .iter()
            .filter(|channel| channel.kind == ChannelKind::Text)
            .map(move |channel| Destination {
                kind: QuickSwitcherKind::Channel,
                id: channel.id.to_string(),
                server_id: guild.id.to_string(),
                name: channel.name.clone(),
                place: guild.name.clone(),
                has_unread: state.has_unread(channel.id),
            })
    });
    let servers = state.guilds().iter().map(|guild| Destination {
        kind: QuickSwitcherKind::Server,
        id: guild.id.to_string(),
        server_id: guild.id.to_string(),
        name: guild.name.clone(),
        place: String::new(),
        has_unread: false,
    });
    let mut destinations: Vec<Destination> = conversations.chain(channels).chain(servers).collect();
    // Stable: the kinds and the order within each stay, unread ones come first.
    destinations.sort_by_key(|destination| !destination.has_unread);
    destinations
}

/// Names the typing line lists before counting the others, as on the web.
const MAXIMUM_TYPING_NAMES_LISTED: usize = 2;
/// Past this many people the line stops counting: "Several people are typing…".
const MAXIMUM_TYPING_PEOPLE_COUNTED: usize = 5;

/// The typing line in pieces, so the names can be set in bold (the web's `typingSegments`):
/// "Ana is typing…", "Ana and Léo are typing…", "Ana, Léo and 2 others are typing…", then
/// "Several people are typing…"; all empty for nobody.
pub fn typing_line(names: &[String]) -> TypingLine {
    let line = |first: &str, joiner: &str, second: &str, tail: String| TypingLine {
        first_name: first.into(),
        joiner: joiner.into(),
        second_name: second.into(),
        tail: tail.into(),
    };
    match names {
        [] => TypingLine::default(),
        _ if names.len() > MAXIMUM_TYPING_PEOPLE_COUNTED => {
            line("", "", "", "Several people are typing…".into())
        }
        [only] => line(only, "", "", " is typing…".into()),
        [first, second] => line(first, " and ", second, " are typing…".into()),
        [first, second, ..] => {
            let others = names.len() - MAXIMUM_TYPING_NAMES_LISTED;
            let noun = if others == 1 { "other" } else { "others" };
            line(
                first,
                ", ",
                second,
                format!(" and {others} {noun} are typing…"),
            )
        }
    }
}

/// What the message list shows above its oldest message (DESIGN.md §5.4 "List states").
pub fn history_start_state(
    state: &ClientState,
    channel_id: Option<Snowflake>,
    has_history_request_failed: bool,
    is_loading_older_history: bool,
) -> HistoryStartState {
    let Some(channel_id) = channel_id else {
        return HistoryStartState::Unknown;
    };
    if has_history_request_failed {
        HistoryStartState::Failed
    } else if !state.has_older_history(channel_id) {
        HistoryStartState::ChannelStart
    // Nothing cached means never loaded: a loaded empty channel has reached its start, and trims
    // keep messages.
    } else if is_loading_older_history || state.messages(channel_id).is_empty() {
        HistoryStartState::Loading
    } else {
        HistoryStartState::Unknown
    }
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, Utc};
    use mellow_client_core::state::HISTORY_PAGE_SIZE;
    use mellow_protocol::{Channel, OverwriteKind, PermissionOverwrite, PublicUser, Ready};

    use super::*;

    #[test]
    fn the_typing_line_names_two_people_then_counts_the_others() {
        let names = |list: &[&str]| {
            list.iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>()
        };
        // Bold names, plain words, as the web's `typingSegments`.
        let read = |list: &[&str]| {
            let line = typing_line(&names(list));
            [line.first_name, line.joiner, line.second_name, line.tail].map(|part| part.to_string())
        };
        assert_eq!(read(&[]), ["", "", "", ""]);
        assert_eq!(read(&["Ana"]), ["Ana", "", "", " is typing…"]);
        assert_eq!(
            read(&["Ana", "Léo"]),
            ["Ana", " and ", "Léo", " are typing…"]
        );
        assert_eq!(
            read(&["Ana", "Léo", "Zoé"]),
            ["Ana", ", ", "Léo", " and 1 other are typing…"]
        );
        assert_eq!(
            read(&["Ana", "Léo", "Zoé", "Inès", "Malik"]),
            ["Ana", ", ", "Léo", " and 3 others are typing…"]
        );
        assert_eq!(
            read(&["Ana", "Léo", "Zoé", "Inès", "Malik", "Hugo"]),
            ["", "", "", "Several people are typing…"]
        );
    }

    mod unread {
        use super::*;

        const ME: u64 = 1;
        const SOMEONE: u64 = 2;

        fn at_minute(minute: u64) -> Snowflake {
            Snowflake::from_parts(1_000_000 + minute * 60_000, 0).unwrap()
        }

        fn message(minute: u64, author: u64) -> Message {
            Message {
                id: at_minute(minute),
                channel_id: Snowflake::from_raw(11),
                author: PublicUser {
                    id: Snowflake::from_raw(author),
                    username: format!("user{author}"),
                    display_name: format!("User {author}"),
                    avatar: None,
                    banner: None,
                },
                content: "hello".into(),
                reply_to_id: None,
                edited_at_unix_milliseconds: None,
                nonce: None,
                attachments: Vec::new(),
                reactions: Vec::new(),
                pinned: false,
            }
        }

        fn marker(last_read: u64, newest_at_open: u64) -> UnreadMarker {
            UnreadMarker {
                last_read_message_id: at_minute(last_read),
                newest_at_open_message_id: at_minute(newest_at_open),
            }
        }

        #[test]
        fn only_what_others_wrote_before_the_visit_counts() {
            // Read up to 0; 1 to 3 were there at opening (2 is the user's own); 4 arrived live.
            let messages = [
                message(0, SOMEONE),
                message(1, SOMEONE),
                message(2, ME),
                message(3, SOMEONE),
                message(4, SOMEONE),
            ];
            let summary = unread_summary(
                &messages,
                &marker(0, 3),
                Some(Snowflake::from_raw(ME)),
                true,
            );
            assert_eq!(
                summary,
                UnreadSummary {
                    count: 2,
                    is_partial: false
                }
            );
        }

        #[test]
        fn unloaded_older_news_make_a_partial_count_and_hold_the_divider_back() {
            let loaded = [message(2, SOMEONE), message(3, SOMEONE)];
            let summary =
                unread_summary(&loaded, &marker(0, 3), Some(Snowflake::from_raw(ME)), false);
            assert_eq!(
                summary,
                UnreadSummary {
                    count: 2,
                    is_partial: true
                }
            );
            assert!(!is_unread_start_loaded(&loaded, &marker(0, 3), false));
            assert!(is_unread_start_loaded(&loaded, &marker(0, 3), true));
            let with_read = [message(0, SOMEONE), message(2, SOMEONE)];
            assert!(is_unread_start_loaded(&with_read, &marker(0, 3), false));
        }

        #[test]
        fn the_label_is_dated_from_the_last_read_message() {
            let read_at = at_minute(0).unix_milliseconds();
            let now = Utc
                .timestamp_millis_opt(i64::try_from(read_at).unwrap() + 600_000)
                .unwrap();
            let time = short_time(read_at, &Utc);
            assert_eq!(
                unread_bar_label_at(
                    &UnreadSummary {
                        count: 2,
                        is_partial: true
                    },
                    read_at,
                    &now
                ),
                format!("2+ new messages since today at {time}")
            );
            assert_eq!(
                unread_bar_label_at(
                    &UnreadSummary {
                        count: 1,
                        is_partial: false
                    },
                    read_at,
                    &now
                ),
                format!("1 new message since today at {time}")
            );
        }
    }

    fn channel(
        id: u64,
        kind: ChannelKind,
        name: &str,
        parent: Option<u64>,
        position: u32,
    ) -> Channel {
        Channel {
            icon: None,
            id: Snowflake::from_raw(id),
            guild_id: Snowflake::from_raw(1),
            kind,
            name: name.into(),
            topic: String::new(),
            parent_id: parent.map(Snowflake::from_raw),
            position,
            permission_overwrites: Vec::new(),
        }
    }

    fn guild() -> Guild {
        Guild {
            icon: None,
            id: Snowflake::from_raw(1),
            name: "Board Games Night".into(),
            owner_id: Snowflake::from_raw(9),
            my_permissions: Permissions::empty(),
            my_role_ids: Vec::new(),
            roles: Vec::new(),
            channels: vec![
                channel(20, ChannelKind::Category, "Voice later", None, 2),
                channel(10, ChannelKind::Category, "Text channels", None, 1),
                channel(12, ChannelKind::Text, "random", Some(10), 1),
                channel(11, ChannelKind::Text, "general", Some(10), 0),
                channel(13, ChannelKind::Text, "rules", None, 0),
                channel(21, ChannelKind::Text, "lobby", Some(20), 0),
            ],
        }
    }

    fn names(rows: &[ChannelListRow]) -> Vec<String> {
        rows.iter().map(|row| row.name.to_string()).collect()
    }

    #[test]
    fn channels_are_grouped_under_their_categories_in_position_order() {
        let state = ClientState::default();
        let rows = channel_rows(&guild(), &state, &BTreeSet::new(), None);
        assert_eq!(
            names(&rows),
            [
                "rules",
                "Text channels",
                "general",
                "random",
                "Voice later",
                "lobby"
            ]
        );
        assert_eq!(rows[4].category_index, 1);
        // A category row carries its own id, so its "+" names the category it was clicked on.
        assert_eq!(rows[1].id, "10");
        assert_eq!(
            first_text_channel_id(&guild()),
            Some(Snowflake::from_raw(13))
        );

        let collapsed = BTreeSet::from([Snowflake::from_raw(10)]);
        let rows = channel_rows(&guild(), &state, &collapsed, Some(Snowflake::from_raw(12)));
        assert_eq!(
            names(&rows),
            ["rules", "Text channels", "random", "Voice later", "lobby"]
        );
        assert!(rows[1].is_collapsed);
    }

    #[test]
    fn channels_of_a_hidden_category_are_listed_uncategorised() {
        let mut guild = guild();
        guild
            .channels
            .retain(|channel| channel.id != Snowflake::from_raw(10));
        let rows = channel_rows(&guild, &ClientState::default(), &BTreeSet::new(), None);
        assert_eq!(
            names(&rows),
            ["general", "rules", "random", "Voice later", "lobby"],
            "sorted with the uncategorised channels, by position"
        );
    }

    #[test]
    fn timestamps_say_today_yesterday_or_the_date() {
        let zone = FixedOffset::east_opt(2 * 3_600).unwrap();
        // 2026-10-19 10:00 UTC = 12:00 at UTC+2.
        let now = Utc
            .with_ymd_and_hms(2026, 10, 19, 10, 0, 0)
            .unwrap()
            .with_timezone(&zone);
        let at = |hours_ago: i64| {
            u64::try_from((now - chrono::Duration::hours(hours_ago)).timestamp_millis()).unwrap()
        };
        assert_eq!(timestamp_label(at(1), &now), "Today at 11:00");
        assert_eq!(timestamp_label(at(13), &now), "Yesterday at 23:00");
        assert_eq!(timestamp_label(at(60), &now), "2026-10-17 00:00");
    }

    #[test]
    fn messages_group_by_author_and_show_reply_previews() {
        let author = |id: u64, name: &str| PublicUser {
            id: Snowflake::from_raw(id),
            username: name.to_lowercase(),
            display_name: name.into(),
            avatar: None,
            banner: None,
        };
        let at_minute =
            |minute: u64, sequence: u16| Snowflake::from_parts(minute * 60_000, sequence).unwrap();
        let message = |id, author, content: &str, reply_to_id| Message {
            id,
            channel_id: Snowflake::from_raw(11),
            author,
            content: content.into(),
            reply_to_id,
            edited_at_unix_milliseconds: None,
            nonce: None,
            attachments: Vec::new(),
            reactions: Vec::new(),
            pinned: false,
        };
        let first = message(at_minute(0, 0), author(1, "Aurore"), "hello", None);
        let messages = vec![
            first.clone(),
            message(at_minute(1, 0), author(1, "Aurore"), "again", None),
            message(at_minute(20, 0), author(1, "Aurore"), "later", None),
            Message {
                edited_at_unix_milliseconds: Some(at_minute(21, 0).unix_milliseconds()),
                ..message(at_minute(20, 1), author(2, "Basile"), "hi", Some(first.id))
            },
        ];
        let rows = message_rows_at(
            &messages,
            Some(Snowflake::from_raw(2)),
            false,
            &PeopleContext {
                role_colors: &HashMap::new(),
                mentionable: &MentionablePeople::default(),
                mention_color: "",
            },
            None,
            &Utc::now(),
        );
        let groups: Vec<_> = rows.iter().map(|row| row.starts_author_group).collect();
        assert_eq!(groups, [true, false, true, true]);

        // The divider sits above the first message someone else wrote after the read position,
        // and breaks the group it falls in.
        let with_divider = message_rows_at(
            &messages,
            Some(Snowflake::from_raw(2)),
            false,
            &PeopleContext {
                role_colors: &HashMap::new(),
                mentionable: &MentionablePeople::default(),
                mention_color: "",
            },
            Some(messages[0].id),
            &Utc::now(),
        );
        assert!(with_divider[1].starts_unread && with_divider[1].starts_author_group);
        assert!(!with_divider[0].starts_unread && !with_divider[2].starts_unread);
        assert_eq!(rows[3].reply_author, "Aurore");
        assert_eq!(rows[3].reply_author_initial, "A");
        assert_eq!(rows[3].reply_preview, "hello");
        assert_eq!(rows[0].reply_preview, "");
        let own: Vec<_> = rows.iter().map(|row| row.is_own).collect();
        assert_eq!(own, [false, false, false, true]);
        let edited: Vec<_> = rows.iter().map(|row| row.is_edited).collect();
        assert_eq!(edited, [false, false, false, true]);
        let deletable: Vec<_> = rows.iter().map(|row| row.can_delete).collect();
        assert_eq!(
            deletable,
            [false, false, false, true],
            "only my own message"
        );
        let moderated = message_rows_at(
            &messages,
            Some(Snowflake::from_raw(2)),
            true,
            &PeopleContext {
                role_colors: &HashMap::new(),
                mentionable: &MentionablePeople::default(),
                mention_color: "",
            },
            None,
            &Utc::now(),
        );
        assert!(moderated.iter().all(|row| row.can_delete));
    }

    #[test]
    fn a_channel_denying_send_messages_is_read_only() {
        let mut guild = guild();
        guild.my_permissions = Permissions::DEFAULT_EVERYONE;
        let guild_id = guild.id;
        let rules = guild
            .channels
            .iter_mut()
            .find(|channel| channel.name == "rules")
            .unwrap();
        rules.permission_overwrites.push(PermissionOverwrite {
            id: guild_id,
            kind: OverwriteKind::Role,
            allow: Permissions::empty(),
            deny: Permissions::SEND_MESSAGES,
        });
        let mut state = ClientState::default();
        assert!(
            can_send_messages(&state, Snowflake::from_raw(13)),
            "unknown before Ready"
        );
        state.apply_ready(Ready {
            session_id: "session".into(),
            user: CurrentUser {
                id: Snowflake::from_raw(2),
                username: "me".into(),
                display_name: "Me".into(),
                email: "me@example.org".into(),
                is_instance_administrator: false,
                avatar: None,
                banner: None,
            },
            guilds: vec![guild.clone()],
            read_states: Vec::new(),
            online_user_ids: Vec::new(),
            direct_message_channels: Vec::new(),
        });
        let read_only: Vec<_> = channel_rows(&guild, &state, &BTreeSet::new(), None)
            .iter()
            .filter(|row| row.is_read_only)
            .map(|row| row.name.to_string())
            .collect();
        assert_eq!(read_only, ["rules"]);
    }

    #[test]
    fn pending_messages_follow_our_group_and_show_their_delivery() {
        let me = CurrentUser {
            id: Snowflake::from_raw(1),
            username: "me".into(),
            display_name: "Me".into(),
            email: "me@example.org".into(),
            is_instance_administrator: false,
            avatar: None,
            banner: None,
        };
        let now = Utc.with_ymd_and_hms(2026, 10, 19, 10, 0, 0).unwrap();
        let minutes_ago = |minutes: i64| {
            let sent_at = (now - chrono::Duration::minutes(minutes)).timestamp_millis();
            let since_epoch =
                u64::try_from(sent_at).unwrap() - mellow_protocol::MELLOW_EPOCH_UNIX_MILLISECONDS;
            Snowflake::from_parts(since_epoch, 0).unwrap()
        };
        let confirmed = |id, author_id| Message {
            id,
            channel_id: Snowflake::from_raw(11),
            author: PublicUser {
                id: Snowflake::from_raw(author_id),
                username: "someone".into(),
                display_name: "Someone".into(),
                avatar: None,
                banner: None,
            },
            content: "earlier".into(),
            reply_to_id: None,
            edited_at_unix_milliseconds: None,
            nonce: None,
            attachments: Vec::new(),
            reactions: Vec::new(),
            pinned: false,
        };
        let pending = |nonce: &str, reply_to_id, status| PendingMessage {
            nonce: nonce.into(),
            channel_id: Snowflake::from_raw(11),
            content: "unsent".into(),
            reply_to_id,
            status,
        };
        let sending = pending("a", None, PendingMessageStatus::Sending);
        let failed_reply = pending("b", Some(minutes_ago(1)), PendingMessageStatus::Failed);
        let ours_a_minute_ago = vec![confirmed(minutes_ago(1), 1)];
        let rows = pending_message_rows_at(
            &[&sending, &failed_reply],
            &ours_a_minute_ago,
            &me,
            &HashMap::new(),
            &now,
        );
        let groups: Vec<_> = rows.iter().map(|row| row.starts_author_group).collect();
        assert_eq!(groups, [false, true]);
        assert_eq!(rows[1].reply_author, "Someone");
        assert_eq!(rows[1].reply_preview, "earlier");
        assert_eq!(
            rows.iter().map(|row| row.delivery).collect::<Vec<_>>(),
            [MessageDelivery::Sending, MessageDelivery::Failed]
        );
        assert!(rows.iter().all(|row| !row.is_own && row.author == "Me"));

        let theirs = vec![confirmed(minutes_ago(1), 2)];
        let ours_long_ago = vec![confirmed(minutes_ago(30), 1)];
        for confirmed in [theirs, ours_long_ago, Vec::new()] {
            let rows = pending_message_rows_at(&[&sending], &confirmed, &me, &HashMap::new(), &now);
            assert!(rows[0].starts_author_group);
        }
    }

    #[test]
    fn history_start_state_follows_failures_loading_and_the_channel_start() {
        let general = Snowflake::from_raw(11);
        let message = Message {
            id: Snowflake::from_raw(100),
            channel_id: general,
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
            nonce: None,
            attachments: Vec::new(),
            reactions: Vec::new(),
            pinned: false,
        };
        let full_page = vec![message.clone(); usize::try_from(HISTORY_PAGE_SIZE).unwrap()];
        let mut state = ClientState::default();
        let at = |state: &ClientState, is_failed, is_loading_older| {
            history_start_state(state, Some(general), is_failed, is_loading_older)
        };

        assert_eq!(
            history_start_state(&state, None, true, true),
            HistoryStartState::Unknown,
            "no channel open"
        );
        assert_eq!(
            at(&state, false, false),
            HistoryStartState::Loading,
            "never loaded"
        );
        assert_eq!(at(&state, true, false), HistoryStartState::Failed);

        state.insert_history_page(general, full_page);
        assert_eq!(at(&state, false, false), HistoryStartState::Unknown);
        assert_eq!(at(&state, false, true), HistoryStartState::Loading);
        assert_eq!(at(&state, true, false), HistoryStartState::Failed);

        state.insert_history_page(general, vec![message]);
        assert_eq!(at(&state, false, false), HistoryStartState::ChannelStart);

        let mut empty_channel = ClientState::default();
        empty_channel.insert_history_page(general, Vec::new());
        assert_eq!(
            at(&empty_channel, false, false),
            HistoryStartState::ChannelStart,
            "an empty channel shows its hero"
        );
    }

    #[test]
    fn conversations_are_named_after_their_recipients_and_unread_only_when_not_open() {
        let recipient = mellow_protocol::PublicUser {
            id: Snowflake::from_raw(7),
            username: "aurore".into(),
            display_name: "Aurore".into(),
            avatar: None,
            banner: None,
        };
        let mut state = ClientState::default();
        state.apply_ready(Ready {
            session_id: "session".into(),
            user: CurrentUser {
                id: Snowflake::from_raw(2),
                username: "me".into(),
                display_name: "Me".into(),
                email: "me@example.org".into(),
                is_instance_administrator: false,
                avatar: None,
                banner: None,
            },
            guilds: Vec::new(),
            read_states: Vec::new(),
            online_user_ids: vec![recipient.id],
            direct_message_channels: vec![mellow_protocol::DirectMessageChannel {
                id: Snowflake::from_raw(30),
                recipients: vec![recipient.clone()],
                last_message_id: Some(Snowflake::from_raw(40)),
            }],
        });
        state.insert_messages(
            Snowflake::from_raw(30),
            vec![Message {
                id: Snowflake::from_raw(40),
                channel_id: Snowflake::from_raw(30),
                author: recipient,
                content: "hi".into(),
                reply_to_id: None,
                edited_at_unix_milliseconds: None,
                nonce: None,
                attachments: Vec::new(),
                reactions: Vec::new(),
                pinned: false,
            }],
        );

        let closed = direct_message_rows(&state, None);
        let open = direct_message_rows(&state, Some(Snowflake::from_raw(30)));
        assert_eq!(closed[0].name, "Aurore");
        assert_eq!(closed[0].status, PresenceStatus::Online);
        assert!(closed[0].has_unread);
        assert!(!open[0].has_unread);
        assert!(has_unread_direct_messages(&state));
    }

    #[test]
    fn only_other_members_can_be_messaged() {
        let member = |id: u64, name: &str| Member {
            user: mellow_protocol::PublicUser {
                id: Snowflake::from_raw(id),
                username: name.to_lowercase(),
                display_name: name.into(),
                avatar: None,
                banner: None,
            },
            role_ids: Vec::new(),
            joined_at_unix_milliseconds: 0,
        };
        let mut state = ClientState::default();
        state.apply_ready(Ready {
            session_id: "session".into(),
            user: CurrentUser {
                id: Snowflake::from_raw(2),
                username: "me".into(),
                display_name: "Me".into(),
                email: "me@example.org".into(),
                is_instance_administrator: false,
                avatar: None,
                banner: None,
            },
            guilds: Vec::new(),
            read_states: Vec::new(),
            online_user_ids: Vec::new(),
            direct_message_channels: Vec::new(),
        });
        let rows = member_rows(&[member(2, "Me"), member(7, "Aurore")], &state, None);
        let can_message: Vec<_> = rows
            .iter()
            .filter(|row| !row.is_group_header)
            .map(|row| {
                (
                    row.label.to_string(),
                    row.user_id.to_string(),
                    row.can_message,
                )
            })
            .collect();
        assert_eq!(
            can_message,
            [
                ("Aurore".to_owned(), "7".to_owned(), true),
                ("Me".to_owned(), "2".to_owned(), false)
            ]
        );
    }

    #[test]
    fn file_sizes_read_as_browsers_show_them() {
        assert_eq!(human_file_size(512), "512 B");
        assert_eq!(human_file_size(2_400_000), "2.4 MB");
        assert_eq!(human_file_size(1_000), "1.0 KB");
        assert_eq!(human_file_size(25_000_000_000), "25.0 GB");
    }

    #[test]
    fn home_melli_shows_connection_trouble_before_unread_direct_messages() {
        for has_unread in [false, true] {
            assert_eq!(
                home_melli_state(ConnectionStatus::Reconnecting, has_unread),
                MelliState::ConnectionLost
            );
            assert_eq!(
                home_melli_state(ConnectionStatus::Connecting, has_unread),
                MelliState::Connecting
            );
        }
        assert_eq!(
            home_melli_state(ConnectionStatus::Connected, true),
            MelliState::Notification
        );
        assert_eq!(
            home_melli_state(ConnectionStatus::Connected, false),
            MelliState::Idle
        );
        assert_eq!(
            home_melli_state(ConnectionStatus::SignedOut, false),
            MelliState::Idle
        );
    }
}
