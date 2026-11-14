//! What the rail's "+" and a server's menu ask of the session (the web's `client.createServer`,
//! `joinServer`, `createChannel` and `leaveServer`). A server or channel created, joined or left
//! changes the state as soon as the instance answers, so the reader lands on it at once; the
//! gateway's event that follows changes nothing more.

use std::future::Future;

use gpui::{App, Context};
use mellow_client_core::{ClientError, RestClient};
use mellow_protocol::{
    Channel, CreateChannelRequest, GatewayEvent, Guild, InitialCategory, Snowflake,
};

use super::{AfterSignOut, Session, StateChanged, now_unix_milliseconds};
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
            // A session being torn down: answer anyway, or the busy dialog could never close.
            cx.defer(move |cx| on_done(Err(failure_summary), cx));
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

    /// Creates a server starting with `initial_categories` (`None`: the instance's default);
    /// `on_done` gets its id, or the failure in words (`failure_summary` first).
    pub fn create_server(
        &mut self,
        name: String,
        initial_categories: Option<Vec<InitialCategory>>,
        failure_summary: String,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<Snowflake, String>, &mut App) + 'static,
    ) {
        self.add_server_from(
            "create server",
            move |rest| async move {
                rest.create_guild_with_categories(&name, initial_categories)
                    .await
            },
            failure_summary,
            cx,
            on_done,
        );
    }

    /// Joins the server an invite code opens; `on_done` gets its id, or the failure in words.
    pub fn join_server(
        &mut self,
        code: String,
        failure_summary: String,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<Snowflake, String>, &mut App) + 'static,
    ) {
        self.add_server_from(
            "join server",
            move |rest| async move { rest.join_with_invite(&code).await },
            failure_summary,
            cx,
            on_done,
        );
    }

    fn add_server_from<Request>(
        &mut self,
        action: &'static str,
        request: impl FnOnce(RestClient) -> Request,
        failure_summary: String,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<Snowflake, String>, &mut App) + 'static,
    ) where
        Request: Future<Output = Result<Guild, ClientError>> + Send + 'static,
    {
        let Some(rest) = self.rest_client().cloned() else {
            cx.defer(move |cx| on_done(Err(failure_summary), cx));
            return;
        };
        self.run_request(cx, request(rest), move |session, answer, cx| match answer {
            Ok(guild) => {
                let guild_id = guild.id;
                let change = session.state.apply_event(
                    GatewayEvent::GuildCreate(Box::new(guild)),
                    now_unix_milliseconds(),
                );
                cx.emit(StateChanged(vec![change]));
                cx.notify();
                cx.defer(move |cx| on_done(Ok(guild_id), cx));
            }
            Err(error) => {
                let message = describe_error(&error, &failure_summary, strings(cx));
                session.report_request_error(&error, action, cx);
                cx.defer(move |cx| on_done(Err(message), cx));
            }
        });
    }

    /// Creates a channel or a category in `guild_id`; `on_done` gets it, or the failure in words.
    pub fn create_channel(
        &mut self,
        guild_id: Snowflake,
        request: CreateChannelRequest,
        failure_summary: String,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<Channel, String>, &mut App) + 'static,
    ) {
        let Some(rest) = self.rest_client().cloned() else {
            cx.defer(move |cx| on_done(Err(failure_summary), cx));
            return;
        };
        self.run_request(
            cx,
            async move { rest.create_channel(guild_id, &request).await },
            move |session, answer, cx| match answer {
                Ok(channel) => {
                    let change = session.state.apply_event(
                        GatewayEvent::ChannelCreate(channel.clone()),
                        now_unix_milliseconds(),
                    );
                    cx.emit(StateChanged(vec![change]));
                    cx.notify();
                    cx.defer(move |cx| on_done(Ok(channel), cx));
                }
                Err(error) => {
                    let message = describe_error(&error, &failure_summary, strings(cx));
                    session.report_request_error(&error, "create channel", cx);
                    cx.defer(move |cx| on_done(Err(message), cx));
                }
            },
        );
    }

    /// Signs out to sign in to another instance ("Connect to another instance" in the rail's
    /// "+"): one session at a time is kept, so the current one ends first. A failure shows in
    /// `sign_out_error`, as for "Sign out".
    pub fn sign_out_for_another_instance(&mut self, cx: &mut Context<Self>) {
        self.after_sign_out = AfterSignOut::AnotherInstance;
        self.sign_out(cx);
    }

    /// Whether the session just ended to pick another instance; asking forgets it.
    pub fn take_leaving_for_another_instance(&mut self) -> bool {
        std::mem::replace(&mut self.after_sign_out, AfterSignOut::SignIn)
            == AfterSignOut::AnotherInstance
    }

    #[must_use]
    pub fn is_signing_out(&self) -> bool {
        self.is_signing_out
    }
}
