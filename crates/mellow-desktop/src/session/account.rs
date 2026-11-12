//! What the user settings ask of the instance about the account itself (the web's
//! `client.updateDisplayName`, `client.changePassword`, `client.rest.twoFactorStatus`, …). Unlike
//! server settings, the page needs the answer (a profile, a secret, recovery codes) and the
//! failure's code (a wrong current password), so both reach it as they are; the answer of a
//! change to the account is applied at once, as the web does, the gateway's `USER_UPDATE` being
//! the same. `action` names the request in logs, never what it carries.

use std::future::Future;

use gpui::{App, Context};
use mellow_client_core::{ClientError, RestClient};
use mellow_protocol::CurrentUser;

use super::{Session, StateChanged};

impl Session {
    /// Sends one account request built by `request` from the REST client; `on_done` gets its
    /// answer, deferred so it may read the session.
    pub fn send_account_request<Answer, Request>(
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

    /// Like [`Self::send_account_request`] for a change answered with the account, which is
    /// shown at once (name, avatar, banner).
    pub fn send_current_user_change<Request>(
        &mut self,
        action: &'static str,
        request: impl FnOnce(RestClient) -> Request,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<(), ClientError>, &mut App) + 'static,
    ) where
        Request: Future<Output = Result<CurrentUser, ClientError>> + Send + 'static,
    {
        let Some(rest) = self.rest_client().cloned() else {
            cx.defer(move |cx| on_done(Err(ClientError::NotSignedIn), cx));
            return;
        };
        self.run_request(cx, request(rest), move |session, answer, cx| {
            let outcome = match answer {
                Ok(user) => {
                    let change = session.state.apply_current_user(user);
                    cx.emit(StateChanged(vec![change]));
                    cx.notify();
                    Ok(())
                }
                Err(error) => {
                    session.report_request_error(&error, action, cx);
                    Err(error)
                }
            };
            cx.defer(move |cx| on_done(outcome, cx));
        });
    }
}
