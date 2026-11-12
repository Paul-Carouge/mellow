//! Bans (`BanSettings.svelte`, DESIGN.md §5.8): who is banned, why and since when, newest first,
//! with "Revoke ban". No gateway event tells about bans, so the list is read when the page opens.

use std::collections::HashMap;

use gpui::{
    Context, Entity, EventEmitter, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, div,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::server_settings::focus_after_removal;
use mellow_protocol::{Ban, Snowflake};

use super::list_rows::{
    empty_sentence, list_row, list_status, person_avatar, secondary_line, status_notice,
    text_column,
};
use crate::components::{Button, with_type};
use crate::i18n::strings;
use crate::messages::{full_timestamp_label, local_date_time};
use crate::session::Session;
use crate::settings::{SettingsPage, SettingsPageEvent, error_alert};
use crate::sign_in_form::describe_error;
use crate::theme::tokens::{TYPE_BODY, TYPE_MEMBER_NAME};
use crate::theme::{color, theme};

pub struct ServerBansPage {
    session: Entity<Session>,
    guild_id: Snowflake,
    /// `None` while loading or after a failure.
    bans: Option<Vec<Ban>>,
    has_failed: bool,
    /// Answers to an earlier load are dropped.
    load_generation: u64,
    busy_user_id: Option<Snowflake>,
    error_message: Option<SharedString>,
    notice: SharedString,
    revoke_focus: HashMap<Snowflake, FocusHandle>,
    empty_focus: FocusHandle,
    /// Where focus goes once the next frame is drawn: a revoked row takes its button with it.
    pending_focus: Option<FocusHandle>,
}

impl EventEmitter<SettingsPageEvent> for ServerBansPage {}

impl SettingsPage for ServerBansPage {
    fn set_attention(&mut self, _attention: u32, _cx: &mut Context<Self>) {}
}

impl std::fmt::Debug for ServerBansPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerBansPage")
            .field("guild_id", &self.guild_id)
            .field("busy_user_id", &self.busy_user_id)
            .finish_non_exhaustive()
    }
}

impl ServerBansPage {
    pub fn new(session: Entity<Session>, guild_id: Snowflake, cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            session,
            guild_id,
            bans: None,
            has_failed: false,
            load_generation: 0,
            busy_user_id: None,
            error_message: None,
            notice: SharedString::default(),
            revoke_focus: HashMap::new(),
            empty_focus: cx.focus_handle().tab_stop(false),
            pending_focus: None,
        };
        page.load(cx);
        page
    }

    #[cfg(test)]
    pub(crate) fn bans(&self) -> Option<&[Ban]> {
        self.bans.as_deref()
    }

    fn load(&mut self, cx: &mut Context<Self>) {
        self.has_failed = false;
        self.bans = None;
        self.load_generation += 1;
        let generation = self.load_generation;
        let guild_id = self.guild_id;
        let page = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.send_settings_request(
                "list bans",
                move |rest| async move { rest.list_bans(guild_id).await },
                cx,
                move |answer, cx| {
                    let _ = page.update(cx, |page, cx| {
                        if page.load_generation != generation {
                            return;
                        }
                        match answer {
                            Ok(bans) => page.bans = Some(bans),
                            Err(_) => page.has_failed = true,
                        }
                        cx.notify();
                    });
                },
            );
        });
        cx.notify();
    }

    pub(crate) fn revoke(&mut self, user_id: Snowflake, cx: &mut Context<Self>) {
        if self.busy_user_id.is_some() {
            return;
        }
        let Some(name) = self.bans.as_ref().and_then(|bans| {
            bans.iter()
                .find(|ban| ban.user.id == user_id)
                .map(|ban| display_name(&ban.user).to_owned())
        }) else {
            return;
        };
        self.busy_user_id = Some(user_id);
        self.error_message = None;
        let guild_id = self.guild_id;
        let page = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.send_settings_request(
                "revoke a ban",
                move |rest| async move { rest.unban_member(guild_id, user_id).await },
                cx,
                move |answer, cx| {
                    let _ = page.update(cx, |page, cx| {
                        page.busy_user_id = None;
                        let words = strings(cx).server_settings().bans();
                        match answer {
                            Ok(()) => page.forget_ban(user_id, words.revoked(&name).into()),
                            Err(error) => {
                                page.error_message = Some(
                                    describe_error(
                                        &error,
                                        &words.revoke_failed(&name),
                                        crate::strings_pending_i18n::strings(cx),
                                    )
                                    .into(),
                                );
                            }
                        }
                        cx.notify();
                    });
                },
            );
        });
        cx.notify();
    }

    /// The button pressed goes with its row: focus moves to a neighbour, or to the empty text.
    fn forget_ban(&mut self, user_id: Snowflake, notice: SharedString) {
        let Some(bans) = self.bans.as_mut() else {
            return;
        };
        let row_ids: Vec<Snowflake> = bans.iter().map(|ban| ban.user.id).collect();
        let next = focus_after_removal(&row_ids, user_id);
        bans.retain(|ban| ban.user.id != user_id);
        self.revoke_focus.remove(&user_id);
        self.pending_focus = Some(
            next.and_then(|next| self.revoke_focus.get(&next).cloned())
                .unwrap_or_else(|| self.empty_focus.clone()),
        );
        self.notice = notice;
    }
}

impl Render for ServerBansPage {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(focus) = self.pending_focus.take() {
            window.focus(&focus, cx);
        }
        let catalogue = strings(cx);
        let words = catalogue.server_settings().bans();
        let notice = status_notice("bans-notice", self.notice.clone());
        let Some(bans) = self.bans.as_ref() else {
            let page = cx.entity().downgrade();
            return div()
                .child(list_status(
                    "bans-status",
                    self.has_failed,
                    words.load_failed(),
                    move |_, cx| {
                        let _ = page.update(cx, ServerBansPage::load);
                    },
                    cx,
                ))
                .child(notice)
                .into_any_element();
        };
        if bans.is_empty() {
            let guild_name = self
                .session
                .read(cx)
                .state()
                .guild(self.guild_id)
                .map(|guild| guild.name.clone())
                .unwrap_or_default();
            return div()
                .child(
                    empty_sentence(words.empty(&guild_name), cx)
                        .id("bans-empty")
                        .track_focus(&self.empty_focus),
                )
                .child(notice)
                .into_any_element();
        }
        let tokens = theme(cx);
        let is_busy = self.busy_user_id.is_some();
        let mut rows = Vec::with_capacity(bans.len());
        for (index, ban) in bans.iter().enumerate() {
            let name = display_name(&ban.user).to_owned();
            let user_id = ban.user.id;
            let focus = self
                .revoke_focus
                .entry(user_id)
                .or_insert_with(|| cx.focus_handle())
                .clone();
            let page = cx.entity().downgrade();
            let reason = if ban.reason.is_empty() {
                words.no_reason().to_owned()
            } else {
                ban.reason.clone()
            };
            let banned_on = words.banned_on(&full_timestamp_label(
                local_date_time(ban.created_at_unix_milliseconds),
                catalogue,
            ));
            rows.push(
                list_row(cx)
                    .id(("ban", index))
                    .role(Role::ListItem)
                    .aria_position_in_set(index + 1)
                    .aria_size_of_set(bans.len())
                    .child(person_avatar(&ban.user, window, cx))
                    .child(
                        text_column()
                            .child(
                                with_type(div(), TYPE_MEMBER_NAME)
                                    .truncate()
                                    .text_color(color(tokens.color_text_heading))
                                    .child(name.clone()),
                            )
                            .child(
                                with_type(div(), TYPE_BODY)
                                    .text_color(color(tokens.component_settings_body_text))
                                    .child(reason),
                            )
                            .child(secondary_line(banned_on, cx)),
                    )
                    .child(
                        Button::new(
                            ("ban-revoke", index),
                            if self.busy_user_id == Some(user_id) {
                                words.revoking()
                            } else {
                                words.revoke()
                            },
                        )
                        .accessible_label(words.revoke_label(&name))
                        .disabled(is_busy)
                        .track_focus(&focus)
                        .on_click(move |_, cx| {
                            let _ = page.update(cx, |page, cx| page.revoke(user_id, cx));
                        }),
                    ),
            );
        }
        div()
            .flex()
            .flex_col()
            .child(error_alert("bans-error", self.error_message.clone(), cx))
            .child(
                div()
                    .id("bans-list")
                    .role(Role::List)
                    .aria_label(catalogue.server_settings().sections().bans())
                    .children(rows),
            )
            .child(notice)
            .into_any_element()
    }
}
