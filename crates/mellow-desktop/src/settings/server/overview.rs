//! Overview (`ServerOverviewSettings.svelte`): the server's name, saved through the unsaved
//! changes bar. A rename by someone else while nothing is typed shows at once; a draft is kept
//! until saved or reset.
//!
//! shortcut: the icon editor and the server preview beside the name are left out: the icon needs
//! the picture crop dialog (`PictureCropDialog.svelte`) that the profile pictures need too; add
//! both with it.

use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::input::{InputEvent, InputState};
use mellow_client_core::shared_logic::server_settings::MAXIMUM_SERVER_NAME_LENGTH;
use mellow_protocol::Snowflake;

use crate::components::{TextField, UnsavedChangesBar};
use crate::i18n::strings;
use crate::session::Session;
use crate::settings::{SettingsPage, SettingsPageEvent, saved_notice, unsaved_changes_labels};
use crate::theme::length;
use crate::theme::tokens::COMPONENT_DIALOG_WIDTH;

pub struct ServerOverviewPage {
    session: Entity<Session>,
    guild_id: Snowflake,
    name_input: Entity<InputState>,
    /// The stored name the field was last set to: the field holds a draft once it differs.
    shown_stored_name: String,
    /// Saved, and waiting for the gateway to bring it: not an unsaved change any more.
    saved_name: Option<String>,
    is_saving: bool,
    error_message: Option<String>,
    notice: Option<SharedString>,
    attention: u32,
    is_reported_changed: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsPageEvent> for ServerOverviewPage {}

impl SettingsPage for ServerOverviewPage {
    fn set_attention(&mut self, attention: u32, cx: &mut Context<Self>) {
        self.attention = attention;
        cx.notify();
    }
}

impl std::fmt::Debug for ServerOverviewPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ServerOverviewPage")
            .field("guild_id", &self.guild_id)
            .field("is_saving", &self.is_saving)
            .finish_non_exhaustive()
    }
}

impl ServerOverviewPage {
    pub fn new(
        session: Entity<Session>,
        guild_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let stored_name = stored_name(&session, guild_id, cx);
        let name_input = cx.new(|cx| {
            let mut input = InputState::new(window, cx)
                .validate(|text, _| text.chars().count() <= MAXIMUM_SERVER_NAME_LENGTH);
            input.set_value(stored_name.clone(), window, cx);
            input
        });
        let subscriptions =
            vec![
                cx.subscribe_in(&name_input, window, |page, _, event: &InputEvent, _, cx| {
                    match event {
                        InputEvent::Change => {
                            page.notice = None;
                            page.refresh(cx);
                        }
                        InputEvent::PressEnter { .. } => page.save(cx),
                        _ => {}
                    }
                }),
                cx.observe_in(&session, window, |page, _, window, cx| {
                    page.follow_stored_name(window, cx);
                }),
            ];
        Self {
            session,
            guild_id,
            name_input,
            shown_stored_name: stored_name,
            saved_name: None,
            is_saving: false,
            error_message: None,
            notice: None,
            attention: 0,
            is_reported_changed: false,
            _subscriptions: subscriptions,
        }
    }

    fn field_value(&self, cx: &gpui::App) -> String {
        self.name_input.read(cx).value().to_string()
    }

    fn is_changed(&self, cx: &gpui::App) -> bool {
        let value = self.field_value(cx);
        let stored = stored_name(&self.session, self.guild_id, cx);
        value != self.shown_stored_name
            && value.trim() != stored
            && self.saved_name.as_deref() != Some(value.trim())
    }

    /// Tells the screen when the field starts or stops differing from the stored name.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let is_changed = self.is_changed(cx);
        if is_changed != self.is_reported_changed {
            self.is_reported_changed = is_changed;
            cx.emit(SettingsPageEvent::UnsavedChanges(is_changed));
        }
        cx.notify();
    }

    /// A rename by someone else shows at once in an untouched field; the stored name catching up
    /// with the field (our save, echoed by the gateway) ends the draft.
    fn follow_stored_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let stored = stored_name(&self.session, self.guild_id, cx);
        if stored == self.shown_stored_name {
            return;
        }
        let value = self.field_value(cx);
        if value == self.shown_stored_name {
            self.name_input
                .update(cx, |input, cx| input.set_value(stored.clone(), window, cx));
        }
        if self.field_value(cx).trim() == stored
            || self.saved_name.as_deref() == Some(stored.as_str())
        {
            self.saved_name = None;
        }
        self.shown_stored_name = stored;
        self.refresh(cx);
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let stored = stored_name(&self.session, self.guild_id, cx);
        self.name_input
            .update(cx, |input, cx| input.set_value(stored.clone(), window, cx));
        self.shown_stored_name = stored;
        self.saved_name = None;
        self.error_message = None;
        self.refresh(cx);
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.is_saving || !self.is_changed(cx) {
            return;
        }
        let catalogue = strings(cx);
        let name = self.field_value(cx).trim().to_owned();
        if name.is_empty() {
            self.error_message = Some(
                catalogue
                    .server_settings()
                    .overview()
                    .name_required()
                    .into(),
            );
            cx.notify();
            return;
        }
        self.is_saving = true;
        self.error_message = None;
        cx.notify();
        let guild_id = self.guild_id;
        let page = cx.entity().downgrade();
        let sent_name = name.clone();
        self.session.update(cx, |session, cx| {
            session.send_settings_change(
                "rename server",
                catalogue
                    .server_settings()
                    .overview()
                    .save_failed()
                    .to_owned(),
                move |rest| async move { rest.rename_guild(guild_id, &name).await },
                cx,
                move |outcome, cx| {
                    let _ = page.update(cx, |page, cx| {
                        page.is_saving = false;
                        match outcome {
                            // The field keeps the saved name until the gateway brings it, unless
                            // the echo came first (it travels on another connection).
                            Ok(()) => {
                                if stored_name(&page.session, page.guild_id, cx) != sent_name {
                                    page.saved_name = Some(sent_name);
                                }
                                page.notice =
                                    Some(strings(cx).server_settings().unsaved().saved().into());
                            }
                            Err(message) => page.error_message = Some(message),
                        }
                        page.refresh(cx);
                    });
                },
            );
        });
    }
}

fn stored_name(session: &Entity<Session>, guild_id: Snowflake, cx: &gpui::App) -> String {
    session
        .read(cx)
        .state()
        .guild(guild_id)
        .map(|guild| guild.name.clone())
        .unwrap_or_default()
}

impl Render for ServerOverviewPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let catalogue = strings(cx);
        let words = catalogue.server_settings().overview();
        let is_changed = self.is_changed(cx);
        let page = cx.entity();
        let reset_page = page.clone();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .child(
                div().max_w(length(COMPONENT_DIALOG_WIDTH)).child(
                    TextField::new("server-name", words.server_name(), &self.name_input)
                        .helper(words.server_name_helper()),
                ),
            )
            .child(saved_notice(
                "server-overview-notice",
                self.notice.clone().filter(|_| !is_changed),
                cx,
            ))
            .child(div().flex_1())
            .when(is_changed, |column| {
                let mut bar = UnsavedChangesBar::new(
                    "server-overview-unsaved",
                    unsaved_changes_labels(catalogue),
                    Arc::new(move |window, cx| {
                        reset_page.update(cx, |page, cx| page.reset(window, cx));
                    }),
                    Arc::new(move |_, cx| page.update(cx, Self::save)),
                )
                .attention(self.attention)
                .saving(self.is_saving);
                if let Some(error) = self.error_message.clone() {
                    bar = bar.error_message(error);
                }
                column.child(bar)
            })
    }
}
