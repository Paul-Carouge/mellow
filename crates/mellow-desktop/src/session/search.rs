//! Message search (`client.searchMessages`, ADR-0029): a server's channels, or the reader's
//! conversations, searched by the instance with the reader's days.

use chrono::{Local, Offset as _};
use gpui::{App, Context};
use mellow_client_core::ClientError;
use mellow_client_core::shared_logic::message_search::SearchRequest;
use mellow_protocol::MessageSearchResults;

use super::Session;

/// Minutes east of UTC now, as the web's `-getTimezoneOffset()`: `before:` and `during:` days are
/// the reader's. Every real offset lies within the instance's -840 to 840.
fn local_utc_offset_minutes() -> i16 {
    let seconds = Local::now().offset().fix().local_minus_utc();
    i16::try_from(seconds / 60).unwrap_or(0)
}

impl Session {
    /// Sends one page of a search; `on_done` gets the answer, deferred so it may read the session.
    pub fn search_messages(
        &mut self,
        request: &SearchRequest,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<MessageSearchResults, ClientError>, &mut App) + 'static,
    ) {
        let Some(rest) = self.rest_client().cloned() else {
            cx.defer(move |cx| on_done(Err(ClientError::NotSignedIn), cx));
            return;
        };
        let body = request.body(local_utc_offset_minutes());
        let guild_id = request.scope_guild_id;
        self.run_request(
            cx,
            async move {
                match guild_id {
                    Some(guild_id) => rest.search_guild_messages(guild_id, &body).await,
                    None => rest.search_my_conversation_messages(&body).await,
                }
            },
            move |session, answer, cx| {
                if let Err(error) = &answer {
                    session.report_request_error(error, "search messages", cx);
                }
                cx.defer(move |cx| on_done(answer, cx));
            },
        );
    }
}
