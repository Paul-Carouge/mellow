//! Channel Overview (`ChannelOverviewSettings.svelte`): the name and the topic, saved together
//! through the unsaved changes bar. A change by someone else to a field not typed in shows at once;
//! a typed field is kept until saved or reset, and only typed fields are sent.
//!
//! shortcut: the channel icon editor needs the picture crop dialog (see the server Overview), and
//! a voice channel's user limit needs a select; voice rows open no settings before M3 step 7, which
//! adds the field (`voice_user_limit_choices` is ready in client-core).

use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::input::{InputEvent, InputState};
use mellow_client_core::shared_logic::channel_settings::{
    ChannelOverviewDrafts, MAXIMUM_CHANNEL_NAME_LENGTH, MAXIMUM_CHANNEL_TOPIC_LENGTH,
    channel_overview_changes, is_channel_update_empty,
};
use mellow_protocol::{Channel, ChannelKind, Snowflake};

use crate::components::{TextField, UnsavedChangesBar};
use crate::i18n::strings;
use crate::session::Session;
use crate::settings::{SettingsPage, SettingsPageEvent, saved_notice, unsaved_changes_labels};
use crate::theme::icons::IconName;
use crate::theme::length;
use crate::theme::tokens::{COMPONENT_DIALOG_WIDTH, COMPONENT_SETTINGS_SECTION_GAP};

/// A text field and the stored value it was last set to: it holds a draft once they differ.
struct OverviewField {
    input: Entity<InputState>,
    shown_stored: String,
}

impl OverviewField {
    fn new(
        stored: String,
        maximum_length: usize,
        window: &mut Window,
        cx: &mut Context<ChannelOverviewPage>,
    ) -> Self {
        let input = cx.new(|cx| {
            let mut input = InputState::new(window, cx)
                .validate(move |text, _| text.chars().count() <= maximum_length);
            input.set_value(stored.clone(), window, cx);
            input
        });
        Self {
            input,
            shown_stored: stored,
        }
    }

    fn value(&self, cx: &gpui::App) -> String {
        self.input.read(cx).value().to_string()
    }

    fn draft(&self, cx: &gpui::App) -> Option<String> {
        Some(self.value(cx)).filter(|value| *value != self.shown_stored)
    }

    /// Shows the stored value again, typed or not.
    fn set(&mut self, stored: String, window: &mut Window, cx: &mut gpui::App) {
        self.input
            .update(cx, |input, cx| input.set_value(stored.clone(), window, cx));
        self.shown_stored = stored;
    }

    /// A change by someone else shows at once in an untouched field.
    fn follow(&mut self, stored: &str, window: &mut Window, cx: &mut gpui::App) {
        if stored == self.shown_stored {
            return;
        }
        if self.value(cx) == self.shown_stored {
            self.set(stored.to_owned(), window, cx);
        } else {
            stored.clone_into(&mut self.shown_stored);
        }
    }
}

pub struct ChannelOverviewPage {
    session: Entity<Session>,
    channel_id: Snowflake,
    name: OverviewField,
    topic: OverviewField,
    /// Saved, and waiting for the gateway to bring it: not an unsaved change any more.
    saved: Option<ChannelOverviewDrafts>,
    is_saving: bool,
    error_message: Option<String>,
    notice: Option<SharedString>,
    attention: u32,
    is_reported_changed: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsPageEvent> for ChannelOverviewPage {}

impl SettingsPage for ChannelOverviewPage {
    fn set_attention(&mut self, attention: u32, cx: &mut Context<Self>) {
        self.attention = attention;
        cx.notify();
    }
}

impl std::fmt::Debug for ChannelOverviewPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChannelOverviewPage")
            .field("channel_id", &self.channel_id)
            .field("is_saving", &self.is_saving)
            .finish_non_exhaustive()
    }
}

fn stored_channel(
    session: &Entity<Session>,
    channel_id: Snowflake,
    cx: &gpui::App,
) -> Option<Channel> {
    session.read(cx).state().guild_channel(channel_id).cloned()
}

impl ChannelOverviewPage {
    pub fn new(
        session: Entity<Session>,
        channel_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let stored = stored_channel(&session, channel_id, cx);
        let (name, topic) = stored
            .map(|channel| (channel.name, channel.topic))
            .unwrap_or_default();
        let name = OverviewField::new(name, MAXIMUM_CHANNEL_NAME_LENGTH, window, cx);
        let topic = OverviewField::new(topic, MAXIMUM_CHANNEL_TOPIC_LENGTH, window, cx);
        let on_input = |page: &mut Self,
                        _: &Entity<InputState>,
                        event: &InputEvent,
                        _: &mut Window,
                        cx: &mut Context<Self>| {
            match event {
                InputEvent::Change => {
                    page.notice = None;
                    page.refresh(cx);
                }
                InputEvent::PressEnter { .. } => page.save(cx),
                _ => {}
            }
        };
        let subscriptions = vec![
            cx.subscribe_in(&name.input, window, on_input),
            cx.subscribe_in(&topic.input, window, on_input),
            cx.observe_in(&session, window, |page, _, window, cx| {
                page.follow_stored_channel(window, cx);
            }),
        ];
        Self {
            session,
            channel_id,
            name,
            topic,
            saved: None,
            is_saving: false,
            error_message: None,
            notice: None,
            attention: 0,
            is_reported_changed: false,
            _subscriptions: subscriptions,
        }
    }

    fn drafts(&self, cx: &gpui::App) -> ChannelOverviewDrafts {
        ChannelOverviewDrafts {
            name: self.name.draft(cx),
            topic: self.topic.draft(cx),
            user_limit: None,
        }
    }

    fn is_changed(&self, cx: &gpui::App) -> bool {
        let Some(channel) = stored_channel(&self.session, self.channel_id, cx) else {
            return false;
        };
        let drafts = self.drafts(cx);
        !is_channel_update_empty(&channel_overview_changes(&channel, &drafts))
            && self.saved.as_ref() != Some(&drafts)
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let is_changed = self.is_changed(cx);
        if is_changed != self.is_reported_changed {
            self.is_reported_changed = is_changed;
            cx.emit(SettingsPageEvent::UnsavedChanges(is_changed));
        }
        cx.notify();
    }

    /// Follows changes by others; the stored channel catching up with every typed field (our
    /// save, echoed by the gateway) ends the wait for it.
    fn follow_stored_channel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(channel) = stored_channel(&self.session, self.channel_id, cx) else {
            return;
        };
        self.name.follow(&channel.name, window, cx);
        self.topic.follow(&channel.topic, window, cx);
        let mut drafts = self.drafts(cx);
        drafts.forget_stored(&channel);
        if drafts.is_empty() {
            self.saved = None;
        }
        self.refresh(cx);
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(channel) = stored_channel(&self.session, self.channel_id, cx) {
            self.name.set(channel.name, window, cx);
            self.topic.set(channel.topic, window, cx);
        }
        self.saved = None;
        self.error_message = None;
        self.refresh(cx);
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        if self.is_saving || !self.is_changed(cx) {
            return;
        }
        let Some(channel) = stored_channel(&self.session, self.channel_id, cx) else {
            return;
        };
        let catalogue = strings(cx);
        let drafts = self.drafts(cx);
        let changes = channel_overview_changes(&channel, &drafts);
        if changes.name.as_deref() == Some("") {
            self.error_message = Some(
                catalogue
                    .channel_settings()
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
        let channel_id = self.channel_id;
        let page = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.send_settings_change(
                "update channel",
                catalogue
                    .channel_settings()
                    .overview()
                    .save_failed()
                    .to_owned(),
                move |rest| async move { rest.update_channel(channel_id, &changes).await },
                cx,
                move |outcome, cx| {
                    let _ = page.update(cx, |page, cx| {
                        page.is_saving = false;
                        match outcome {
                            // The fields keep the saved text until the gateway brings it, unless
                            // the echo came first (it travels on another connection).
                            Ok(()) => {
                                let mut waiting = drafts.clone();
                                if let Some(channel) =
                                    stored_channel(&page.session, page.channel_id, cx)
                                {
                                    waiting.forget_stored(&channel);
                                }
                                if !waiting.is_empty() {
                                    page.saved = Some(drafts);
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

impl Render for ChannelOverviewPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let catalogue = strings(cx);
        let words = catalogue.channel_settings().overview();
        let is_voice = stored_channel(&self.session, self.channel_id, cx)
            .is_some_and(|channel| channel.kind == ChannelKind::Voice);
        let is_changed = self.is_changed(cx);
        let page = cx.entity();
        let reset_page = page.clone();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .child(
                div()
                    .max_w(length(COMPONENT_DIALOG_WIDTH))
                    .flex()
                    .flex_col()
                    .gap(length(COMPONENT_SETTINGS_SECTION_GAP))
                    .child(
                        TextField::new("channel-name", words.channel_name(), &self.name.input)
                            .helper(words.channel_name_helper())
                            .leading_icon(if is_voice {
                                IconName::SpeakerHigh
                            } else {
                                IconName::Hash
                            }),
                    )
                    .when(!is_voice, |fields| {
                        fields.child(
                            TextField::new("channel-topic", words.topic(), &self.topic.input)
                                .helper(words.topic_helper()),
                        )
                    }),
            )
            .child(saved_notice(
                "channel-overview-notice",
                self.notice.clone().filter(|_| !is_changed),
                cx,
            ))
            .child(div().flex_1())
            .when(is_changed, |column| {
                let mut bar = UnsavedChangesBar::new(
                    "channel-overview-unsaved",
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
