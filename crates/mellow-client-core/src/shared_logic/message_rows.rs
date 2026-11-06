//! How the message list lays out messages (`web/src/components/message-rows.ts`): Discord's
//! author groups, day dividers, the unread divider and folded runs of blocked authors.

use std::collections::HashSet;
use std::hash::BuildHasher;

use mellow_protocol::{Message, Snowflake};

use super::format::CivilDate;

/// Discord starts a new author group after this long, even for the same author.
pub const AUTHOR_GROUP_MAXIMUM_GAP_MILLISECONDS: u64 = 7 * 60 * 1_000;

/// A message of the outbox, not confirmed yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingRowFacts<'a> {
    pub nonce: &'a str,
    pub is_reply: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageRowKind {
    Sent {
        /// Index in the messages given to [`build_message_rows`].
        message_index: usize,
        message_id: Snowflake,
        author_id: Snowflake,
        sent_at: u64,
        /// The first message others sent after the visit's unread marker: the "New" divider goes above it.
        is_first_unread: bool,
    },
    Pending {
        /// Index in the outbox given to [`build_message_rows`].
        pending_index: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageListRow {
    /// The message's id, or the pending message's nonce.
    pub key: String,
    pub kind: MessageRowKind,
    pub starts_author_group: bool,
    /// The local day this row opens: a day divider goes above it.
    pub day_start: Option<CivilDate>,
}

impl MessageListRow {
    fn is_first_unread(&self) -> bool {
        matches!(
            self.kind,
            MessageRowKind::Sent {
                is_first_unread: true,
                ..
            }
        )
    }
}

/// Confirmed messages oldest first, then the user's outbox; `now` dates the outbox rows and
/// `local_day_of` reads an instant's day in the reader's time zone. The unread divider and a day
/// divider start a new author group, as on Discord. The oldest loaded message opens its day only
/// at the start of the channel: an older page could still hold the same day.
#[must_use]
pub fn build_message_rows(
    messages: &[Message],
    pending_messages: &[PendingRowFacts<'_>],
    current_user_id: Option<Snowflake>,
    now: u64,
    unread_marker: Option<Snowflake>,
    has_reached_start: bool,
    local_day_of: impl Fn(u64) -> CivilDate,
) -> Vec<MessageListRow> {
    let mut rows = Vec::with_capacity(messages.len() + pending_messages.len());
    // The author is `None` for the outbox of a reader not signed in, as the web's empty id.
    let mut previous: Option<(Option<Snowflake>, u64)> = None;
    let mut previous_day: Option<CivilDate> = None;
    let mut has_placed_unread_divider = false;
    let mut day_opened_at = |sent_at: u64| {
        let day = local_day_of(sent_at);
        let opens_day = previous_day.map_or(has_reached_start, |previous_day| previous_day != day);
        previous_day = Some(day);
        opens_day.then_some(day)
    };
    let continues_group = |previous: Option<(Option<Snowflake>, u64)>,
                           author_id: Option<Snowflake>,
                           sent_at: u64,
                           is_reply: bool| {
        !is_reply
            && previous.is_some_and(|(previous_author, previous_sent_at)| {
                previous_author == author_id
                    && sent_at.saturating_sub(previous_sent_at)
                        <= AUTHOR_GROUP_MAXIMUM_GAP_MILLISECONDS
            })
    };

    for (message_index, message) in messages.iter().enumerate() {
        let sent_at = message.id.unix_milliseconds();
        let is_first_unread = !has_placed_unread_divider
            && unread_marker.is_some_and(|marker| message.id > marker)
            && Some(message.author.id) != current_user_id;
        has_placed_unread_divider |= is_first_unread;
        let day_start = day_opened_at(sent_at);
        let starts_author_group = is_first_unread
            || day_start.is_some()
            || !continues_group(
                previous,
                Some(message.author.id),
                sent_at,
                message.reply_to_id.is_some(),
            );
        rows.push(MessageListRow {
            key: message.id.raw().to_string(),
            kind: MessageRowKind::Sent {
                message_index,
                message_id: message.id,
                author_id: message.author.id,
                sent_at,
                is_first_unread,
            },
            starts_author_group,
            day_start,
        });
        previous = Some((Some(message.author.id), sent_at));
    }
    for (pending_index, pending) in pending_messages.iter().enumerate() {
        let day_start = day_opened_at(now);
        rows.push(MessageListRow {
            key: pending.nonce.to_owned(),
            kind: MessageRowKind::Pending { pending_index },
            starts_author_group: day_start.is_some()
                || !continues_group(previous, current_user_id, now, pending.is_reply),
            day_start,
        });
        previous = Some((current_user_id, now));
    }
    rows
}

/// What the reader had read when they opened the channel.
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

/// What the jump bar counts (DESIGN.md §5.4): never live arrivals, never the user's own messages.
/// The bar dates it from the last message read ("3 new messages since today at 14:32").
#[must_use]
pub fn unread_summary(
    messages: &[Message],
    marker: UnreadMarker,
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

/// Messages others sent after `latest_message_id_when_leaving`, the newest message when the
/// reader scrolled away from the present: "Jump to present · 3 new" (DESIGN.md §5.4).
#[must_use]
pub fn arrivals_since_leaving_present(
    messages: &[Message],
    latest_message_id_when_leaving: Snowflake,
    current_user_id: Option<Snowflake>,
) -> usize {
    // Oldest first: only the tail can be newer, so the scan stops at the first message that is not.
    messages
        .iter()
        .rev()
        .take_while(|message| message.id > latest_message_id_when_leaving)
        .filter(|message| Some(message.author.id) != current_user_id)
        .count()
}

/// True when the first new message is loaded, so the "New" divider can sit above it.
#[must_use]
pub fn is_unread_start_loaded(
    messages: &[Message],
    marker: UnreadMarker,
    has_reached_start: bool,
) -> bool {
    has_reached_start
        || messages
            .first()
            .is_some_and(|oldest| oldest.id <= marker.last_read_message_id)
}

/// A run of messages from someone the user blocked, folded into one row (Discord's behaviour).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedRun {
    /// From the run's first message; older pages extending the run change it.
    pub key: String,
    /// From the run's last message: stable while older pages are prepended (the scroll anchor).
    pub anchor_key: String,
    /// Oldest first: Show reveals these, and a jump to one of them reveals the run.
    pub message_ids: Vec<Snowflake>,
    pub is_revealed: bool,
    /// Folded and holding the first unread message: the "New" divider goes above the run. Once
    /// revealed, the message itself carries the divider, so it is drawn once.
    pub is_first_unread: bool,
    /// When the first unread message was sent, else the first message: dates the divider.
    pub sent_at: u64,
    /// Folded and opening a day: the day divider goes above the run. Once revealed, the first
    /// message carries it, as with the unread divider.
    pub day_start: Option<CivilDate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageListEntry {
    Row(MessageListRow),
    BlockedRun(BlockedRun),
}

fn sent_facts(row: &MessageListRow) -> Option<(Snowflake, Snowflake, u64)> {
    match row.kind {
        MessageRowKind::Sent {
            message_id,
            author_id,
            sent_at,
            ..
        } => Some((message_id, author_id, sent_at)),
        MessageRowKind::Pending { .. } => None,
    }
}

/// `rows` with each run of consecutive messages by one blocked author folded into a
/// [`BlockedRun`]; a run is revealed when any of its messages is in `revealed_message_ids`, and
/// then keeps its rows after it. The outbox is the user's own, so never folded.
#[must_use]
pub fn collapse_blocked_runs<S: BuildHasher>(
    rows: &[MessageListRow],
    is_blocked: impl Fn(Snowflake) -> bool,
    revealed_message_ids: &HashSet<Snowflake, S>,
) -> Vec<MessageListEntry> {
    let mut entries = Vec::with_capacity(rows.len());
    let mut run_rows: Vec<&MessageListRow> = Vec::new();
    let close_run = |run_rows: &mut Vec<&MessageListRow>, entries: &mut Vec<MessageListEntry>| {
        let (Some(first), Some(last)) = (run_rows.first(), run_rows.last()) else {
            return;
        };
        let message_ids: Vec<Snowflake> = run_rows
            .iter()
            .filter_map(|row| sent_facts(row))
            .map(|(id, _, _)| id)
            .collect();
        let is_revealed = message_ids
            .iter()
            .any(|id| revealed_message_ids.contains(id));
        let first_unread = run_rows.iter().find(|row| row.is_first_unread());
        let sent_at =
            sent_facts(first_unread.unwrap_or(first)).map_or(0, |(_, _, sent_at)| sent_at);
        entries.push(MessageListEntry::BlockedRun(BlockedRun {
            key: format!("blocked-{}", first.key),
            anchor_key: format!("blocked-{}", last.key),
            message_ids,
            is_revealed,
            is_first_unread: !is_revealed && first_unread.is_some(),
            sent_at,
            day_start: if is_revealed { None } else { first.day_start },
        }));
        if is_revealed {
            entries.extend(
                run_rows
                    .iter()
                    .map(|row| MessageListEntry::Row((*row).clone())),
            );
        }
        run_rows.clear();
    };
    for row in rows {
        let blocked_author = sent_facts(row)
            .map(|(_, author_id, _)| author_id)
            .filter(|author_id| is_blocked(*author_id));
        let Some(author_id) = blocked_author else {
            close_run(&mut run_rows, &mut entries);
            entries.push(MessageListEntry::Row(row.clone()));
            continue;
        };
        // A day divider ends a run, so a folded run never hides one.
        let run_author = run_rows
            .first()
            .and_then(|first| sent_facts(first))
            .map(|(_, author, _)| author);
        if run_author.is_some_and(|run_author| run_author != author_id || row.day_start.is_some()) {
            close_run(&mut run_rows, &mut entries);
        }
        run_rows.push(row);
    }
    close_run(&mut run_rows, &mut entries);
    entries
}

#[cfg(test)]
mod tests {
    use mellow_protocol::MELLOW_EPOCH_UNIX_MILLISECONDS;
    use serde_json::json;

    use super::*;

    const START: u64 = MELLOW_EPOCH_UNIX_MILLISECONDS + 1_000_000;
    const HOUR_MILLISECONDS: i64 = 3_600_000;

    fn id_sent_at(unix_milliseconds: u64, sequence: u64) -> Snowflake {
        Snowflake::from_raw(
            ((unix_milliseconds - MELLOW_EPOCH_UNIX_MILLISECONDS) << 22) | (sequence << 12),
        )
    }

    fn message(id: Snowflake, author_id: u64, reply_to: Option<u64>) -> Message {
        serde_json::from_value(json!({
            "id": id.raw().to_string(), "channel_id": "10", "content": "hello",
            "reply_to_id": reply_to.map(|reply| reply.to_string()), "edited_at_unix_milliseconds": null,
            "author": { "id": author_id.to_string(), "username": format!("user{author_id}"), "display_name": "" },
        }))
        .expect("a valid message")
    }

    /// The reader's days in Paris: UTC+1, UTC+2 from the last Sunday of March to the last Sunday
    /// of October at 01:00 UTC (the clock changes of 2026).
    fn paris_day(unix_milliseconds: u64) -> CivilDate {
        let summer_start = 1_774_746_000_000_u64; // 2026-03-29T01:00:00Z
        let summer_end = 1_792_890_000_000_u64; // 2026-10-25T01:00:00Z
        let offset_hours = if (summer_start..summer_end).contains(&unix_milliseconds) {
            2
        } else {
            1
        };
        let local =
            i64::try_from(unix_milliseconds).unwrap_or(i64::MAX) + offset_hours * HOUR_MILLISECONDS;
        CivilDate::from_days_since_unix_epoch(local.div_euclid(86_400_000))
    }

    /// An instant written as a Paris wall-clock time with its offset.
    fn paris(year: i32, month: u8, day: u8, hour: i64, minute: i64, offset_hours: i64) -> u64 {
        let days = CivilDate { year, month, day }.days_since_unix_epoch();
        let milliseconds =
            days * 86_400_000 + (hour - offset_hours) * HOUR_MILLISECONDS + minute * 60_000;
        u64::try_from(milliseconds).unwrap_or(0)
    }

    fn rows(
        messages: &[Message],
        pending: &[PendingRowFacts<'_>],
        now: u64,
        marker: Option<Snowflake>,
        has_reached_start: bool,
    ) -> Vec<MessageListRow> {
        build_message_rows(
            messages,
            pending,
            Some(Snowflake::from_raw(1)),
            now,
            marker,
            has_reached_start,
            paris_day,
        )
    }

    fn group_starts(rows: &[MessageListRow]) -> Vec<bool> {
        rows.iter().map(|row| row.starts_author_group).collect()
    }

    fn days(rows: &[MessageListRow]) -> Vec<(String, CivilDate)> {
        rows.iter()
            .filter_map(|row| row.day_start.map(|day| (row.key.clone(), day)))
            .collect()
    }

    #[test]
    fn groups_consecutive_messages_of_one_author_within_seven_minutes() {
        let built = rows(
            &[
                message(id_sent_at(START, 0), 1, None),
                message(id_sent_at(START + 60_000, 0), 1, None),
                message(
                    id_sent_at(
                        START + 60_000 + AUTHOR_GROUP_MAXIMUM_GAP_MILLISECONDS + 1,
                        0,
                    ),
                    1,
                    None,
                ),
                message(id_sent_at(START + 9_000_000, 0), 2, None),
                message(id_sent_at(START + 9_000_001, 0), 2, Some(5)),
            ],
            &[],
            START,
            None,
            false,
        );
        assert_eq!(group_starts(&built), vec![true, false, true, true, true]);
    }

    #[test]
    fn marks_the_first_message_from_others_after_the_unread_marker_which_starts_a_group() {
        let ids: Vec<Snowflake> = [0, 60_000, 120_000, 180_000]
            .iter()
            .map(|offset| id_sent_at(START + offset, 0))
            .collect();
        let messages: Vec<Message> = ids
            .iter()
            .zip([2, 1, 1, 1])
            .map(|(id, author)| message(*id, author, None))
            .collect();
        let built = build_message_rows(
            &messages,
            &[],
            Some(Snowflake::from_raw(2)),
            START,
            ids.first().copied(),
            false,
            paris_day,
        );
        assert_eq!(
            built
                .iter()
                .map(MessageListRow::is_first_unread)
                .collect::<Vec<_>>(),
            vec![false, true, false, false]
        );
        assert_eq!(group_starts(&built), vec![true, true, false, false]);
    }

    #[test]
    fn puts_the_outbox_last_continuing_the_users_own_group() {
        let pending = [PendingRowFacts {
            nonce: "n1",
            is_reply: false,
        }];
        let built = rows(
            &[message(id_sent_at(START, 0), 1, None)],
            &pending,
            START + 1_000,
            None,
            false,
        );
        assert_eq!(
            built
                .iter()
                .map(|row| (row.key.as_str(), row.starts_author_group))
                .collect::<Vec<_>>()
                .get(1),
            Some(&("n1", false))
        );
        assert_eq!(group_starts(&built), vec![true, false]);
    }

    #[test]
    fn counts_only_what_others_wrote_between_the_last_read_message_and_the_visits_opening() {
        let ids: Vec<Snowflake> = (0..5)
            .map(|minute| id_sent_at(START + minute * 60_000, 0))
            .collect();
        let id = |index: usize| ids.get(index).copied().unwrap_or(Snowflake::from_raw(0));
        let me = Some(Snowflake::from_raw(1));
        let messages: Vec<Message> = [2, 2, 1, 2, 2]
            .iter()
            .enumerate()
            .map(|(index, author)| message(id(index), *author, None))
            .collect();
        let marker = UnreadMarker {
            last_read_message_id: id(0),
            newest_at_open_message_id: id(3),
        };
        assert_eq!(
            unread_summary(&messages, marker, me, true),
            UnreadSummary {
                count: 2,
                is_partial: false
            }
        );
        let loaded = [message(id(2), 2, None), message(id(3), 2, None)];
        assert_eq!(
            unread_summary(&loaded, marker, me, false),
            UnreadSummary {
                count: 2,
                is_partial: true
            }
        );
        assert!(!is_unread_start_loaded(&loaded, marker, false));
        assert!(is_unread_start_loaded(&messages, marker, false));
    }

    #[test]
    fn counts_what_others_sent_after_the_reader_scrolled_away() {
        let me = Some(Snowflake::from_raw(1));
        let raw = |value: u64| Snowflake::from_raw(value);
        let messages = [
            message(raw(4_194_304_009), 2, None),
            message(raw(4_194_304_010), 2, None),
            message(raw(4_194_304_011), 1, None),
            message(raw(4_194_304_012), 2, None),
            message(raw(4_194_304_013), 2, None),
        ];
        assert_eq!(
            arrivals_since_leaving_present(&messages, raw(4_194_304_010), me),
            2
        );
        assert_eq!(
            arrivals_since_leaving_present(&messages, raw(4_194_304_013), me),
            0
        );
    }

    const BLOCKED: u64 = 66;
    const OTHER_BLOCKED: u64 = 67;

    fn is_blocked(user_id: Snowflake) -> bool {
        user_id.raw() == BLOCKED || user_id.raw() == OTHER_BLOCKED
    }

    fn by(author_id: u64, raw_id: u64) -> Message {
        message(Snowflake::from_raw(raw_id), author_id, None)
    }

    fn shape(entries: &[MessageListEntry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                MessageListEntry::BlockedRun(run) => format!("run {}", run.message_ids.len()),
                MessageListEntry::Row(row) => row.key.clone(),
            })
            .collect()
    }

    fn run_in(entries: &[MessageListEntry]) -> BlockedRun {
        entries
            .iter()
            .find_map(|entry| {
                if let MessageListEntry::BlockedRun(run) = entry {
                    Some(run.clone())
                } else {
                    None
                }
            })
            .expect("a folded run")
    }

    fn dividers(entries: &[MessageListEntry]) -> Vec<String> {
        entries
            .iter()
            .filter_map(|entry| match entry {
                MessageListEntry::BlockedRun(run) if run.is_first_unread => Some(run.key.clone()),
                MessageListEntry::Row(row) if row.is_first_unread() => Some(row.key.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn folds_each_run_of_one_blocked_author_into_one_row() {
        let built = rows(
            &[
                by(2, 100),
                by(BLOCKED, 101),
                by(BLOCKED, 102),
                by(OTHER_BLOCKED, 103),
                by(2, 104),
                by(BLOCKED, 105),
            ],
            &[],
            0,
            None,
            false,
        );
        assert_eq!(
            shape(&collapse_blocked_runs(&built, is_blocked, &HashSet::new())),
            vec!["100", "run 2", "run 1", "104", "run 1"]
        );
        assert_eq!(
            shape(&collapse_blocked_runs(&built, |_| false, &HashSet::new())),
            vec!["100", "101", "102", "103", "104", "105"]
        );
    }

    #[test]
    fn shows_a_revealed_runs_messages_after_its_row_and_keeps_it_revealed_as_it_grows() {
        let built = rows(
            &[by(2, 100), by(BLOCKED, 101), by(BLOCKED, 102)],
            &[],
            0,
            None,
            false,
        );
        let revealed: HashSet<Snowflake> =
            run_in(&collapse_blocked_runs(&built, is_blocked, &HashSet::new()))
                .message_ids
                .into_iter()
                .collect();
        let grown = rows(
            &[
                by(2, 100),
                by(BLOCKED, 101),
                by(BLOCKED, 102),
                by(BLOCKED, 103),
            ],
            &[],
            0,
            None,
            false,
        );
        assert_eq!(
            shape(&collapse_blocked_runs(&grown, is_blocked, &revealed)),
            vec!["100", "run 3", "101", "102", "103"]
        );
    }

    #[test]
    fn keeps_the_scroll_anchor_and_the_revealed_state_when_older_messages_extend_the_run() {
        let newest = rows(
            &[by(BLOCKED, 102), by(BLOCKED, 103), by(2, 104)],
            &[],
            0,
            None,
            false,
        );
        let before = run_in(&collapse_blocked_runs(&newest, is_blocked, &HashSet::new()));
        let revealed: HashSet<Snowflake> = before.message_ids.iter().copied().collect();
        let with_older_page = rows(
            &[
                by(BLOCKED, 100),
                by(BLOCKED, 101),
                by(BLOCKED, 102),
                by(BLOCKED, 103),
                by(2, 104),
            ],
            &[],
            0,
            None,
            false,
        );
        let after = collapse_blocked_runs(&with_older_page, is_blocked, &revealed);
        assert_eq!(run_in(&after).anchor_key, before.anchor_key);
        assert!(run_in(&after).is_revealed);
        assert_eq!(
            shape(&after),
            vec!["run 4", "100", "101", "102", "103", "104"]
        );
    }

    #[test]
    fn draws_the_unread_divider_once_above_the_folded_run_or_above_the_message_once_revealed() {
        let built = rows(
            &[by(2, 100), by(BLOCKED, 101), by(BLOCKED, 102)],
            &[],
            0,
            Some(Snowflake::from_raw(101)),
            false,
        );
        let folded = collapse_blocked_runs(&built, is_blocked, &HashSet::new());
        assert_eq!(dividers(&folded), vec![run_in(&folded).key]);
        let revealed = collapse_blocked_runs(
            &built,
            is_blocked,
            &HashSet::from([Snowflake::from_raw(101)]),
        );
        assert_eq!(dividers(&revealed), vec!["102".to_owned()]);
    }

    fn day(year: i32, month: u8, day: u8) -> CivilDate {
        CivilDate { year, month, day }
    }

    #[test]
    fn opens_each_local_day_once_and_breaks_the_author_group_at_midnight() {
        let before_midnight = id_sent_at(paris(2026, 11, 4, 23, 58, 1), 0);
        let after_midnight = id_sent_at(paris(2026, 11, 5, 0, 2, 1), 0);
        // 00:30 UTC: a new day in UTC, still the 5th in Paris.
        let same_local_day = id_sent_at(paris(2026, 11, 5, 1, 30, 1), 0);
        let built = rows(
            &[
                message(before_midnight, 2, None),
                message(after_midnight, 2, None),
                message(same_local_day, 2, None),
            ],
            &[],
            0,
            None,
            false,
        );
        assert_eq!(
            days(&built),
            vec![(after_midnight.raw().to_string(), day(2026, 11, 5))]
        );
        assert_eq!(group_starts(&built), vec![true, true, true]);
    }

    #[test]
    fn follows_the_clock_change() {
        let saturday = id_sent_at(paris(2026, 10, 24, 23, 30, 2), 0);
        let sunday = id_sent_at(paris(2026, 10, 25, 23, 30, 1), 0);
        let monday = id_sent_at(paris(2026, 10, 26, 0, 30, 1), 0);
        let built = rows(
            &[
                message(saturday, 2, None),
                message(sunday, 2, None),
                message(monday, 2, None),
            ],
            &[],
            0,
            None,
            false,
        );
        assert_eq!(
            days(&built),
            vec![
                (sunday.raw().to_string(), day(2026, 10, 25)),
                (monday.raw().to_string(), day(2026, 10, 26))
            ]
        );
    }

    #[test]
    fn dates_the_oldest_loaded_message_only_at_the_start_of_the_channel() {
        let first = id_sent_at(paris(2026, 11, 5, 9, 0, 1), 0);
        assert_eq!(
            days(&rows(&[message(first, 2, None)], &[], 0, None, false)),
            vec![]
        );
        assert_eq!(
            days(&rows(&[message(first, 2, None)], &[], 0, None, true)),
            vec![(first.raw().to_string(), day(2026, 11, 5))]
        );
    }

    #[test]
    fn opens_a_new_day_above_the_outbox_when_it_is_sent_the_next_day() {
        let yesterday = id_sent_at(paris(2026, 11, 4, 22, 0, 1), 0);
        let pending = [PendingRowFacts {
            nonce: "n1",
            is_reply: false,
        }];
        let built = rows(
            &[message(yesterday, 1, None)],
            &pending,
            paris(2026, 11, 5, 8, 0, 1),
            None,
            false,
        );
        assert_eq!(days(&built), vec![("n1".to_owned(), day(2026, 11, 5))]);
        assert_eq!(built.last().map(|row| row.starts_author_group), Some(true));
    }

    #[test]
    fn puts_the_unread_and_day_dividers_on_one_row_when_new_messages_start_a_day() {
        let read = id_sent_at(paris(2026, 11, 4, 23, 50, 1), 0);
        let first_new = id_sent_at(paris(2026, 11, 5, 8, 5, 1), 0);
        let built = rows(
            &[message(read, 2, None), message(first_new, 2, None)],
            &[],
            0,
            Some(read),
            false,
        );
        let row = built.get(1).expect("two rows");
        assert!(row.is_first_unread());
        assert_eq!(row.day_start, Some(day(2026, 11, 5)));
    }

    #[test]
    fn ends_a_folded_run_at_a_day_divider_and_hands_the_divider_to_the_message_once_revealed() {
        let late = id_sent_at(paris(2026, 11, 4, 23, 59, 1), 0);
        let early = id_sent_at(paris(2026, 11, 5, 0, 1, 1), 0);
        let built = rows(
            &[message(late, BLOCKED, None), message(early, BLOCKED, None)],
            &[],
            0,
            None,
            false,
        );
        let opens_day = |entry: &MessageListEntry| match entry {
            MessageListEntry::BlockedRun(run) => (
                format!(
                    "blocked-{}",
                    run.message_ids.first().map_or(0, |id| id.raw())
                ),
                run.day_start.is_some(),
            ),
            MessageListEntry::Row(row) => (row.key.clone(), row.day_start.is_some()),
        };
        let folded = collapse_blocked_runs(&built, is_blocked, &HashSet::new());
        assert_eq!(
            folded
                .iter()
                .map(opens_day)
                .map(|(_, opens)| opens)
                .collect::<Vec<_>>(),
            vec![false, true]
        );
        let revealed = collapse_blocked_runs(&built, is_blocked, &HashSet::from([early]));
        assert_eq!(
            revealed.iter().map(opens_day).collect::<Vec<_>>(),
            vec![
                (format!("blocked-{}", late.raw()), false),
                (format!("blocked-{}", early.raw()), false),
                (early.raw().to_string(), true)
            ]
        );
    }
}
