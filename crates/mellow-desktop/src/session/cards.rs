//! Polls, checklists and progress bars (ADR-0043, ADR-0047) as the conversation view asks for them
//! (the web's `client.svelte.ts`): sending one through the outbox, votes and ticks shown at once
//! then kept, undone or settled by the instance's copy (client-core's optimistic rules), who voted,
//! ending a poll, and an author's or moderator's changes to items and levels.
//!
//! Every callback is deferred: the session is still leased when an answer is applied, and a
//! callback that reads it would panic.

use gpui::{App, Context};
use mellow_client_core::state::{ChangeOutcome, MessageCard, StateChange};
use mellow_client_core::{ClientError, RestClient, generate_message_nonce};
use mellow_protocol::{
    ApiErrorCode, Message, MessageHistoryQuery, NewProgressBar, PublicUser, ReactionUsersQuery,
    Snowflake,
};

use super::{Session, now_unix_milliseconds};
use crate::sign_in_form::describe_error;
use crate::strings_pending_i18n::strings;

/// Who voted for an answer, a page at a time (the web's `POLL_VOTERS_PAGE_SIZE`).
pub const POLL_VOTERS_PAGE_SIZE: u32 = 100;

/// A change an author or a moderator makes to a checklist's items; the card waits for the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChecklistItemChange {
    Add(String),
    Rename { item_id: u32, text: String },
    Remove { item_id: u32 },
}

/// The instance's copy of a message after a lost answer: the page just before the next id holds
/// it alone (there is no single-message route); `None` when it was deleted meanwhile.
async fn instance_copy_of(
    rest: &RestClient,
    channel_id: Snowflake,
    message_id: Snowflake,
) -> Result<Option<Message>, ClientError> {
    let query = MessageHistoryQuery {
        before: Some(Snowflake::from_raw(message_id.raw().saturating_add(1))),
        after: None,
        limit: Some(1),
    };
    let page = rest.list_messages(channel_id, &query).await?;
    Ok(page.into_iter().find(|message| message.id == message_id))
}

/// A refusal undoes the change; a lost answer is settled by the instance's copy, or undone when
/// that copy is out of reach too.
async fn outcome_of_failure<Answer>(
    error: &ClientError,
    rest: &RestClient,
    channel_id: Snowflake,
    message_id: Snowflake,
) -> ChangeOutcome<Answer> {
    if error.is_refused_by_instance() {
        return ChangeOutcome::Undone;
    }
    match instance_copy_of(rest, channel_id, message_id).await {
        Ok(stored) => ChangeOutcome::SettledBy(stored),
        Err(_) => ChangeOutcome::Undone,
    }
}

/// "This poll has ended" when the instance refused a vote for it, else what failed.
fn vote_failure_sentence(
    error: &ClientError,
    failure_summary: &str,
    has_ended: &str,
    cx: &App,
) -> String {
    match error {
        ClientError::Api {
            code: ApiErrorCode::ValidationFailed,
            ..
        } => has_ended.to_owned(),
        _ => describe_error(error, failure_summary, strings(cx)),
    }
}

impl Session {
    /// Queues a poll, checklist or progress bar, shows it at once and sends it.
    ///
    /// # Errors
    /// `RandomGeneratorFailed` (nothing is queued), or `NotSignedIn`.
    pub fn send_card(
        &mut self,
        channel_id: Snowflake,
        card: MessageCard,
        cx: &mut Context<Self>,
    ) -> Result<(), ClientError> {
        let Some(rest) = self.cloned_rest_client() else {
            return Err(ClientError::NotSignedIn);
        };
        let nonce = generate_message_nonce()?;
        let request = self
            .state
            .queue_card_message(channel_id, card, nonce.clone());
        Self::emit_changes(cx, vec![StateChange::Messages { channel_id }]);
        self.run_request(
            cx,
            async move { rest.send_message(channel_id, &request).await },
            move |session, answer, cx| {
                session.apply_send_answer(channel_id, &nonce, answer, cx);
            },
        );
        Ok(())
    }

    /// The reader's choice of an answer (`Some`), or "Remove my vote" (`None`): shown at once,
    /// then kept, undone or settled. A click while the poll's previous change is on its way does
    /// nothing, so changes never cross.
    #[allow(clippy::too_many_arguments)] // The web's arguments, plus the two sentences it may say.
    pub fn change_own_poll_votes(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        answer_id: Option<u32>,
        failure_summary: String,
        has_ended: String,
        on_failed: impl FnOnce(String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(rest) = self.cloned_rest_client() else {
            return;
        };
        let begun = match answer_id {
            Some(answer_id) => self
                .state
                .begin_poll_choice(channel_id, message_id, answer_id),
            None => self
                .state
                .begin_removing_own_poll_votes(channel_id, message_id),
        };
        let Some((changes, change)) = begun else {
            return;
        };
        Self::emit_changes(cx, vec![change]);
        let requests = changes.requests();
        let request = async move {
            let mut sent = 0;
            for vote in requests {
                let answer = if vote.is_adding {
                    rest.add_poll_vote(channel_id, message_id, vote.answer_id)
                        .await
                } else {
                    rest.remove_poll_vote(channel_id, message_id, vote.answer_id)
                        .await
                };
                if let Err(error) = answer {
                    let outcome = outcome_of_failure(&error, &rest, channel_id, message_id).await;
                    return (outcome, sent, Some(error));
                }
                sent += 1;
            }
            (ChangeOutcome::Kept(()), sent, None)
        };
        self.run_request(
            cx,
            async move { Ok::<_, ClientError>(request.await) },
            move |session, answer, cx| {
                let Ok((outcome, sent, error)) = answer else {
                    return;
                };
                let change = session
                    .state
                    .finish_poll_vote_changes(&changes, sent, outcome);
                Self::emit_changes(cx, vec![change]);
                if let Some(error) = error {
                    let sentence = vote_failure_sentence(&error, &failure_summary, &has_ended, cx);
                    session.report_request_error(&error, "vote in poll", cx);
                    cx.defer(move |cx| on_failed(sentence, cx));
                }
            },
        );
    }

    /// Who voted for an answer, the page after `after` by ascending id.
    #[allow(clippy::too_many_arguments)] // The answer's place, the page and the reply.
    pub fn list_poll_voters(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        answer_id: u32,
        after: Option<Snowflake>,
        failure_summary: String,
        on_done: impl FnOnce(Result<Vec<PublicUser>, String>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(rest) = self.cloned_rest_client() else {
            cx.defer(move |cx| on_done(Err(failure_summary), cx));
            return;
        };
        let query = ReactionUsersQuery {
            after,
            limit: Some(POLL_VOTERS_PAGE_SIZE),
        };
        self.run_conversation_request(
            cx,
            async move {
                rest.list_poll_voters(channel_id, message_id, answer_id, &query)
                    .await
            },
            failure_summary,
            "load poll voters",
            move |_, answer, cx| cx.defer(move |cx| on_done(answer, cx)),
        );
    }

    /// Its author ends the poll now: shown once the instance agrees, and its `MESSAGE_UPDATE`
    /// confirms it.
    pub fn end_poll(
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
        self.run_conversation_request(
            cx,
            async move { rest.end_poll(channel_id, message_id).await },
            failure_summary,
            "end poll",
            move |session, answer, cx| match answer {
                Ok(()) => {
                    let change =
                        session
                            .state
                            .end_poll(channel_id, message_id, now_unix_milliseconds());
                    Self::emit_changes(cx, vec![change]);
                }
                Err(sentence) => cx.defer(move |cx| on_failed(sentence, cx)),
            },
        );
    }

    /// Ticks or unticks an item for the reader: shown at once, then the instance's answer replaces
    /// it. A click while the item's previous tick is on its way does nothing.
    pub fn toggle_checklist_item(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        item_id: u32,
        failure_summary: String,
        on_failed: impl FnOnce(String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(rest) = self.cloned_rest_client() else {
            return;
        };
        let Some((tick, change)) = self.state.begin_checklist_tick(
            channel_id,
            message_id,
            item_id,
            now_unix_milliseconds(),
        ) else {
            return;
        };
        Self::emit_changes(cx, vec![change]);
        let is_ticking = tick.is_ticking;
        let request = async move {
            match rest
                .set_checklist_item_ticked(channel_id, message_id, item_id, is_ticking)
                .await
            {
                Ok(checklist) => (ChangeOutcome::Kept(checklist), None),
                Err(error) => (
                    outcome_of_failure(&error, &rest, channel_id, message_id).await,
                    Some(error),
                ),
            }
        };
        self.run_request(
            cx,
            async move { Ok::<_, ClientError>(request.await) },
            move |session, answer, cx| {
                let Ok((outcome, error)) = answer else {
                    return;
                };
                let change = session.state.finish_checklist_tick(&tick, outcome);
                Self::emit_changes(cx, vec![change]);
                if let Some(error) = error {
                    let sentence = describe_error(&error, &failure_summary, strings(cx));
                    session.report_request_error(&error, "tick checklist item", cx);
                    cx.defer(move |cx| on_failed(sentence, cx));
                }
            },
        );
    }

    /// Adds, renames or removes an item; `on_done` hears whether the instance took it.
    pub fn change_checklist_item(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        item_change: ChecklistItemChange,
        failure_summary: String,
        on_done: impl FnOnce(Result<(), String>, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(rest) = self.cloned_rest_client() else {
            cx.defer(move |cx| on_done(Err(failure_summary), cx));
            return;
        };
        let request = async move {
            match item_change {
                ChecklistItemChange::Add(text) => {
                    rest.add_checklist_item(channel_id, message_id, &text).await
                }
                ChecklistItemChange::Rename { item_id, text } => {
                    rest.rename_checklist_item(channel_id, message_id, item_id, &text)
                        .await
                }
                ChecklistItemChange::Remove { item_id } => {
                    rest.remove_checklist_item(channel_id, message_id, item_id)
                        .await
                }
            }
        };
        self.run_conversation_request(
            cx,
            request,
            failure_summary,
            "change checklist",
            move |session, answer, cx| {
                let answer = answer.map(|checklist| {
                    let change = session
                        .state
                        .apply_checklist(channel_id, message_id, &checklist);
                    Self::emit_changes(cx, vec![change]);
                });
                cx.defer(move |cx| on_done(answer, cx));
            },
        );
    }

    /// Its author or a moderator sets a new label and level; the card moves on the answer.
    pub fn update_progress_bar(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        bar: NewProgressBar,
        failure_summary: String,
        on_failed: impl FnOnce(String, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(rest) = self.cloned_rest_client() else {
            return;
        };
        self.run_conversation_request(
            cx,
            async move { rest.update_progress_bar(channel_id, message_id, &bar).await },
            failure_summary,
            "update progress bar",
            move |session, answer, cx| match answer {
                Ok(bar) => {
                    let change = session
                        .state
                        .apply_progress_bar(channel_id, message_id, &bar);
                    Self::emit_changes(cx, vec![change]);
                }
                Err(sentence) => cx.defer(move |cx| on_failed(sentence, cx)),
            },
        );
    }
}
