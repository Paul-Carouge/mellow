//! What a server's menu asks of the session (the web's `client.leaveServer`): leaving it. The
//! server goes from the state as soon as the instance answers; the gateway's `GUILD_DELETE` that
//! follows changes nothing more.

use gpui::{App, Context};
use mellow_protocol::{GatewayEvent, Snowflake};

use super::{Session, StateChanged, now_unix_milliseconds};
use crate::sign_in_form::describe_error;
use crate::strings_pending_i18n::strings;

impl Session {
    /// Leaves `guild_id`; `on_done` gets the failure in words (`failure_summary` first), deferred
    /// so it may read the session.
    pub fn leave_server(
        &mut self,
        guild_id: Snowflake,
        failure_summary: String,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<(), String>, &mut App) + 'static,
    ) {
        let Some(rest) = self.rest_client().cloned() else {
            return;
        };
        self.run_request(
            cx,
            async move { rest.leave_guild(guild_id).await },
            move |session, answer, cx| match answer {
                Ok(()) => {
                    let change = session.state.apply_event(
                        GatewayEvent::GuildDelete { id: guild_id },
                        now_unix_milliseconds(),
                    );
                    cx.emit(StateChanged(vec![change]));
                    cx.notify();
                    cx.defer(move |cx| on_done(Ok(()), cx));
                }
                Err(error) => {
                    let message = describe_error(&error, &failure_summary, strings(cx));
                    session.report_request_error(&error, "leave server", cx);
                    cx.defer(move |cx| on_done(Err(message), cx));
                }
            },
        );
    }
}
