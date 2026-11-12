//! Who reacted with what (`ReactionsDialog.svelte`, DESIGN.md §5.4): one emoji at a time as tabs,
//! and the people who reacted with it, asked from the instance (the web's one page of 100) again
//! whenever the emoji or its count changes, so reactions added or taken back meanwhile show.
//! Moderators remove someone's reaction from here; focus then goes to the next person's Remove,
//! or to the dialog when nobody is left.

use std::collections::HashMap;

use gpui::{
    Context, Entity, FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _, Render,
    Role, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _,
};
use mellow_client_core::DownloadableFile;
use mellow_client_core::shared_logic::format::display_name;
use mellow_protocol::{CustomEmoji, PublicUser, ReactionSummary, Snowflake};

use crate::components::{Avatar, AvatarPerson, Button, ButtonVariant, Tabs, icon, with_type};
use crate::i18n::strings;
use crate::messages::{picture_animation, picture_for_box};
use crate::session::Session;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_DIALOG_SECTION_GAP, COMPONENT_MEMBER_ROW_AVATAR_SIZE, COMPONENT_MEMBER_ROW_HEIGHT,
    COMPONENT_SETTINGS_ERROR_ICON_GAP, COMPONENT_SETTINGS_ERROR_ICON_SIZE, SPACE_SM, TYPE_BODY,
    TYPE_MEMBER_NAME,
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
    /// `MANAGE_MESSAGES` in a server channel whose history is readable, read when it opened.
    can_remove_others: bool,
    /// Each person's Remove button, so focus can land on the next one after a removal.
    remove_focus: HashMap<Snowflake, FocusHandle>,
    /// Where the removed person's Remove button was, until the list reloads without them.
    removed_row_index: Option<usize>,
    /// Set when that reload arrives: the next frame, which has every row's handle, focuses there.
    focus_row_after_reload: Option<usize>,
    /// Where focus lands when no Remove button is left (the web falls back to the tab): the
    /// emoji's last reaction went, or every reaction did.
    body_focus: FocusHandle,
    error_message: Option<SharedString>,
    _session_changes: Subscription,
}

impl ReactionsDialogBody {
    pub(super) fn new(
        session: Entity<Session>,
        channel_id: Snowflake,
        message_id: Snowflake,
        custom_emojis: HashMap<Snowflake, CustomEmoji>,
        can_remove_others: bool,
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
            can_remove_others,
            remove_focus: HashMap::new(),
            removed_row_index: None,
            focus_row_after_reload: None,
            body_focus: cx.focus_handle().tab_stop(false),
            error_message: None,
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
                        if view.question == question {
                            view.show_who_reacted(answer, cx);
                        }
                    });
                },
                cx,
            );
        });
    }

    /// The people asked for; after a removal, the next frame moves focus (`focus_after_removal`).
    fn show_who_reacted(
        &mut self,
        answer: Result<Vec<PublicUser>, String>,
        cx: &mut Context<Self>,
    ) {
        match answer {
            Ok(users) => {
                self.users = users;
                self.load_state = LoadState::Loaded;
            }
            Err(_) => self.load_state = LoadState::Failed,
        }
        self.focus_row_after_reload = self.removed_row_index.take();
        cx.notify();
    }

    /// A moderator removes `user_id`'s reaction with `key`; the gateway's event updates the count,
    /// which asks again who reacted.
    fn remove(&mut self, key: String, user_id: Snowflake, cx: &mut Context<Self>) {
        self.error_message = None;
        // Noted before the request: its event may reload the list before the answer arrives.
        self.removed_row_index = self.users.iter().position(|user| user.id == user_id);
        let (channel_id, message_id) = (self.channel_id, self.message_id);
        let view = cx.entity().downgrade();
        let summary = strings(cx).reactions().remove_failed().to_owned();
        self.session.update(cx, |session, cx| {
            session.remove_user_reaction(
                channel_id,
                message_id,
                key,
                user_id,
                summary,
                move |sentence, cx| {
                    let _ = view.update(cx, |view, cx| {
                        view.removed_row_index = None;
                        view.error_message = Some(sentence.into());
                        cx.notify();
                    });
                },
                cx,
            );
        });
        cx.notify();
    }

    /// After a removal, the next person's Remove button, or the previous one at the end; nobody
    /// left, the dialog itself, so focus never leaves it.
    fn focus_after_removal(
        &mut self,
        remove_focus: &HashMap<Snowflake, FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.focus_row_after_reload.take() else {
            return;
        };
        let next = self
            .users
            .get(index.min(self.users.len().saturating_sub(1)))
            .and_then(|user| remove_focus.get(&user.id))
            .unwrap_or(&self.body_focus);
        window.focus(next, cx);
    }

    fn render_people(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
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
            LoadState::Failed => {
                // No Remove button is drawn: a pending focus falls back to the dialog.
                self.focus_after_removal(&HashMap::new(), window, cx);
                note(reactions.who_failed()).into_any_element()
            }
            LoadState::Loaded => {
                let selected_key = self.asked.as_ref().map(|(key, _)| key.clone());
                let can_remove = self.can_remove_others && selected_key.is_some();
                let mut remove_focus = std::mem::take(&mut self.remove_focus);
                remove_focus.retain(|user_id, _| self.users.iter().any(|user| user.id == *user_id));
                let view = cx.entity().downgrade();
                let list = div()
                    .id("reactions-people")
                    .role(Role::List)
                    .flex()
                    .flex_col()
                    .children(self.users.iter().enumerate().map(|(index, user)| {
                        let picture = user.avatar.as_ref().map(|avatar| {
                            picture_for_box(
                                DownloadableFile::ProfileImage(avatar.id),
                                COMPONENT_MEMBER_ROW_AVATAR_SIZE,
                                COMPONENT_MEMBER_ROW_AVATAR_SIZE,
                                picture_animation(cx),
                                window,
                            )
                        });
                        let name = display_name(user).to_owned();
                        let remove_button = can_remove.then(|| {
                            let focus = remove_focus
                                .entry(user.id)
                                .or_insert_with(|| cx.focus_handle())
                                .clone();
                            let (view, key, user_id) = (
                                view.clone(),
                                selected_key.clone().unwrap_or_default(),
                                user.id,
                            );
                            Button::new(("reaction-remove", index), reactions.remove_short())
                                .variant(ButtonVariant::Ghost)
                                .accessible_label(reactions.remove(&name))
                                .track_focus(&focus)
                                .on_click(move |_, cx| {
                                    let _ = view.update(cx, |view, cx| {
                                        view.remove(key.clone(), user_id, cx);
                                    });
                                })
                        });
                        div()
                            .id(("reactions-person", index))
                            .role(Role::ListItem)
                            .aria_label(name.clone())
                            .aria_position_in_set(index + 1)
                            .aria_size_of_set(self.users.len())
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
                                    .child(name),
                            )
                            .children(remove_button)
                    }));
                self.focus_after_removal(&remove_focus, window, cx);
                self.remove_focus = remove_focus;
                list.into_any_element()
            }
        }
    }

    /// A removal the instance refused, said at once.
    fn render_error(&self, cx: &Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        div().id("reactions-error").role(Role::Alert).when_some(
            self.error_message.clone(),
            |slot, error| {
                slot.child(
                    with_type(div(), TYPE_BODY)
                        .flex()
                        .gap(length(COMPONENT_SETTINGS_ERROR_ICON_GAP))
                        .text_color(color(tokens.component_settings_error_text))
                        .child(icon(
                            IconName::WarningCircle,
                            COMPONENT_SETTINGS_ERROR_ICON_SIZE,
                            color(tokens.component_settings_error_icon),
                        ))
                        .child(error),
                )
            },
        )
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
            if self.removed_row_index.take().is_some() {
                window.focus(&self.body_focus, cx);
            }
            return with_type(div(), TYPE_BODY)
                .track_focus(&self.body_focus)
                .text_color(color(theme(cx).color_text_muted))
                .child(catalogue.reactions().no_reactions())
                .into_any_element();
        };
        let (selected_key, selected_count) = (selected.emoji.clone(), selected.count);
        self.ask_who_reacted(&selected_key, selected_count, cx);
        let (labels, accessible_labels): (Vec<SharedString>, Vec<SharedString>) = reactions
            .iter()
            .map(|reaction| {
                let emoji = super::reaction_label(&reaction.emoji, &self.custom_emojis, catalogue);
                (
                    SharedString::from(format!("{emoji} {}", reaction.count)),
                    catalogue
                        .reactions()
                        .chip(&emoji, i64::from(reaction.count))
                        .into(),
                )
            })
            .unzip();
        let keys: Vec<String> = reactions
            .into_iter()
            .map(|reaction| reaction.emoji)
            .collect();
        let view = cx.entity().downgrade();
        div()
            .track_focus(&self.body_focus)
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
                .accessible_labels(accessible_labels)
                .on_select(move |index, _, cx| {
                    let key = keys.get(index).cloned();
                    let _ = view.update(cx, |view, cx| {
                        view.chosen_key = key;
                        cx.notify();
                    });
                }),
            )
            .child(self.render_error(cx))
            .child(self.render_people(window, cx))
            .into_any_element()
    }
}

#[cfg(test)]
#[path = "reactions_dialog_tests.rs"]
mod tests;
