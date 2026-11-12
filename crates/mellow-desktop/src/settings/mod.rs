//! The server, channel and user settings screens (`ServerSettings.svelte`,
//! `ChannelSettings.svelte`, `UserSettings.svelte`, DESIGN.md §5.8) in the full-window `SettingsSheet`. Each section shows only to members holding
//! its permission (client-core's `server_settings` and `channel_settings` rules) and only once the
//! desktop has its page; the screen follows the session, so a lost permission or a deleted server
//! or channel closes what can no longer be used. A page with unsaved changes keeps the reader in
//! it: leaving shakes its bar instead (`SettingsNavigation`).
//!
//! # Adding a section
//!
//! A track adds one page module beside `overview.rs` (in `server/` or `channel/`): a view that
//! implements [`SettingsPage`], reports `SettingsPageEvent::UnsavedChanges` when its form differs
//! from what is stored (only if it has a form) and draws the `UnsavedChangesBar` with the
//! attention it is given. Then, in the screen's `mod.rs`, it turns the section's arm to `true` in
//! `has_page` and makes its page in `make_page`. Nothing else changes: the navigation, its
//! groups and order, the narrow layout and the unsaved-changes guard come from the shared rules.
//! Edits are sent with `Session::send_settings_change` and shown from the gateway's echo.

pub mod channel;
mod navigation;
pub mod server;
pub mod user;

use gpui::{
    App, InteractiveElement as _, IntoElement, ParentElement as _, Role, SharedString,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _,
};

pub use navigation::{SettingsNavigation, SettingsPage, SettingsPageEvent, ShownPage, shown_page};

use crate::components::{UnsavedChangesLabels, icon, with_type};
use crate::i18n::Strings;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    BREAKPOINT_NAVIGATOR, COMPONENT_SETTINGS_ACTION_GAP, COMPONENT_SETTINGS_ERROR_ICON_GAP,
    COMPONENT_SETTINGS_ERROR_ICON_SIZE, TYPE_BODY,
};
use crate::theme::{color, length, theme, window_width_at_default_zoom};

/// A settings screen asks its host to close it; focus goes back where it came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingsClosed;

/// Navigation and page side by side from the navigator breakpoint, as the web's `isWide`.
#[must_use]
pub fn is_wide(window: &Window) -> bool {
    window_width_at_default_zoom(window) >= BREAKPOINT_NAVIGATOR
}

/// The unsaved changes bar's words (`serverSettings.unsaved`), shared by every settings page.
#[must_use]
pub fn unsaved_changes_labels(catalogue: Strings) -> UnsavedChangesLabels {
    let words = catalogue.server_settings().unsaved();
    UnsavedChangesLabels {
        careful: words.careful().into(),
        save_or_reset: words.save_or_reset().into(),
        reset: words.reset().into(),
        save_changes: words.save_changes().into(),
        saving: words.saving().into(),
    }
}

/// "Changes saved" under a form once the bar is gone. The status node is always drawn, empty
/// when nothing is said, because screen readers often miss a live region inserted with its text.
pub fn saved_notice(id: &'static str, notice: Option<SharedString>, cx: &App) -> impl IntoElement {
    let tokens = theme(cx);
    let notice_color = color(tokens.component_settings_notice_text);
    div()
        .child(
            div()
                .id(id)
                .role(Role::Status)
                .aria_label(notice.clone().unwrap_or_default())
                .h_0()
                .overflow_hidden(),
        )
        .when_some(notice, |wrapper, notice| {
            wrapper.child(
                with_type(div(), TYPE_BODY)
                    .mt(length(COMPONENT_SETTINGS_ACTION_GAP))
                    .flex()
                    .gap(length(COMPONENT_SETTINGS_ERROR_ICON_GAP))
                    .text_color(notice_color)
                    .child(icon(
                        IconName::Check,
                        COMPONENT_SETTINGS_ERROR_ICON_SIZE,
                        notice_color,
                    ))
                    .child(notice),
            )
        })
}
