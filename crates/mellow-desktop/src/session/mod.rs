//! The signed-in session as one GPUI entity (ADR-0051 "Threads"): it owns the client state, the
//! REST client, the gateway handle and the pump that brings gateway updates to the main thread.
//!
//! Network calls run on the Tokio runtime (`tokio_bridge`); their answers come back here, and an
//! answer that left under an earlier session (a sign-out or a new sign-in since) is dropped
//! (`generation`). Gateway updates arrive through a bounded channel and are applied a burst at a
//! time, with one `notify` per burst, so a flood of events costs one repaint.

use std::collections::HashMap;
use std::future::Future;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use gpui::{App, Context, Entity, EventEmitter, Task};
use mellow_client_core::state::{ClientState, StateChange};
use mellow_client_core::{
    ClientError, GatewayHandle, GatewayUpdate, InstanceAddress, RestClient, run_gateway,
};
use mellow_protocol::{
    ChosenStatus, GatewayEvent, Message, SendMessageRequest, Snowflake, UserProfile,
};
use tokio::sync::mpsc;
use tokio::task::JoinError;
use zeroize::Zeroizing;

mod blocks;
mod cards;
mod forwards;
mod messages;
mod relationships;
mod servers;

pub use cards::{ChecklistItemChange, POLL_VOTERS_PAGE_SIZE};
pub use messages::{HistoryReach, HistoryStatus, READ_STATE_SEND_DELAY, SendRefused};

use crate::attachment_audio::{end_attachment_audio_session, start_attachment_audio_session};
use crate::composer::{forget_saved_drafts, forget_saved_drafts_now};
use crate::pictures::{end_picture_session, start_picture_session};
use crate::session_store;
use crate::sign_in_form::describe_error;
use crate::strings_pending_i18n::strings;
use crate::tokio_bridge::spawn_on_network_runtime;

/// Bounds what a stalled UI lets the gateway buffer; the gateway task waits when it is full.
const GATEWAY_UPDATE_QUEUE_CAPACITY: usize = 256;
/// Quitting waits this long at most for a sign-out to reach the instance.
pub const LOGOUT_ON_EXIT_TIMEOUT: Duration = Duration::from_secs(3);

/// Where the app stands with its session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionPhase {
    /// Reading the secret store at launch: no screen yet, so the sign-in card does not flash.
    RestoringStoredSession,
    SignedOut,
    SignedIn,
}

/// What the gateway connection is doing (the web's `ConnectionStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionStatus {
    SignedOut,
    /// Until the first `Ready` of the session.
    Connecting,
    Connected,
    /// The connection dropped; the gateway reconnects with backoff.
    Reconnecting,
}

/// A server's member list on its way, or refused (the member sheet offers "Try again").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MembersLoad {
    Loading,
    Failed,
}

/// What a burst of gateway updates touched, so views refresh only that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateChanged(pub Vec<StateChange>);

/// A message new to this session, once the state took it in (the web's `onIncomingMessage`):
/// the notifier decides whether to tell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingMessage(pub Message);

struct Connection {
    rest: RestClient,
    gateway: GatewayHandle,
    /// Dropping it aborts the gateway task on the network runtime.
    _gateway_task: Task<Result<(), JoinError>>,
    /// Ends when the gateway task drops its sender.
    _gateway_pump: Task<()>,
    /// False until the secret store took it, or when it refused: such a session ends with the app.
    is_session_stored: bool,
}

pub struct Session {
    phase: SessionPhase,
    status: ConnectionStatus,
    /// Bumped whenever a session starts or ends, so late answers of an earlier session (a 401 after
    /// signing out, buffered gateway updates) never act on the current one.
    generation: u64,
    connection: Option<Connection>,
    state: ClientState,
    /// The instance of the current or last session, for the sign-in card after a sign-out.
    last_instance: Option<InstanceAddress>,
    /// The instance ended the session (expired, revoked elsewhere), not a sign-out from here: the
    /// sign-in card says so.
    has_ended_by_itself: bool,
    is_signing_out: bool,
    /// Built with `describe_error`; `None` hides it.
    sign_out_error: Option<String>,
    /// Member lists asked for in this `Ready` (the web's `requestedMembers` and
    /// `failedMemberGuildIds`); a loaded list is in the state.
    member_loads: HashMap<Snowflake, MembersLoad>,
    /// Why the last status change failed, shown in the user card until the next try.
    status_error: Option<String>,
    /// False in tests: nothing goes to the secret store or the preferences file of the machine.
    is_kept_on_this_device: bool,
    /// Kept, not detached, so quitting can wait for it: a revoked token left in the secret store
    /// would greet the next launch with "your session ended".
    stored_session_forget_in_flight: Option<Task<Result<(), JoinError>>>,
    /// History pages, read positions on their way (`messages`).
    conversation: messages::ConversationRequests,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Session")
            .field("phase", &self.phase)
            .field("status", &self.status)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

impl EventEmitter<StateChanged> for Session {}
impl EventEmitter<SendRefused> for Session {}
impl EventEmitter<IncomingMessage> for Session {}

impl Session {
    /// A session that first looks for one kept in the secret store.
    pub fn new_restoring_stored_session(cx: &mut Context<Self>) -> Self {
        let session = Self::signed_out(true);
        let load = spawn_on_network_runtime(cx, session_store::load_stored_session());
        cx.spawn(async move |this, cx| {
            let stored_session = load.await.ok().flatten();
            let _ = this.update(cx, |session, cx| {
                match stored_session
                    .map(|stored| Self::authenticated_client(stored.instance, stored.token))
                {
                    Some(Ok(rest)) => session.connect(rest, true, cx),
                    Some(Err(error)) => {
                        tracing::warn!(%error, "cannot resume the stored session");
                        session.phase = SessionPhase::SignedOut;
                    }
                    None => session.phase = SessionPhase::SignedOut,
                }
                cx.notify();
            });
        })
        .detach();
        Self {
            phase: SessionPhase::RestoringStoredSession,
            ..session
        }
    }

    /// A signed-out session that keeps nothing on the machine: no stored session, no preferences.
    #[must_use]
    pub fn signed_out_kept_in_memory_only() -> Self {
        Self::signed_out(false)
    }

    fn signed_out(is_kept_on_this_device: bool) -> Self {
        Self {
            phase: SessionPhase::SignedOut,
            status: ConnectionStatus::SignedOut,
            generation: 0,
            connection: None,
            state: ClientState::default(),
            last_instance: None,
            has_ended_by_itself: false,
            is_signing_out: false,
            sign_out_error: None,
            member_loads: HashMap::new(),
            status_error: None,
            is_kept_on_this_device,
            stored_session_forget_in_flight: None,
            conversation: messages::ConversationRequests::default(),
        }
    }

    /// Whether the sign-in may remember things on this machine (the instance address).
    #[must_use]
    pub fn is_kept_on_this_device(&self) -> bool {
        self.is_kept_on_this_device
    }

    fn authenticated_client(
        instance: InstanceAddress,
        token: Zeroizing<String>,
    ) -> Result<RestClient, ClientError> {
        let mut rest = RestClient::new(instance)?;
        rest.set_token(token);
        Ok(rest)
    }

    #[must_use]
    pub fn phase(&self) -> SessionPhase {
        self.phase
    }

    #[must_use]
    pub fn status(&self) -> ConnectionStatus {
        self.status
    }

    #[must_use]
    pub fn state(&self) -> &ClientState {
        &self.state
    }

    #[must_use]
    pub fn last_instance(&self) -> Option<&InstanceAddress> {
        self.last_instance.as_ref()
    }

    #[must_use]
    pub fn has_ended_by_itself(&self) -> bool {
        self.has_ended_by_itself
    }

    #[must_use]
    pub fn sign_out_error(&self) -> Option<&str> {
        self.sign_out_error.as_deref()
    }

    /// Starts the session `rest` signed in to, and keeps it in the secret store for the next launch.
    pub fn begin_signed_in_session(&mut self, rest: RestClient, cx: &mut Context<Self>) {
        let instance = rest.instance().clone();
        let token = rest.token().map(|token| Zeroizing::new(token.to_owned()));
        self.connect(rest, false, cx);
        let Some(token) = token.filter(|_| self.is_kept_on_this_device) else {
            return;
        };
        let generation = self.generation;
        let store = spawn_on_network_runtime(cx, session_store::store_session(instance, token));
        cx.spawn(async move |this, cx| {
            let is_session_stored = store.await.unwrap_or(false);
            let _ = this.update(cx, |session, _| {
                if session.generation == generation
                    && let Some(connection) = &mut session.connection
                {
                    connection.is_session_stored = is_session_stored;
                }
            });
        })
        .detach();
        cx.notify();
    }

    fn connect(&mut self, rest: RestClient, is_session_stored: bool, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        self.last_instance = Some(rest.instance().clone());
        self.state = ClientState::default();
        self.phase = SessionPhase::SignedIn;
        self.status = ConnectionStatus::Connecting;
        self.has_ended_by_itself = false;
        self.sign_out_error = None;
        self.member_loads.clear();
        self.status_error = None;
        self.conversation = messages::ConversationRequests::default();
        // Nothing of the pictures may reach the disk of a session kept in memory only.
        if self.is_kept_on_this_device {
            start_picture_session(rest.clone(), cx);
        }
        start_attachment_audio_session(rest.clone(), cx);
        let (gateway, commands) = GatewayHandle::new();
        let (sender, mut updates) = mpsc::channel(GATEWAY_UPDATE_QUEUE_CAPACITY);
        let token = Zeroizing::new(rest.token().unwrap_or_default().to_owned());
        let gateway_task = spawn_on_network_runtime(
            cx,
            run_gateway(rest.instance().clone(), token, sender, commands),
        );
        // Tokio's receiver wakes any executor, so the pump runs on GPUI's main thread.
        let gateway_pump = cx.spawn(async move |this, cx| {
            while let Some(first_update) = updates.recv().await {
                let mut burst = vec![first_update];
                while let Ok(update) = updates.try_recv() {
                    burst.push(update);
                }
                let is_entity_gone = this
                    .update(cx, |session, cx| {
                        session.apply_gateway_updates(generation, burst, cx);
                    })
                    .is_err();
                if is_entity_gone {
                    break;
                }
            }
        });
        // Replacing the previous connection drops its tasks, which aborts its gateway.
        self.connection = Some(Connection {
            rest,
            gateway,
            _gateway_task: gateway_task,
            _gateway_pump: gateway_pump,
            is_session_stored,
        });
    }

    fn apply_gateway_updates(
        &mut self,
        generation: u64,
        burst: Vec<GatewayUpdate>,
        cx: &mut Context<Self>,
    ) {
        if self.generation != generation {
            return;
        }
        let mut changes: Vec<StateChange> = Vec::new();
        let mut should_resend_failed_messages = false;
        let mut incoming_messages = Vec::new();
        let now = now_unix_milliseconds();
        for update in burst {
            let change = match update {
                GatewayUpdate::Ready(ready) => {
                    self.status = ConnectionStatus::Connected;
                    should_resend_failed_messages = true;
                    // `Ready` emptied the member lists and the history: the views ask again.
                    self.member_loads.clear();
                    self.conversation.forget_history();
                    self.state.apply_ready(*ready)
                }
                GatewayUpdate::Event(event) => {
                    if let GatewayEvent::MessageCreate(message) = &event
                        && self.state.is_message_new_to_session(message)
                    {
                        incoming_messages.push(message.clone());
                    }
                    self.state.apply_event(event, now)
                }
                GatewayUpdate::Resumed => {
                    self.status = ConnectionStatus::Connected;
                    should_resend_failed_messages = true;
                    // Pages that failed while the link was down are asked again (the web's
                    // `retryFailedLoads`): their views hear it and ask.
                    for channel_id in self.conversation.forget_failed_history() {
                        let change = StateChange::Messages { channel_id };
                        if !changes.contains(&change) {
                            changes.push(change);
                        }
                    }
                    StateChange::Nothing
                }
                GatewayUpdate::Disconnected => {
                    self.status = ConnectionStatus::Reconnecting;
                    StateChange::Nothing
                }
                GatewayUpdate::SignedOut => {
                    self.end_session(true, cx);
                    return;
                }
            };
            if change != StateChange::Nothing && !changes.contains(&change) {
                changes.push(change);
            }
        }
        if should_resend_failed_messages {
            self.resend_failed_messages(cx);
        }
        if !changes.is_empty() {
            cx.emit(StateChanged(changes));
        }
        for message in incoming_messages {
            cx.emit(IncomingMessage(message));
        }
        cx.notify();
    }

    /// Skips the gateway's backoff wait (the banner's "Retry now").
    pub fn reconnect_now(&self) {
        if let Some(connection) = &self.connection {
            connection.gateway.reconnect_now();
        }
    }

    /// Runs `request` on the network runtime, then `on_done` here with its answer, unless the
    /// session changed in between. `on_done` runs while the session is leased: a caller's own
    /// callback must be deferred (`cx.defer`) before it reads the session.
    fn run_request<T: Send + 'static>(
        &self,
        cx: &mut Context<Self>,
        request: impl Future<Output = Result<T, ClientError>> + Send + 'static,
        on_done: impl FnOnce(&mut Self, Result<T, ClientError>, &mut Context<Self>) + 'static,
    ) {
        let generation = self.generation;
        let task = spawn_on_network_runtime(cx, request);
        cx.spawn(async move |this, cx| {
            let answer = task.await;
            let _ = this.update(cx, |session, cx| {
                if session.generation != generation {
                    return;
                }
                match answer {
                    Ok(answer) => on_done(session, answer, cx),
                    Err(error) => tracing::error!(%error, "a network task failed"),
                }
            });
        })
        .detach();
    }

    /// Ends the session when the instance no longer accepts it; otherwise only logs.
    fn report_request_error(&mut self, error: &ClientError, action: &str, cx: &mut Context<Self>) {
        if error.needs_new_login() {
            self.end_session(true, cx);
        } else {
            tracing::warn!(%error, action, "request failed");
        }
    }

    /// Signs out here and on the instance. As on the web, an instance that cannot be reached keeps
    /// the session (signing out locally would leave it valid there) and the error says so.
    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        let Some(connection) = &self.connection else {
            return;
        };
        if self.is_signing_out {
            return;
        }
        self.is_signing_out = true;
        self.sign_out_error = None;
        let mut rest = connection.rest.clone();
        self.run_request(
            cx,
            async move { rest.logout().await },
            |session, answer, cx| {
                session.is_signing_out = false;
                match answer {
                    Ok(()) => session.end_session(false, cx),
                    // A session the instance already refuses is as good as revoked.
                    Err(error) if error.needs_new_login() => session.end_session(false, cx),
                    Err(error) => {
                        let strings = strings(cx);
                        session.sign_out_error =
                            Some(describe_error(&error, strings.sign_out_failed, strings));
                        cx.notify();
                    }
                }
            },
        );
        cx.notify();
    }

    /// Back to the sign-in screen, forgetting the stored session: the instance no longer accepts
    /// it, or the user signed out.
    fn end_session(&mut self, has_ended_by_itself: bool, cx: &mut Context<Self>) {
        // Drops the gateway task (aborting it) and the pump.
        self.connection = None;
        self.generation += 1;
        self.state = ClientState::default();
        self.phase = SessionPhase::SignedOut;
        self.status = ConnectionStatus::SignedOut;
        self.has_ended_by_itself = has_ended_by_itself;
        self.is_signing_out = false;
        self.sign_out_error = None;
        self.member_loads.clear();
        self.status_error = None;
        self.conversation = messages::ConversationRequests::default();
        end_attachment_audio_session(cx);
        if self.is_kept_on_this_device {
            end_picture_session(cx);
            forget_saved_drafts(cx);
            // The previous forget keeps running when detached; only the latest matters at quit.
            if let Some(previous_forget) = self.stored_session_forget_in_flight.take() {
                previous_forget.detach();
            }
            self.stored_session_forget_in_flight = Some(spawn_on_network_runtime(
                cx,
                session_store::forget_stored_session(),
            ));
        }
        cx.notify();
    }

    /// Where a server's member list stands; `None` once loaded or before it is asked for.
    #[must_use]
    pub fn members_load(&self, guild_id: Snowflake) -> Option<MembersLoad> {
        self.member_loads.get(&guild_id).copied()
    }

    /// Asks for a server's member list once per `Ready`, as the web's `ensureMembersLoaded`;
    /// `retry` asks again after a failure ("Try again").
    /// shortcut: a join or a role change while the list is on its way is not asked again
    /// (the web's `membersChangedWhileLoading`); the next `Ready` brings it.
    pub fn load_members(&mut self, guild_id: Snowflake, retry: bool, cx: &mut Context<Self>) {
        let Some(rest) = self.rest_client().cloned() else {
            return;
        };
        match self.member_loads.get(&guild_id) {
            Some(MembersLoad::Loading) => return,
            Some(MembersLoad::Failed) if !retry => return,
            None if !self.state.members(guild_id).is_empty() => return,
            _ => {}
        }
        self.member_loads.insert(guild_id, MembersLoad::Loading);
        let history_generation = self.state.history_generation();
        self.run_request(
            cx,
            async move { rest.list_members(guild_id).await },
            move |session, answer, cx| {
                // A `Ready` or a removal meanwhile: the answer belongs to a state that is gone.
                if session.state.history_generation() != history_generation
                    || session.state.guild(guild_id).is_none()
                {
                    return;
                }
                match answer {
                    Ok(members) => {
                        session.member_loads.remove(&guild_id);
                        let change = session.state.set_members(guild_id, members);
                        cx.emit(StateChanged(vec![change]));
                    }
                    Err(error) => {
                        session.member_loads.insert(guild_id, MembersLoad::Failed);
                        session.report_request_error(&error, "load members", cx);
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    /// Why the last status change failed, in words.
    #[must_use]
    pub fn status_error(&self) -> Option<&str> {
        self.status_error.as_deref()
    }

    /// Shows `status` to others, keeping the custom status (the user card's menu).
    pub fn choose_status(
        &mut self,
        status: ChosenStatus,
        failure_summary: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(rest) = self.rest_client().cloned() else {
            return;
        };
        let custom_status = self
            .state
            .current_user()
            .and_then(|user| user.custom_status.clone());
        self.status_error = None;
        let failure_summary = failure_summary.to_owned();
        self.run_request(
            cx,
            async move { rest.update_presence(status, custom_status).await },
            move |session, answer, cx| {
                match answer {
                    Ok(user) => {
                        let change = session.state.apply_current_user(user);
                        cx.emit(StateChanged(vec![change]));
                    }
                    Err(error) => {
                        session.status_error =
                            Some(describe_error(&error, &failure_summary, strings(cx)));
                        session.report_request_error(&error, "change status", cx);
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    /// Replaces the status and custom status (blank clears it), as the web's `updatePresence`;
    /// `on_done` gets the outcome (the custom status dialog).
    pub fn update_presence(
        &mut self,
        status: ChosenStatus,
        custom_status: Option<String>,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<(), ClientError>, &mut App) + 'static,
    ) {
        let Some(rest) = self.rest_client().cloned() else {
            // Later, as an answer would come: the caller may still be updating its own view.
            cx.defer(move |cx| on_done(Err(ClientError::NotSignedIn), cx));
            return;
        };
        let custom_status = custom_status
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty());
        self.run_request(
            cx,
            async move { rest.update_presence(status, custom_status).await },
            move |session, answer, cx| match answer {
                Ok(user) => {
                    let change = session.state.apply_current_user(user);
                    cx.emit(StateChanged(vec![change]));
                    cx.notify();
                    cx.defer(move |cx| on_done(Ok(()), cx));
                }
                Err(error) => {
                    session.report_request_error(&error, "change custom status", cx);
                    cx.defer(move |cx| on_done(Err(error), cx));
                }
            },
        );
    }

    /// Reads someone's "about me" and links (ADR-0050) for the profile on screen.
    pub fn load_user_profile(
        &mut self,
        user_id: Snowflake,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<UserProfile, ClientError>, &mut App) + 'static,
    ) {
        let Some(rest) = self.rest_client().cloned() else {
            // Later, as an answer would come: the caller may still be updating its own view.
            cx.defer(move |cx| on_done(Err(ClientError::NotSignedIn), cx));
            return;
        };
        self.run_request(
            cx,
            async move { rest.user_profile(user_id).await },
            move |session, answer, cx| {
                if let Err(error) = &answer {
                    session.report_request_error(error, "load profile", cx);
                }
                cx.defer(move |cx| on_done(answer, cx));
            },
        );
    }

    /// Opens the one-to-one conversation with `user_id` (creating it on the instance if needed),
    /// then hands its id, or what went wrong in words, to `on_done`.
    pub fn open_direct_message(
        &mut self,
        user_id: Snowflake,
        failure_summary: String,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<Snowflake, String>, &mut App) + 'static,
    ) {
        let Some(rest) = self.rest_client().cloned() else {
            return;
        };
        self.run_request(
            cx,
            async move { rest.open_direct_message(user_id).await },
            move |session, answer, cx| match answer {
                Ok(conversation) => {
                    let conversation_id = conversation.id;
                    let change = session.state.upsert_direct_message_channel(conversation);
                    cx.emit(StateChanged(vec![change]));
                    cx.notify();
                    cx.defer(move |cx| on_done(Ok(conversation_id), cx));
                }
                Err(error) => {
                    let message = describe_error(&error, &failure_summary, strings(cx));
                    session.report_request_error(&error, "open conversation", cx);
                    cx.defer(move |cx| on_done(Err(message), cx));
                }
            },
        );
    }

    /// After a reconnect, sends every failed message again, one after another so they keep their
    /// order. The first failure the instance does not refuse stops the batch: the rest stay failed.
    fn resend_failed_messages(&mut self, cx: &mut Context<Self>) {
        let batch = self.state.retry_failed_messages();
        if batch.is_empty() {
            return;
        }
        let mut channel_ids: Vec<Snowflake> =
            batch.iter().map(|(channel_id, _, _)| *channel_id).collect();
        channel_ids.dedup();
        cx.emit(StateChanged(
            channel_ids
                .into_iter()
                .map(|channel_id| StateChange::Messages { channel_id })
                .collect(),
        ));
        self.resend_next_message(batch.into_iter(), cx);
    }

    /// One request at a time, each through `run_request`, so a sign-out stops the rest of the batch.
    fn resend_next_message(
        &mut self,
        mut remaining: std::vec::IntoIter<(Snowflake, String, SendMessageRequest)>,
        cx: &mut Context<Self>,
    ) {
        let Some((channel_id, nonce, request)) = remaining.next() else {
            return;
        };
        let Some(rest) = self
            .connection
            .as_ref()
            .map(|connection| connection.rest.clone())
        else {
            return;
        };
        self.run_request(
            cx,
            async move { rest.send_message(channel_id, &request).await },
            move |session, answer, cx| {
                let should_continue = answer
                    .as_ref()
                    .map_or_else(ClientError::refuses_message_for_good, |_| true);
                session.apply_send_answer(channel_id, &nonce, answer, cx);
                if should_continue {
                    session.resend_next_message(remaining, cx);
                    return;
                }
                let changes: Vec<StateChange> = remaining
                    .map(|(_, nonce, _)| session.state.mark_send_failed(&nonce))
                    .collect();
                cx.emit(StateChanged(changes));
                cx.notify();
            },
        );
    }

    /// A refused message stays failed in the outbox until the view takes its text back
    /// (`SendRefused`, `take_refused_message`), so nothing typed is lost.
    fn apply_send_answer(
        &mut self,
        channel_id: Snowflake,
        nonce: &str,
        answer: Result<Message, ClientError>,
        cx: &mut Context<Self>,
    ) {
        let change = match answer {
            Ok(message) => self.state.confirm_sent_message(nonce, message),
            Err(error) => {
                let change = self.state.mark_send_failed(nonce);
                self.report_request_error(&error, "send message", cx);
                if self.phase == SessionPhase::SignedIn && change != StateChange::Nothing {
                    Self::report_refused_send(channel_id, nonce, &error, cx);
                }
                change
            }
        };
        if self.phase == SessionPhase::SignedIn {
            let change = if change == StateChange::Nothing {
                StateChange::Messages { channel_id }
            } else {
                change
            };
            cx.emit(StateChanged(vec![change]));
            cx.notify();
        }
    }

    /// Called as the app quits. Read positions not sent yet go out first. A stored session stays
    /// open for the next launch. One the secret
    /// store did not take is revoked, since nobody could use it again, and so is one whose sign-out
    /// was still in flight: the runtime drops unfinished tasks at exit, so that revocation might
    /// never leave, and the next launch would sign the user back in. A revoked session is also
    /// forgotten, and a forget still running is awaited. Blocks the main thread for at most
    /// `LOGOUT_ON_EXIT_TIMEOUT`: the window is already closing.
    fn end_connection_before_exit(&mut self, cx: &App) {
        let pending_forget = self.stored_session_forget_in_flight.take();
        let is_signing_out = self.is_signing_out;
        // Reads of the last second would otherwise stay unread on the user's other devices.
        let read_states = self.state.take_read_states_to_send();
        let acknowledge = self
            .rest_client()
            .cloned()
            .filter(|_| !read_states.is_empty())
            .map(|rest| {
                spawn_on_network_runtime(cx, async move {
                    for read_state in read_states {
                        let sent = rest
                            .acknowledge(read_state.channel_id, read_state.last_read_message_id)
                            .await;
                        if let Err(error) = sent {
                            tracing::warn!(%error, "could not send a read position before exiting");
                        }
                    }
                })
            });
        let session_to_revoke = self
            .connection
            .take()
            .filter(|connection| is_signing_out || !connection.is_session_stored)
            .map(|connection| connection.rest);
        // A revoked session, or a sign-out just made, leaves no drafts: the next launch is signed
        // out, and a deletion left running at sign-out may never finish.
        if self.is_kept_on_this_device && (session_to_revoke.is_some() || pending_forget.is_some())
        {
            forget_saved_drafts_now();
        }
        if session_to_revoke.is_none() && pending_forget.is_none() && acknowledge.is_none() {
            return;
        }
        let is_kept_on_this_device = self.is_kept_on_this_device;
        let revoke = session_to_revoke.map(|mut rest| {
            spawn_on_network_runtime(cx, async move {
                let is_revoked =
                    match tokio::time::timeout(LOGOUT_ON_EXIT_TIMEOUT, rest.logout()).await {
                        Ok(Ok(())) => true,
                        // A session the instance already refuses is as good as revoked.
                        Ok(Err(error)) => error.needs_new_login(),
                        Err(_elapsed) => false,
                    };
                if is_revoked && is_kept_on_this_device {
                    session_store::forget_stored_session().await;
                }
                is_revoked
            })
        });
        let exit_work = async move {
            if let Some(acknowledge) = acknowledge {
                let _ = acknowledge.await;
            }
            if let Some(pending_forget) = pending_forget {
                let _ = pending_forget.await;
            }
            match revoke {
                Some(revoke) => matches!(revoke.await, Ok(true)),
                None => true,
            }
        };
        let is_done = cx
            .foreground_executor()
            .block_with_timeout(LOGOUT_ON_EXIT_TIMEOUT, exit_work);
        if !matches!(is_done, Ok(true)) {
            tracing::warn!("could not end the session on the instance before exiting");
        }
    }

    /// Ends the connection when the app quits (see `end_connection_before_exit`).
    pub fn end_connection_when_the_app_quits(session: &Entity<Self>, cx: &mut App) {
        let session = session.downgrade();
        cx.on_app_quit(move |cx| {
            let _ = session.update(cx, |session, cx| session.end_connection_before_exit(cx));
            std::future::ready(())
        })
        .detach();
    }

    /// The signed-in user's REST client, for the views that call the API themselves.
    #[must_use]
    pub fn rest_client(&self) -> Option<&RestClient> {
        self.connection.as_ref().map(|connection| &connection.rest)
    }

    /// Tests stand in for the gateway with these.
    #[cfg(test)]
    pub(crate) fn state_mut(&mut self) -> &mut ClientState {
        &mut self.state
    }

    #[cfg(test)]
    pub(crate) fn apply_gateway_updates_of_this_session(
        &mut self,
        burst: Vec<GatewayUpdate>,
        cx: &mut Context<Self>,
    ) {
        self.apply_gateway_updates(self.generation, burst, cx);
    }
}

pub(crate) fn now_unix_milliseconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::{AppContext as _, TestAppContext};

    use super::*;

    #[gpui::test]
    fn updates_of_an_ended_session_are_dropped(cx: &mut TestAppContext) {
        let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
        let ended_generation = session.read_with(cx, |session, _| session.generation);
        session.update(cx, |session, cx| session.end_session(false, cx));

        session.update(cx, |session, cx| {
            session.apply_gateway_updates(ended_generation, vec![GatewayUpdate::Disconnected], cx);
        });

        session.read_with(cx, |session, _| {
            assert_eq!(session.status(), ConnectionStatus::SignedOut);
            assert_eq!(session.phase(), SessionPhase::SignedOut);
        });
    }

    #[gpui::test]
    fn a_burst_of_updates_costs_one_notify(cx: &mut TestAppContext) {
        let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
        let notify_count = Rc::new(Cell::new(0_u32));
        let counted = Rc::clone(&notify_count);
        let _subscription =
            cx.update(|cx| cx.observe(&session, move |_, _| counted.set(counted.get() + 1)));

        session.update(cx, |session, cx| {
            session.apply_gateway_updates_of_this_session(
                vec![
                    GatewayUpdate::Disconnected,
                    GatewayUpdate::Resumed,
                    GatewayUpdate::Disconnected,
                ],
                cx,
            );
        });
        cx.run_until_parked();

        assert_eq!(notify_count.get(), 1);
        session.read_with(cx, |session, _| {
            assert_eq!(session.status(), ConnectionStatus::Reconnecting);
        });
    }
}
