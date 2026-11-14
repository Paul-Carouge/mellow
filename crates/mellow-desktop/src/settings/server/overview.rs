//! Overview (`ServerOverviewSettings.svelte`): the icon editor, saved at once, then the server's
//! name, saved through the unsaved changes bar, beside a preview of the navigator's header
//! (`ServerPreview.svelte`) that follows what is typed. A rename by someone else while nothing is
//! typed shows at once; a draft is kept until saved or reset.

use std::sync::Arc;

use gpui::{
    AppContext as _, Context, Entity, EventEmitter, IntoElement, ObjectFit, ParentElement as _,
    Render, SharedString, Styled as _, StyledImage as _, Subscription, Window, div, img,
    prelude::FluentBuilder as _,
};
use gpui_component::input::{InputEvent, InputState};
use mellow_client_core::DownloadableFile;
use mellow_client_core::shared_logic::format::server_initials;
use mellow_client_core::shared_logic::server_settings::MAXIMUM_SERVER_NAME_LENGTH;
use mellow_protocol::Snowflake;

use crate::components::{TextField, UnsavedChangesBar, icon, with_type};
use crate::i18n::strings;
use crate::messages::picture_for_box;
use crate::pictures::Animation;
use crate::session::Session;
use crate::settings::icon_editor::{IconEditor, IconTarget};
use crate::settings::{SettingsPage, SettingsPageEvent, saved_notice, unsaved_changes_labels};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_DIALOG_SECTION_GAP, COMPONENT_DIALOG_WIDTH, COMPONENT_NAVIGATOR_STRIP_WIDTH,
    COMPONENT_RAIL_ICON_RADIUS_ACTIVE, COMPONENT_RAIL_ITEM_SIZE, COMPONENT_RAIL_PADDING_TOP,
    COMPONENT_RAIL_PILL_HEIGHT_SELECTED, COMPONENT_RAIL_PILL_WIDTH,
    COMPONENT_SERVER_PREVIEW_BORDER_WIDTH, COMPONENT_SERVER_PREVIEW_RADIUS,
    COMPONENT_SERVER_PREVIEW_WIDTH, COMPONENT_SETTINGS_SECTION_GAP,
    COMPONENT_SIDEBAR_HEADER_CHEVRON_SIZE, COMPONENT_SIDEBAR_HEADER_HEIGHT,
    COMPONENT_TEXT_FIELD_LABEL_GAP, SPACE_XS, TYPE_CATEGORY, TYPE_HEADING_MD,
};
use crate::theme::{color, length, theme};

/// The name field takes the room first and the preview what is left (the web's
/// `flex-[999_1_…]` beside `flex-[1_1_…]`), until the preview wraps under it.
const NAME_FIELD_GROWTH: f32 = 999.0;

pub struct ServerOverviewPage {
    session: Entity<Session>,
    guild_id: Snowflake,
    name_input: Entity<InputState>,
    icon_editor: Entity<IconEditor>,
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
        let icon_editor =
            cx.new(|cx| IconEditor::new(session.clone(), IconTarget::Server(guild_id), cx));
        Self {
            session,
            guild_id,
            name_input,
            icon_editor,
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

/// The navigator's header in miniature (`ServerPreview.svelte`, DESIGN.md §5.2): the server's
/// icon selected in the strip with the apricot pill, and its name as the sidebar shows it, at
/// their real size. It repeats the form for the eye, so it is not in the accessibility tree.
fn server_preview(
    name: &str,
    icon_image: Option<mellow_protocol::ProfileImage>,
    window: &Window,
    cx: &gpui::App,
) -> gpui::Div {
    let tokens = theme(cx);
    let catalogue = strings(cx);
    let shown_name = name.trim();
    let is_placeholder = shown_name.is_empty();
    let shown_name = if is_placeholder {
        catalogue.add_server().preview_placeholder()
    } else {
        shown_name
    };
    let pill_top = COMPONENT_RAIL_PADDING_TOP
        + (COMPONENT_RAIL_ITEM_SIZE - COMPONENT_RAIL_PILL_HEIGHT_SELECTED) / 2.0;
    let server_icon = with_type(div(), TYPE_HEADING_MD)
        .relative()
        .flex()
        .items_center()
        .justify_center()
        .overflow_hidden()
        .size(length(COMPONENT_RAIL_ITEM_SIZE))
        .rounded(length(COMPONENT_RAIL_ICON_RADIUS_ACTIVE))
        .bg(color(tokens.component_rail_icon_background_active))
        .text_color(color(tokens.component_rail_icon_text_active))
        .map(|server_icon| match icon_image {
            Some(image) => server_icon.child(
                img(picture_for_box(
                    DownloadableFile::Icon(image.id),
                    COMPONENT_RAIL_ITEM_SIZE,
                    COMPONENT_RAIL_ITEM_SIZE,
                    Animation::StillFrame,
                    window,
                ))
                .size_full()
                .object_fit(ObjectFit::Cover)
                .rounded(length(COMPONENT_RAIL_ICON_RADIUS_ACTIVE)),
            ),
            None => server_icon.child(server_initials(shown_name)),
        });
    div()
        .flex()
        .overflow_hidden()
        .rounded(length(COMPONENT_SERVER_PREVIEW_RADIUS))
        .border(length(COMPONENT_SERVER_PREVIEW_BORDER_WIDTH))
        .border_color(color(tokens.component_server_preview_border))
        .bg(color(tokens.component_server_preview_background))
        .child(
            div()
                .relative()
                .flex()
                .flex_none()
                .justify_center()
                .w(length(COMPONENT_NAVIGATOR_STRIP_WIDTH))
                .py(length(COMPONENT_RAIL_PADDING_TOP))
                .child(
                    div()
                        .absolute()
                        .left(-length(COMPONENT_RAIL_PILL_WIDTH / 2.0))
                        .top(length(pill_top))
                        .w(length(COMPONENT_RAIL_PILL_WIDTH))
                        .h(length(COMPONENT_RAIL_PILL_HEIGHT_SELECTED))
                        .rounded_full()
                        .bg(color(tokens.component_rail_pill_color_selected)),
                )
                .child(server_icon),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .pr(length(SPACE_XS))
                .child(
                    with_type(div(), TYPE_HEADING_MD)
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap(length(SPACE_XS))
                        .h(length(COMPONENT_SIDEBAR_HEADER_HEIGHT))
                        .px(length(SPACE_XS))
                        .text_color(color(if is_placeholder {
                            tokens.component_server_preview_placeholder_text
                        } else {
                            tokens.component_sidebar_header_text
                        }))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(shown_name.to_owned()),
                        )
                        .child(icon(
                            IconName::CaretDown,
                            COMPONENT_SIDEBAR_HEADER_CHEVRON_SIZE,
                            color(tokens.color_text_secondary),
                        )),
                ),
        )
}

impl Render for ServerOverviewPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let catalogue = strings(cx);
        let words = catalogue.server_settings().overview();
        let is_changed = self.is_changed(cx);
        let page = cx.entity();
        let reset_page = page.clone();
        let icon_image = self
            .session
            .read(cx)
            .state()
            .guild(self.guild_id)
            .and_then(|guild| guild.icon);
        let tokens = theme(cx);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .child(self.icon_editor.clone())
            .child(
                // Side by side while both fit, as in "Create a server"; the preview wraps under.
                div()
                    .mt(length(COMPONENT_SETTINGS_SECTION_GAP))
                    .flex()
                    .flex_wrap()
                    .items_start()
                    .gap(length(COMPONENT_DIALOG_SECTION_GAP))
                    .child(
                        div()
                            .flex_grow(NAME_FIELD_GROWTH)
                            .flex_basis(length(COMPONENT_SERVER_PREVIEW_WIDTH))
                            .min_w_0()
                            .max_w(length(COMPONENT_DIALOG_WIDTH))
                            .child(
                                TextField::new(
                                    "server-name",
                                    words.server_name(),
                                    &self.name_input,
                                )
                                .helper(words.server_name_helper()),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_grow(1.0)
                            .flex_basis(length(COMPONENT_SERVER_PREVIEW_WIDTH))
                            .min_w_0()
                            .gap(length(COMPONENT_TEXT_FIELD_LABEL_GAP))
                            .child(
                                with_type(div(), TYPE_CATEGORY)
                                    .text_color(color(tokens.component_text_field_label_text))
                                    .child(catalogue.add_server().preview()),
                            )
                            .child(server_preview(
                                &self.field_value(cx),
                                icon_image,
                                window,
                                cx,
                            )),
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
