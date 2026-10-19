//! Pure conversions from the client state to the rows the Slint shell displays.

use std::collections::BTreeSet;

use chrono::{DateTime, Local, TimeZone};
use mellow_client_core::state::ClientState;
use mellow_protocol::{ChannelKind, Guild, Member, Message, Snowflake};
use slint::SharedString;

use crate::generated_ui::{ChannelListRow, MemberListRow, MessageRow, PresenceStatus, ServerItem};
use crate::message_markup::styled_text_from_message_body;
use crate::shell_rows::{first_letter, saturating_int, server_initials};

/// Discord starts a new author group after this long, even for the same author.
const AUTHOR_GROUP_MAXIMUM_GAP_MILLISECONDS: u64 = 7 * 60 * 1_000;
const REPLY_PREVIEW_MAXIMUM_CHARACTERS: usize = 80;

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
    let text_channels_under = |parent_id: Option<Snowflake>, is_collapsed: bool| {
        let mut channels: Vec<_> = guild
            .channels
            .iter()
            .filter(|channel| channel.kind == ChannelKind::Text && channel.parent_id == parent_id)
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
            display_name: channel.name.as_str().into(),
            has_unread: state.has_unread(channel.id),
            mention_count: 0,
        })
    };

    let uncategorized = text_channels_under(None, false);
    let categorized = sorted_category_ids(guild)
        .into_iter()
        .enumerate()
        .filter_map(|(category_index, category_id)| {
            let category = guild
                .channels
                .iter()
                .find(|channel| channel.id == category_id)?;
            let is_collapsed = collapsed_category_ids.contains(&category_id);
            let header = ChannelListRow {
                id: SharedString::default(),
                is_category: true,
                is_collapsed,
                category_index: saturating_int(category_index),
                name: category.name.as_str().into(),
                display_name: category.name.to_uppercase().into(),
                has_unread: false,
                mention_count: 0,
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

fn reply_preview(reply_to_id: Option<Snowflake>, messages: &[Message]) -> String {
    let Some(reply_to_id) = reply_to_id else {
        return String::new();
    };
    match messages.iter().find(|message| message.id == reply_to_id) {
        Some(original) => {
            let snippet: String = original
                .content
                .chars()
                .take(REPLY_PREVIEW_MAXIMUM_CHARACTERS)
                .collect();
            format!("@{} {snippet}", original.author.display_name)
        }
        // Not in the loaded history, or deleted since.
        None => "Original message was deleted or is not loaded".into(),
    }
}

pub fn message_rows(messages: &[Message], current_user_id: Option<Snowflake>) -> Vec<MessageRow> {
    message_rows_at(messages, current_user_id, &Local::now())
}

fn message_rows_at<Zone: TimeZone>(
    messages: &[Message],
    current_user_id: Option<Snowflake>,
    now: &DateTime<Zone>,
) -> Vec<MessageRow>
where
    Zone::Offset: std::fmt::Display,
{
    let mut previous: Option<&Message> = None;
    messages
        .iter()
        .map(|message| {
            let sent_at = message.id.unix_milliseconds();
            let starts_author_group = message.reply_to_id.is_some()
                || previous.is_none_or(|previous| {
                    previous.author.id != message.author.id
                        || sent_at.saturating_sub(previous.id.unix_milliseconds())
                            > AUTHOR_GROUP_MAXIMUM_GAP_MILLISECONDS
                });
            previous = Some(message);
            MessageRow {
                author: message.author.display_name.as_str().into(),
                author_initial: first_letter(&message.author.display_name).into(),
                timestamp: timestamp_label(sent_at, now).into(),
                short_time: short_time(sent_at, &now.timezone()).into(),
                body: styled_text_from_message_body(&message.content),
                body_plain: message.content.as_str().into(),
                starts_author_group,
                reply_preview: reply_preview(message.reply_to_id, messages).into(),
                // shortcut: mentions are not parsed yet (M2).
                mentions_current_user: false,
                is_own: Some(message.author.id) == current_user_id,
                is_edited: message.edited_at_unix_milliseconds.is_some(),
            }
        })
        .collect()
}

/// Online members first, then offline, each sorted by display name.
pub fn member_rows(members: &[Member], state: &ClientState) -> Vec<MemberListRow> {
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
            display_label: label.to_uppercase().into(),
            member_count: saturating_int(group.len()),
            initial: SharedString::default(),
            activity: SharedString::default(),
            status: PresenceStatus::Offline,
        };
        let rows: Vec<_> = group
            .into_iter()
            .map(|member| MemberListRow {
                is_group_header: false,
                label: member.user.display_name.as_str().into(),
                display_label: member.user.display_name.as_str().into(),
                member_count: 0,
                initial: first_letter(&member.user.display_name).into(),
                activity: SharedString::default(),
                status,
            })
            .collect();
        std::iter::once(header).chain(rows)
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, Utc};
    use mellow_protocol::{Channel, Permissions, PublicUser};

    use super::*;

    fn channel(
        id: u64,
        kind: ChannelKind,
        name: &str,
        parent: Option<u64>,
        position: u32,
    ) -> Channel {
        Channel {
            id: Snowflake::from_raw(id),
            guild_id: Snowflake::from_raw(1),
            kind,
            name: name.into(),
            topic: String::new(),
            parent_id: parent.map(Snowflake::from_raw),
            position,
        }
    }

    fn guild() -> Guild {
        Guild {
            id: Snowflake::from_raw(1),
            name: "Board Games Night".into(),
            owner_id: Snowflake::from_raw(9),
            my_permissions: Permissions::empty(),
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
        assert_eq!(rows[1].display_name, "TEXT CHANNELS");
        assert_eq!(rows[4].category_index, 1);
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
        let rows = message_rows_at(&messages, Some(Snowflake::from_raw(2)), &Utc::now());
        let groups: Vec<_> = rows.iter().map(|row| row.starts_author_group).collect();
        assert_eq!(groups, [true, false, true, true]);
        assert_eq!(rows[3].reply_preview, "@Aurore hello");
        assert_eq!(rows[0].reply_preview, "");
        let own: Vec<_> = rows.iter().map(|row| row.is_own).collect();
        assert_eq!(own, [false, false, false, true]);
        let edited: Vec<_> = rows.iter().map(|row| row.is_edited).collect();
        assert_eq!(edited, [false, false, false, true]);
    }
}
