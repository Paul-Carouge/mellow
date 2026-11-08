//! Unblocking from a conversation locked by a block (ADR-0025): the composer's notice offers it,
//! as `Composer.svelte` does.

use gpui::{App, Context};
use mellow_client_core::ClientError;
use mellow_protocol::{ApiErrorCode, Snowflake};

use super::{Session, StateChanged};
use crate::sign_in_form::describe_error;
use crate::strings_pending_i18n::strings;

impl Session {
    /// Unblocks `user_id`, then hands what went wrong, in words, to `on_done`. A removal answered
    /// `NOT_FOUND` counts as done, as on the web: another session already removed it.
    pub fn unblock(
        &mut self,
        user_id: Snowflake,
        failure_summary: String,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<(), String>, &mut App) + 'static,
    ) {
        let Some(rest) = self.rest_client().cloned() else {
            // Later: the caller may be inside its own update.
            App::defer(cx, move |cx| on_done(Err(failure_summary), cx));
            return;
        };
        self.run_request(
            cx,
            async move { rest.remove_relationship(user_id).await },
            move |session, answer, cx| match answer {
                Ok(())
                | Err(ClientError::Api {
                    code: ApiErrorCode::NotFound,
                    ..
                }) => {
                    let change = session.state.forget_relationship(user_id);
                    cx.emit(StateChanged(vec![change]));
                    cx.notify();
                    on_done(Ok(()), cx);
                }
                Err(error) => {
                    let message = describe_error(&error, &failure_summary, strings(cx));
                    session.report_request_error(&error, "unblock", cx);
                    on_done(Err(message), cx);
                }
            },
        );
    }
}
