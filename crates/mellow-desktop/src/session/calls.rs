//! Starting and declining private calls (ADR-0062), as the web's `startCall` and `declineCall`:
//! answering and joining go through the voice session with the conversation's id.

use gpui::{App, Context};
use mellow_client_core::ClientError;
use mellow_protocol::{GatewayEvent, Snowflake};

use super::{Session, StateChanged, now_unix_milliseconds};

impl Session {
    /// Starts the conversation's call, or answers the one going on, then hands the outcome to
    /// `on_done`. The call is known at once: its answer may come before the gateway's event, and
    /// both carry the whole call.
    pub fn start_call(
        &mut self,
        channel_id: Snowflake,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<(), ClientError>, &mut App) + 'static,
    ) {
        let Some(rest) = self.rest_client().cloned() else {
            // Later: the caller may be inside its own update.
            App::defer(cx, move |cx| on_done(Err(ClientError::NotSignedIn), cx));
            return;
        };
        self.run_request(
            cx,
            async move { rest.start_call(channel_id).await },
            move |session, answer, cx| match answer {
                Ok(call) => {
                    if session.state.call(channel_id).is_none() {
                        let change = session
                            .state
                            .apply_event(GatewayEvent::CallCreate(call), now_unix_milliseconds());
                        cx.emit(StateChanged(vec![change]));
                        cx.notify();
                    }
                    App::defer(cx, move |cx| on_done(Ok(()), cx));
                }
                Err(error) => {
                    session.report_request_error(&error, "start a call", cx);
                    App::defer(cx, move |cx| on_done(Err(error), cx));
                }
            },
        );
    }

    /// Stops the call ringing on the user's other devices. A failure (offline) leaves them to
    /// stop when ringing times out.
    pub fn decline_call(&mut self, channel_id: Snowflake, cx: &mut Context<Self>) {
        let Some(rest) = self.rest_client().cloned() else {
            return;
        };
        self.run_request(
            cx,
            async move { rest.decline_call(channel_id).await },
            move |session, answer, cx| {
                if let Err(error) = answer {
                    session.report_request_error(&error, "decline a call", cx);
                }
            },
        );
    }
}
