//! Private calls (ADR-0062): a conversation's voice session, relayed by the SFU as a virtual voice
//! channel with the conversation's id. Who may take part and who rings are decided in `domain`
//! (`Messages::call_access`, `Messages::call_ringing_plan`, `Messages::conversation_call_members`);
//! the lifecycle is `domain::call_rules::CallLifecycle`; this module keeps the calls going on and
//! tells their conversations.
//!
//! Locks, always in this order: `call_starts` (async), `participants` (async), `calls` (std, never
//! held across an await nor while publishing).

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use mellow_protocol::{
    Call, CallParticipant, GatewayEvent, Snowflake, VoiceDisconnectReason, VoiceJoinRefusal,
};

use super::{AdmittedVoiceAccess, MediaCommand, Participant, VideoSource, VoiceScope};
use super::{VoiceService, VoiceShared};
use crate::domain::call_rules::CallLifecycle;
use crate::domain::messages::{CallAccess, CallMembers, MessageError, Messages};
use crate::events::{AccessChanges, EventHub};
use crate::http::rate_limit::CALL_RINGS_RECEIVED_FROM_STRANGERS;

/// How long the end of a call waits before its one retry, when its record could not be written.
const END_RECORD_RETRY_DELAY: Duration = Duration::from_secs(1);

/// A call going on in a conversation.
#[derive(Debug)]
pub(crate) struct ActiveCall {
    message_id: Snowflake,
    lifecycle: CallLifecycle,
    /// The conversation's participants, who receive the call's events.
    audience: Vec<Snowflake>,
    is_group: bool,
    /// Listed as rung like anyone else, but never on their own devices: a block either way with
    /// the caller, or their budget of rings from strangers spent. Kept here only, so nobody
    /// learns of a block (ADR-0062).
    silent_ids: HashSet<Snowflake>,
    /// The ring timeout's task, stopped when the call ends first.
    ring_timer: Option<tokio::task::AbortHandle>,
}

/// A call just ended, whose record is still to write.
#[derive(Debug)]
struct EndedCall {
    channel_id: Snowflake,
    message_id: Snowflake,
    joined_ids: Vec<Snowflake>,
}

/// Call events prepared under the `calls` lock and published once it is released.
#[derive(Debug, Default)]
#[must_use]
struct CallEvents(Vec<(Vec<Snowflake>, GatewayEvent)>);

impl CallEvents {
    fn publish(self, events: &EventHub) {
        for (user_ids, event) in self.0 {
            events.publish_to_users(&user_ids, &event);
        }
    }
}

/// What `recheck_calls` needs of a call, read under the lock.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CallSnapshot {
    channel_id: Snowflake,
    audience: Vec<Snowflake>,
    is_group: bool,
    caller_id: Snowflake,
    ringing_ids: Vec<Snowflake>,
}

/// Why a call cannot be started.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CallError {
    /// Not a conversation of the caller, or one they may no longer write in: like a hidden one.
    #[error("conversation not found")]
    NotFound,
    /// Voice is off on this instance, or its UDP port could not be opened.
    #[error("voice is unavailable")]
    VoiceUnavailable,
    /// As many calls go on as the instance has voice places (`[voice] max_participants`).
    #[error("too many calls on the instance")]
    LimitReached,
    /// The caller's previous call still rings, unanswered: one at a time.
    #[error("another call of the caller still rings")]
    AlreadyCalling,
    #[error(transparent)]
    Message(#[from] MessageError),
}

impl VoiceService {
    /// Starts a call in the conversation `channel_id`, or answers the one going on there: the
    /// caller then joins it with `voice_join`. Posts the call's message and tells the
    /// conversation (`CALL_CREATE`); the call rings until the ring timeout.
    pub(crate) async fn start_call(
        &self,
        caller_id: Snowflake,
        channel_id: Snowflake,
        now_unix_milliseconds: u64,
    ) -> Result<Call, CallError> {
        if !self.is_available() {
            return Err(CallError::VoiceUnavailable);
        }
        let shared = self.shared.clone();
        // A detached task, as a message send: a client hanging up mid-start must not leave a
        // posted message without its call.
        tokio::spawn(async move {
            shared
                .start_call(caller_id, channel_id, now_unix_milliseconds)
                .await
        })
        .await
        .map_err(|error| CallError::Message(MessageError::Internal(Box::new(error))))?
    }

    /// The user declines a call that rings them: it stops ringing on all their devices. Nothing
    /// for a call that does not ring them, so the answer tells nothing.
    pub(crate) async fn decline_call(&self, user_id: Snowflake, channel_id: Snowflake) {
        let shared = &self.shared;
        let mut participants = shared.participants.lock().await;
        let stopped = shared
            .calls()
            .get_mut(&channel_id)
            .is_some_and(|call| call.lifecycle.stop_ringing_for(user_id));
        if stopped {
            shared.after_ringing_change(&mut participants, channel_id);
        }
    }

    /// The calls going on in `channel_ids` (the user's conversations), as `user_id` sees them, for
    /// `Ready`.
    pub(crate) async fn calls_in(
        &self,
        user_id: Snowflake,
        channel_ids: &[Snowflake],
    ) -> Vec<Call> {
        let participants = self.shared.participants.lock().await;
        let calls = self.shared.calls();
        calls
            .iter()
            .filter(|(channel_id, _)| channel_ids.contains(channel_id))
            .map(|(channel_id, call)| view_for(&participants, *channel_id, call, user_id))
            .collect()
    }

    /// Ends every call going on and writes their records, before the server stops and closes the
    /// database (ADR-0062).
    pub(crate) async fn end_all_calls(&self) {
        let shared = &self.shared;
        let ended: Vec<EndedCall> = {
            let mut participants = shared.participants.lock().await;
            let channel_ids: Vec<Snowflake> = shared.calls().keys().copied().collect();
            channel_ids
                .into_iter()
                .filter_map(|channel_id| {
                    shared.take_ended_call(
                        &mut participants,
                        channel_id,
                        VoiceDisconnectReason::ServerStopping,
                    )
                })
                .collect()
        };
        for call in ended {
            record_end(&shared.messages, call).await;
        }
    }

    /// Ends ringing now, as the ring timeout does, for tests that cannot wait for it.
    #[cfg(test)]
    pub(crate) async fn end_ringing(&self, channel_id: Snowflake) {
        let message_id = self
            .shared
            .calls()
            .get(&channel_id)
            .map(|call| call.message_id);
        if let Some(message_id) = message_id {
            self.shared.ring_timed_out(channel_id, message_id).await;
        }
    }

    /// Calls started from now on ring for `ring_timeout`, for tests that end ringing themselves.
    #[cfg(test)]
    pub(crate) fn set_ring_timeout(&self, ring_timeout: Duration) {
        self.shared.ring_timeout_milliseconds.store(
            u64::try_from(ring_timeout.as_millis()).unwrap_or(u64::MAX),
            std::sync::atomic::Ordering::Relaxed,
        );
    }
}

impl VoiceShared {
    pub(super) fn calls(&self) -> MutexGuard<'_, HashMap<Snowflake, ActiveCall>> {
        // Every update is a single insert, remove or field change: consistent after a panic.
        self.calls.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(super) fn has_call(&self, channel_id: Snowflake) -> bool {
        self.calls().contains_key(&channel_id)
    }

    async fn start_call(
        self: Arc<Self>,
        caller_id: Snowflake,
        channel_id: Snowflake,
        now_unix_milliseconds: u64,
    ) -> Result<Call, CallError> {
        // Access is read under the start lock, which `recheck_calls` takes too: a block
        // committed meanwhile is seen by one or the other.
        let _call_start = self.call_starts.lock().await;
        let CallAccess::Allowed {
            participant_ids,
            is_group,
        } = self.messages.call_access(caller_id, channel_id).await?
        else {
            return Err(CallError::NotFound);
        };
        {
            let participants = self.participants.lock().await;
            let calls = self.calls();
            if let Some(call) = calls.get(&channel_id) {
                return Ok(view_for(&participants, channel_id, call, caller_id));
            }
            if calls.values().any(|call| {
                call.lifecycle.caller_id() == caller_id && !call.lifecycle.is_answered()
            }) {
                return Err(CallError::AlreadyCalling);
            }
            if calls.len() >= self.max_participants {
                return Err(CallError::LimitReached);
            }
        }
        let plan = self
            .messages
            .call_ringing_plan(caller_id, &participant_ids)
            .await?;
        let mut silent_ids = plan.silent;
        let now = Instant::now();
        for stranger_id in plan.strangers {
            let key = format!("call-ring-received:{stranger_id}");
            if !self
                .rate_limiter
                .record_event(&key, CALL_RINGS_RECEIVED_FROM_STRANGERS, now)
            {
                silent_ids.insert(stranger_id);
            }
        }
        let message = self
            .messages
            .post_call_message(
                caller_id,
                channel_id,
                participant_ids.clone(),
                now_unix_milliseconds,
            )
            .await?;
        let participants = self.participants.lock().await;
        let (caller_view, events) = {
            let mut calls = self.calls();
            let call = ActiveCall {
                message_id: message.id,
                lifecycle: CallLifecycle::new(caller_id, plan.rung),
                audience: participant_ids,
                is_group,
                silent_ids,
                ring_timer: Some(self.start_ring_timer(channel_id, message.id)),
            };
            let caller_view = view_for(&participants, channel_id, &call, caller_id);
            let events = views_for(
                &participants,
                channel_id,
                &call,
                &call.audience,
                GatewayEvent::CallCreate,
            );
            calls.insert(channel_id, call);
            (caller_view, events)
        };
        events.publish(&self.events);
        Ok(caller_view)
    }

    /// The ring timeout of the call of `channel_id` started with `message_id`: one task per call
    /// going on, and calls are capped, so bounded; stopped when the call ends first.
    fn start_ring_timer(
        self: &Arc<Self>,
        channel_id: Snowflake,
        message_id: Snowflake,
    ) -> tokio::task::AbortHandle {
        let shared = Arc::downgrade(self);
        let ring_timeout = Duration::from_millis(
            self.ring_timeout_milliseconds
                .load(std::sync::atomic::Ordering::Relaxed),
        );
        tokio::spawn(async move {
            tokio::time::sleep(ring_timeout).await;
            if let Some(shared) = shared.upgrade() {
                shared.ring_timed_out(channel_id, message_id).await;
            }
        })
        .abort_handle()
    }

    /// A join into the call of `channel_id`: allowed to anyone who may take part, who may speak
    /// and show a camera there (ADR-0062).
    pub(super) async fn call_join_access(
        &self,
        user_id: Snowflake,
        channel_id: Snowflake,
    ) -> Result<AdmittedVoiceAccess, VoiceJoinRefusal> {
        match self.messages.call_access(user_id, channel_id).await {
            Ok(CallAccess::Allowed { .. }) if self.has_call(channel_id) => {
                Ok(AdmittedVoiceAccess {
                    scope: VoiceScope::Call,
                    user_limit: None,
                    may_speak: true,
                    may_use_soundboard: false,
                })
            }
            Ok(_) => Err(VoiceJoinRefusal::MissingPermissions),
            Err(error) => {
                tracing::warn!(%error, "voice join: cannot read the conversation");
                Err(VoiceJoinRefusal::VoiceUnavailable)
            }
        }
    }

    /// `user_id` joined the call of `channel_id`: they stop ringing, and their first join is
    /// recorded at once. The caller announces it.
    pub(super) fn call_joined(
        &self,
        participants: &mut HashMap<Snowflake, Participant>,
        user_id: Snowflake,
        channel_id: Snowflake,
    ) {
        let (first_join_of, ended) = {
            let mut calls = self.calls();
            let Some(call) = calls.get_mut(&channel_id) else {
                return;
            };
            let is_first_join = !call.lifecycle.joined_ids().contains(&user_id);
            call.lifecycle.join(user_id);
            (
                is_first_join.then_some(call.message_id),
                call.lifecycle
                    .has_ended(connected_to(participants, channel_id)),
            )
        };
        if let Some(message_id) = first_join_of {
            let messages = self.messages.clone();
            // Detached: the voice lock must not wait on the writer; the end writes them all again.
            tokio::spawn(async move {
                if let Err(error) = messages.record_call_participant(message_id, user_id).await {
                    tracing::warn!(%error, "calls: cannot record who joined");
                }
            });
        }
        // Joining cannot end a call; checked so the lifecycle stays the only judge.
        if ended {
            self.end_call(participants, channel_id);
        }
    }

    /// Someone left the call of `channel_id` (already removed from `participants`): it ends or
    /// is announced again.
    pub(super) fn call_left(
        &self,
        participants: &mut HashMap<Snowflake, Participant>,
        channel_id: Snowflake,
    ) {
        self.after_ringing_change(participants, channel_id);
    }

    /// Ends the call if it is over, or tells its conversation how it is now.
    fn after_ringing_change(
        &self,
        participants: &mut HashMap<Snowflake, Participant>,
        channel_id: Snowflake,
    ) {
        let outcome = self.calls().get(&channel_id).map(|call| {
            if call
                .lifecycle
                .has_ended(connected_to(participants, channel_id))
            {
                None
            } else {
                Some(views_for(
                    participants,
                    channel_id,
                    call,
                    &call.audience,
                    GatewayEvent::CallUpdate,
                ))
            }
        });
        match outcome {
            Some(None) => self.end_call(participants, channel_id),
            Some(Some(events)) => events.publish(&self.events),
            None => {}
        }
    }

    /// `CALL_UPDATE` with the call as it is now, to its conversation.
    pub(super) fn publish_call_update(
        &self,
        participants: &HashMap<Snowflake, Participant>,
        channel_id: Snowflake,
    ) {
        let events = self.calls().get(&channel_id).map(|call| {
            views_for(
                participants,
                channel_id,
                call,
                &call.audience,
                GatewayEvent::CallUpdate,
            )
        });
        if let Some(events) = events {
            events.publish(&self.events);
        }
    }

    pub(super) fn publish_call_speaking(
        &self,
        channel_id: Snowflake,
        user_id: Snowflake,
        is_speaking: bool,
    ) {
        let audience = self
            .calls()
            .get(&channel_id)
            .map(|call| call.audience.clone());
        if let Some(audience) = audience {
            self.events.publish_to_users(
                &audience,
                &GatewayEvent::CallSpeaking {
                    channel_id,
                    user_id,
                    speaking: is_speaking,
                },
            );
        }
    }

    /// Ends the call and writes its record in the background (`MESSAGE_UPDATE`).
    fn end_call(&self, participants: &mut HashMap<Snowflake, Participant>, channel_id: Snowflake) {
        if let Some(ended) =
            self.take_ended_call(participants, channel_id, VoiceDisconnectReason::CallEnded)
        {
            let messages = self.messages.clone();
            // Detached: the voice lock must not wait on the writer.
            tokio::spawn(async move { record_end(&messages, ended).await });
        }
    }

    /// Removes the call: its ring timer stops, whoever is still connected is disconnected for
    /// `reason` (`call_ended`, or `server_stopping` on a stop) and the conversation gets
    /// `CALL_DELETE`. Its record is left to write.
    fn take_ended_call(
        &self,
        participants: &mut HashMap<Snowflake, Participant>,
        channel_id: Snowflake,
        reason: VoiceDisconnectReason,
    ) -> Option<EndedCall> {
        let call = self.calls().remove(&channel_id)?;
        if let Some(ring_timer) = &call.ring_timer {
            ring_timer.abort();
        }
        let remaining: Vec<Snowflake> = participants
            .iter()
            .filter(|(_, participant)| participant.channel_id == channel_id)
            .map(|(user_id, _)| *user_id)
            .collect();
        for user_id in remaining {
            if let Some(participant) = participants.remove(&user_id) {
                self.send_media_command(MediaCommand::Leave { user_id });
                self.events.publish_to_session(
                    participant.session,
                    &GatewayEvent::VoiceDisconnected { channel_id, reason },
                );
            }
        }
        self.events
            .publish_to_users(&call.audience, &GatewayEvent::CallDelete { channel_id });
        Some(EndedCall {
            channel_id,
            message_id: call.message_id,
            joined_ids: call.lifecycle.joined_ids().to_vec(),
        })
    }

    /// The call of `channel_id` started with `message_id` rang long enough: nobody rings any
    /// more, and the call ends if nobody answered, even if no one was ringing.
    pub(super) async fn ring_timed_out(&self, channel_id: Snowflake, message_id: Snowflake) {
        let mut participants = self.participants.lock().await;
        let is_this_call = self
            .calls()
            .get_mut(&channel_id)
            .filter(|call| call.message_id == message_id)
            .map(|call| call.lifecycle.stop_ringing())
            .is_some();
        if is_this_call {
            self.after_ringing_change(&mut participants, channel_id);
        }
    }

    /// Checks again the calls an access change may concern (ADR-0062): who lost access is
    /// disconnected and no longer rung, anyone with a new block with the caller rings silently,
    /// the group's newcomers learn of the call (`CALL_CREATE`) and those who went forget it
    /// (`CALL_DELETE`). The reads happen before the voice lock is taken (`admit` checks again
    /// whoever joins meanwhile), the changes under it. A read that fails refuses (fail closed).
    pub(super) async fn recheck_calls(&self, changes: &AccessChanges) {
        let _call_start = self.call_starts.lock().await;
        let snapshots: Vec<CallSnapshot> = self
            .calls()
            .iter()
            .map(|(channel_id, call)| CallSnapshot {
                channel_id: *channel_id,
                audience: call.audience.clone(),
                is_group: call.is_group,
                caller_id: call.lifecycle.caller_id(),
                ringing_ids: call.lifecycle.ringing_ids().to_vec(),
            })
            .collect();
        let mut checked = Vec::new();
        for snapshot in calls_to_recheck(snapshots, changes) {
            let members = match self
                .messages
                .conversation_call_members(snapshot.channel_id)
                .await
            {
                Ok(members) => members,
                Err(error) => {
                    tracing::warn!(%error, "calls: cannot read a conversation again");
                    CallMembers {
                        participant_ids: snapshot.audience.clone(),
                        allowed_ids: HashSet::new(),
                    }
                }
            };
            let blocked_ids = match self
                .messages
                .ids_with_a_block_with(snapshot.caller_id, &snapshot.ringing_ids)
                .await
            {
                Ok(blocked_ids) => blocked_ids,
                Err(error) => {
                    tracing::warn!(%error, "calls: cannot read blocks again");
                    snapshot.ringing_ids.iter().copied().collect()
                }
            };
            checked.push((snapshot.channel_id, members, blocked_ids));
        }
        if checked.is_empty() {
            return;
        }
        let mut participants = self.participants.lock().await;
        for (channel_id, members, blocked_ids) in checked {
            let lost_connection: Vec<Snowflake> = participants
                .iter()
                .filter(|(user_id, participant)| {
                    participant.channel_id == channel_id && !members.allowed_ids.contains(user_id)
                })
                .map(|(user_id, _)| *user_id)
                .collect();
            for user_id in lost_connection {
                self.disconnect(
                    &mut participants,
                    user_id,
                    Some(VoiceDisconnectReason::PermissionsChanged),
                )
                .await;
            }
            self.apply_recheck(&mut participants, channel_id, &members, &blocked_ids);
        }
    }

    /// Stops ringing those who may no longer take part, silences those with a new block, and
    /// gives the call its new audience.
    fn apply_recheck(
        &self,
        participants: &mut HashMap<Snowflake, Participant>,
        channel_id: Snowflake,
        members: &CallMembers,
        blocked_ids: &HashSet<Snowflake>,
    ) {
        let (ringing_changed, silence_changed, events) = {
            let mut calls = self.calls();
            // Ended meanwhile, by the disconnections above.
            let Some(call) = calls.get_mut(&channel_id) else {
                return;
            };
            let mut ringing_changed = false;
            let mut silence_changed = false;
            for user_id in call.lifecycle.ringing_ids().to_vec() {
                if !members.allowed_ids.contains(&user_id) {
                    ringing_changed |= call.lifecycle.stop_ringing_for(user_id);
                } else if blocked_ids.contains(&user_id) {
                    silence_changed |= call.silent_ids.insert(user_id);
                }
            }
            let gone: Vec<Snowflake> = call
                .audience
                .iter()
                .filter(|user_id| !members.participant_ids.contains(user_id))
                .copied()
                .collect();
            let newcomers: Vec<Snowflake> = members
                .participant_ids
                .iter()
                .filter(|user_id| !call.audience.contains(user_id))
                .copied()
                .collect();
            call.audience.clone_from(&members.participant_ids);
            let mut events = views_for(
                participants,
                channel_id,
                call,
                &newcomers,
                GatewayEvent::CallCreate,
            );
            events
                .0
                .push((gone, GatewayEvent::CallDelete { channel_id }));
            (ringing_changed, silence_changed, events)
        };
        events.publish(&self.events);
        if ringing_changed {
            self.after_ringing_change(participants, channel_id);
        } else if silence_changed {
            // Their devices stop ringing; everyone else's view is unchanged.
            self.publish_call_update(participants, channel_id);
        }
    }
}

/// The calls `changes` may concern: a group someone joined or left, a conversation holding both
/// people of a changed relationship, and every one-to-one call after a guild change (the only
/// shared guild may be gone; checking it is the same single read as checking membership).
fn calls_to_recheck(snapshots: Vec<CallSnapshot>, changes: &AccessChanges) -> Vec<CallSnapshot> {
    snapshots
        .into_iter()
        .filter(|snapshot| {
            changes.conversation_ids.contains(&snapshot.channel_id)
                || changes.relationship_pairs.iter().any(|(first, second)| {
                    snapshot.audience.contains(first) && snapshot.audience.contains(second)
                })
                || (!snapshot.is_group && !changes.guild_ids.is_empty())
        })
        .collect()
}

/// Writes the record of an ended call, retrying once after `END_RECORD_RETRY_DELAY`.
async fn record_end(messages: &Messages, ended: EndedCall) {
    for attempt in 0..2 {
        if attempt > 0 {
            tokio::time::sleep(END_RECORD_RETRY_DELAY).await;
        }
        let now = crate::domain::clock::now_unix_milliseconds();
        match messages
            .end_call_message(ended.channel_id, ended.message_id, &ended.joined_ids, now)
            .await
        {
            Ok(()) => return,
            Err(error) => tracing::warn!(%error, attempt, "calls: cannot record the end of a call"),
        }
    }
}

/// How many are connected to the call of `channel_id`.
fn connected_to(participants: &HashMap<Snowflake, Participant>, channel_id: Snowflake) -> usize {
    participants
        .values()
        .filter(|participant| participant.channel_id == channel_id)
        .count()
}

/// `wrap`ped views of the call for `recipients`: one event for all, except for those who ring
/// silently, whose own view leaves them out so their devices stay quiet.
fn views_for(
    participants: &HashMap<Snowflake, Participant>,
    channel_id: Snowflake,
    call: &ActiveCall,
    recipients: &[Snowflake],
    wrap: fn(Call) -> GatewayEvent,
) -> CallEvents {
    let view = call_view(participants, channel_id, call);
    let (silenced, loud): (Vec<Snowflake>, Vec<Snowflake>) =
        recipients.iter().copied().partition(|user_id| {
            call.silent_ids.contains(user_id) && view.ringing_user_ids.contains(user_id)
        });
    let mut events = Vec::with_capacity(silenced.len() + 1);
    for user_id in silenced {
        events.push((vec![user_id], wrap(without_ringing(view.clone(), user_id))));
    }
    if !loud.is_empty() {
        events.push((loud, wrap(view)));
    }
    CallEvents(events)
}

/// The call as `user_id` sees it.
fn view_for(
    participants: &HashMap<Snowflake, Participant>,
    channel_id: Snowflake,
    call: &ActiveCall,
    user_id: Snowflake,
) -> Call {
    let view = call_view(participants, channel_id, call);
    if call.silent_ids.contains(&user_id) {
        without_ringing(view, user_id)
    } else {
        view
    }
}

fn without_ringing(mut view: Call, user_id: Snowflake) -> Call {
    view.ringing_user_ids.retain(|id| *id != user_id);
    view
}

/// The call as its conversation sees it; participants in the order they came.
fn call_view(
    participants: &HashMap<Snowflake, Participant>,
    channel_id: Snowflake,
    call: &ActiveCall,
) -> Call {
    let mut connected: Vec<(&Snowflake, &Participant)> = participants
        .iter()
        .filter(|(_, participant)| participant.channel_id == channel_id)
        .collect();
    connected.sort_unstable_by_key(|(_, participant)| participant.connection_id);
    Call {
        channel_id,
        message_id: call.message_id,
        caller_id: call.lifecycle.caller_id(),
        ringing_user_ids: call.lifecycle.ringing_ids().to_vec(),
        participants: connected
            .into_iter()
            .map(|(user_id, participant)| CallParticipant {
                user_id: *user_id,
                self_mute: participant.self_state.self_mute,
                self_deaf: participant.self_state.self_deaf,
                self_video: participant.self_state.video == Some(VideoSource::Camera),
                self_stream: participant.self_state.video == Some(VideoSource::Screen),
                video_codec: participant.video_codec,
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ANA: Snowflake = Snowflake::from_raw(1);
    const BOB: Snowflake = Snowflake::from_raw(2);
    const CLEO: Snowflake = Snowflake::from_raw(3);

    fn snapshot(channel_id: u64, audience: &[Snowflake], is_group: bool) -> CallSnapshot {
        CallSnapshot {
            channel_id: Snowflake::from_raw(channel_id),
            audience: audience.to_vec(),
            is_group,
            caller_id: ANA,
            ringing_ids: Vec::new(),
        }
    }

    fn rechecked(changes: &AccessChanges) -> Vec<u64> {
        let snapshots = vec![
            snapshot(10, &[ANA, BOB], false),
            snapshot(20, &[ANA, BOB, CLEO], true),
            snapshot(30, &[BOB, CLEO], false),
        ];
        calls_to_recheck(snapshots, changes)
            .iter()
            .map(|snapshot| snapshot.channel_id.raw())
            .collect()
    }

    #[test]
    fn only_the_calls_a_change_may_concern_are_checked_again() {
        assert_eq!(rechecked(&AccessChanges::default()), Vec::<u64>::new());
        let block = AccessChanges {
            relationship_pairs: HashSet::from([(ANA, BOB)]),
            ..AccessChanges::default()
        };
        assert_eq!(rechecked(&block), [10, 20], "the calls holding both");
        let group_change = AccessChanges {
            conversation_ids: HashSet::from([Snowflake::from_raw(20)]),
            ..AccessChanges::default()
        };
        assert_eq!(rechecked(&group_change), [20]);
        let guild_change = AccessChanges {
            guild_ids: HashSet::from([Snowflake::from_raw(99)]),
            ..AccessChanges::default()
        };
        assert_eq!(
            rechecked(&guild_change),
            [10, 30],
            "one-to-one calls only: a group does not need a shared guild"
        );
    }
}
