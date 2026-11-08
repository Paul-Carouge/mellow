//! What the Friends space and group conversations ask of the session (the web's
//! `relationships.svelte.ts` and `client.svelte.ts`, ADR-0025, ADR-0026): friend requests,
//! answers, blocks, and groups created, renamed, joined and left. Each change runs on the network
//! runtime through `run_request` and is applied to the state as soon as the instance answers; the
//! gateway brings the same change again, which is harmless. The view gets the error itself, to put
//! a refusal (an unknown name, a full group) into its own words.

use std::future::Future;

use gpui::{App, Context};
use mellow_client_core::state::{ClientState, StateChange};
use mellow_client_core::{ClientError, RestClient};
use mellow_protocol::{
    ApiErrorCode, CreateGroupConversationRequest, Relationship, SendFriendRequest,
    SetRelationshipKind, Snowflake,
};

use super::{Session, StateChanged};

impl Session {
    /// Runs `request`, applies its answer with `apply`, then tells `on_done`. A session that is
    /// not signed in answers `NotSignedIn` right after the caller's update, so a waiting view never
    /// stays busy and can still update itself.
    fn run_change<Answer, Outcome, Request>(
        &mut self,
        cx: &mut Context<Self>,
        action: &'static str,
        request: impl FnOnce(RestClient) -> Request,
        apply: impl FnOnce(&mut ClientState, Answer) -> (StateChange, Outcome) + 'static,
        on_done: impl FnOnce(Result<Outcome, ClientError>, &mut App) + 'static,
    ) where
        Answer: Send + 'static,
        Outcome: 'static,
        Request: Future<Output = Result<Answer, ClientError>> + Send + 'static,
    {
        let Some(rest) = self.rest_client().cloned() else {
            cx.defer(move |cx| on_done(Err(ClientError::NotSignedIn), cx));
            return;
        };
        self.run_request(cx, request(rest), move |session, answer, cx| match answer {
            Ok(answer) => {
                let (change, outcome) = apply(&mut session.state, answer);
                if change != StateChange::Nothing {
                    cx.emit(StateChanged(vec![change]));
                }
                cx.notify();
                on_done(Ok(outcome), cx);
            }
            Err(error) => {
                session.report_request_error(&error, action, cx);
                on_done(Err(error), cx);
            }
        });
    }

    /// Asks the account with this name or friend code to be friends, or accepts their request.
    pub fn send_friend_request(
        &mut self,
        request: SendFriendRequest,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<Relationship, ClientError>, &mut App) + 'static,
    ) {
        self.run_change(
            cx,
            "send friend request",
            move |rest| async move { rest.send_friend_request(&request).await },
            |state, relationship| (state.apply_relationship(relationship.clone()), relationship),
            on_done,
        );
    }

    /// `Friend` accepts their request; `Blocked` blocks them, whatever the relationship was.
    pub fn set_relationship(
        &mut self,
        user_id: Snowflake,
        kind: SetRelationshipKind,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<(), ClientError>, &mut App) + 'static,
    ) {
        self.run_change(
            cx,
            "change relationship",
            move |rest| async move { rest.set_relationship(user_id, kind).await },
            |state, relationship| (state.apply_relationship(relationship), ()),
            on_done,
        );
    }

    /// Removes a friend, refuses or cancels a request, or unblocks. Already gone (the other person
    /// removed it, or another session did) counts as done: the wanted state is reached.
    pub fn remove_relationship(
        &mut self,
        user_id: Snowflake,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<(), ClientError>, &mut App) + 'static,
    ) {
        self.run_change(
            cx,
            "remove relationship",
            move |rest| async move {
                match rest.remove_relationship(user_id).await {
                    Err(ClientError::Api {
                        code: ApiErrorCode::NotFound,
                        ..
                    }) => Ok(()),
                    answer => answer,
                }
            },
            move |state, ()| (state.forget_relationship(user_id), ()),
            on_done,
        );
    }

    /// A new group with these friends, named or not; `on_done` gets its id.
    pub fn create_group_conversation(
        &mut self,
        recipient_ids: Vec<Snowflake>,
        name: &str,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<Snowflake, ClientError>, &mut App) + 'static,
    ) {
        let request = CreateGroupConversationRequest {
            recipient_ids,
            name: name.trim().to_owned(),
        };
        self.run_change(
            cx,
            "create group",
            move |rest| async move { rest.create_group_conversation(&request).await },
            |state, group| {
                let group_id = group.id;
                (state.upsert_direct_message_channel(group), group_id)
            },
            on_done,
        );
    }

    /// Anyone in a group renames it; an empty name brings back its people's names.
    pub fn rename_group_conversation(
        &mut self,
        channel_id: Snowflake,
        name: &str,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<(), ClientError>, &mut App) + 'static,
    ) {
        let name = name.trim().to_owned();
        self.run_change(
            cx,
            "rename group",
            move |rest| async move { rest.rename_group_conversation(channel_id, &name).await },
            |state, group| (state.apply_group_answer(group), ()),
            on_done,
        );
    }

    /// Adds a friend to a group.
    pub fn add_group_participant(
        &mut self,
        channel_id: Snowflake,
        user_id: Snowflake,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<(), ClientError>, &mut App) + 'static,
    ) {
        self.run_change(
            cx,
            "add to group",
            move |rest| async move { rest.add_group_participant(channel_id, user_id).await },
            |state, group| (state.apply_group_answer(group), ()),
            on_done,
        );
    }

    /// The owner removes someone, who leaves the group at once (not on the gateway's update), so
    /// focus can move to the next row.
    pub fn remove_group_participant(
        &mut self,
        channel_id: Snowflake,
        user_id: Snowflake,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<(), ClientError>, &mut App) + 'static,
    ) {
        self.run_change(
            cx,
            "remove from group",
            move |rest| async move { rest.remove_group_participant(channel_id, user_id).await },
            move |state, ()| (state.remove_group_participant(channel_id, user_id), ()),
            on_done,
        );
    }

    /// Leaves a group: it goes from the list, with everything kept about it.
    pub fn leave_group_conversation(
        &mut self,
        channel_id: Snowflake,
        cx: &mut Context<Self>,
        on_done: impl FnOnce(Result<(), ClientError>, &mut App) + 'static,
    ) {
        let Some(user_id) = self.state.current_user().map(|user| user.id) else {
            cx.defer(move |cx| on_done(Err(ClientError::NotSignedIn), cx));
            return;
        };
        self.run_change(
            cx,
            "leave group",
            move |rest| async move { rest.remove_group_participant(channel_id, user_id).await },
            move |state, ()| (state.remove_direct_message_channel(channel_id), ()),
            on_done,
        );
    }
}
