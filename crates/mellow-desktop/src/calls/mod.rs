//! Private calls on the desktop (ADR-0062, DESIGN.md §5.6.3), what the web's `client.svelte.ts`
//! call methods and `call-alerts.svelte.ts` do: starting, joining and declining a conversation's
//! call, why the last start failed, and what a call does beyond the screen: the owner's ringtone
//! looped while a call rings the reader (Christmas's in December), a short cue instead when they
//! are already in a call, nothing under Do not disturb, the caller's waiting tone, and a system
//! notification with Answer and Decline while the window is in the background or closed.
//!
//! [`Calls`] lives with the app, not the window: closed, Mellow keeps running in the background
//! (ADR-0056), still rings, and the notification answers. Every sound stops when the call stops
//! ringing the reader, and at its 30 seconds at the latest even without an event: each sound is
//! only as long as the ringing may last, and a timer stops it then.
//!
//! The incoming call card, the join banner and the call's message are the shell's
//! (`shell/incoming_call.rs`, `conversation/call_banner.rs`, `messages/call_message.rs`).

mod sounds;
pub mod words;

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Context, Entity, Global, SharedString, Subscription, SystemNotification,
    SystemNotificationAction, Task,
};
use mellow_client_core::ClientError;
use mellow_client_core::ringtones::{Ringtone, ringtone_for_month};
use mellow_client_core::state::{
    CallPhase, CallSound, call_sound, ringing_ends_at_unix_milliseconds,
};
use mellow_media::LocalSound;
use mellow_protocol::{
    ApiErrorCode, CALL_RING_TIMEOUT_MILLISECONDS, Call, ChosenStatus, Snowflake,
};

use crate::i18n::strings;
use crate::preferences::PreferencesStore;
use crate::session::{Session, SessionPhase, StateChanged, now_unix_milliseconds};
use crate::voice::{Voice, voice};

/// A call's system notification: one per conversation.
const NOTIFICATION_TAG_PREFIX: &str = "mellow-call-";
const ANSWER_ACTION_ID: &str = "answer";
const DECLINE_ACTION_ID: &str = "decline";

/// Why starting a call failed (`callStartFailureOf` on the web), in the words the voice panel's
/// card uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallStartFailure {
    /// `CONFLICT`: an unanswered call of theirs still rings (one at a time).
    AlreadyCalling,
    RateLimited,
    /// `NOT_FOUND`: the conversation is not callable for them any more.
    Unreachable,
    /// `VALIDATION_FAILED`: voice is off on the instance.
    VoiceUnavailable,
    /// Anything else, the network included.
    Other,
}

impl CallStartFailure {
    #[must_use]
    pub fn of(error: &ClientError) -> Self {
        match error {
            ClientError::Api { code, .. } => match code {
                ApiErrorCode::Conflict => Self::AlreadyCalling,
                ApiErrorCode::RateLimited => Self::RateLimited,
                ApiErrorCode::NotFound => Self::Unreachable,
                ApiErrorCode::ValidationFailed => Self::VoiceUnavailable,
                _ => Self::Other,
            },
            _ => Self::Other,
        }
    }

    /// "Call again" is offered where a second try may work.
    #[must_use]
    pub const fn may_work_again(self) -> bool {
        matches!(self, Self::RateLimited | Self::Other)
    }
}

/// Which ringing a sound is for: a new call, or Do not disturb turned on or off, is a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CallSoundKey {
    channel_id: Snowflake,
    message_id: Snowflake,
    is_do_not_disturb: bool,
}

/// A sound playing, or waiting for its samples, until `ends_at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PlayingCallSound {
    key: CallSoundKey,
    sound: CallSound,
    ends_at_unix_milliseconds: u64,
    /// Handed to the sound player (a ringtone waits for its file to be decoded).
    has_started: bool,
}

/// Why a call could not start, shown in the voice panel's slot until dismissed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallStartProblem {
    pub channel_id: Snowflake,
    pub failure: CallStartFailure,
}

/// The samples the sounds are cut from, made once off the interface thread: the ringtone and
/// the waiting tone as long as a call may ring.
#[derive(Default)]
struct CallSoundSamples {
    ringtone: Option<(Ringtone, LocalSound)>,
    ringback: Option<LocalSound>,
    call_waiting: Option<LocalSound>,
    preparing: Option<Task<()>>,
}

/// The app's calls, as a global.
struct CallsGlobal(Entity<Calls>);

impl Global for CallsGlobal {}

/// Creates the app's calls, following `session`; after `voice::install_voice`.
pub fn install_calls(session: &Entity<Session>, cx: &mut App) -> Entity<Calls> {
    let calls = cx.new(|cx| Calls::new(session.clone(), Rc::new(now_unix_milliseconds), cx));
    cx.set_global(CallsGlobal(calls.clone()));
    calls
}

/// The app's calls on a clock a test moves.
#[cfg(test)]
pub(crate) fn install_calls_with_clock(
    session: &Entity<Session>,
    clock: Rc<dyn Fn() -> u64>,
    cx: &mut App,
) -> Entity<Calls> {
    let calls = cx.new(|cx| Calls::new(session.clone(), clock, cx));
    cx.set_global(CallsGlobal(calls.clone()));
    calls
}

/// The app's calls; `None` in tests of other views.
#[must_use]
pub fn calls(cx: &App) -> Option<Entity<Calls>> {
    cx.try_global::<CallsGlobal>()
        .map(|global| global.0.clone())
}

/// The conversation a call notification is about, from its tag.
#[must_use]
pub fn call_of_notification_tag(tag: &str) -> Option<Snowflake> {
    tag.strip_prefix(NOTIFICATION_TAG_PREFIX)?.parse().ok()
}

fn notification_tag(channel_id: Snowflake) -> String {
    format!("{NOTIFICATION_TAG_PREFIX}{channel_id}")
}

/// A call notification was pressed: Answer opens the window at the conversation and joins the
/// call, Decline stops the ringing, the notification itself opens the conversation. Mellow may
/// have run in the background, its window closed: the window opens first.
pub fn follow_call_notification(channel_id: Snowflake, action_id: Option<&str>, cx: &mut App) {
    let Some(calls) = calls(cx) else {
        return;
    };
    if action_id == Some(DECLINE_ACTION_ID) {
        calls.update(cx, |calls, cx| calls.decline_call(channel_id, cx));
        return;
    }
    crate::background::show_main_window(cx);
    let Some(window) = crate::background::main_window(cx) else {
        return;
    };
    let should_answer = action_id == Some(ANSWER_ACTION_ID);
    let shown = window.update(cx, |main_window, window, cx| {
        let Some(shell) = main_window.shell().cloned() else {
            return;
        };
        shell.update(cx, |shell, cx| {
            if should_answer {
                shell.answer_call(channel_id, window, cx);
            } else {
                shell.open_channel(channel_id, window, cx);
            }
        });
    });
    if shown.is_err() {
        tracing::debug!("the call notification's window is gone");
    }
}

pub struct Calls {
    session: Entity<Session>,
    start_problem: Option<CallStartProblem>,
    /// Conversations whose call start is waiting for the instance.
    starting_channel_ids: HashSet<Snowflake>,
    /// Calls declined on this device: silent at once, before the instance confirms.
    declined: HashSet<(Snowflake, Snowflake)>,
    incoming_sound: Option<PlayingCallSound>,
    ringback_sound: Option<PlayingCallSound>,
    samples: CallSoundSamples,
    /// The ringing call a notification was shown for.
    notified_call: Option<(Snowflake, Snowflake)>,
    /// Wakes when the earliest ringing ends, so the card and the sounds go on time.
    deadline: Option<(u64, Task<()>)>,
    /// When this device first saw each call ring, by its local clock: ringing is bounded by it
    /// (`ringing_end`), whatever the instance's clock says.
    first_rung_at: HashMap<(Snowflake, Snowflake), u64>,
    /// The call the card shows, to notify the shell when it changes.
    shown_ringing_key: Option<(Snowflake, Snowflake)>,
    /// Unix milliseconds now; tests move it by hand.
    clock: Rc<dyn Fn() -> u64>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for Calls {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Calls")
            .field("start_problem", &self.start_problem)
            .field("incoming_sound", &self.incoming_sound)
            .field("ringback_sound", &self.ringback_sound)
            .finish_non_exhaustive()
    }
}

impl Calls {
    pub(crate) fn new(
        session: Entity<Session>,
        clock: Rc<dyn Fn() -> u64>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut subscriptions = vec![
            cx.subscribe(&session, |calls, _, _: &StateChanged, cx| calls.refresh(cx)),
            cx.observe(&session, |calls, session, cx| {
                if session.read(cx).phase() == SessionPhase::SignedOut {
                    calls.end(cx);
                }
            }),
            // The output volume and the notification choices.
            cx.observe_global::<PreferencesStore>(Self::refresh),
        ];
        if let Some(voice) = voice(cx) {
            subscriptions.push(cx.observe(&voice, |calls, _, cx| calls.refresh(cx)));
        }
        Self {
            session,
            start_problem: None,
            starting_channel_ids: HashSet::new(),
            declined: HashSet::new(),
            incoming_sound: None,
            ringback_sound: None,
            samples: CallSoundSamples::default(),
            notified_call: None,
            deadline: None,
            first_rung_at: HashMap::new(),
            shown_ringing_key: None,
            clock,
            _subscriptions: subscriptions,
        }
    }

    // ---- What the interface reads ----

    /// The call ringing the reader now, oldest first, the one the incoming call card shows: not
    /// declined here, and within its 30 seconds.
    #[must_use]
    pub fn ringing_call(&self, cx: &App) -> Option<Call> {
        let now = (self.clock)();
        self.session
            .read(cx)
            .state()
            .incoming_calls()
            .find(|call| {
                !self.declined.contains(&(call.channel_id, call.message_id))
                    && now < self.ringing_end(call)
            })
            .cloned()
    }

    /// When `call` stops ringing here at the latest: the instance's 30 seconds from the call's
    /// start, but never more than 30 seconds after this device first saw it ring, and the full 30
    /// seconds from then when the instance's clock says it is already over (clocks drift).
    fn ringing_end(&self, call: &Call) -> u64 {
        let first_seen = self
            .first_rung_at
            .get(&(call.channel_id, call.message_id))
            .copied()
            .unwrap_or_else(|| (self.clock)());
        let local_end = first_seen.saturating_add(CALL_RING_TIMEOUT_MILLISECONDS);
        let instance_end = ringing_ends_at_unix_milliseconds(call);
        if instance_end > first_seen {
            instance_end.min(local_end)
        } else {
            local_end
        }
    }

    /// Notes when each call ringing now was first seen, and forgets calls gone.
    fn note_ringing_calls(&mut self, cx: &App) {
        let now = (self.clock)();
        let state = self.session.read(cx).state();
        let ringing: Vec<(Snowflake, Snowflake)> = state
            .calls()
            .iter()
            .filter(|call| !call.ringing_user_ids.is_empty())
            .map(|call| (call.channel_id, call.message_id))
            .collect();
        self.first_rung_at.retain(|key, _| ringing.contains(key));
        for key in ringing {
            self.first_rung_at.entry(key).or_insert(now);
        }
        let calls: Vec<(Snowflake, Snowflake)> = state
            .calls()
            .iter()
            .map(|call| (call.channel_id, call.message_id))
            .collect();
        // A declined call stays declined until it is gone: a late update never rings it again.
        self.declined.retain(|key| calls.contains(key));
    }

    #[must_use]
    pub fn start_problem(&self) -> Option<CallStartProblem> {
        self.start_problem
    }

    #[must_use]
    pub fn is_starting(&self, channel_id: Snowflake) -> bool {
        self.starting_channel_ids.contains(&channel_id)
    }

    /// The sound playing for a call ringing the reader, if any.
    #[must_use]
    pub fn incoming_sound(&self) -> Option<CallSound> {
        self.incoming_sound.map(|playing| playing.sound)
    }

    /// The waiting tone, while the reader's own call rings the others.
    #[must_use]
    pub fn is_ringback_playing(&self) -> bool {
        self.ringback_sound.is_some()
    }

    fn is_do_not_disturb(&self, cx: &App) -> bool {
        self.session
            .read(cx)
            .state()
            .current_user()
            .is_some_and(|user| user.status == ChosenStatus::DoNotDisturb)
    }

    // ---- Actions ----

    /// Starts the conversation's call (or answers the one going on), then joins it and shows it.
    /// A refusal goes to `start_problem`, which the voice panel shows, and to `on_done`.
    pub fn start_call(
        &mut self,
        channel_id: Snowflake,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<(), CallStartFailure>, &mut App) + 'static,
    ) {
        if let Some(voice) = voice(cx)
            && voice.read(cx).call_conversation_id() == Some(channel_id)
        {
            voice.update(cx, Voice::open_call_view);
            cx.defer(move |cx| on_done(Ok(()), cx));
            return;
        }
        if !self.starting_channel_ids.insert(channel_id) {
            return;
        }
        self.start_problem = None;
        if let Some(voice) = voice(cx) {
            voice.update(cx, Voice::dismiss_problem);
        }
        let calls = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.start_call(channel_id, cx, move |answer, cx| {
                let outcome = answer.map_err(|error| CallStartFailure::of(&error));
                let _ = calls.update(cx, |calls, cx| {
                    calls.starting_channel_ids.remove(&channel_id);
                    match outcome {
                        Ok(()) => calls.join_call(channel_id, cx),
                        Err(failure) => {
                            calls.start_problem = Some(CallStartProblem {
                                channel_id,
                                failure,
                            });
                        }
                    }
                    cx.notify();
                });
                on_done(outcome, cx);
            });
        });
        cx.notify();
    }

    /// Answering is joining (ADR-0062): this device holds the connection, the others stop
    /// ringing. The call view opens.
    pub fn join_call(&mut self, channel_id: Snowflake, cx: &mut Context<Self>) {
        self.start_problem = None;
        if let Some(voice) = voice(cx) {
            voice.update(cx, |voice, cx| {
                voice.join_call(channel_id, cx);
                voice.open_call_view(cx);
            });
        }
        cx.notify();
    }

    /// Stops the ringing here at once and on the reader's other devices; they may still join
    /// the call later.
    pub fn decline_call(&mut self, channel_id: Snowflake, cx: &mut Context<Self>) {
        let ringing = self
            .session
            .read(cx)
            .state()
            .call(channel_id)
            .map(|call| (call.channel_id, call.message_id));
        if let Some(ringing) = ringing {
            self.declined.insert(ringing);
        }
        self.session
            .update(cx, |session, cx| session.decline_call(channel_id, cx));
        self.refresh(cx);
        cx.notify();
    }

    pub fn dismiss_start_problem(&mut self, cx: &mut Context<Self>) {
        if self.start_problem.take().is_some() {
            cx.notify();
        }
    }

    /// Signed out: everything stops and is forgotten.
    fn end(&mut self, cx: &mut Context<Self>) {
        self.start_problem = None;
        self.starting_channel_ids.clear();
        self.declined.clear();
        self.first_rung_at.clear();
        self.shown_ringing_key = None;
        if let Some((channel_id, _)) = self.notified_call.take() {
            cx.dismiss_system_notification(&notification_tag(channel_id));
        }
        self.deadline = None;
        if self.incoming_sound.take().is_some() | self.ringback_sound.take().is_some() {
            Self::stop_looped_sounds(cx);
        }
        cx.notify();
    }

    // ---- Sounds and notifications ----

    /// The sound for the call ringing the reader: chosen when it starts ringing (the key), so
    /// answering, which puts the reader in a call a moment before it stops ringing them, never
    /// plays the call-waiting cue.
    fn wanted_incoming_sound(&self, cx: &App) -> Option<PlayingCallSound> {
        let call = self.ringing_call(cx)?;
        let is_do_not_disturb = self.is_do_not_disturb(cx);
        let key = CallSoundKey {
            channel_id: call.channel_id,
            message_id: call.message_id,
            is_do_not_disturb,
        };
        if let Some(playing) = self.incoming_sound.filter(|playing| playing.key == key) {
            return Some(playing);
        }
        let is_busy = self
            .session
            .read(cx)
            .state()
            .is_in_a_call_or_voice_channel()
            || voice(cx).is_some_and(|voice| voice.read(cx).channel_id().is_some());
        let sound = call_sound(CallPhase::Incoming, is_busy, is_do_not_disturb)?;
        Some(PlayingCallSound {
            key,
            sound,
            ends_at_unix_milliseconds: self.ringing_end(&call),
            has_started: false,
        })
    }

    /// The waiting tone while the reader's own call rings the others, Do not disturb or not (it
    /// is about being disturbed, not calling).
    fn wanted_ringback_sound(&self, cx: &App) -> Option<PlayingCallSound> {
        let conversation_id = voice(cx)?.read(cx).call_conversation_id()?;
        let state = self.session.read(cx).state();
        if state.call_phase(conversation_id) != Some(CallPhase::Outgoing) {
            return None;
        }
        let call = state.call(conversation_id)?;
        let ends_at = self.ringing_end(call);
        if (self.clock)() >= ends_at {
            return None;
        }
        let key = CallSoundKey {
            channel_id: call.channel_id,
            message_id: call.message_id,
            is_do_not_disturb: false,
        };
        if let Some(playing) = self.ringback_sound.filter(|playing| playing.key == key) {
            return Some(playing);
        }
        Some(PlayingCallSound {
            key,
            sound: CallSound::Ringback,
            ends_at_unix_milliseconds: ends_at,
            has_started: false,
        })
    }

    /// Plays, stops and notifies as the calls are now.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.note_ringing_calls(cx);
        let ringing_key = self
            .ringing_call(cx)
            .map(|call| (call.channel_id, call.message_id));
        let is_card_changed = ringing_key != self.shown_ringing_key;
        self.shown_ringing_key = ringing_key;
        let incoming = self.wanted_incoming_sound(cx);
        let ringback = self.wanted_ringback_sound(cx);
        let is_looped = |playing: Option<PlayingCallSound>| {
            playing.is_some_and(|playing| {
                playing.has_started && playing.sound != CallSound::CallWaiting
            })
        };
        let stops_looped = (is_looped(self.incoming_sound)
            && incoming.map(|wanted| wanted.key) != self.incoming_sound.map(|playing| playing.key))
            || (is_looped(self.ringback_sound)
                && ringback.map(|wanted| wanted.key)
                    != self.ringback_sound.map(|playing| playing.key));
        let changed = incoming != self.incoming_sound || ringback != self.ringback_sound;
        self.incoming_sound = incoming;
        self.ringback_sound = ringback;
        if stops_looped {
            // The player stops every looped call sound at once: the one kept starts again with
            // the time it has left.
            Self::stop_looped_sounds(cx);
            for playing in [&mut self.incoming_sound, &mut self.ringback_sound]
                .into_iter()
                .flatten()
            {
                if playing.sound != CallSound::CallWaiting {
                    playing.has_started = false;
                }
            }
        }
        self.start_waiting_sounds(cx);
        self.follow_notification(cx);
        self.wake_at_next_deadline(cx);
        if changed || is_card_changed {
            if let Some(playing) = self.incoming_sound {
                tracing::info!(sound = ?playing.sound, "a call rings");
            }
            cx.notify();
        }
    }

    fn stop_looped_sounds(cx: &mut App) {
        if let Some(voice) = voice(cx) {
            voice.read(cx).stop_call_sounds();
        }
        tracing::info!("call sounds stopped");
    }

    /// Hands the sounds not started yet to the player, once their samples are ready.
    fn start_waiting_sounds(&mut self, cx: &mut Context<Self>) {
        let now = (self.clock)();
        let month = crate::messages::local_date_time(now).date.month;
        let ringtone = ringtone_for_month(month);
        let mut is_waiting_for_samples = false;
        for slot in [&mut self.incoming_sound, &mut self.ringback_sound] {
            let Some(playing) = slot.as_mut().filter(|playing| !playing.has_started) else {
                continue;
            };
            let left = playing.ends_at_unix_milliseconds.saturating_sub(now);
            let samples = match playing.sound {
                CallSound::Ringtone => self
                    .samples
                    .ringtone
                    .as_ref()
                    .filter(|(kind, _)| *kind == ringtone)
                    .map(|(_, samples)| cut(samples, left)),
                CallSound::Ringback => self
                    .samples
                    .ringback
                    .as_ref()
                    .map(|samples| cut(samples, left)),
                CallSound::CallWaiting => self.samples.call_waiting.clone(),
            };
            let Some(samples) = samples else {
                is_waiting_for_samples = true;
                continue;
            };
            playing.has_started = true;
            tracing::info!(sound = ?playing.sound, milliseconds = left, "call sound started");
            if let Some(voice) = voice(cx) {
                let sound = playing.sound;
                voice.update(cx, |voice, cx| {
                    if sound == CallSound::CallWaiting {
                        voice.play_call_cue_sound(samples, cx);
                    } else {
                        voice.play_looped_call_sound(samples, cx);
                    }
                });
            }
        }
        if is_waiting_for_samples {
            self.prepare_samples(ringtone, cx);
        }
    }

    /// Decodes the ringtone and renders the tones, off the interface thread, once.
    fn prepare_samples(&mut self, ringtone: Ringtone, cx: &mut Context<Self>) {
        let has_ringtone = self
            .samples
            .ringtone
            .as_ref()
            .is_some_and(|(kind, _)| *kind == ringtone);
        if self.samples.preparing.is_some()
            || (has_ringtone
                && self.samples.ringback.is_some()
                && self.samples.call_waiting.is_some())
        {
            return;
        }
        let ringing_samples = sounds::samples_in_milliseconds(CALL_RING_TIMEOUT_MILLISECONDS);
        let made = cx.background_spawn(async move {
            let ringtone_samples = sounds::decode_ringtone(ringtone)
                .map(|decoded| sounds::looped_ringtone(&decoded, ringing_samples));
            (
                ringtone_samples,
                sounds::render_ringback(ringing_samples),
                sounds::render_call_waiting_cue(),
            )
        });
        self.samples.preparing = Some(cx.spawn(async move |calls, cx| {
            let (ringtone_samples, ringback, call_waiting) = made.await;
            let _ = calls.update(cx, |calls, cx| {
                calls.samples.preparing = None;
                // An unreadable ringtone rings with the call-waiting cue's notes instead.
                let ringtone_samples = ringtone_samples.unwrap_or_else(|| {
                    repeated(
                        &call_waiting,
                        ringing_samples,
                        sounds::SAMPLE_RATE as usize * 2,
                    )
                });
                calls.samples.ringtone = Some((ringtone, ringtone_samples));
                calls.samples.ringback = Some(ringback);
                calls.samples.call_waiting = Some(call_waiting);
                calls.start_waiting_sounds(cx);
            });
        }));
    }

    /// One wake-up at the earliest end of what rings now: the card and the sounds go then.
    fn wake_at_next_deadline(&mut self, cx: &mut Context<Self>) {
        let state = self.session.read(cx).state();
        let ringing_end = self.ringing_call(cx).map(|call| self.ringing_end(&call));
        let own_end = voice(cx)
            .and_then(|voice| voice.read(cx).call_conversation_id())
            .and_then(|conversation_id| state.call(conversation_id))
            .filter(|call| !call.ringing_user_ids.is_empty())
            .map(|call| self.ringing_end(call));
        let Some(next) = [ringing_end, own_end].into_iter().flatten().min() else {
            self.deadline = None;
            return;
        };
        if self.deadline.as_ref().is_some_and(|(at, _)| *at == next) {
            return;
        }
        let wait = Duration::from_millis(next.saturating_sub((self.clock)()));
        let task = cx.spawn(async move |calls, cx| {
            cx.background_executor().timer(wait).await;
            let _ = calls.update(cx, |calls, cx| {
                calls.deadline = None;
                calls.refresh(cx);
                // The card goes even when no sound changed (Do not disturb).
                cx.notify();
            });
        });
        self.deadline = Some((next, task));
    }

    /// The system notification while the window is in the background or closed, when desktop
    /// notifications are on: once per ringing call, gone when it stops ringing. Never under Do
    /// not disturb.
    fn follow_notification(&mut self, cx: &mut Context<Self>) {
        let ringing = self.ringing_call(cx);
        let ringing_key = ringing
            .as_ref()
            .map(|call| (call.channel_id, call.message_id));
        if self.notified_call != ringing_key
            && let Some((channel_id, _)) = self.notified_call.take()
        {
            cx.dismiss_system_notification(&notification_tag(channel_id));
        }
        let Some(call) = ringing else {
            return;
        };
        if self.notified_call == ringing_key || self.is_do_not_disturb(cx) {
            return;
        }
        let state = self.session.read(cx).state();
        let Some(user_id) = state.current_user().map(|user| user.id) else {
            return;
        };
        let Some(conversation) = state.direct_message_channel(call.channel_id) else {
            return;
        };
        let words = strings(cx);
        let sentence = words::incoming_call_sentence(&call, conversation, Some(user_id), words);
        let are_notifications_on = crate::notifier::notification_settings_of(user_id, cx)
            .are_desktop_notifications_enabled;
        // Marked even when not shown: a window brought to the front later shows the card.
        self.notified_call = ringing_key;
        if !are_notifications_on || crate::background::is_main_window_active(cx) {
            return;
        }
        let calls = words.calls();
        cx.show_system_notification(SystemNotification {
            tag: notification_tag(call.channel_id).into(),
            title: SharedString::from(crate::notifier::plain_text_for_the_system(&sentence)),
            body: calls.notification_body().into(),
            actions: vec![
                SystemNotificationAction {
                    id: ANSWER_ACTION_ID.into(),
                    label: calls.answer().into(),
                },
                SystemNotificationAction {
                    id: DECLINE_ACTION_ID.into(),
                    label: calls.decline().into(),
                },
            ],
        });
        tracing::info!("the call's notification is shown");
    }
}

/// The first `milliseconds` of `samples`.
fn cut(samples: &LocalSound, milliseconds: u64) -> LocalSound {
    let length = sounds::samples_in_milliseconds(milliseconds).min(samples.len());
    samples[..length].into()
}

/// `cue` every `period` samples over `length`: the ring of an unreadable ringtone.
fn repeated(cue: &[f32], length: usize, period: usize) -> LocalSound {
    let mut samples = vec![0.0_f32; length];
    for start in (0..length).step_by(period.max(1)) {
        for (sample, value) in samples.iter_mut().skip(start).zip(cue) {
            *sample = *value;
        }
    }
    samples.into()
}

#[cfg(test)]
pub(crate) mod tests;
