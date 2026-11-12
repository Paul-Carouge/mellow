//! Voice channels on the desktop (`voice-session.svelte.ts`, `voice.ts`, `voice-toggles.ts`,
//! ADR-0030, ADR-0051): one [`Voice`] entity for the app, kept as a GPUI global, joins and leaves
//! channels with `mellow_media::VoiceSession` (str0m), keeps the microphone and headset toggles,
//! the voice settings and each person's volume in the preferences, plays the cues and announces
//! who joins and leaves.
//!
//! Threads: the engine runs on its own threads; starting it (resolving the instance's address,
//! opening the devices) runs on the network runtime's blocking pool, and stopping it (joining its
//! threads) on a short-lived thread, never on the UI thread. Its events reach the UI a burst at a
//! time with one `notify` (a few per second at most: phases, the round trip every 2 s and
//! speaking edges, never audio frames). The engine's own event channel is unbounded: a stalled UI
//! only stops the forwarding thread, so later event sources must stay this rare.
//!
//! Only one start runs at a time: joining another channel while it runs retargets it, and leaving
//! keeps it aside so a join soon after reuses it. A second start could otherwise send a stale
//! `voice_leave` (it names no channel) that ends the newer call.
//!
//! How later work plugs in:
//! - **Voice settings section**: read `preferences(cx).voice_settings`, change it only through
//!   [`change_voice_settings`]; the entity notices and applies the difference to a live call
//!   (devices, input mode and key, processing, volumes) and to the sounds outside it. List the
//!   devices with [`Voice::audio_devices`] off the UI thread (tests inject synthetic ones). The
//!   push-to-talk key is a W3C code (`shared_logic::push_to_talk_keys`).
//! - **Global push-to-talk**: call [`Voice::press_push_to_talk`] and
//!   [`Voice::release_push_to_talk`] from the listener, as the in-window handler
//!   ([`Voice::push_to_talk_key_down`] and friends) does; both are idempotent, so a key heard by
//!   both while the window is focused presses once.
//! - **Soundboard**: [`Voice::engine`] plays sounds ([`mellow_media::VoiceSession::play_soundboard_sound`]),
//!   [`Voice::connected_place`] says where; the panel's button slot is marked in
//!   `shell/voice_panel.rs`.
//!   A peer's join sound replaces the peer-join cue in [`Voice::note_peer_change`].

pub mod media_settings;

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Context, Entity, Global, Modifiers, SharedString, Subscription, Task,
};
use mellow_client_core::GatewayHandle;
use mellow_client_core::shared_logic::push_to_talk_keys::{
    ModifierKey, PushToTalkKeyEffect, code_of_modifier, code_of_toolkit_key, push_to_talk_key_down,
    push_to_talk_key_up,
};
use mellow_client_core::shared_logic::voice_panel::{
    ParticipantChange, VoiceProblem, participant_change,
};
use mellow_client_core::shared_logic::voice_settings::{
    MAXIMUM_VOLUME_PERCENT, PersonVoiceSetting, StoredVoiceToggles, VoiceInputMode, VoiceSettings,
    is_character_key, is_remembered_device_present,
};
use mellow_client_core::state::StateChange;
use mellow_media::{
    AudioDeviceDescription, AudioDeviceError, AudioDevices, AudioDirection, ConnectionProblem,
    InMemoryPersonVoiceSettings, PersonSoundSource, PersonVoiceSettingsStore as _, SoundPlayer,
    SystemAudioDevices, VoiceCue, VoicePhase, VoiceSession, VoiceSessionEvent, VoiceSessionOptions,
    VoiceToggles,
};
use mellow_protocol::Snowflake;

use crate::preferences::{PreferencesStore, change_preferences, preferences};
use crate::session::{Session, StateChanged, VoiceSignal};
use crate::tokio_bridge::spawn_on_network_runtime;

use self::media_settings::{gain_of_percent, input_mode_of, person_setting_of, processing_of};

/// Bounds the forwarding queue to the UI; when it is full the forwarding thread waits (the
/// engine's own channel behind it is unbounded).
const VOICE_EVENT_QUEUE_CAPACITY: usize = 64;

/// Opens the audio devices; tests hand synthetic ones.
pub type AudioDevicesFactory = Arc<dyn Fn() -> Box<dyn AudioDevices> + Send + Sync>;

/// Where a call stands, as the panel says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallPhase {
    /// Until media flows in this channel for the first time.
    Connecting,
    Connected,
    /// Media flowed, then stopped: offering again in silence.
    Reconnecting,
}

/// The joined channel.
struct Call {
    channel_id: Snowflake,
    guild_id: Snowflake,
    phase: CallPhase,
    round_trip: Option<Duration>,
    is_self_speaking: bool,
    devices: DeviceNotices,
    /// The attempt whose start brings this call's engine.
    start_attempt: u64,
    /// The settings the engine was started with: what changed during the start is applied once
    /// it is claimed.
    start_settings: VoiceSettings,
    /// Shared with the engine: a person's volume set during the start reaches it.
    person_settings: Arc<InMemoryPersonVoiceSettings>,
    /// `None` while it starts.
    engine: Option<VoiceSession>,
    /// Brings the engine's events here; dropped with the call.
    event_pump: Option<Task<()>>,
}

pub struct Voice {
    session: Entity<Session>,
    devices: AudioDevicesFactory,
    toggles: VoiceToggles,
    /// The stored toggles are read once, when the preferences are.
    has_read_stored_toggles: bool,
    call: Option<Call>,
    /// A call left while its engine was still starting: a join reuses the start, else the engine
    /// is stopped when it arrives.
    abandoned_start: Option<Call>,
    problem: Option<VoiceProblem>,
    /// Who joined or left the call last, read out politely.
    announcement: SharedString,
    is_push_to_talk_pressed: bool,
    /// Modifier keys held at the last change, to tell a press from a release.
    held_modifiers: Modifiers,
    /// The settings last applied to the engine and the sounds.
    applied_settings: VoiceSettings,
    /// Sounds outside a call: the leave cue, toggles, the chime. Started at the first one.
    sound_player: Option<SoundPlayer>,
    /// Bumped by each join: a start that finishes after a leave or another join is stopped.
    attempt: u64,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for Voice {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Voice")
            .field("toggles", &self.toggles)
            .field("channel_id", &self.channel_id())
            .field("phase", &self.phase())
            .field("problem", &self.problem)
            .finish_non_exhaustive()
    }
}

/// The app's voice, as a global.
struct VoiceGlobal(Entity<Voice>);

impl Global for VoiceGlobal {}

/// Creates the app's voice, following `session`.
pub fn install_voice(
    session: &Entity<Session>,
    devices: AudioDevicesFactory,
    cx: &mut App,
) -> Entity<Voice> {
    let voice = cx.new(|cx| Voice::new(session.clone(), devices, cx));
    cx.set_global(VoiceGlobal(voice.clone()));
    voice
}

/// The system's devices.
#[must_use]
pub fn system_audio_devices() -> AudioDevicesFactory {
    Arc::new(|| Box::new(SystemAudioDevices))
}

/// The app's voice; `None` before [`install_voice`] (tests of other views).
#[must_use]
pub fn voice(cx: &App) -> Option<Entity<Voice>> {
    cx.try_global::<VoiceGlobal>()
        .map(|global| global.0.clone())
}

/// Changes the voice settings and saves them; a live call follows.
pub fn change_voice_settings(cx: &mut App, change: impl FnOnce(&mut VoiceSettings) + 'static) {
    change_preferences(cx, move |preferences| {
        change(&mut preferences.voice_settings);
    });
}

/// The message chime: through the call when there is one (the echo canceller hears it), else
/// on the chosen output.
pub fn play_notification_chime(cx: &mut App) {
    // No app voice: a test of another view, which plays nothing.
    if let Some(voice) = voice(cx) {
        voice.update(cx, |voice, _| voice.play_notification_chime());
    }
}

fn stored_toggles(toggles: VoiceToggles) -> StoredVoiceToggles {
    StoredVoiceToggles {
        is_microphone_muted: toggles.is_muted(),
        is_deafened: toggles.is_deafened(),
        was_microphone_muted_before_deafening: toggles.was_muted_before_deafening(),
    }
}

fn current_settings(cx: &App) -> VoiceSettings {
    preferences(cx).map_or(VoiceSettings::DEFAULT, |preferences| {
        preferences.voice_settings.clone()
    })
}

/// The instance's IPv4 address (its media socket listens on IPv4 only).
async fn instance_ipv4_address(host: String) -> Option<Ipv4Addr> {
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_owned();
    if let Ok(address) = host.parse::<IpAddr>() {
        return match address {
            IpAddr::V4(address) => Some(address),
            IpAddr::V6(_) => None,
        };
    }
    tokio::net::lookup_host((host.as_str(), 0))
        .await
        .ok()?
        .find_map(|address| match address.ip() {
            IpAddr::V4(address) => Some(address),
            IpAddr::V6(_) => None,
        })
}

/// What starting the engine found.
struct StartedEngine {
    engine: VoiceSession,
    events: std::sync::mpsc::Receiver<VoiceSessionEvent>,
    is_input_device_missing: bool,
    is_output_device_missing: bool,
}

/// What starting an engine needs.
struct EngineStart {
    host: String,
    channel_id: Snowflake,
    own_user_id: Snowflake,
    settings: VoiceSettings,
    toggles: VoiceToggles,
    devices: Box<dyn AudioDevices>,
    gateway: GatewayHandle,
    store: Arc<InMemoryPersonVoiceSettings>,
}

/// Resolves the instance, lists the devices (to tell a chosen one unplugged) and starts the
/// engine, on the network runtime; opening devices blocks, so it runs on the blocking pool.
async fn start_engine(start: EngineStart) -> Option<StartedEngine> {
    let EngineStart {
        host,
        channel_id,
        own_user_id,
        settings,
        toggles,
        devices,
        gateway,
        store,
    } = start;
    let Some(server_address) = instance_ipv4_address(host).await else {
        tracing::warn!("the instance has no IPv4 address for voice");
        return None;
    };
    let mut options = VoiceSessionOptions::new(channel_id, own_user_id, server_address);
    options.toggles = toggles;
    options.input_mode = input_mode_of(&settings);
    options.processing = processing_of(&settings);
    options.output_volume = gain_of_percent(settings.output_volume_percent);
    options.soundboard_volume = gain_of_percent(settings.soundboard_volume_percent);
    options
        .input_device_id
        .clone_from(&settings.input_device_id);
    options
        .output_device_id
        .clone_from(&settings.output_device_id);
    let started = tokio::task::spawn_blocking(move || {
        let device_ids = |listed: Result<Vec<AudioDeviceDescription>, AudioDeviceError>| {
            listed.ok().map(|devices| {
                devices
                    .into_iter()
                    .map(|device| device.id)
                    .collect::<Vec<_>>()
            })
        };
        let inputs = device_ids(devices.input_devices());
        let outputs = device_ids(devices.output_devices());
        let signalling = Box::new(move |frame| gateway.send(frame));
        VoiceSession::join(options, devices, signalling, store).map(|(engine, events)| {
            StartedEngine {
                engine,
                events,
                is_input_device_missing: !is_remembered_device_present(
                    inputs.as_deref(),
                    settings.input_device_id.as_deref(),
                ),
                is_output_device_missing: !is_remembered_device_present(
                    outputs.as_deref(),
                    settings.output_device_id.as_deref(),
                ),
            }
        })
    })
    .await;
    match started {
        Ok(Ok(started)) => Some(started),
        Ok(Err(error)) => {
            tracing::warn!(%error, "voice could not start");
            None
        }
        Err(error) => {
            tracing::warn!(%error, "voice's start was cut short");
            None
        }
    }
}

/// Stops an engine off the UI thread: dropping it joins its threads. `should_tell_the_server`
/// sends `voice_leave` first.
fn stop_engine(engine: VoiceSession, should_tell_the_server: bool) {
    let stop = move || {
        if should_tell_the_server {
            engine.leave();
        } else {
            drop(engine);
        }
    };
    if let Err(error) = std::thread::Builder::new()
        .name("mellow-voice-stop".into())
        .spawn(stop)
    {
        // The engine was moved into the closure that failed to start, and dropped with it.
        tracing::warn!(error = %error.kind(), "cannot start the thread stopping voice");
    }
}

impl Voice {
    fn new(session: Entity<Session>, devices: AudioDevicesFactory, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![
            cx.subscribe(&session, |voice, _, signal: &VoiceSignal, cx| {
                voice.follow_gateway(signal, cx);
            }),
            cx.subscribe(&session, |voice, _, changes: &StateChanged, cx| {
                voice.follow_state(&changes.0, cx);
            }),
            cx.observe_global::<PreferencesStore>(Self::follow_preferences),
        ];
        let mut voice = Self {
            session,
            devices,
            toggles: VoiceToggles::default(),
            has_read_stored_toggles: false,
            call: None,
            abandoned_start: None,
            problem: None,
            announcement: SharedString::default(),
            is_push_to_talk_pressed: false,
            held_modifiers: Modifiers::default(),
            applied_settings: VoiceSettings::DEFAULT,
            sound_player: None,
            attempt: 0,
            _subscriptions: subscriptions,
        };
        voice.follow_preferences(cx);
        voice
    }

    // ---- What the interface reads ----

    /// The joined channel, also while connecting.
    #[must_use]
    pub fn channel_id(&self) -> Option<Snowflake> {
        self.call.as_ref().map(|call| call.channel_id)
    }

    /// The joined channel and its server.
    #[must_use]
    pub fn connected_place(&self) -> Option<(Snowflake, Snowflake)> {
        self.call
            .as_ref()
            .map(|call| (call.guild_id, call.channel_id))
    }

    #[must_use]
    pub fn phase(&self) -> Option<CallPhase> {
        self.call.as_ref().map(|call| call.phase)
    }

    /// The round trip to the instance, once measured.
    #[must_use]
    pub fn round_trip(&self) -> Option<Duration> {
        self.call.as_ref().and_then(|call| call.round_trip)
    }

    /// The user's own speaking ring (the server never reports a user to themselves).
    #[must_use]
    pub fn is_self_speaking(&self) -> bool {
        self.call.as_ref().is_some_and(|call| call.is_self_speaking)
    }

    #[must_use]
    pub fn toggles(&self) -> VoiceToggles {
        self.toggles
    }

    #[must_use]
    pub fn problem(&self) -> Option<VoiceProblem> {
        self.problem
    }

    /// Who joined or left the call last.
    #[must_use]
    pub fn announcement(&self) -> SharedString {
        self.announcement.clone()
    }

    /// What the panel says about devices: a chosen one unplugged, or none at all.
    #[must_use]
    pub fn device_notices(&self) -> DeviceNotices {
        self.call
            .as_ref()
            .map_or_else(DeviceNotices::default, |call| call.devices)
    }

    /// The live engine, once started: the soundboard plays through it.
    #[must_use]
    pub fn engine(&self) -> Option<&VoiceSession> {
        self.call.as_ref().and_then(|call| call.engine.as_ref())
    }

    /// The devices, to list them off the UI thread (enumerating blocks).
    #[must_use]
    pub fn audio_devices(&self) -> Box<dyn AudioDevices> {
        (self.devices)()
    }

    /// The volume and local mute the reader gave `user_id`.
    #[must_use]
    pub fn person_setting(user_id: Snowflake, cx: &App) -> PersonVoiceSetting {
        preferences(cx).map_or_else(PersonVoiceSetting::default, |preferences| {
            preferences.voice_person(user_id)
        })
    }

    // ---- Joining and leaving ----

    /// Joins `channel_id` (a click on its row). Joining another channel while in one moves
    /// there with the same microphone, as the web does.
    pub fn join(&mut self, channel_id: Snowflake, cx: &mut Context<Self>) {
        if self.channel_id() == Some(channel_id) {
            return;
        }
        let session = self.session.read(cx);
        let state = session.state();
        let Some(guild_id) = state.guild_of_channel(channel_id).map(|guild| guild.id) else {
            return;
        };
        if !state.is_voice_enabled() {
            self.problem = Some(VoiceProblem::Refused {
                channel_id,
                reason: mellow_protocol::VoiceJoinRefusal::VoiceUnavailable,
            });
            cx.notify();
            return;
        }
        self.problem = None;
        self.announcement = SharedString::default();
        if self.call.is_none() {
            self.call = self.abandoned_start.take();
        }
        if let Some(call) = self.call.as_mut() {
            // Connected: move there. Still starting: the start is retargeted, and the engine
            // moves when it is claimed.
            if let Some(engine) = call.engine.as_mut() {
                engine.move_to(channel_id);
            }
            call.channel_id = channel_id;
            call.guild_id = guild_id;
            call.phase = CallPhase::Connecting;
            call.round_trip = None;
            call.is_self_speaking = false;
            cx.notify();
            return;
        }
        let (Some(gateway), Some(own_user_id), Some(host)) = (
            session.gateway_handle(),
            state.current_user().map(|user| user.id),
            session
                .rest_client()
                .map(|rest| rest.instance().host().to_owned()),
        ) else {
            return;
        };
        self.attempt += 1;
        let attempt = self.attempt;
        let settings = current_settings(cx);
        self.applied_settings = settings.clone();
        let store = Arc::new(InMemoryPersonVoiceSettings::default());
        if let Some(preferences) = preferences(cx) {
            for (user_id, setting) in &preferences.voice_people {
                store.write_person_setting(
                    PersonSoundSource::Voice,
                    *user_id,
                    person_setting_of(*setting),
                );
            }
        }
        let start = spawn_on_network_runtime(
            cx,
            start_engine(EngineStart {
                host,
                channel_id,
                own_user_id,
                settings: settings.clone(),
                toggles: self.toggles,
                devices: (self.devices)(),
                gateway,
                store: store.clone(),
            }),
        );
        // Detached, not kept: a start finishing after a leave still has to be stopped, with a
        // `voice_leave` (its offer may have gone out).
        cx.spawn(async move |this, cx| {
            let started = start.await.ok().flatten();
            let unclaimed = this
                .update(cx, |voice, cx| {
                    voice.take_started_engine(attempt, started, cx)
                })
                .unwrap_or_else(|_| None);
            if let Some(started) = unclaimed {
                stop_engine(started.engine, true);
            }
        })
        .detach();
        self.call = Some(Call {
            channel_id,
            guild_id,
            phase: CallPhase::Connecting,
            round_trip: None,
            is_self_speaking: false,
            devices: DeviceNotices::default(),
            start_attempt: attempt,
            start_settings: settings,
            person_settings: store,
            engine: None,
            event_pump: None,
        });
        cx.notify();
    }

    /// The engine started: kept if the call it was started for is still wanted, else handed back
    /// to be stopped.
    fn take_started_engine(
        &mut self,
        attempt: u64,
        started: Option<StartedEngine>,
        cx: &mut Context<Self>,
    ) -> Option<StartedEngine> {
        let is_wanted = self
            .call
            .as_ref()
            .is_some_and(|call| call.engine.is_none() && call.start_attempt == attempt);
        if !is_wanted {
            if self
                .abandoned_start
                .as_ref()
                .is_some_and(|call| call.start_attempt == attempt)
            {
                self.abandoned_start = None;
            }
            return started;
        }
        let Some(started) = started else {
            let channel_id = self.channel_id();
            self.call = None;
            self.is_push_to_talk_pressed = false;
            self.problem = channel_id.map(|channel_id| VoiceProblem::CouldNotStart { channel_id });
            cx.notify();
            return None;
        };
        let event_pump = Self::pump_engine_events(self.attempt, started.events, cx);
        let toggles = self.toggles;
        if let Some(call) = self.call.as_mut() {
            let mut engine = started.engine;
            // A move asked for while it started: the engine joined the first channel.
            if engine.channel_id() != call.channel_id {
                engine.move_to(call.channel_id);
            }
            // Toggles changed during the start never reached it: a mute must hold.
            engine.set_self_deafened(toggles.is_deafened());
            engine.set_self_muted(toggles.is_muted());
            call.engine = Some(engine);
            call.event_pump = event_pump;
            call.devices.is_input_device_missing = started.is_input_device_missing;
            call.devices.is_output_device_missing = started.is_output_device_missing;
            // Settings changed during the start were never applied to it.
            self.applied_settings = call.start_settings.clone();
        }
        self.apply_settings(cx);
        cx.notify();
        None
    }

    /// Brings the engine's events to the UI thread through a bounded channel, a burst at a time.
    fn pump_engine_events(
        attempt: u64,
        events: std::sync::mpsc::Receiver<VoiceSessionEvent>,
        cx: &mut Context<Self>,
    ) -> Option<Task<()>> {
        let (sender, mut receiver) = tokio::sync::mpsc::channel(VOICE_EVENT_QUEUE_CAPACITY);
        // The engine's channel is a blocking one: a small thread moves its events over. It ends
        // when the engine's threads drop their senders, or when the UI side is gone.
        let forward = move || {
            while let Ok(event) = events.recv() {
                if sender.blocking_send(event).is_err() {
                    break;
                }
            }
        };
        if let Err(error) = std::thread::Builder::new()
            .name("mellow-voice-events".into())
            .spawn(forward)
        {
            tracing::warn!(error = %error.kind(), "cannot start the voice event thread");
            return None;
        }
        Some(cx.spawn(async move |this, cx| {
            while let Some(first_event) = receiver.recv().await {
                let mut burst = vec![first_event];
                while let Ok(event) = receiver.try_recv() {
                    burst.push(event);
                }
                let is_gone = this
                    .update(cx, |voice, cx| {
                        if voice.attempt == attempt {
                            for event in burst {
                                voice.follow_engine(&event, cx);
                            }
                            cx.notify();
                        }
                    })
                    .is_err();
                if is_gone {
                    break;
                }
            }
        }))
    }

    fn follow_engine(&mut self, event: &VoiceSessionEvent, cx: &mut Context<Self>) {
        let Some(call) = self.call.as_mut() else {
            return;
        };
        let channel_id = call.channel_id;
        match event {
            VoiceSessionEvent::Connected => {
                call.phase = CallPhase::Connected;
                if let Some(engine) = call.engine.as_ref() {
                    engine.play_cue(VoiceCue::SelfJoin);
                }
            }
            VoiceSessionEvent::PhaseChanged(phase) => {
                call.phase = match *phase {
                    VoicePhase::Connecting => CallPhase::Connecting,
                    VoicePhase::Connected => CallPhase::Connected,
                    VoicePhase::Reconnecting => CallPhase::Reconnecting,
                };
            }
            VoiceSessionEvent::RoundTrip(round_trip) => call.round_trip = Some(*round_trip),
            VoiceSessionEvent::SelfSpeaking(is_speaking) => call.is_self_speaking = *is_speaking,
            VoiceSessionEvent::DeviceUnavailable(AudioDirection::Input) => {
                call.devices.is_microphone_unavailable = true;
            }
            VoiceSessionEvent::DeviceUnavailable(AudioDirection::Output) => {
                call.devices.is_output_unavailable = true;
            }
            VoiceSessionEvent::JoinRefused(reason) => {
                let reason = *reason;
                self.end_with(VoiceProblem::Refused { channel_id, reason }, cx);
            }
            VoiceSessionEvent::Disconnected(reason) => {
                let reason = *reason;
                self.end_with(VoiceProblem::Disconnected { channel_id, reason }, cx);
            }
            VoiceSessionEvent::ConnectionFailed(problem) => {
                let problem = match *problem {
                    ConnectionProblem::NoAnswer => VoiceProblem::NoAnswer { channel_id },
                    ConnectionProblem::Unreachable => VoiceProblem::Unreachable { channel_id },
                    ConnectionProblem::ConnectionLost => {
                        VoiceProblem::ConnectionLost { channel_id }
                    }
                };
                self.end_with(problem, cx);
            }
            // Cameras and screens come with the call view (M3 step 8).
            VoiceSessionEvent::VideoRefused(_)
            | VoiceSessionEvent::VideoStoppedByServer
            | VoiceSessionEvent::VideoPaused { .. } => {}
        }
    }

    /// The engine stopped by itself (it told the server when it had to).
    fn end_with(&mut self, problem: VoiceProblem, cx: &mut Context<Self>) {
        self.stop_call(false);
        self.problem = Some(problem);
        cx.notify();
    }

    /// Leaves the call (the panel's Disconnect).
    pub fn leave(&mut self, cx: &mut Context<Self>) {
        if self.call.is_none() {
            return;
        }
        self.stop_call(true);
        self.play_cue_outside_a_call(VoiceCue::SelfLeave, cx);
        cx.notify();
    }

    fn stop_call(&mut self, should_tell_the_server: bool) {
        self.attempt += 1;
        self.is_push_to_talk_pressed = false;
        self.announcement = SharedString::default();
        match self.call.take() {
            Some(Call {
                engine: Some(engine),
                ..
            }) => stop_engine(engine, should_tell_the_server),
            // Still starting: kept aside, its engine is stopped (or reused) when it arrives.
            Some(call) if should_tell_the_server => self.abandoned_start = Some(call),
            Some(_) | None => {}
        }
    }

    /// Joins the channel of the problem again (its Try again).
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        if let Some(problem) = self.problem.take() {
            self.join(problem.channel_id(), cx);
        }
    }

    pub fn dismiss_problem(&mut self, cx: &mut Context<Self>) {
        self.problem = None;
        cx.notify();
    }

    // ---- The gateway and the state ----

    fn follow_gateway(&mut self, signal: &VoiceSignal, cx: &mut Context<Self>) {
        if let VoiceSignal::SessionEnded = signal {
            // Signed out: the gateway is gone, so nothing is sent.
            self.stop_call(false);
            self.abandoned_start = None;
            self.problem = None;
            cx.notify();
            return;
        }
        let Some(engine) = self.engine() else {
            return;
        };
        match signal {
            VoiceSignal::Event(event) => engine.handle_gateway_event(event),
            VoiceSignal::Ready => engine.note_gateway_ready(),
            VoiceSignal::Resumed => engine.note_gateway_resumed(),
            VoiceSignal::SessionEnded => {}
        }
    }

    fn follow_state(&mut self, changes: &[StateChange], cx: &mut Context<Self>) {
        for change in changes {
            if let StateChange::VoiceParticipant {
                guild_id,
                user_id,
                previous_channel_id,
                channel_id,
            } = *change
            {
                let change = participant_change(previous_channel_id, channel_id, self.channel_id());
                if let Some(change) = change {
                    self.note_peer_change(guild_id, user_id, change, cx);
                }
            }
        }
    }

    /// Someone else came into or left the call: an announcement and a cue (a join sound, once
    /// the soundboard plays them, replaces the join cue).
    fn note_peer_change(
        &mut self,
        guild_id: Snowflake,
        user_id: Snowflake,
        change: ParticipantChange,
        cx: &mut Context<Self>,
    ) {
        let state = self.session.read(cx).state();
        let is_me = state.current_user().is_some_and(|user| user.id == user_id);
        if is_me || self.phase() != Some(CallPhase::Connected) {
            return;
        }
        let strings = crate::i18n::strings(cx);
        let name = state
            .members(guild_id)
            .iter()
            .find(|member| member.user.id == user_id)
            .map_or_else(
                || strings.voice().someone().to_owned(),
                |member| {
                    mellow_client_core::shared_logic::format::display_name(&member.user).to_owned()
                },
            );
        let channel_name = self
            .channel_id()
            .and_then(|channel_id| state.guild_channel(channel_id))
            .map(|channel| channel.name.clone())
            .unwrap_or_default();
        let (sentence, cue) = match change {
            ParticipantChange::Joined => (
                strings.voice().peer_joined(&name, &channel_name),
                VoiceCue::PeerJoin,
            ),
            ParticipantChange::Left => (
                strings.voice().peer_left(&name, &channel_name),
                VoiceCue::PeerLeave,
            ),
        };
        self.announcement = sentence.into();
        if let Some(engine) = self.engine() {
            engine.play_cue(cue);
        }
        cx.notify();
    }

    // ---- Toggles ----

    /// The microphone toggle (the user card, Ctrl+Shift+M).
    pub fn toggle_microphone(&mut self, cx: &mut Context<Self>) {
        let mut toggles = self.toggles;
        toggles.set_muted(!toggles.is_muted());
        self.set_toggles(toggles, cx);
        self.play_cue(
            if toggles.is_muted() {
                VoiceCue::Mute
            } else {
                VoiceCue::Unmute
            },
            cx,
        );
    }

    /// The headset toggle (the user card, Ctrl+Shift+D): deafening also mutes.
    pub fn toggle_deafened(&mut self, cx: &mut Context<Self>) {
        let mut toggles = self.toggles;
        toggles.set_deafened(!toggles.is_deafened());
        self.set_toggles(toggles, cx);
        self.play_cue(
            if toggles.is_deafened() {
                VoiceCue::Deafen
            } else {
                VoiceCue::Undeafen
            },
            cx,
        );
    }

    fn set_toggles(&mut self, toggles: VoiceToggles, cx: &mut Context<Self>) {
        self.toggles = toggles;
        if let Some(engine) = self.call.as_mut().and_then(|call| call.engine.as_mut()) {
            // Deafen first: its own rule may touch the microphone, which is then set exactly.
            engine.set_self_deafened(toggles.is_deafened());
            engine.set_self_muted(toggles.is_muted());
        }
        let stored = stored_toggles(toggles);
        change_preferences(cx, move |preferences| preferences.voice_toggles = stored);
        cx.notify();
    }

    // ---- Push-to-talk ----

    /// The push-to-talk key went down (repeats included): the microphone opens while in a call.
    /// Called by the in-window handler and by a global listener.
    pub fn press_push_to_talk(&mut self, cx: &mut Context<Self>) {
        let settings = current_settings(cx);
        if settings.input_mode != VoiceInputMode::PushToTalk || self.is_push_to_talk_pressed {
            return;
        }
        let Some(engine) = self.engine() else {
            return;
        };
        engine.set_push_to_talk_pressed(true);
        if settings.are_push_to_talk_sounds_on && !self.toggles.is_muted() {
            engine.play_cue(VoiceCue::PushToTalkPress);
        }
        self.is_push_to_talk_pressed = true;
    }

    /// The key came up, or the window lost focus with it held; the engine closes the
    /// microphone after the release delay.
    pub fn release_push_to_talk(&mut self, cx: &mut Context<Self>) {
        if !self.is_push_to_talk_pressed {
            return;
        }
        self.is_push_to_talk_pressed = false;
        let settings = current_settings(cx);
        if let Some(engine) = self.engine() {
            engine.set_push_to_talk_pressed(false);
            if settings.are_push_to_talk_sounds_on && !self.toggles.is_muted() {
                engine.play_cue(VoiceCue::PushToTalkRelease);
            }
        }
    }

    /// A key went down in the window; true when it must not type (a character key held for
    /// push-to-talk). `is_in_key_recorder`: the settings' recorder has focus and takes it.
    pub fn push_to_talk_key_down(
        &mut self,
        key: &str,
        key_char: Option<&str>,
        modifiers: Modifiers,
        is_in_key_recorder: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(code) = code_of_toolkit_key(key) else {
            return false;
        };
        let settings = current_settings(cx);
        // The toolkit names Space "space": the typed character tells a character key.
        let is_character_key = is_character_key(
            key_char.unwrap_or_default(),
            modifiers.control || modifiers.platform || modifiers.alt,
        );
        match push_to_talk_key_down(
            &settings,
            self.engine().is_some(),
            &code,
            is_character_key,
            is_in_key_recorder,
        ) {
            PushToTalkKeyEffect::Press { should_swallow } => {
                self.press_push_to_talk(cx);
                should_swallow
            }
            PushToTalkKeyEffect::Release | PushToTalkKeyEffect::Nothing => false,
        }
    }

    /// A key came up in the window, wherever focus went meanwhile.
    pub fn push_to_talk_key_up(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(code) = code_of_toolkit_key(key) else {
            return;
        };
        let settings = current_settings(cx);
        if push_to_talk_key_up(&settings, self.engine().is_some(), &code)
            == PushToTalkKeyEffect::Release
        {
            self.release_push_to_talk(cx);
        }
    }

    /// Modifier keys pressed or released alone: the toolkit reports them as a state change.
    pub fn push_to_talk_modifiers_changed(&mut self, modifiers: Modifiers, cx: &mut Context<Self>) {
        let previous = std::mem::replace(&mut self.held_modifiers, modifiers);
        let settings = current_settings(cx);
        let is_in_call = self.engine().is_some();
        for (modifier, was_held, is_held) in [
            (ModifierKey::Control, previous.control, modifiers.control),
            (ModifierKey::Shift, previous.shift, modifiers.shift),
            (ModifierKey::Alt, previous.alt, modifiers.alt),
            (ModifierKey::Meta, previous.platform, modifiers.platform),
        ] {
            let code = code_of_modifier(modifier);
            if is_held && !was_held {
                if let PushToTalkKeyEffect::Press { .. } =
                    push_to_talk_key_down(&settings, is_in_call, code, false, false)
                {
                    self.press_push_to_talk(cx);
                }
            } else if was_held
                && !is_held
                && push_to_talk_key_up(&settings, is_in_call, code) == PushToTalkKeyEffect::Release
            {
                self.release_push_to_talk(cx);
            }
        }
    }

    /// The window lost focus: a held key would never come up here.
    pub fn window_deactivated(&mut self, cx: &mut Context<Self>) {
        self.held_modifiers = Modifiers::default();
        self.release_push_to_talk(cx);
    }

    // ---- Per person ----

    /// Their volume, for the reader only (0 to 200 %).
    pub fn set_person_volume(
        &mut self,
        user_id: Snowflake,
        volume_percent: u32,
        cx: &mut Context<Self>,
    ) {
        let setting = PersonVoiceSetting {
            volume_percent: volume_percent.min(MAXIMUM_VOLUME_PERCENT),
            ..Self::person_setting(user_id, cx)
        };
        self.set_person_setting(user_id, setting, cx);
    }

    /// Their local mute ("Mute for me").
    pub fn toggle_person_muted(&mut self, user_id: Snowflake, cx: &mut Context<Self>) {
        let current = Self::person_setting(user_id, cx);
        let setting = PersonVoiceSetting {
            is_locally_muted: !current.is_locally_muted,
            ..current
        };
        self.set_person_setting(user_id, setting, cx);
    }

    fn set_person_setting(
        &mut self,
        user_id: Snowflake,
        setting: PersonVoiceSetting,
        cx: &mut Context<Self>,
    ) {
        if let Some(call) = self.call.as_ref() {
            match call.engine.as_ref() {
                Some(engine) => engine.set_person_setting(
                    PersonSoundSource::Voice,
                    user_id,
                    person_setting_of(setting),
                ),
                // The engine reads its store when it starts.
                None => call.person_settings.write_person_setting(
                    PersonSoundSource::Voice,
                    user_id,
                    person_setting_of(setting).clamped(),
                ),
            }
        }
        change_preferences(cx, move |preferences| {
            preferences.set_voice_person(user_id, setting);
        });
        cx.notify();
    }

    // ---- Settings and sounds ----

    fn follow_preferences(&mut self, cx: &mut Context<Self>) {
        let Some(stored) = preferences(cx).map(|preferences| preferences.voice_toggles) else {
            return;
        };
        if !self.has_read_stored_toggles {
            self.has_read_stored_toggles = true;
            self.toggles = VoiceToggles::new(
                stored.is_microphone_muted,
                stored.is_deafened,
                stored.was_microphone_muted_before_deafening,
            );
            if let Some(engine) = self.call.as_mut().and_then(|call| call.engine.as_mut()) {
                engine.set_self_deafened(self.toggles.is_deafened());
                engine.set_self_muted(self.toggles.is_muted());
            }
            cx.notify();
        }
        self.apply_settings(cx);
    }

    /// Applies what changed in the settings to the call, the sounds and the attachments.
    fn apply_settings(&mut self, cx: &mut Context<Self>) {
        let next = current_settings(cx);
        let previous = std::mem::replace(&mut self.applied_settings, next.clone());
        if next.output_device_id != previous.output_device_id {
            if let Some(player) = self.sound_player.as_ref() {
                player.set_output_device(next.output_device_id.clone());
            }
            crate::attachment_audio::follow_voice_output_device(next.output_device_id.clone(), cx);
        }
        let mode_changed = next.input_mode != previous.input_mode
            || next.push_to_talk_key != previous.push_to_talk_key;
        if mode_changed {
            self.release_push_to_talk(cx);
        }
        let Some(call) = self.call.as_mut() else {
            return;
        };
        if next.input_device_id != previous.input_device_id {
            call.devices.is_input_device_missing = false;
            call.devices.is_microphone_unavailable = false;
        }
        if next.output_device_id != previous.output_device_id {
            call.devices.is_output_device_missing = false;
            call.devices.is_output_unavailable = false;
        }
        let Some(engine) = call.engine.as_ref() else {
            return;
        };
        if next.input_device_id != previous.input_device_id {
            engine.set_input_device(next.input_device_id.clone());
        }
        if next.output_device_id != previous.output_device_id {
            engine.set_output_device(next.output_device_id.clone());
        }
        if input_mode_of(&next) != input_mode_of(&previous) {
            engine.set_input_mode(input_mode_of(&next));
        }
        if processing_of(&next) != processing_of(&previous) {
            engine.set_processing(processing_of(&next));
        }
        if next.output_volume_percent != previous.output_volume_percent {
            engine.set_output_volume(gain_of_percent(next.output_volume_percent));
        }
        if next.soundboard_volume_percent != previous.soundboard_volume_percent {
            engine.set_soundboard_volume(gain_of_percent(next.soundboard_volume_percent));
        }
    }

    /// Through the call when there is one, where the echo canceller hears it.
    fn play_cue(&mut self, cue: VoiceCue, cx: &mut Context<Self>) {
        match self.engine() {
            Some(engine) => engine.play_cue(cue),
            None => self.play_cue_outside_a_call(cue, cx),
        }
    }

    fn play_cue_outside_a_call(&mut self, cue: VoiceCue, cx: &App) {
        if let Some(player) = self.sound_player(cx) {
            player.play_cue(cue);
        }
    }

    fn play_notification_chime(&mut self) {
        if let Some(engine) = self.engine() {
            engine.play_notification_chime();
            return;
        }
        let output_device_id = self.applied_settings.output_device_id.clone();
        if let Some(player) = self.sound_player_on(output_device_id) {
            player.play_notification_chime();
        }
    }

    fn sound_player(&mut self, cx: &App) -> Option<&SoundPlayer> {
        self.sound_player_on(current_settings(cx).output_device_id)
    }

    /// The sound player, started on the chosen output the first time.
    fn sound_player_on(&mut self, output_device_id: Option<String>) -> Option<&SoundPlayer> {
        if self.sound_player.is_none() {
            match SoundPlayer::new((self.devices)()) {
                Ok(player) => {
                    player.set_output_device(output_device_id);
                    self.sound_player = Some(player);
                }
                Err(error) => {
                    tracing::warn!(error = %error.kind(), "cannot start the sound player");
                }
            }
        }
        self.sound_player.as_ref()
    }
}

/// The device sentences the panel shows during a call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // Four independent notices.
pub struct DeviceNotices {
    pub is_input_device_missing: bool,
    pub is_output_device_missing: bool,
    pub is_microphone_unavailable: bool,
    pub is_output_unavailable: bool,
}

#[cfg(test)]
mod tests;
