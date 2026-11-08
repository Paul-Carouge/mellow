//! How a poll reads (the web's `poll-view.ts`, ADR-0043): its results, the time it has left, and
//! its look while still in the outbox. Words stay with the client's catalogue.

use mellow_protocol::{NewPoll, Poll, PollAnswer};

use crate::state::is_poll_ended;

const MILLISECONDS_PER_MINUTE: u64 = 60_000;
const MINUTES_PER_HOUR: u64 = 60;
const HOURS_PER_DAY: u64 = 24;
/// Below two days, hours read better than "1 day" for anything between 24 and 47 hours.
const HOURS_SHOWN_AS_DAYS_FROM: u64 = 2 * HOURS_PER_DAY;

/// One answer with its share of the votes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollAnswerResult<'poll> {
    pub answer: &'poll PollAnswer,
    /// Of every vote cast in the poll, rounded; 0 when nobody voted.
    pub percent: u32,
    /// Among the most voted, once anyone voted; only shown when the poll has ended.
    pub is_leading: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollResults<'poll> {
    pub total_votes: u32,
    pub has_reader_voted: bool,
    pub answers: Vec<PollAnswerResult<'poll>>,
}

#[must_use]
pub fn poll_results(poll: &Poll) -> PollResults<'_> {
    let total_votes = poll
        .answers
        .iter()
        .fold(0_u32, |sum, answer| sum.saturating_add(answer.vote_count));
    let most_votes = poll
        .answers
        .iter()
        .map(|answer| answer.vote_count)
        .max()
        .unwrap_or(0);
    PollResults {
        total_votes,
        has_reader_voted: poll.answers.iter().any(|answer| answer.me_voted),
        answers: poll
            .answers
            .iter()
            .map(|answer| PollAnswerResult {
                answer,
                percent: rounded_percent(answer.vote_count, total_votes),
                is_leading: most_votes > 0 && answer.vote_count == most_votes,
            })
            .collect(),
    }
}

/// `Math.round(part / whole * 100)`: halves round up.
fn rounded_percent(part: u32, whole: u32) -> u32 {
    if whole == 0 {
        return 0;
    }
    let (part, whole) = (u64::from(part), u64::from(whole));
    u32::try_from((part * 200 + whole) / (2 * whole)).unwrap_or(100)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollTimeUnit {
    Minutes,
    Hours,
    Days,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollTimeLeft {
    Ended,
    LessThanAMinute,
    Left { unit: PollTimeUnit, amount: u64 },
}

/// Rounded down, so a poll never seems to last longer than it does.
#[must_use]
pub fn poll_time_left(poll: &Poll, now_unix_milliseconds: u64) -> PollTimeLeft {
    if is_poll_ended(poll, now_unix_milliseconds) {
        return PollTimeLeft::Ended;
    }
    let minutes =
        (poll.expires_at_unix_milliseconds - now_unix_milliseconds) / MILLISECONDS_PER_MINUTE;
    let hours = minutes / MINUTES_PER_HOUR;
    let (unit, amount) = if hours >= HOURS_SHOWN_AS_DAYS_FROM {
        (PollTimeUnit::Days, hours / HOURS_PER_DAY)
    } else if hours >= 1 {
        (PollTimeUnit::Hours, hours)
    } else if minutes >= 1 {
        (PollTimeUnit::Minutes, minutes)
    } else {
        return PollTimeLeft::LessThanAMinute;
    };
    PollTimeLeft::Left { unit, amount }
}

/// When the poll stopped taking votes, or will.
#[must_use]
pub fn poll_end_unix_milliseconds(poll: &Poll) -> u64 {
    poll.ended_at_unix_milliseconds
        .unwrap_or(poll.expires_at_unix_milliseconds)
}

/// A poll still in the outbox, shown as it will arrive: no votes yet, its clock not started.
#[must_use]
pub fn pending_poll_preview(poll: &NewPoll, now_unix_milliseconds: u64) -> Poll {
    Poll {
        question: poll.question.clone(),
        answers: (1..)
            .zip(&poll.answers)
            .map(|(id, text)| PollAnswer {
                id,
                text: text.clone(),
                vote_count: 0,
                me_voted: false,
            })
            .collect(),
        allows_multiple_answers: poll.allows_multiple_answers,
        expires_at_unix_milliseconds: now_unix_milliseconds.saturating_add(
            u64::from(poll.duration_hours) * MINUTES_PER_HOUR * MILLISECONDS_PER_MINUTE,
        ),
        ended_at_unix_milliseconds: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: u64 = 60_000;
    const HOUR: u64 = 60 * MINUTE;
    const NOW: u64 = 1_000_000_000;

    fn poll(counts: &[u32]) -> Poll {
        Poll {
            question: "Lunch?".into(),
            answers: (1..)
                .zip(counts)
                .map(|(id, &vote_count)| PollAnswer {
                    id,
                    text: format!("A{id}"),
                    vote_count,
                    me_voted: false,
                })
                .collect(),
            allows_multiple_answers: false,
            expires_at_unix_milliseconds: NOW + HOUR,
            ended_at_unix_milliseconds: None,
        }
    }

    fn shares(poll: &Poll) -> Vec<(u32, bool)> {
        poll_results(poll)
            .answers
            .iter()
            .map(|answer| (answer.percent, answer.is_leading))
            .collect()
    }

    #[test]
    fn gives_each_answer_its_share_of_every_vote_rounded_and_finds_the_leaders() {
        let lunch = poll(&[1, 2, 0]);
        let results = poll_results(&lunch);
        assert_eq!(results.total_votes, 3);
        assert!(!results.has_reader_voted);
        assert_eq!(shares(&lunch), [(33, false), (67, true), (0, false)]);
        assert_eq!(
            shares(&poll(&[1, 1, 0, 0, 0, 0, 0, 6])).first(),
            Some(&(13, false))
        );
    }

    #[test]
    fn leads_with_every_tied_answer_and_with_none_before_anyone_voted() {
        let leaders: Vec<bool> = shares(&poll(&[2, 2, 1]))
            .into_iter()
            .map(|(_, is)| is)
            .collect();
        assert_eq!(leaders, [true, true, false]);
        assert_eq!(shares(&poll(&[0, 0])), [(0, false), (0, false)]);
    }

    #[test]
    fn knows_whether_the_reader_voted() {
        let mut voted = poll(&[1, 0]);
        voted.answers[0].me_voted = true;
        assert!(poll_results(&voted).has_reader_voted);
    }

    #[test]
    fn rounds_down_to_minutes_then_hours_then_days_from_two_days() {
        let ending_in = |milliseconds: u64| {
            let mut lunch = poll(&[0, 0]);
            lunch.expires_at_unix_milliseconds = NOW + milliseconds;
            poll_time_left(&lunch, NOW)
        };
        let left = |unit, amount| PollTimeLeft::Left { unit, amount };
        assert_eq!(ending_in(30_000), PollTimeLeft::LessThanAMinute);
        assert_eq!(ending_in(MINUTE), left(PollTimeUnit::Minutes, 1));
        assert_eq!(
            ending_in(59 * MINUTE + 59_000),
            left(PollTimeUnit::Minutes, 59)
        );
        assert_eq!(ending_in(HOUR), left(PollTimeUnit::Hours, 1));
        assert_eq!(
            ending_in(47 * HOUR + 59 * MINUTE),
            left(PollTimeUnit::Hours, 47)
        );
        assert_eq!(ending_in(48 * HOUR), left(PollTimeUnit::Days, 2));
        assert_eq!(ending_in(336 * HOUR), left(PollTimeUnit::Days, 14));
        assert_eq!(ending_in(0), PollTimeLeft::Ended);
    }

    #[test]
    fn ends_at_its_expiry_or_when_its_author_ended_it() {
        let mut ended = poll(&[0, 0]);
        ended.ended_at_unix_milliseconds = Some(NOW - 1);
        assert_eq!(poll_time_left(&ended, NOW), PollTimeLeft::Ended);
        assert_eq!(poll_end_unix_milliseconds(&ended), NOW - 1);
        assert_eq!(poll_end_unix_milliseconds(&poll(&[0])), NOW + HOUR);
    }

    #[test]
    fn shows_a_poll_still_in_the_outbox_with_no_votes_for_its_whole_duration() {
        let preview = pending_poll_preview(
            &NewPoll {
                question: "Lunch?".into(),
                answers: vec!["Pizza".into(), "Soup".into()],
                allows_multiple_answers: true,
                duration_hours: 4,
            },
            NOW,
        );
        let answers: Vec<(u32, &str, u32, bool)> = preview
            .answers
            .iter()
            .map(|answer| {
                (
                    answer.id,
                    answer.text.as_str(),
                    answer.vote_count,
                    answer.me_voted,
                )
            })
            .collect();
        assert_eq!(answers, [(1, "Pizza", 0, false), (2, "Soup", 0, false)]);
        assert!(preview.allows_multiple_answers);
        assert_eq!(preview.expires_at_unix_milliseconds, NOW + 4 * HOUR);
    }
}
