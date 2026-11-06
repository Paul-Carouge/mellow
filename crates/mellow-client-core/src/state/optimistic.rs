//! The user's own reactions, poll votes and checklist ticks: shown at once, then kept, undone or
//! settled by the instance's copy, with the web client's rules. The app sends the requests; each
//! change has a `begin_*` (applied locally, `None` while the same change is still on its way so
//! two cannot arrive out of order) and a `finish_*` with the outcome.

use std::collections::HashSet;

use mellow_protocol::{Checklist, ChecklistTick, Message, PollVote, ProgressBar, Snowflake};

use super::checklists::{keep_newest, set_item_tick};
use super::polls::{PollVoteChange, count_poll_vote, own_vote_changes_for_choice};
use super::reactions::{add_reaction, reaction_key, remove_reaction, take_emoji_from};
use super::{ClientState, StateChange};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct InFlightChanges {
    /// (message, emoji key) of reaction clicks.
    reactions: HashSet<(Snowflake, String)>,
    /// Polls (by message id) with a vote change.
    poll_votes: HashSet<Snowflake>,
    /// (message, item) of checklist ticks.
    checklist_ticks: HashSet<(Snowflake, u32)>,
}

/// How a change the user made ended.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::large_enum_variant,
    reason = "made once per failed click and consumed at once, never stored"
)]
pub enum ChangeOutcome<Answer = ()> {
    /// The instance kept it, and answered this.
    Kept(Answer),
    /// The instance refused it (`ClientError::is_refused_by_instance`), or its answer was lost
    /// and the instance's copy of the message could not be read either: the guess is undone.
    Undone,
    /// The answer was lost, so the change may or may not have been kept: this is the instance's
    /// copy of the message read afterwards (`None`: deleted meanwhile, its `MESSAGE_DELETE`
    /// takes the row away). Only what the change touched is taken from it, so the user's other
    /// changes in flight keep their state.
    SettledBy(Option<Message>),
}

/// A reaction click on its way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReactionToggle {
    pub channel_id: Snowflake,
    pub message_id: Snowflake,
    /// As the user picked it, for the request.
    pub emoji: String,
    key: String,
    /// The user had reacted with it: the request takes it back.
    pub is_removing: bool,
}

/// The user's vote changes on one poll, on their way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PollVoteChanges {
    pub channel_id: Snowflake,
    pub message_id: Snowflake,
    changes: Vec<PollVoteChange>,
}

impl PollVoteChanges {
    /// The requests to send, one after another. A single-choice move is one `PUT`: the instance
    /// takes the old vote back itself.
    #[must_use]
    pub fn requests(&self) -> Vec<PollVoteChange> {
        let is_moving = self.changes.iter().any(|change| change.is_adding);
        self.changes
            .iter()
            .filter(|change| !is_moving || change.is_adding)
            .copied()
            .collect()
    }
}

/// A checklist tick on its way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecklistTickChange {
    pub channel_id: Snowflake,
    pub message_id: Snowflake,
    pub item_id: u32,
    /// Ticked by the user now; false unticks it.
    pub is_ticking: bool,
    previous_tick: Option<ChecklistTick>,
    revision_before: u64,
}

impl ClientState {
    /// Adds the user's reaction, or takes it back if they already reacted with it.
    pub fn begin_reaction_toggle(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: &str,
    ) -> Option<(ReactionToggle, StateChange)> {
        let key = reaction_key(emoji);
        if !self.in_flight.reactions.insert((message_id, key.clone())) {
            return None;
        }
        let is_removing = self
            .cached_message(channel_id, message_id)
            .is_some_and(|message| {
                message
                    .reactions
                    .iter()
                    .any(|reaction| reaction.emoji == key && reaction.me)
            });
        let toggle = ReactionToggle {
            channel_id,
            message_id,
            emoji: emoji.to_owned(),
            key,
            is_removing,
        };
        let change = self.apply_own_reaction(&toggle, !is_removing);
        Some((toggle, change))
    }

    pub fn finish_reaction_toggle(
        &mut self,
        toggle: &ReactionToggle,
        outcome: ChangeOutcome,
    ) -> StateChange {
        self.in_flight
            .reactions
            .remove(&(toggle.message_id, toggle.key.clone()));
        match outcome {
            ChangeOutcome::Kept(()) | ChangeOutcome::SettledBy(None) => StateChange::Nothing,
            ChangeOutcome::Undone => self.apply_own_reaction(toggle, toggle.is_removing),
            ChangeOutcome::SettledBy(Some(stored)) => {
                self.update_cached_message(toggle.channel_id, toggle.message_id, |message| {
                    take_emoji_from(&mut message.reactions, &toggle.key, &stored.reactions)
                })
            }
        }
    }

    fn apply_own_reaction(&mut self, toggle: &ReactionToggle, is_adding: bool) -> StateChange {
        self.update_cached_message(toggle.channel_id, toggle.message_id, |message| {
            if is_adding {
                add_reaction(&mut message.reactions, &toggle.key, true)
            } else {
                remove_reaction(&mut message.reactions, &toggle.key, true)
            }
        })
    }

    /// A moderator's "remove all", or `MESSAGE_REACTION_REMOVE_ALL`.
    pub fn remove_all_reactions(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> StateChange {
        self.update_cached_message(channel_id, message_id, |message| {
            let had_reactions = !message.reactions.is_empty();
            message.reactions.clear();
            had_reactions
        })
    }

    /// The user's choice of `answer_id`: a single-choice poll moves the vote there, a
    /// multiple-choice one toggles it.
    pub fn begin_poll_choice(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        answer_id: u32,
    ) -> Option<(PollVoteChanges, StateChange)> {
        let poll = self.cached_message(channel_id, message_id)?.poll.as_ref()?;
        let changes = own_vote_changes_for_choice(poll, answer_id);
        self.begin_poll_vote_changes(channel_id, message_id, changes)
    }

    /// Takes back every vote of the user on this poll ("Remove my vote").
    pub fn begin_removing_own_poll_votes(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> Option<(PollVoteChanges, StateChange)> {
        let poll = self.cached_message(channel_id, message_id)?.poll.as_ref()?;
        let changes = poll
            .answers
            .iter()
            .filter(|answer| answer.me_voted)
            .map(|answer| PollVoteChange {
                answer_id: answer.id,
                is_adding: false,
            })
            .collect();
        self.begin_poll_vote_changes(channel_id, message_id, changes)
    }

    fn begin_poll_vote_changes(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        changes: Vec<PollVoteChange>,
    ) -> Option<(PollVoteChanges, StateChange)> {
        if changes.is_empty() || !self.in_flight.poll_votes.insert(message_id) {
            return None;
        }
        let change = self.apply_own_poll_votes(channel_id, message_id, &changes);
        let changes = PollVoteChanges {
            channel_id,
            message_id,
            changes,
        };
        Some((changes, change))
    }

    /// `requests_sent` is how many of `requests()` the instance answered before the failure. A
    /// refusal undoes only what the instance did not take: a move is one request, so all of it;
    /// a "Remove my vote" keeps the removals that went through.
    pub fn finish_poll_vote_changes(
        &mut self,
        changes: &PollVoteChanges,
        requests_sent: usize,
        outcome: ChangeOutcome,
    ) -> StateChange {
        self.in_flight.poll_votes.remove(&changes.message_id);
        let (channel_id, message_id) = (changes.channel_id, changes.message_id);
        match outcome {
            ChangeOutcome::Kept(()) | ChangeOutcome::SettledBy(None) => StateChange::Nothing,
            ChangeOutcome::Undone => {
                let undone = changes
                    .changes
                    .iter()
                    .skip(requests_sent)
                    .rev()
                    .map(|change| PollVoteChange {
                        answer_id: change.answer_id,
                        is_adding: !change.is_adding,
                    })
                    .collect::<Vec<_>>();
                self.apply_own_poll_votes(channel_id, message_id, &undone)
            }
            ChangeOutcome::SettledBy(Some(stored)) => {
                self.update_cached_message(channel_id, message_id, |message| {
                    let Some(poll) = &stored.poll else {
                        return false;
                    };
                    let is_changed = message.poll.as_ref() != Some(poll);
                    message.poll = Some(poll.clone());
                    is_changed
                })
            }
        }
    }

    fn apply_own_poll_votes(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        changes: &[PollVoteChange],
    ) -> StateChange {
        self.update_cached_message(channel_id, message_id, |message| {
            let Some(poll) = message.poll.as_mut() else {
                return false;
            };
            changes.iter().fold(false, |is_changed, change| {
                count_poll_vote(poll, *change, true) | is_changed
            })
        })
    }

    /// `MESSAGE_POLL_VOTE_ADD` and `_REMOVE`. An echo of the user's own vote changes nothing.
    pub(super) fn count_poll_vote_event(&mut self, vote: PollVote, is_adding: bool) -> StateChange {
        let is_mine = Some(vote.user_id) == self.current_user_id();
        let change = PollVoteChange {
            answer_id: vote.answer_id,
            is_adding,
        };
        self.update_cached_message(vote.channel_id, vote.message_id, |message| {
            message
                .poll
                .as_mut()
                .is_some_and(|poll| count_poll_vote(poll, change, is_mine))
        })
    }

    /// The author ended the poll (`POST …/poll/end` answered): shown at once, and the instance's
    /// `MESSAGE_UPDATE` confirms it.
    pub fn end_poll(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        now_unix_milliseconds: u64,
    ) -> StateChange {
        self.update_cached_message(channel_id, message_id, |message| {
            match message.poll.as_mut() {
                Some(poll) if poll.ended_at_unix_milliseconds.is_none() => {
                    poll.ended_at_unix_milliseconds = Some(now_unix_milliseconds);
                    true
                }
                _ => false,
            }
        })
    }

    /// Ticks or unticks an item for the user, shown at once with the user as who ticked it.
    pub fn begin_checklist_tick(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        item_id: u32,
        now_unix_milliseconds: u64,
    ) -> Option<(ChecklistTickChange, StateChange)> {
        let user_id = self.current_user_id()?;
        let checklist = self
            .cached_message(channel_id, message_id)?
            .checklist
            .as_ref()?;
        let item = checklist.items.iter().find(|item| item.id == item_id)?;
        let tick_change = ChecklistTickChange {
            channel_id,
            message_id,
            item_id,
            is_ticking: item.checked.is_none(),
            previous_tick: item.checked,
            revision_before: checklist.revision,
        };
        if !self.in_flight.checklist_ticks.insert((message_id, item_id)) {
            return None;
        }
        let tick = tick_change.is_ticking.then_some(ChecklistTick {
            user_id,
            at_unix_milliseconds: now_unix_milliseconds,
        });
        let change = self.set_own_tick(&tick_change, tick);
        Some((tick_change, change))
    }

    /// The instance's answer replaces the guess. A refusal undoes it only when nothing newer
    /// came meanwhile; a lost answer may hide a kept tick, so the instance's copy settles it.
    pub fn finish_checklist_tick(
        &mut self,
        tick_change: &ChecklistTickChange,
        outcome: ChangeOutcome<Checklist>,
    ) -> StateChange {
        self.in_flight
            .checklist_ticks
            .remove(&(tick_change.message_id, tick_change.item_id));
        let (channel_id, message_id) = (tick_change.channel_id, tick_change.message_id);
        match outcome {
            ChangeOutcome::Kept(checklist) => {
                self.apply_checklist(channel_id, message_id, &checklist)
            }
            ChangeOutcome::SettledBy(None) => StateChange::Nothing,
            ChangeOutcome::SettledBy(Some(stored)) => {
                self.update_cached_message(channel_id, message_id, |message| {
                    match (&stored.checklist, &mut message.checklist) {
                        // The instance's copy wins even at the same revision: the cached one
                        // holds the guess.
                        (Some(stored), Some(cached)) if stored.revision >= cached.revision => {
                            let is_changed = cached != stored;
                            cached.clone_from(stored);
                            is_changed
                        }
                        _ => false,
                    }
                })
            }
            ChangeOutcome::Undone => {
                let revision_now = self
                    .cached_message(channel_id, message_id)
                    .and_then(|message| message.checklist.as_ref())
                    .map(|checklist| checklist.revision);
                if revision_now == Some(tick_change.revision_before) {
                    self.set_own_tick(tick_change, tick_change.previous_tick)
                } else {
                    StateChange::Nothing
                }
            }
        }
    }

    fn set_own_tick(
        &mut self,
        tick_change: &ChecklistTickChange,
        tick: Option<ChecklistTick>,
    ) -> StateChange {
        self.update_cached_message(tick_change.channel_id, tick_change.message_id, |message| {
            message
                .checklist
                .as_mut()
                .is_some_and(|checklist| set_item_tick(checklist, tick_change.item_id, tick))
        })
    }

    /// `MESSAGE_CHECKLIST_UPDATE`, or the instance's answer to adding, renaming or removing an
    /// item: kept only when newer than the cached copy.
    pub fn apply_checklist(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        checklist: &Checklist,
    ) -> StateChange {
        self.update_cached_message(channel_id, message_id, |message| {
            keep_newest(&mut message.checklist, Some(checklist.clone()))
        })
    }

    /// `MESSAGE_PROGRESS_BAR_UPDATE`, or the instance's answer to updating one.
    pub fn apply_progress_bar(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        progress_bar: &ProgressBar,
    ) -> StateChange {
        self.update_cached_message(channel_id, message_id, |message| {
            keep_newest(&mut message.progress_bar, Some(progress_bar.clone()))
        })
    }
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{
        ChecklistItem, GatewayEvent, MessageReaction, Poll, PollAnswer, ReactionSummary,
    };

    use super::super::tests::{GENERAL, ME, NOW, OTHER, message, ready_state};
    use super::*;

    const MESSAGE: Snowflake = Snowflake::from_raw(11);

    fn reactions(state: &ClientState) -> Vec<ReactionSummary> {
        state.messages(GENERAL)[0].reactions.clone()
    }

    fn thumbs(count: u32, me: bool) -> ReactionSummary {
        ReactionSummary {
            emoji: "👍".into(),
            count,
            me,
        }
    }

    fn reaction_event(user_id: Snowflake) -> MessageReaction {
        MessageReaction {
            guild_id: None,
            channel_id: GENERAL,
            message_id: MESSAGE,
            user_id,
            emoji: "👍".into(),
        }
    }

    fn state_with(build: impl FnOnce(&mut Message)) -> ClientState {
        let mut state = ready_state();
        let mut cached = message(11, OTHER);
        build(&mut cached);
        state.insert_messages(GENERAL, vec![cached]);
        state
    }

    #[test]
    fn a_reaction_shows_at_once_counts_once_with_its_echo_and_ignores_a_second_click() {
        let mut state = state_with(|_| {});
        let (toggle, change) = state.begin_reaction_toggle(GENERAL, MESSAGE, "👍️").unwrap();
        assert_eq!(
            change,
            StateChange::Message {
                channel_id: GENERAL,
                message_id: MESSAGE
            }
        );
        assert!(!toggle.is_removing);
        assert_eq!(reactions(&state), [thumbs(1, true)]);
        assert!(
            state
                .begin_reaction_toggle(GENERAL, MESSAGE, "👍")
                .is_none(),
            "the same emoji is on its way"
        );
        state.apply_event(GatewayEvent::MessageReactionAdd(reaction_event(ME)), NOW);
        assert_eq!(reactions(&state), [thumbs(1, true)], "its echo");
        state.finish_reaction_toggle(&toggle, ChangeOutcome::Kept(()));
        let (toggle, _) = state.begin_reaction_toggle(GENERAL, MESSAGE, "👍").unwrap();
        assert!(toggle.is_removing);
        assert_eq!(reactions(&state), []);
    }

    #[test]
    fn a_refused_reaction_is_undone_and_a_lost_answer_is_settled_by_the_instance_copy() {
        let mut state = state_with(|_| {});
        let (toggle, _) = state.begin_reaction_toggle(GENERAL, MESSAGE, "👍").unwrap();
        state.apply_event(GatewayEvent::MessageReactionAdd(reaction_event(OTHER)), NOW);
        state.finish_reaction_toggle(&toggle, ChangeOutcome::Undone);
        assert_eq!(reactions(&state), [thumbs(1, false)]);

        let (toggle, _) = state.begin_reaction_toggle(GENERAL, MESSAGE, "👍").unwrap();
        let mut stored = message(11, OTHER);
        stored.reactions = vec![thumbs(5, true)];
        state.finish_reaction_toggle(&toggle, ChangeOutcome::SettledBy(Some(stored)));
        assert_eq!(reactions(&state), [thumbs(5, true)]);
    }

    fn lunch(allows_multiple_answers: bool) -> Poll {
        let answer = |id, me_voted| PollAnswer {
            id,
            text: format!("answer {id}"),
            vote_count: u32::from(me_voted),
            me_voted,
        };
        Poll {
            question: "Lunch?".into(),
            answers: vec![answer(1, true), answer(2, false), answer(3, true)],
            allows_multiple_answers,
            expires_at_unix_milliseconds: NOW * 2,
            ended_at_unix_milliseconds: None,
        }
    }

    fn votes(state: &ClientState) -> Vec<(u32, bool)> {
        state.messages(GENERAL)[0]
            .poll
            .as_ref()
            .unwrap()
            .answers
            .iter()
            .map(|answer| (answer.vote_count, answer.me_voted))
            .collect()
    }

    #[test]
    fn a_single_choice_move_is_one_request_and_undone_whole_when_refused() {
        let mut state = state_with(|message| {
            let mut poll = lunch(false);
            poll.answers[2].me_voted = false;
            poll.answers[2].vote_count = 0;
            message.poll = Some(poll);
        });
        let (changes, _) = state.begin_poll_choice(GENERAL, MESSAGE, 2).unwrap();
        assert_eq!(
            changes.requests(),
            [PollVoteChange {
                answer_id: 2,
                is_adding: true
            }]
        );
        assert_eq!(votes(&state), [(0, false), (1, true), (0, false)]);
        assert!(
            state.begin_poll_choice(GENERAL, MESSAGE, 3).is_none(),
            "the poll's previous change is on its way"
        );
        state.finish_poll_vote_changes(&changes, 0, ChangeOutcome::Undone);
        assert_eq!(votes(&state), [(1, true), (0, false), (0, false)]);
    }

    #[test]
    fn removing_my_votes_keeps_the_removals_that_went_through() {
        let mut state = state_with(|message| message.poll = Some(lunch(true)));
        let (changes, _) = state
            .begin_removing_own_poll_votes(GENERAL, MESSAGE)
            .unwrap();
        assert_eq!(changes.requests().len(), 2);
        assert_eq!(votes(&state), [(0, false), (0, false), (0, false)]);
        state.finish_poll_vote_changes(&changes, 1, ChangeOutcome::Undone);
        assert_eq!(votes(&state), [(0, false), (0, false), (1, true)]);
        let (changes, _) = state.begin_poll_choice(GENERAL, MESSAGE, 2).unwrap();
        let mut stored = message(11, OTHER);
        stored.poll = Some(lunch(true));
        state.finish_poll_vote_changes(&changes, 0, ChangeOutcome::SettledBy(Some(stored)));
        assert_eq!(votes(&state), [(1, true), (0, false), (1, true)]);
    }

    #[test]
    fn others_votes_count_and_my_echo_changes_nothing() {
        let mut state = state_with(|message| message.poll = Some(lunch(true)));
        let vote = |user_id| PollVote {
            guild_id: None,
            channel_id: GENERAL,
            message_id: MESSAGE,
            answer_id: 2,
            user_id,
        };
        state.apply_event(GatewayEvent::MessagePollVoteAdd(vote(OTHER)), NOW);
        assert_eq!(votes(&state)[1], (1, false));
        let (_, _) = state.begin_poll_choice(GENERAL, MESSAGE, 2).unwrap();
        assert_eq!(votes(&state)[1], (2, true));
        assert_eq!(
            state.apply_event(GatewayEvent::MessagePollVoteAdd(vote(ME)), NOW),
            StateChange::Nothing
        );
        state.end_poll(GENERAL, MESSAGE, NOW);
        state.end_poll(GENERAL, MESSAGE, NOW + 5);
        let poll = state.messages(GENERAL)[0].poll.clone().unwrap();
        assert_eq!(poll.ended_at_unix_milliseconds, Some(NOW));
    }

    fn checklist(revision: u64, is_ticked: bool) -> Checklist {
        Checklist {
            title: "Release".into(),
            items: vec![ChecklistItem {
                id: 1,
                text: "Tag".into(),
                checked: is_ticked.then_some(ChecklistTick {
                    user_id: OTHER,
                    at_unix_milliseconds: 5,
                }),
            }],
            revision,
        }
    }

    fn cached_checklist(state: &ClientState) -> Checklist {
        state.messages(GENERAL)[0].checklist.clone().unwrap()
    }

    #[test]
    fn a_tick_shows_at_once_and_the_answer_or_a_newer_revision_wins() {
        let mut state = state_with(|message| message.checklist = Some(checklist(4, false)));
        let (tick, _) = state
            .begin_checklist_tick(GENERAL, MESSAGE, 1, NOW)
            .unwrap();
        assert!(tick.is_ticking);
        assert_eq!(
            cached_checklist(&state).items[0].checked,
            Some(ChecklistTick {
                user_id: ME,
                at_unix_milliseconds: NOW
            })
        );
        assert!(
            state
                .begin_checklist_tick(GENERAL, MESSAGE, 1, NOW)
                .is_none()
        );
        state.finish_checklist_tick(&tick, ChangeOutcome::Kept(checklist(5, true)));
        assert_eq!(cached_checklist(&state), checklist(5, true));
        state.apply_event(
            GatewayEvent::MessageChecklistUpdate {
                guild_id: None,
                channel_id: GENERAL,
                message_id: MESSAGE,
                checklist: checklist(3, false),
            },
            NOW,
        );
        assert_eq!(
            cached_checklist(&state),
            checklist(5, true),
            "older revision"
        );
    }

    #[test]
    fn a_refused_tick_is_undone_unless_something_newer_came_meanwhile() {
        let mut state = state_with(|message| message.checklist = Some(checklist(4, false)));
        let (tick, _) = state
            .begin_checklist_tick(GENERAL, MESSAGE, 1, NOW)
            .unwrap();
        state.finish_checklist_tick(&tick, ChangeOutcome::Undone);
        assert_eq!(cached_checklist(&state), checklist(4, false));

        let (tick, _) = state
            .begin_checklist_tick(GENERAL, MESSAGE, 1, NOW)
            .unwrap();
        let mut newer = checklist(6, true);
        newer.title = "Someone else's".into();
        state.apply_checklist(GENERAL, MESSAGE, &newer);
        state.finish_checklist_tick(&tick, ChangeOutcome::Undone);
        assert_eq!(cached_checklist(&state), newer);

        let (tick, _) = state
            .begin_checklist_tick(GENERAL, MESSAGE, 1, NOW)
            .unwrap();
        let mut stored = message(11, OTHER);
        stored.checklist = Some(checklist(6, false));
        state.finish_checklist_tick(&tick, ChangeOutcome::SettledBy(Some(stored)));
        assert_eq!(
            cached_checklist(&state),
            checklist(6, false),
            "the instance's copy wins at the same revision"
        );
    }

    #[test]
    fn progress_bars_keep_the_highest_revision() {
        let bar = |revision, value| ProgressBar {
            label: "Upload".into(),
            value,
            maximum: 10,
            revision,
        };
        let mut state = state_with(|message| message.progress_bar = Some(bar(2, 3)));
        state.apply_progress_bar(GENERAL, MESSAGE, &bar(1, 9));
        state.apply_event(
            GatewayEvent::MessageProgressBarUpdate {
                guild_id: None,
                channel_id: GENERAL,
                message_id: MESSAGE,
                progress_bar: bar(3, 5),
            },
            NOW,
        );
        assert_eq!(state.messages(GENERAL)[0].progress_bar, Some(bar(3, 5)));
    }
}
