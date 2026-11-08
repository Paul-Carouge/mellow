//! What the conversation view asks of the session (the web's `client.svelte.ts`): history pages,
//! sends with their files, edits, deletions, reactions, pins, typing and read positions. Each
//! request runs on the network runtime through `run_request`, so an answer from an earlier session
//! is dropped, and failures come back as a sentence for the view to show.

use std::collections::HashSet;
use std::future::Future;
use std::time::Duration;

use gpui::{App, Context, Task};
use mellow_client_core::ClientError;
use mellow_client_core::generate_message_nonce;
use mellow_client_core::state::{
    ChangeOutcome, HISTORY_PAGE_SIZE, HistoryReachStep, MAXIMUM_HISTORY_PAGE_SIZE, PendingMessage,
    PinsAnswer, StateChange,
};
use mellow_protocol::{
    Attachment, EditMessageRequest, GatewayEvent, Message, MessageHistoryQuery, PublicUser,
    ReactionUsersQuery, Snowflake,
};

use super::{Session, StateChanged, now_unix_milliseconds};
use crate::sign_in_form::describe_error;
use crate::strings_pending_i18n::strings;

/// Who reacted, asked once per emoji: the instance's largest page (the web's
/// `REACTING_USERS_PAGE_SIZE`).
const REACTING_USERS_PAGE_SIZE: u32 = 100;

/// Read positions go to the instance at most this often; reads in between only move them (the
/// web's `READ_STATE_SEND_DELAY_MILLISECONDS`).
pub const READ_STATE_SEND_DELAY: Duration = Duration::from_secs(1);

/// A send the instance refused for good (`ClientError::refuses_message_for_good`): the message
/// stays failed in the outbox until the view takes its text back with
/// [`Session::take_refused_message`], so nothing typed is lost if it cannot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendRefused {
    pub channel_id: Snowflake,
    pub nonce: String,
    /// "Couldn't send the message: …".
    pub reason: String,
}

/// What [`Session::reach_message`] calls back once it knows.
type ReachDone = Box<dyn FnOnce(HistoryReach, &mut App)>;

/// A jump [`Session::reach_message`] is loading pages for.
struct ReachRequest {
    channel_id: Snowflake,
    message_id: Snowflake,
    history_generation: u64,
    /// Whether the view still wants the jump (same channel, no newer jump), the web's `"left"`.
    is_still_wanted: Box<dyn Fn(&App) -> bool>,
    on_done: ReachDone,
}

/// How a jump to a message older than the cached history ended (the web's `HistoryReach`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryReach {
    Loaded,
    /// Deleted since, or no longer readable.
    Missing,
    /// More than `MAXIMUM_PAGES_LOADED_TO_REACH_A_MESSAGE` pages back.
    TooFar,
    Failed,
}

/// Where a channel's history stands, beyond what the state caches.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct HistoryStatus {
    pub is_loading_latest: bool,
    pub is_loading_older: bool,
    /// The last page asked failed: the list offers "Try again" instead of loading forever.
    pub has_failed: bool,
}

/// The session's own bookkeeping for conversations (the web's `requestedHistory`,
/// `failedHistoryChannelIds`, `isLoadingOlder` and read-state timer).
#[derive(Debug, Default)]
pub(super) struct ConversationRequests {
    /// First pages asked in this `Ready`.
    history_requested: HashSet<Snowflake>,
    history_failed: HashSet<Snowflake>,
    loading_older_channel_id: Option<Snowflake>,
    /// The pending send of read positions: at most one per `READ_STATE_SEND_DELAY`.
    read_state_send: Option<Task<()>>,
}

impl ConversationRequests {
    /// `Ready` emptied the cache: pages are asked again, failures forgotten.
    pub(super) fn forget_history(&mut self) {
        self.history_requested.clear();
        self.history_failed.clear();
        self.loading_older_channel_id = None;
    }

    /// The link is back: failed pages may be asked again. Returns the channels they were in.
    pub(super) fn forget_failed_history(&mut self) -> Vec<Snowflake> {
        self.history_failed.drain().collect()
    }
}

impl Session {
    fn cloned_rest_client(&self) -> Option<mellow_client_core::RestClient> {
        self.rest_client().cloned()
    }

    /// `request`'s failure as a sentence starting with `failure_summary`; a session the instance
    /// no longer accepts ends here.
    fn run_conversation_request<T: Send + 'static>(
        &self,
        cx: &mut Context<Self>,
        request: impl Future<Output = Result<T, ClientError>> + Send + 'static,
        failure_summary: String,
        action: &'static str,
        on_done: impl FnOnce(&mut Self, Result<T, String>, &mut Context<Self>) + 'static,
    ) {
        self.run_request(cx, request, move |session, answer, cx| {
            let answer = answer.map_err(|error| {
                let sentence = describe_error(&error, &failure_summary, strings(cx));
                session.report_request_error(&error, action, cx);
                sentence
            });
            on_done(session, answer, cx);
        });
    }

    fn emit_changes(cx: &mut Context<Self>, changes: Vec<StateChange>) {
        let changes: Vec<StateChange> = changes
            .into_iter()
            .filter(|change| *change != StateChange::Nothing)
            .collect();
        if !changes.is_empty() {
            cx.emit(StateChanged(changes));
        }
        cx.notify();
    }

    // ---- History ----

    #[must_use]
    pub fn history_status(&self, channel_id: Snowflake) -> HistoryStatus {
        let requests = &self.conversation;
        HistoryStatus {
            is_loading_latest: requests.history_requested.contains(&channel_id)
                && self.state.messages(channel_id).is_empty()
                && !requests.history_failed.contains(&channel_id)
                && self.state.has_older_history(channel_id),
            is_loading_older: requests.loading_older_channel_id == Some(channel_id),
            has_failed: requests.history_failed.contains(&channel_id),
        }
    }

    /// Asks for the newest page once per `Ready` (the web's `ensureHistoryLoaded`).
    pub fn ensure_history_loaded(&mut self, channel_id: Snowflake, cx: &mut Context<Self>) {
        let Some(rest) = self.cloned_rest_client() else {
            return;
        };
        let requests = &mut self.conversation;
        if requests.history_failed.contains(&channel_id)
            || !requests.history_requested.insert(channel_id)
        {
            return;
        }
        let history_generation = self.state.history_generation();
        let query = MessageHistoryQuery {
            before: None,
            after: None,
            limit: Some(HISTORY_PAGE_SIZE),
        };
        self.run_request(
            cx,
            async move { rest.list_messages(channel_id, &query).await },
            move |session, answer, cx| {
                if session.state.history_generation() != history_generation {
                    return;
                }
                let change = match answer {
                    Ok(page) => session.state.insert_history_page(channel_id, page),
                    Err(error) => {
                        session.conversation.history_requested.remove(&channel_id);
                        session.conversation.history_failed.insert(channel_id);
                        session.report_request_error(&error, "load messages", cx);
                        StateChange::Messages { channel_id }
                    }
                };
                Self::emit_changes(cx, vec![change]);
            },
        );
        cx.notify();
    }

    /// The page before the oldest cached message, one request at a time; never retried by
    /// itself, or scrolling at the top would ask on every frame (the web's `loadOlderMessages`).
    pub fn load_older_messages(&mut self, channel_id: Snowflake, cx: &mut Context<Self>) {
        let Some(rest) = self.cloned_rest_client() else {
            return;
        };
        let requests = &self.conversation;
        if requests.loading_older_channel_id.is_some()
            || requests.history_failed.contains(&channel_id)
            || !self.state.has_older_history(channel_id)
        {
            return;
        }
        let Some(oldest_id) = self
            .state
            .messages(channel_id)
            .first()
            .map(|oldest| oldest.id)
        else {
            return;
        };
        self.conversation.loading_older_channel_id = Some(channel_id);
        let history_generation = self.state.history_generation();
        let query = MessageHistoryQuery {
            before: Some(oldest_id),
            after: None,
            limit: Some(HISTORY_PAGE_SIZE),
        };
        self.run_request(
            cx,
            async move { rest.list_messages(channel_id, &query).await },
            move |session, answer, cx| {
                if session.conversation.loading_older_channel_id == Some(channel_id) {
                    session.conversation.loading_older_channel_id = None;
                }
                if session.state.history_generation() != history_generation {
                    cx.notify();
                    return;
                }
                let change = match answer {
                    // Only next to the cursor: a page landing after a trim would leave a gap.
                    Ok(page)
                        if session.state.messages(channel_id).first().map(|m| m.id)
                            == Some(oldest_id) =>
                    {
                        session.state.insert_history_page(channel_id, page)
                    }
                    Ok(_) => StateChange::Nothing,
                    Err(error) => {
                        session.conversation.history_failed.insert(channel_id);
                        session.report_request_error(&error, "load older messages", cx);
                        StateChange::Messages { channel_id }
                    }
                };
                Self::emit_changes(cx, vec![change]);
            },
        );
        cx.notify();
    }

    /// Loads older pages until `message_id` is cached, for a pinned message far back (the web's
    /// `loadHistoryUntilMessage`); `on_done` is not called if a `Ready` empties the history meanwhile,
    /// nor once `is_still_wanted` says the reader left or jumped elsewhere: no further page loads.
    /// `is_still_wanted` is only asked when an answer arrives, never while the caller is updated.
    pub fn reach_message(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        is_still_wanted: impl Fn(&App) -> bool + 'static,
        on_done: impl FnOnce(HistoryReach, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let request = ReachRequest {
            channel_id,
            message_id,
            history_generation: self.state.history_generation(),
            is_still_wanted: Box::new(is_still_wanted),
            on_done: Box::new(on_done),
        };
        self.reach_message_after(request, 0, cx);
    }

    fn reach_message_after(
        &mut self,
        request: ReachRequest,
        pages_loaded: u32,
        cx: &mut Context<Self>,
    ) {
        let ReachRequest {
            channel_id,
            message_id,
            history_generation,
            ..
        } = request;
        if self.state.history_generation() != history_generation {
            return;
        }
        let rest = self.cloned_rest_client();
        let (before, rest) = match (
            self.state
                .history_reach_step(channel_id, message_id, pages_loaded),
            rest,
        ) {
            (HistoryReachStep::LoadBefore(before), Some(rest)) => (before, rest),
            (step, _) => {
                let reach = match step {
                    HistoryReachStep::Loaded => HistoryReach::Loaded,
                    HistoryReachStep::Missing => HistoryReach::Missing,
                    HistoryReachStep::TooFar => HistoryReach::TooFar,
                    HistoryReachStep::LoadBefore(_) => HistoryReach::Failed,
                };
                // Deferred: the view asking is still being updated when the answer is known at once.
                cx.defer(move |cx| (request.on_done)(reach, cx));
                return;
            }
        };
        let query = MessageHistoryQuery {
            before,
            after: None,
            limit: Some(MAXIMUM_HISTORY_PAGE_SIZE),
        };
        self.run_request(
            cx,
            async move { rest.list_messages(channel_id, &query).await },
            move |session, answer, cx| {
                if session.state.history_generation() != history_generation
                    || !(request.is_still_wanted)(cx)
                {
                    return;
                }
                match answer {
                    Ok(page) => {
                        // Only next to the cursor; a page that landed meanwhile is fine too.
                        if session.state.messages(channel_id).first().map(|m| m.id) == before {
                            let change = session.state.insert_reach_page(
                                channel_id,
                                page,
                                MAXIMUM_HISTORY_PAGE_SIZE,
                            );
                            Self::emit_changes(cx, vec![change]);
                        }
                        session.reach_message_after(request, pages_loaded + 1, cx);
                    }
                    Err(error) => {
                        session.report_request_error(&error, "load history around a message", cx);
                        (request.on_done)(HistoryReach::Failed, cx);
                    }
                }
            },
        );
    }

    /// "Try again": the first page if none loaded, else the older one.
    pub fn retry_loading_history(&mut self, channel_id: Snowflake, cx: &mut Context<Self>) {
        self.conversation.history_failed.remove(&channel_id);
        if self.state.messages(channel_id).is_empty() {
            self.conversation.history_requested.remove(&channel_id);
            self.ensure_history_loaded(channel_id, cx);
        } else {
            self.load_older_messages(channel_id, cx);
        }
        cx.notify();
    }

    /// Back at the latest message, or leaving the channel: older pages read meanwhile leave the
    /// cache (`trim_history`).
    pub fn trim_history(&mut self, channel_id: Snowflake, cx: &mut Context<Self>) {
        let before = self.state.messages(channel_id).len();
        self.state.trim_history(channel_id);
        if self.state.messages(channel_id).len() != before {
            Self::emit_changes(cx, vec![StateChange::Messages { channel_id }]);
        }
    }

    // ---- Read positions ----

    /// The reader sees `channel_id` down to its newest message: read here at once, and the
    /// instance hears it with the next coalesced send.
    pub fn mark_channel_read(&mut self, channel_id: Snowflake, cx: &mut Context<Self>) {
        let Some(newest_id) = self
            .state
            .messages(channel_id)
            .last()
            .map(|newest| newest.id)
        else {
            return;
        };
        let was_unread =
            self.state.has_unread(channel_id) || self.state.mention_count(channel_id) > 0;
        let change = self.state.mark_read(channel_id, newest_id);
        self.schedule_read_state_send(cx);
        if was_unread {
            Self::emit_changes(cx, vec![change]);
        }
    }

    /// At most one send per delay: the reads in between only move what it sends.
    pub(super) fn schedule_read_state_send(&mut self, cx: &mut Context<Self>) {
        if self.conversation.read_state_send.is_some() || self.rest_client().is_none() {
            return;
        }
        let generation = self.generation;
        self.conversation.read_state_send = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(READ_STATE_SEND_DELAY).await;
            let _ = this.update(cx, |session, cx| {
                if session.generation == generation {
                    session.send_read_states(cx);
                }
            });
        }));
    }

    /// Whether a send of read positions waits for its delay (tests and the quit path).
    #[must_use]
    pub fn is_read_state_send_scheduled(&self) -> bool {
        self.conversation.read_state_send.is_some()
    }

    /// Sends every read position the instance does not have yet, now (leaving a channel, or the
    /// delay ended). A failure worth retrying keeps the channel for the next send.
    pub fn send_read_states(&mut self, cx: &mut Context<Self>) {
        self.conversation.read_state_send = None;
        let Some(rest) = self.cloned_rest_client() else {
            return;
        };
        for read_state in self.state.take_read_states_to_send() {
            let rest = rest.clone();
            let sent = read_state.clone();
            self.run_request(
                cx,
                async move {
                    rest.acknowledge(sent.channel_id, sent.last_read_message_id)
                        .await
                },
                move |session, answer, cx| match answer {
                    Ok(()) => session.state.confirm_read_state_sent(&read_state),
                    Err(error) => {
                        session.state.note_read_state_send_failed(
                            read_state.channel_id,
                            error.is_worth_retrying(),
                        );
                        session.report_request_error(&error, "acknowledge", cx);
                    }
                },
            );
        }
    }

    // ---- Sending ----

    /// Queues the message with its uploaded files, shows it at once and sends it.
    ///
    /// # Errors
    /// `RandomGeneratorFailed` (nothing is queued, the caller keeps the text), or `NotSignedIn`.
    pub fn send_message(
        &mut self,
        channel_id: Snowflake,
        content: String,
        reply_to_id: Option<Snowflake>,
        attachments: Vec<Attachment>,
        cx: &mut Context<Self>,
    ) -> Result<(), ClientError> {
        let Some(rest) = self.cloned_rest_client() else {
            return Err(ClientError::NotSignedIn);
        };
        let nonce = generate_message_nonce()?;
        let request = self.state.queue_message_with_attachments(
            channel_id,
            content,
            reply_to_id,
            attachments,
            nonce.clone(),
        );
        cx.emit(StateChanged(vec![StateChange::Messages { channel_id }]));
        cx.notify();
        self.run_request(
            cx,
            async move { rest.send_message(channel_id, &request).await },
            move |session, answer, cx| {
                session.apply_send_answer(channel_id, &nonce, answer, cx);
            },
        );
        Ok(())
    }

    /// Tells the view a send was refused for good, so the text can go back to the composer.
    pub(super) fn report_refused_send(
        channel_id: Snowflake,
        nonce: &str,
        error: &ClientError,
        cx: &mut Context<Self>,
    ) {
        if !error.refuses_message_for_good() {
            return;
        }
        let summary = crate::i18n::strings(cx).chat().send_failed().to_owned();
        cx.emit(SendRefused {
            channel_id,
            nonce: nonce.to_owned(),
            reason: describe_error(error, &summary, strings(cx)),
        });
    }

    /// Takes a refused message out of the outbox, its text going back to the composer.
    pub fn take_refused_message(
        &mut self,
        nonce: &str,
        cx: &mut Context<Self>,
    ) -> Option<PendingMessage> {
        let pending = self.state.remove_refused_message(nonce)?;
        Self::emit_changes(
            cx,
            vec![StateChange::Messages {
                channel_id: pending.channel_id,
            }],
        );
        Some(pending)
    }

    /// A failed message's Retry: sent again with the same nonce, to the channel it was meant for.
    pub fn retry_message(&mut self, nonce: &str, cx: &mut Context<Self>) {
        let Some(rest) = self.cloned_rest_client() else {
            return;
        };
        let Some((channel_id, request)) = self.state.retry_pending_message(nonce) else {
            return;
        };
        Self::emit_changes(cx, vec![StateChange::Messages { channel_id }]);
        let nonce = nonce.to_owned();
        self.run_request(
            cx,
            async move { rest.send_message(channel_id, &request).await },
            move |session, answer, cx| {
                session.apply_send_answer(channel_id, &nonce, answer, cx);
            },
        );
    }

    /// A failed message's Delete: it leaves the outbox (only a failed one, never one sending).
    pub fn discard_message(&mut self, nonce: &str, cx: &mut Context<Self>) {
        let change = self.state.discard_failed_message(nonce);
        Self::emit_changes(cx, vec![change]);
    }

    /// The composer's typing notice, throttled by the composer.
    pub fn start_typing(&mut self, channel_id: Snowflake, cx: &mut Context<Self>) {
        let Some(rest) = self.cloned_rest_client() else {
            return;
        };
        self.run_request(
            cx,
            async move { rest.start_typing(channel_id).await },
            |session, answer, cx| {
                if let Err(error) = answer {
                    session.report_request_error(&error, "typing", cx);
                }
            },
        );
    }

    // ---- Edits, deletions, reactions, pins ----

    /// Saves `content` as the message's text; the answer is applied as its own update would be.
    pub fn edit_message(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        content: String,
        failure_summary: String,
        on_failed: impl FnOnce(String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(rest) = self.cloned_rest_client() else {
            return;
        };
        let request = EditMessageRequest { content };
        self.run_conversation_request(
            cx,
            async move { rest.edit_message(channel_id, message_id, &request).await },
            failure_summary,
            "edit message",
            move |session, answer, cx| match answer {
                Ok(updated) => session.apply_own_event(GatewayEvent::MessageUpdate(updated), cx),
                Err(sentence) => on_failed(sentence, cx),
            },
        );
    }

    /// Deletes the message; one already gone counts as deleted.
    pub fn delete_message(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        failure_summary: String,
        on_failed: impl FnOnce(String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(rest) = self.cloned_rest_client() else {
            return;
        };
        let request = async move {
            match rest.delete_message(channel_id, message_id).await {
                Err(ClientError::Api {
                    code: mellow_protocol::ApiErrorCode::NotFound,
                    ..
                }) => Ok(()),
                answer => answer,
            }
        };
        self.run_conversation_request(
            cx,
            request,
            failure_summary,
            "delete message",
            move |session, answer, cx| match answer {
                Ok(()) => session.apply_own_event(
                    GatewayEvent::MessageDelete {
                        guild_id: None,
                        channel_id,
                        id: message_id,
                    },
                    cx,
                ),
                Err(sentence) => on_failed(sentence, cx),
            },
        );
    }

    fn apply_own_event(&mut self, event: GatewayEvent, cx: &mut Context<Self>) {
        let change = self.state.apply_event(event, now_unix_milliseconds());
        Self::emit_changes(cx, vec![change]);
    }

    /// Adds the reader's reaction, or takes it back: shown at once, then kept, undone or settled
    /// by the instance's copy (client-core's optimistic rules). A second click while the first is
    /// on its way does nothing.
    pub fn toggle_reaction(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: &str,
        failure_summary: String,
        on_failed: impl FnOnce(String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(rest) = self.cloned_rest_client() else {
            return;
        };
        let Some((toggle, change)) = self
            .state
            .begin_reaction_toggle(channel_id, message_id, emoji)
        else {
            return;
        };
        Self::emit_changes(cx, vec![change]);
        let request_toggle = toggle.clone();
        let request = async move {
            let answer = if request_toggle.is_removing {
                rest.remove_own_reaction(channel_id, message_id, &request_toggle.emoji)
                    .await
            } else {
                rest.add_reaction(channel_id, message_id, &request_toggle.emoji)
                    .await
            };
            match answer {
                Ok(()) => (ChangeOutcome::Kept(()), None),
                Err(error) if error.is_refused_by_instance() => {
                    (ChangeOutcome::Undone, Some(error))
                }
                // The answer was lost: the instance's copy says whether it was kept. The page
                // just before the next id holds the message alone (there is no single-message
                // route); another message there means it was deleted meanwhile.
                Err(error) => {
                    let query = MessageHistoryQuery {
                        before: Some(Snowflake::from_raw(message_id.raw().saturating_add(1))),
                        after: None,
                        limit: Some(1),
                    };
                    match rest.list_messages(channel_id, &query).await {
                        Ok(page) => (
                            ChangeOutcome::SettledBy(
                                page.into_iter().find(|message| message.id == message_id),
                            ),
                            Some(error),
                        ),
                        Err(_) => (ChangeOutcome::Undone, Some(error)),
                    }
                }
            }
        };
        self.run_request(
            cx,
            async move { Ok::<_, ClientError>(request.await) },
            move |session, answer, cx| {
                let Ok((outcome, error)) = answer else {
                    return;
                };
                let change = session.state.finish_reaction_toggle(&toggle, outcome);
                Self::emit_changes(cx, vec![change]);
                if let Some(error) = error {
                    let sentence = describe_error(&error, &failure_summary, strings(cx));
                    session.report_request_error(&error, "react", cx);
                    on_failed(sentence, cx);
                }
            },
        );
    }

    /// Who reacted to a message with `emoji`, the first `REACTING_USERS_PAGE_SIZE` by ascending
    /// id, as the web asks; a failure comes back as a sentence starting with `failure_summary`.
    pub fn list_reacting_users(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        emoji: String,
        failure_summary: String,
        on_done: impl FnOnce(Result<Vec<PublicUser>, String>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(rest) = self.cloned_rest_client() else {
            cx.defer(move |cx| on_done(Err(failure_summary), cx));
            return;
        };
        let query = ReactionUsersQuery {
            after: None,
            limit: Some(REACTING_USERS_PAGE_SIZE),
        };
        self.run_conversation_request(
            cx,
            async move {
                rest.list_reacting_users(channel_id, message_id, &emoji, &query)
                    .await
            },
            failure_summary,
            "load who reacted",
            move |_, answer, cx| on_done(answer, cx),
        );
    }

    /// Pins or unpins; the instance answers with no message, so the change is applied here and
    /// its event repeats it.
    pub fn set_pinned(
        &mut self,
        message: Message,
        is_pinned: bool,
        failure_summary: String,
        on_failed: impl FnOnce(String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(rest) = self.cloned_rest_client() else {
            return;
        };
        let (channel_id, message_id) = (message.channel_id, message.id);
        let request = async move {
            if is_pinned {
                rest.pin_message(channel_id, message_id).await
            } else {
                rest.unpin_message(channel_id, message_id).await
            }
        };
        self.run_conversation_request(
            cx,
            request,
            failure_summary,
            if is_pinned { "pin" } else { "unpin" },
            move |session, answer, cx| match answer {
                Ok(()) => {
                    let change = session.state.set_pinned(&message, is_pinned);
                    Self::emit_changes(cx, vec![change, StateChange::Pins { channel_id }]);
                }
                Err(sentence) => on_failed(sentence, cx),
            },
        );
    }

    /// Asks for the channel's pins unless loaded or on their way; `force` is "Try again".
    pub fn load_pins(&mut self, channel_id: Snowflake, force: bool, cx: &mut Context<Self>) {
        let Some(rest) = self.cloned_rest_client() else {
            return;
        };
        if !self.state.begin_loading_pins(channel_id, force) {
            return;
        }
        Self::emit_changes(cx, vec![StateChange::Pins { channel_id }]);
        self.run_request(
            cx,
            async move { rest.list_pins(channel_id).await },
            move |session, answer, cx| match answer {
                Ok(messages) => match session.state.finish_loading_pins(channel_id, messages) {
                    PinsAnswer::AskAgain => session.load_pins(channel_id, true, cx),
                    PinsAnswer::Applied | PinsAnswer::Dropped => {
                        Self::emit_changes(cx, vec![StateChange::Pins { channel_id }]);
                    }
                },
                Err(error) => {
                    let change = session.state.fail_loading_pins(channel_id);
                    session.report_request_error(&error, "load pins", cx);
                    Self::emit_changes(cx, vec![change]);
                }
            },
        );
    }
}
