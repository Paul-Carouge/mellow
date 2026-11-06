//! Polls (ADR-0043) as the client keeps them (the web's `polls.ts`).

use mellow_protocol::Poll;

/// One of the reader's own votes to add or take back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PollVoteChange {
    pub answer_id: u32,
    pub is_adding: bool,
}

/// One vote added to or taken from `answer_id`; `is_mine` keeps the reader's mark right. The
/// reader's own vote counts once: their click already counted it, so the event that follows
/// changes nothing. True when it changed.
pub fn count_poll_vote(poll: &mut Poll, change: PollVoteChange, is_mine: bool) -> bool {
    let Some(answer) = poll
        .answers
        .iter_mut()
        .find(|answer| answer.id == change.answer_id)
    else {
        return false;
    };
    if is_mine && answer.me_voted == change.is_adding {
        return false;
    }
    answer.vote_count = if change.is_adding {
        answer.vote_count.saturating_add(1)
    } else {
        answer.vote_count.saturating_sub(1)
    };
    if is_mine {
        answer.me_voted = change.is_adding;
    }
    true
}

/// Whether no more votes are taken at `now_unix_milliseconds`.
#[must_use]
pub fn is_poll_ended(poll: &Poll, now_unix_milliseconds: u64) -> bool {
    poll.ended_at_unix_milliseconds.is_some()
        || poll.expires_at_unix_milliseconds <= now_unix_milliseconds
}

/// What choosing `answer_id` changes in the reader's votes: a single-choice poll moves the vote
/// there (choosing it again changes nothing), a multiple-choice one toggles that answer.
#[must_use]
pub fn own_vote_changes_for_choice(poll: &Poll, answer_id: u32) -> Vec<PollVoteChange> {
    let Some(chosen) = poll.answers.iter().find(|answer| answer.id == answer_id) else {
        return Vec::new();
    };
    if poll.allows_multiple_answers {
        return vec![PollVoteChange {
            answer_id,
            is_adding: !chosen.me_voted,
        }];
    }
    if chosen.me_voted {
        return Vec::new();
    }
    poll.answers
        .iter()
        .filter(|answer| answer.me_voted)
        .map(|answer| PollVoteChange {
            answer_id: answer.id,
            is_adding: false,
        })
        .chain(std::iter::once(PollVoteChange {
            answer_id,
            is_adding: true,
        }))
        .collect()
}

#[cfg(test)]
mod tests {
    use mellow_protocol::PollAnswer;

    use super::*;

    const ADD: bool = true;
    const REMOVE: bool = false;

    fn change(answer_id: u32, is_adding: bool) -> PollVoteChange {
        PollVoteChange {
            answer_id,
            is_adding,
        }
    }

    fn lunch() -> Poll {
        let answer = |id, text: &str, vote_count| PollAnswer {
            id,
            text: text.into(),
            vote_count,
            me_voted: false,
        };
        Poll {
            question: "Lunch?".into(),
            answers: vec![answer(1, "Pizza", 1), answer(2, "Soup", 0)],
            allows_multiple_answers: false,
            expires_at_unix_milliseconds: 1_000,
            ended_at_unix_milliseconds: None,
        }
    }

    fn counts(poll: &Poll) -> Vec<(u32, bool)> {
        poll.answers
            .iter()
            .map(|answer| (answer.vote_count, answer.me_voted))
            .collect()
    }

    #[test]
    fn counts_votes_and_marks_the_readers_own() {
        let mut poll = lunch();
        count_poll_vote(&mut poll, change(2, ADD), true);
        assert_eq!(counts(&poll), [(1, false), (1, true)]);
        count_poll_vote(&mut poll, change(1, REMOVE), false);
        assert_eq!(counts(&poll)[0], (0, false));
        count_poll_vote(&mut poll, change(1, REMOVE), false);
        assert_eq!(counts(&poll)[0], (0, false), "never below zero");
    }

    #[test]
    fn counts_the_readers_own_vote_once_when_its_event_follows_the_click() {
        let mut poll = lunch();
        count_poll_vote(&mut poll, change(2, ADD), true);
        assert!(!count_poll_vote(&mut poll, change(2, ADD), true));
        count_poll_vote(&mut poll, change(2, REMOVE), true);
        assert_eq!(counts(&poll)[1], (0, false));
        assert!(!count_poll_vote(&mut poll, change(2, REMOVE), true));
        assert!(!count_poll_vote(&mut poll, change(9, ADD), false));
    }

    #[test]
    fn ends_at_its_expiry_or_when_ended_early() {
        let poll = lunch();
        assert!(!is_poll_ended(&poll, 999));
        assert!(is_poll_ended(&poll, 1_000));
        let ended = Poll {
            ended_at_unix_milliseconds: Some(5),
            ..lunch()
        };
        assert!(is_poll_ended(&ended, 0));
    }

    #[test]
    fn moves_a_single_choice_and_toggles_a_multiple_one() {
        let poll = lunch();
        let mut voted_soup = lunch();
        count_poll_vote(&mut voted_soup, change(2, ADD), true);
        assert_eq!(own_vote_changes_for_choice(&poll, 1), [change(1, ADD)]);
        assert_eq!(
            own_vote_changes_for_choice(&voted_soup, 1),
            [change(2, REMOVE), change(1, ADD)]
        );
        assert_eq!(own_vote_changes_for_choice(&voted_soup, 2), []);
        let multiple = Poll {
            allows_multiple_answers: true,
            ..voted_soup
        };
        assert_eq!(own_vote_changes_for_choice(&multiple, 1), [change(1, ADD)]);
        assert_eq!(
            own_vote_changes_for_choice(&multiple, 2),
            [change(2, REMOVE)]
        );
        assert_eq!(own_vote_changes_for_choice(&poll, 9), []);
    }
}
