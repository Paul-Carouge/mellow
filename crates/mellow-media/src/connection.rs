//! When to offer again, give up or report: the web client's reconnection rules
//! (`web/src/lib/voice-session.svelte.ts`) as a pure state machine. Time comes in as an argument,
//! so tests drive it with a fake clock; the network thread performs its actions.

use std::time::{Duration, Instant};

use mellow_protocol::{Snowflake, VoiceDisconnectReason, VoiceJoinRefusal};

use crate::session::VoiceSessionEvent;

/// From the offer sent to audio flowing: past it, the instance did not answer or its UDP port is
/// unreachable.
pub(crate) const MEDIA_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// ICE `disconnected` often heals by itself; past this it counts as lost.
pub(crate) const ICE_DISCONNECTED_GRACE: Duration = Duration::from_secs(3);
/// Silent attempts in a row before the session says the connection is lost.
pub(crate) const MAXIMUM_RECONNECT_ATTEMPTS: u32 = 5;
/// The server dropped this user's voice state (a resume after a long break): a fresh offer
/// follows unless `VOICE_DISCONNECTED` says why meanwhile, whatever order the two come in.
pub(crate) const OWN_STATE_DROPPED_GRACE: Duration = Duration::from_secs(1);

/// Where the session stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoicePhase {
    /// The first offer in this channel, until media flows.
    Connecting,
    Connected,
    /// Media flowed in this channel, then stopped: offering again, silently.
    Reconnecting,
}

/// Why the media connection gave up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionProblem {
    /// The instance never answered the offer.
    NoAnswer,
    /// Answered, but audio never flowed: usually the instance's UDP port is closed.
    Unreachable,
    /// Connected, then lost, and the silent attempts did not bring it back.
    ConnectionLost,
}

/// What happened, for the machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectionInput {
    /// The answer to `offer_id` was applied to its peer.
    AnswerApplied {
        offer_id: u32,
    },
    Refused {
        channel_id: Snowflake,
        offer_id: u32,
        reason: VoiceJoinRefusal,
    },
    ServerDisconnected {
        channel_id: Snowflake,
        reason: VoiceDisconnectReason,
    },
    /// The server no longer lists this user in a channel.
    OwnStateDropped {
        channel_id: Snowflake,
    },
    /// The peer of `offer_id` carries media (DTLS done, ICE connected).
    MediaConnected {
        offer_id: u32,
    },
    /// The peer of `offer_id` stopped hearing the server; it may heal.
    IceDisconnected {
        offer_id: u32,
    },
    /// The peer of `offer_id` failed for good.
    MediaFailed {
        offer_id: u32,
    },
    /// A new gateway session: it knows nothing of the old one's voice.
    GatewayReady,
    /// A resumed gateway session kept its voice if audio still flows.
    GatewayResumed,
    /// The system is back online: audio over the old network path will not come back.
    BackOnline,
    MoveTo {
        channel_id: Snowflake,
    },
    /// The camera line must send another codec (ADR-0052: fixed per media session): offer the
    /// session again in the same channel, without calling it a reconnection.
    Renegotiate,
}

/// What the network thread must do, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ConnectionAction {
    /// Drop the current peer, make a new one and send `voice_join` with this id; then report
    /// with [`ConnectionMachine::offer_sent`].
    Offer {
        channel_id: Snowflake,
        offer_id: u32,
    },
    /// Send the current `voice_state` again; report with
    /// [`ConnectionMachine::voice_state_sent`].
    SendVoiceState,
    /// End the session (after `voice_leave` when asked).
    Stop {
        should_send_leave: bool,
    },
    Emit(VoiceSessionEvent),
}

#[derive(Debug, Clone)]
#[allow(clippy::struct_excessive_bools)] // the web session's flags, one each
pub(crate) struct ConnectionMachine {
    channel_id: Snowflake,
    offer_id: u32,
    phase: VoicePhase,
    is_stopped: bool,
    has_connected_once: bool,
    reconnect_attempts: u32,
    has_retried_after_media_timeout: bool,
    has_answer: bool,
    is_ice_connected: bool,
    /// The last offer could not be sent: the next `READY` or `RESUMED` offers again.
    is_awaiting_gateway: bool,
    has_unsent_voice_state: bool,
    connect_deadline: Option<Instant>,
    ice_grace_deadline: Option<Instant>,
    own_state_dropped_deadline: Option<Instant>,
}

impl ConnectionMachine {
    /// A session about to make its first offer in `channel_id`.
    pub(crate) fn new(channel_id: Snowflake) -> Self {
        Self {
            channel_id,
            offer_id: 0,
            phase: VoicePhase::Connecting,
            is_stopped: false,
            has_connected_once: false,
            reconnect_attempts: 0,
            has_retried_after_media_timeout: false,
            has_answer: false,
            is_ice_connected: false,
            is_awaiting_gateway: false,
            has_unsent_voice_state: false,
            connect_deadline: None,
            ice_grace_deadline: None,
            own_state_dropped_deadline: None,
        }
    }

    /// The first offer.
    pub(crate) fn start(&mut self) -> Vec<ConnectionAction> {
        let mut actions = Vec::new();
        self.negotiate(&mut actions);
        actions
    }

    pub(crate) fn channel_id(&self) -> Snowflake {
        self.channel_id
    }

    #[cfg(test)]
    pub(crate) fn offer_id(&self) -> u32 {
        self.offer_id
    }

    pub(crate) fn is_stopped(&self) -> bool {
        self.is_stopped
    }

    /// Whether an answer or refusal is for the latest offer: older ones (sent before a
    /// reconnection or a move) do not match the current peer.
    pub(crate) fn is_current_offer(&self, channel_id: Snowflake, offer_id: u32) -> bool {
        !self.is_stopped && channel_id == self.channel_id && offer_id == self.offer_id
    }

    /// The user hung up: nothing more happens.
    pub(crate) fn leave(&mut self) {
        self.is_stopped = true;
        self.connect_deadline = None;
        self.ice_grace_deadline = None;
        self.own_state_dropped_deadline = None;
    }

    /// The earliest timer, to wake up for.
    pub(crate) fn next_deadline(&self) -> Option<Instant> {
        [
            self.connect_deadline,
            self.ice_grace_deadline,
            self.own_state_dropped_deadline,
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// The offer of the last `Offer` action went out, or the gateway was down.
    pub(crate) fn offer_sent(&mut self, is_sent: bool, now: Instant) {
        self.is_awaiting_gateway = !is_sent;
        if is_sent {
            self.has_unsent_voice_state = false;
            self.connect_deadline = Some(now + MEDIA_CONNECT_TIMEOUT);
        }
    }

    /// A `voice_state` went out, or the gateway was down: `RESUMED` sends it again.
    pub(crate) fn voice_state_sent(&mut self, is_sent: bool) {
        self.has_unsent_voice_state = !is_sent;
    }

    /// Fires the timers due at `now`.
    pub(crate) fn on_deadlines(&mut self, now: Instant) -> Vec<ConnectionAction> {
        let mut actions = Vec::new();
        let is_due = |deadline: &mut Option<Instant>| {
            let is_due = deadline.is_some_and(|deadline| deadline <= now);
            if is_due {
                *deadline = None;
            }
            is_due
        };
        if is_due(&mut self.own_state_dropped_deadline) {
            self.rejoin(&mut actions);
        }
        if is_due(&mut self.connect_deadline) | is_due(&mut self.ice_grace_deadline) {
            self.connection_failed(&mut actions);
        }
        actions
    }

    pub(crate) fn handle(&mut self, input: ConnectionInput, now: Instant) -> Vec<ConnectionAction> {
        let mut actions = Vec::new();
        if self.is_stopped {
            return actions;
        }
        let is_current = |offer_id: u32| offer_id == self.offer_id;
        match input {
            ConnectionInput::AnswerApplied { offer_id } if is_current(offer_id) => {
                self.has_answer = true;
            }
            ConnectionInput::Refused {
                channel_id,
                offer_id,
                reason,
            } if self.is_current_offer(channel_id, offer_id) => {
                // A refused move may leave the old connection on the instance: it ends too.
                self.stop(true, VoiceSessionEvent::JoinRefused(reason), &mut actions);
            }
            ConnectionInput::ServerDisconnected { channel_id, reason }
                if channel_id == self.channel_id =>
            {
                self.own_state_dropped_deadline = None;
                if reason == VoiceDisconnectReason::MediaTimeout
                    && !self.has_retried_after_media_timeout
                {
                    self.has_retried_after_media_timeout = true;
                    self.rejoin(&mut actions);
                } else {
                    self.stop(false, VoiceSessionEvent::Disconnected(reason), &mut actions);
                }
            }
            ConnectionInput::OwnStateDropped { channel_id } if channel_id == self.channel_id => {
                self.own_state_dropped_deadline
                    .get_or_insert(now + OWN_STATE_DROPPED_GRACE);
            }
            ConnectionInput::MediaConnected { offer_id } if is_current(offer_id) => {
                self.media_connected(&mut actions);
            }
            ConnectionInput::IceDisconnected { offer_id } if is_current(offer_id) => {
                self.is_ice_connected = false;
                self.ice_grace_deadline
                    .get_or_insert(now + ICE_DISCONNECTED_GRACE);
            }
            ConnectionInput::MediaFailed { offer_id } if is_current(offer_id) => {
                self.ice_grace_deadline = None;
                self.connection_failed(&mut actions);
            }
            ConnectionInput::GatewayReady => self.rejoin(&mut actions),
            ConnectionInput::GatewayResumed => {
                if self.is_awaiting_gateway || !self.is_media_connected() {
                    // The offer carries the current mute and deafen.
                    self.rejoin(&mut actions);
                } else if self.has_unsent_voice_state {
                    actions.push(ConnectionAction::SendVoiceState);
                }
            }
            ConnectionInput::BackOnline => {
                if !self.is_media_connected() {
                    self.rejoin(&mut actions);
                }
            }
            ConnectionInput::Renegotiate => self.negotiate(&mut actions),
            ConnectionInput::MoveTo { channel_id } if channel_id != self.channel_id => {
                self.channel_id = channel_id;
                self.reconnect_attempts = 0;
                self.has_connected_once = false;
                self.has_retried_after_media_timeout = false;
                self.own_state_dropped_deadline = None;
                self.set_phase(VoicePhase::Connecting, &mut actions);
                self.negotiate(&mut actions);
            }
            _ => {}
        }
        actions
    }

    fn is_media_connected(&self) -> bool {
        self.phase == VoicePhase::Connected && self.is_ice_connected
    }

    fn set_phase(&mut self, phase: VoicePhase, actions: &mut Vec<ConnectionAction>) {
        if self.phase != phase {
            self.phase = phase;
            actions.push(ConnectionAction::Emit(VoiceSessionEvent::PhaseChanged(
                phase,
            )));
        }
    }

    fn negotiate(&mut self, actions: &mut Vec<ConnectionAction>) {
        self.offer_id = self.offer_id.wrapping_add(1);
        self.has_answer = false;
        self.is_ice_connected = false;
        self.connect_deadline = None;
        self.ice_grace_deadline = None;
        actions.push(ConnectionAction::Offer {
            channel_id: self.channel_id,
            offer_id: self.offer_id,
        });
    }

    fn rejoin(&mut self, actions: &mut Vec<ConnectionAction>) {
        if self.is_stopped {
            return;
        }
        let phase = if self.has_connected_once {
            VoicePhase::Reconnecting
        } else {
            VoicePhase::Connecting
        };
        self.set_phase(phase, actions);
        self.negotiate(actions);
    }

    fn media_connected(&mut self, actions: &mut Vec<ConnectionAction>) {
        self.is_ice_connected = true;
        self.ice_grace_deadline = None;
        self.connect_deadline = None;
        if self.phase == VoicePhase::Connected {
            return;
        }
        self.set_phase(VoicePhase::Connected, actions);
        self.reconnect_attempts = 0;
        // Audio flows again: a later media timeout deserves its own silent retry.
        self.has_retried_after_media_timeout = false;
        if !self.has_connected_once {
            self.has_connected_once = true;
            actions.push(ConnectionAction::Emit(VoiceSessionEvent::Connected));
        }
    }

    /// No answer, no audio, or audio lost: a few silent attempts, then the session says so.
    fn connection_failed(&mut self, actions: &mut Vec<ConnectionAction>) {
        if self.is_stopped {
            return;
        }
        self.connect_deadline = None;
        self.ice_grace_deadline = None;
        // A first join that fails is not retried in silence: the reason is usually lasting.
        if !self.has_connected_once || self.reconnect_attempts >= MAXIMUM_RECONNECT_ATTEMPTS {
            let problem = if self.has_connected_once {
                ConnectionProblem::ConnectionLost
            } else if self.has_answer {
                ConnectionProblem::Unreachable
            } else {
                ConnectionProblem::NoAnswer
            };
            self.stop(true, VoiceSessionEvent::ConnectionFailed(problem), actions);
            return;
        }
        self.reconnect_attempts += 1;
        self.rejoin(actions);
    }

    fn stop(
        &mut self,
        should_send_leave: bool,
        event: VoiceSessionEvent,
        actions: &mut Vec<ConnectionAction>,
    ) {
        self.is_stopped = true;
        self.connect_deadline = None;
        self.ice_grace_deadline = None;
        self.own_state_dropped_deadline = None;
        actions.push(ConnectionAction::Stop { should_send_leave });
        actions.push(ConnectionAction::Emit(event));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channel() -> Snowflake {
        Snowflake::from_raw(10)
    }

    /// A machine whose first offer went out at `start`.
    fn started(start: Instant) -> ConnectionMachine {
        let mut machine = ConnectionMachine::new(channel());
        assert_eq!(
            machine.start(),
            vec![ConnectionAction::Offer {
                channel_id: channel(),
                offer_id: 1,
            }]
        );
        machine.offer_sent(true, start);
        machine
    }

    /// A machine connected at `start`.
    fn connected(start: Instant) -> ConnectionMachine {
        let mut machine = started(start);
        machine.handle(ConnectionInput::AnswerApplied { offer_id: 1 }, start);
        let actions = machine.handle(ConnectionInput::MediaConnected { offer_id: 1 }, start);
        assert_eq!(
            actions,
            vec![
                ConnectionAction::Emit(VoiceSessionEvent::PhaseChanged(VoicePhase::Connected)),
                ConnectionAction::Emit(VoiceSessionEvent::Connected),
            ]
        );
        machine
    }

    fn offers(actions: &[ConnectionAction]) -> Vec<u32> {
        actions
            .iter()
            .filter_map(|action| match action {
                ConnectionAction::Offer { offer_id, .. } => Some(*offer_id),
                _ => None,
            })
            .collect()
    }

    fn failure(actions: &[ConnectionAction]) -> Option<ConnectionProblem> {
        actions.iter().find_map(|action| match action {
            ConnectionAction::Emit(VoiceSessionEvent::ConnectionFailed(problem)) => Some(*problem),
            _ => None,
        })
    }

    /// A codec change offers again in silence: no phase change, and the call stays connected
    /// once the new peer carries media.
    #[test]
    fn a_renegotiation_offers_again_without_a_reconnection() {
        let start = Instant::now();
        let mut machine = connected(start);
        let actions = machine.handle(ConnectionInput::Renegotiate, start);
        assert_eq!(
            actions,
            [ConnectionAction::Offer {
                channel_id: channel(),
                offer_id: 2,
            }]
        );
        machine.offer_sent(true, start);
        machine.handle(ConnectionInput::AnswerApplied { offer_id: 2 }, start);
        assert_eq!(
            machine.handle(ConnectionInput::MediaConnected { offer_id: 2 }, start),
            []
        );
        assert_eq!(machine.next_deadline(), None);
        machine.leave();
        assert_eq!(machine.handle(ConnectionInput::Renegotiate, start), []);
    }

    #[test]
    fn a_first_join_without_an_answer_gives_up_after_fifteen_seconds() {
        let start = Instant::now();
        let mut machine = started(start);
        assert_eq!(machine.next_deadline(), Some(start + MEDIA_CONNECT_TIMEOUT));
        assert_eq!(
            machine.on_deadlines(
                start + MEDIA_CONNECT_TIMEOUT.saturating_sub(Duration::from_millis(1))
            ),
            []
        );
        let actions = machine.on_deadlines(start + MEDIA_CONNECT_TIMEOUT);
        assert_eq!(failure(&actions), Some(ConnectionProblem::NoAnswer));
        assert!(actions.contains(&ConnectionAction::Stop {
            should_send_leave: true
        }));
        assert!(machine.is_stopped());
    }

    #[test]
    fn an_answer_without_media_is_an_unreachable_port() {
        let start = Instant::now();
        let mut machine = started(start);
        machine.handle(ConnectionInput::AnswerApplied { offer_id: 1 }, start);
        let actions = machine.on_deadlines(start + MEDIA_CONNECT_TIMEOUT);
        assert_eq!(failure(&actions), Some(ConnectionProblem::Unreachable));
    }

    #[test]
    fn a_lost_connection_re_offers_after_the_ice_grace_and_heals_without_a_cue() {
        let start = Instant::now();
        let mut machine = connected(start);
        let lost_at = start + Duration::from_secs(60);
        assert_eq!(
            machine.handle(ConnectionInput::IceDisconnected { offer_id: 1 }, lost_at),
            []
        );
        assert_eq!(
            machine.next_deadline(),
            Some(lost_at + ICE_DISCONNECTED_GRACE)
        );
        let actions = machine.on_deadlines(lost_at + ICE_DISCONNECTED_GRACE);
        assert_eq!(
            actions,
            vec![
                ConnectionAction::Emit(VoiceSessionEvent::PhaseChanged(VoicePhase::Reconnecting)),
                ConnectionAction::Offer {
                    channel_id: channel(),
                    offer_id: 2,
                },
            ]
        );
        // The old peer's late news means nothing now.
        assert_eq!(
            machine.handle(ConnectionInput::MediaConnected { offer_id: 1 }, lost_at),
            []
        );
        let actions = machine.handle(ConnectionInput::MediaConnected { offer_id: 2 }, lost_at);
        assert_eq!(
            actions,
            vec![ConnectionAction::Emit(VoiceSessionEvent::PhaseChanged(
                VoicePhase::Connected
            ))],
            "no second join cue"
        );
    }

    #[test]
    fn ice_that_heals_within_the_grace_keeps_the_connection() {
        let start = Instant::now();
        let mut machine = connected(start);
        machine.handle(ConnectionInput::IceDisconnected { offer_id: 1 }, start);
        assert_eq!(
            machine.handle(ConnectionInput::MediaConnected { offer_id: 1 }, start),
            []
        );
        assert_eq!(machine.next_deadline(), None);
    }

    #[test]
    fn five_silent_attempts_then_the_connection_is_lost() {
        let start = Instant::now();
        let mut machine = connected(start);
        let mut now = start;
        for attempt in 0..MAXIMUM_RECONNECT_ATTEMPTS {
            let offer_id = machine.offer_id();
            let actions = machine.handle(ConnectionInput::MediaFailed { offer_id }, now);
            assert_eq!(offers(&actions), vec![offer_id + 1], "attempt {attempt}");
            machine.offer_sent(true, now);
            now += MEDIA_CONNECT_TIMEOUT / 2;
        }
        let offer_id = machine.offer_id();
        let actions = machine.handle(ConnectionInput::MediaFailed { offer_id }, now);
        assert_eq!(failure(&actions), Some(ConnectionProblem::ConnectionLost));
    }

    #[test]
    fn a_reconnection_resets_the_attempts() {
        let start = Instant::now();
        let mut machine = connected(start);
        for _ in 0..10 {
            let offer_id = machine.offer_id();
            machine.handle(ConnectionInput::MediaFailed { offer_id }, start);
            let offer_id = machine.offer_id();
            machine.handle(ConnectionInput::MediaConnected { offer_id }, start);
        }
        assert!(!machine.is_stopped());
    }

    #[test]
    fn refusals_and_disconnections_stop_with_their_reason() {
        let start = Instant::now();
        let mut machine = started(start);
        let actions = machine.handle(
            ConnectionInput::Refused {
                channel_id: channel(),
                offer_id: 1,
                reason: VoiceJoinRefusal::ChannelFull,
            },
            start,
        );
        assert_eq!(
            actions,
            vec![
                ConnectionAction::Stop {
                    should_send_leave: true
                },
                ConnectionAction::Emit(VoiceSessionEvent::JoinRefused(
                    VoiceJoinRefusal::ChannelFull
                )),
            ]
        );
        let mut machine = connected(start);
        let actions = machine.handle(
            ConnectionInput::ServerDisconnected {
                channel_id: channel(),
                reason: VoiceDisconnectReason::ChannelDeleted,
            },
            start,
        );
        assert_eq!(
            actions,
            vec![
                ConnectionAction::Stop {
                    should_send_leave: false
                },
                ConnectionAction::Emit(VoiceSessionEvent::Disconnected(
                    VoiceDisconnectReason::ChannelDeleted
                )),
            ]
        );
    }

    #[test]
    fn a_refusal_of_an_older_offer_or_another_channel_is_ignored() {
        let start = Instant::now();
        let mut machine = connected(start);
        machine.handle(ConnectionInput::MediaFailed { offer_id: 1 }, start);
        for (channel_id, offer_id) in [(channel(), 1), (Snowflake::from_raw(11), 2)] {
            assert_eq!(
                machine.handle(
                    ConnectionInput::Refused {
                        channel_id,
                        offer_id,
                        reason: VoiceJoinRefusal::RateLimited,
                    },
                    start,
                ),
                []
            );
        }
    }

    #[test]
    fn a_media_timeout_is_retried_once_silently() {
        let start = Instant::now();
        let mut machine = connected(start);
        let timeout = ConnectionInput::ServerDisconnected {
            channel_id: channel(),
            reason: VoiceDisconnectReason::MediaTimeout,
        };
        assert_eq!(offers(&machine.handle(timeout, start)), vec![2]);
        let actions = machine.handle(timeout, start);
        assert!(
            actions.contains(&ConnectionAction::Emit(VoiceSessionEvent::Disconnected(
                VoiceDisconnectReason::MediaTimeout
            )))
        );
    }

    #[test]
    fn a_dropped_own_state_re_offers_after_a_second_unless_disconnected_meanwhile() {
        let start = Instant::now();
        let mut machine = connected(start);
        let dropped = ConnectionInput::OwnStateDropped {
            channel_id: channel(),
        };
        machine.handle(dropped, start);
        assert_eq!(
            offers(&machine.on_deadlines(start + OWN_STATE_DROPPED_GRACE)),
            vec![2]
        );

        let mut machine = connected(start);
        machine.handle(dropped, start);
        machine.handle(
            ConnectionInput::ServerDisconnected {
                channel_id: channel(),
                reason: VoiceDisconnectReason::MovedElsewhere,
            },
            start,
        );
        assert_eq!(machine.on_deadlines(start + OWN_STATE_DROPPED_GRACE), []);
    }

    #[test]
    fn the_gateway_and_the_network_coming_back_re_offer_only_when_needed() {
        let start = Instant::now();
        let mut machine = connected(start);
        assert_eq!(machine.handle(ConnectionInput::GatewayResumed, start), []);
        assert_eq!(machine.handle(ConnectionInput::BackOnline, start), []);
        machine.voice_state_sent(false);
        assert_eq!(
            machine.handle(ConnectionInput::GatewayResumed, start),
            vec![ConnectionAction::SendVoiceState]
        );
        assert_eq!(
            offers(&machine.handle(ConnectionInput::GatewayReady, start)),
            vec![2]
        );

        // Not connected: both re-offer.
        let mut machine = started(start);
        assert_eq!(
            offers(&machine.handle(ConnectionInput::BackOnline, start)),
            vec![2]
        );
        assert_eq!(
            offers(&machine.handle(ConnectionInput::GatewayResumed, start)),
            vec![3]
        );
    }

    #[test]
    fn an_offer_the_gateway_could_not_carry_waits_for_it_without_a_timer() {
        let start = Instant::now();
        let mut machine = ConnectionMachine::new(channel());
        machine.start();
        machine.offer_sent(false, start);
        assert_eq!(machine.next_deadline(), None);
        assert_eq!(
            offers(&machine.handle(ConnectionInput::GatewayResumed, start)),
            vec![2]
        );
    }

    #[test]
    fn an_offer_made_offline_waits_for_the_network_without_using_up_attempts() {
        let start = Instant::now();
        let mut machine = connected(start);
        machine.handle(ConnectionInput::IceDisconnected { offer_id: 1 }, start);
        let lost = start + ICE_DISCONNECTED_GRACE;
        assert_eq!(offers(&machine.on_deadlines(lost)), vec![2]);
        assert_eq!(machine.reconnect_attempts, 1);
        // The transport found no route for the new peer.
        machine.offer_sent(false, lost);
        assert_eq!(machine.next_deadline(), None);
        assert_eq!(
            offers(&machine.handle(ConnectionInput::BackOnline, lost)),
            vec![3]
        );
        assert_eq!(machine.reconnect_attempts, 1);
        assert!(!machine.is_stopped());
    }

    #[test]
    fn moving_offers_with_a_new_id_and_plays_the_join_cue_again() {
        let start = Instant::now();
        let mut machine = connected(start);
        let elsewhere = Snowflake::from_raw(11);
        let actions = machine.handle(
            ConnectionInput::MoveTo {
                channel_id: elsewhere,
            },
            start,
        );
        assert_eq!(
            actions,
            vec![
                ConnectionAction::Emit(VoiceSessionEvent::PhaseChanged(VoicePhase::Connecting)),
                ConnectionAction::Offer {
                    channel_id: elsewhere,
                    offer_id: 2,
                },
            ]
        );
        assert_eq!(
            machine.handle(
                ConnectionInput::MoveTo {
                    channel_id: elsewhere
                },
                start
            ),
            [],
            "already there"
        );
        let actions = machine.handle(ConnectionInput::MediaConnected { offer_id: 2 }, start);
        assert!(actions.contains(&ConnectionAction::Emit(VoiceSessionEvent::Connected)));
        // A failure in the new channel before it connected is not retried.
        let mut machine = connected(start);
        machine.handle(
            ConnectionInput::MoveTo {
                channel_id: elsewhere,
            },
            start,
        );
        let actions = machine.handle(ConnectionInput::MediaFailed { offer_id: 2 }, start);
        assert_eq!(failure(&actions), Some(ConnectionProblem::NoAnswer));
    }

    #[test]
    fn a_stopped_machine_does_nothing() {
        let start = Instant::now();
        let mut machine = started(start);
        machine.on_deadlines(start + MEDIA_CONNECT_TIMEOUT);
        assert_eq!(machine.handle(ConnectionInput::GatewayReady, start), []);
        assert_eq!(machine.on_deadlines(start + Duration::from_secs(600)), []);
    }
}
