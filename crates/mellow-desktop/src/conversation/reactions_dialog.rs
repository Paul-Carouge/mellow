//! Who reacted with what (`ReactionsDialog.svelte`, DESIGN.md §5.4): one emoji at a time as tabs,
//! and the people who reacted with it, asked from the instance (the web's one page of 100) again
//! whenever the emoji or its count changes, so reactions added or taken back meanwhile show.
//!
//! shortcut: moderators cannot remove someone's reaction from here yet (the web's Remove); add it
//! with the other moderator actions on reactions ("Remove all reactions").

use std::collections::HashMap;

use gpui::{
    Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
};
use mellow_client_core::DownloadableFile;
use mellow_client_core::shared_logic::format::display_name;
use mellow_protocol::{CustomEmoji, PublicUser, ReactionSummary, Snowflake};

use crate::components::{Avatar, AvatarPerson, Tabs, with_type};
use crate::i18n::strings;
use crate::messages::{picture_animation, picture_for_box};
use crate::session::Session;
use crate::theme::tokens::{
    COMPONENT_DIALOG_SECTION_GAP, COMPONENT_MEMBER_ROW_AVATAR_SIZE, COMPONENT_MEMBER_ROW_HEIGHT,
    SPACE_SM, TYPE_BODY, TYPE_MEMBER_NAME,
};
use crate::theme::{color, length, theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoadState {
    Loading,
    Loaded,
    Failed,
}

pub(super) struct ReactionsDialogBody {
    session: Entity<Session>,
    channel_id: Snowflake,
    message_id: Snowflake,
    custom_emojis: HashMap<Snowflake, CustomEmoji>,
    /// The tab chosen; a removed emoji falls back to the first one left.
    chosen_key: Option<String>,
    users: Vec<PublicUser>,
    load_state: LoadState,
    /// The emoji and count `users` were last asked for.
    asked: Option<(String, u32)>,
    /// Answers to an earlier question are dropped.
    question: u64,
    _session_changes: Subscription,
}

impl ReactionsDialogBody {
    pub(super) fn new(
        session: Entity<Session>,
        channel_id: Snowflake,
        message_id: Snowflake,
        custom_emojis: HashMap<Snowflake, CustomEmoji>,
        cx: &mut Context<Self>,
    ) -> Self {
        // Counts move with gateway events: the tabs follow, and a new count asks again.
        let session_changes = cx.observe(&session, |_, _, cx| cx.notify());
        Self {
            session,
            channel_id,
            message_id,
            custom_emojis,
            chosen_key: None,
            users: Vec::new(),
            load_state: LoadState::Loading,
            asked: None,
            question: 0,
            _session_changes: session_changes,
        }
    }

    fn reactions(&self, cx: &gpui::App) -> Vec<ReactionSummary> {
        self.session
            .read(cx)
            .state()
            .cached_message(self.channel_id, self.message_id)
            .map(|message| message.reactions.clone())
            .unwrap_or_default()
    }

    /// Asks who reacted with `key` unless that count was already asked; the people shown stay
    /// while the same emoji reloads.
    fn ask_who_reacted(&mut self, key: &str, count: u32, cx: &mut Context<Self>) {
        if self
            .asked
            .as_ref()
            .is_some_and(|(asked_key, asked_count)| asked_key == key && *asked_count == count)
        {
            return;
        }
        if self
            .asked
            .as_ref()
            .is_none_or(|(asked_key, _)| asked_key != key)
        {
            self.users.clear();
            self.load_state = LoadState::Loading;
        }
        self.asked = Some((key.to_owned(), count));
        self.question += 1;
        let question = self.question;
        let view = cx.entity().downgrade();
        let failure = strings(cx).reactions().who_failed().to_owned();
        let (channel_id, message_id, key) = (self.channel_id, self.message_id, key.to_owned());
        self.session.update(cx, |session, cx| {
            session.list_reacting_users(
                channel_id,
                message_id,
                key,
                failure,
                move |answer, cx| {
                    let _ = view.update(cx, |view, cx| {
                        if view.question != question {
                            return;
                        }
                        match answer {
                            Ok(users) => {
                                view.users = users;
                                view.load_state = LoadState::Loaded;
                            }
                            Err(_) => view.load_state = LoadState::Failed,
                        }
                        cx.notify();
                    });
                },
                cx,
            );
        });
    }

    fn render_people(&self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let tokens = theme(cx);
        let reactions = strings(cx).reactions();
        let note = |text: &'static str| {
            with_type(div(), TYPE_BODY)
                .text_color(color(tokens.color_text_muted))
                .child(text)
        };
        match self.load_state {
            LoadState::Loading => note(reactions.loading_who())
                .id("reactions-loading")
                .role(Role::Status)
                .into_any_element(),
            LoadState::Failed => note(reactions.who_failed()).into_any_element(),
            LoadState::Loaded => div()
                .id("reactions-people")
                .role(Role::List)
                .flex()
                .flex_col()
                .children(self.users.iter().map(|user| {
                    let picture = user.avatar.as_ref().map(|avatar| {
                        picture_for_box(
                            DownloadableFile::ProfileImage(avatar.id),
                            COMPONENT_MEMBER_ROW_AVATAR_SIZE,
                            COMPONENT_MEMBER_ROW_AVATAR_SIZE,
                            picture_animation(cx),
                            window,
                        )
                    });
                    div()
                        .flex()
                        .items_center()
                        .gap(length(SPACE_SM))
                        .min_h(length(COMPONENT_MEMBER_ROW_HEIGHT))
                        .child(Avatar::new(
                            AvatarPerson::from_user(user, picture),
                            COMPONENT_MEMBER_ROW_AVATAR_SIZE,
                        ))
                        .child(
                            with_type(div(), TYPE_MEMBER_NAME)
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_color(color(tokens.color_text_normal))
                                .child(display_name(user).to_owned()),
                        )
                }))
                .into_any_element(),
        }
    }
}

impl Render for ReactionsDialogBody {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let catalogue = strings(cx);
        let reactions = self.reactions(cx);
        let selected_index = self
            .chosen_key
            .as_ref()
            .and_then(|key| reactions.iter().position(|reaction| reaction.emoji == *key))
            .unwrap_or(0);
        let Some(selected) = reactions.get(selected_index) else {
            return with_type(div(), TYPE_BODY)
                .text_color(color(theme(cx).color_text_muted))
                .child(catalogue.reactions().no_reactions())
                .into_any_element();
        };
        let (selected_key, selected_count) = (selected.emoji.clone(), selected.count);
        self.ask_who_reacted(&selected_key, selected_count, cx);
        let labels: Vec<SharedString> = reactions
            .iter()
            .map(|reaction| {
                let emoji = super::reaction_label(&reaction.emoji, &self.custom_emojis, catalogue);
                SharedString::from(format!("{emoji} {}", reaction.count))
            })
            .collect();
        let keys: Vec<String> = reactions
            .into_iter()
            .map(|reaction| reaction.emoji)
            .collect();
        let view = cx.entity().downgrade();
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_DIALOG_SECTION_GAP))
            .child(
                Tabs::new(
                    "reaction-tabs",
                    catalogue.reactions().reactions(),
                    labels,
                    selected_index,
                )
                .on_select(move |index, _, cx| {
                    let key = keys.get(index).cloned();
                    let _ = view.update(cx, |view, cx| {
                        view.chosen_key = key;
                        cx.notify();
                    });
                }),
            )
            .child(self.render_people(window, cx))
            .into_any_element()
    }
}
