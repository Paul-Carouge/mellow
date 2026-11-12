//! What the settings pages ask of the instance (the web's `client.renameServer`,
//! `client.updateChannel`, …). As on the web, an edit is applied from the gateway's echo, never
//! from the answer (a late answer would put older state back): the page only learns whether the
//! instance took it. Only a created role and removals are read from the answer.

use std::future::Future;

use gpui::{App, Context};
use mellow_client_core::{ClientError, RestClient};

use super::Session;
use crate::sign_in_form::describe_error;
use crate::strings_pending_i18n::strings;

impl Session {
    /// Sends one settings change built by `request` from the REST client; `on_done` gets the
    /// failure in words (`failure_summary` first), deferred so it may read the session. `action`
    /// names the request in logs, never what it carries.
    pub fn send_settings_change<Answer, Request>(
        &mut self,
        action: &'static str,
        failure_summary: String,
        request: impl FnOnce(RestClient) -> Request,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<(), String>, &mut App) + 'static,
    ) where
        Answer: Send + 'static,
        Request: Future<Output = Result<Answer, ClientError>> + Send + 'static,
    {
        self.send_settings_request(action, request, cx, move |answer, cx| {
            let outcome = answer
                .map(|_| ())
                .map_err(|error| describe_error(&error, &failure_summary, strings(cx)));
            on_done(outcome, cx);
        });
    }

    /// As [`Self::send_settings_change`], for a page that needs the answer (a created role, a
    /// list it reads) or words its own failures (a role icon or an emoji too large); a lost session
    /// answers `NotSignedIn`.
    pub fn send_settings_request<Answer, Request>(
        &mut self,
        action: &'static str,
        request: impl FnOnce(RestClient) -> Request,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<Answer, ClientError>, &mut App) + 'static,
    ) where
        Answer: Send + 'static,
        Request: Future<Output = Result<Answer, ClientError>> + Send + 'static,
    {
        let Some(rest) = self.rest_client().cloned() else {
            // A session being torn down: answer anyway, or the page would stay busy.
            cx.defer(move |cx| on_done(Err(ClientError::NotSignedIn), cx));
            return;
        };
        self.run_request(cx, request(rest), move |session, answer, cx| {
            if let Err(error) = &answer {
                session.report_request_error(error, action, cx);
            }
            cx.defer(move |cx| on_done(answer, cx));
        });
    }
}
