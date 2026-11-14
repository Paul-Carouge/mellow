//! What the moderation pages' lists share (`BanSettings.svelte`, `InviteSettings.svelte`,
//! `ModerationLogSettings.svelte`, `EmojiSettings.svelte`): the row, a person's avatar, the line
//! of secondary text and the status line read out after an action.

use gpui::{
    App, Div, InteractiveElement as _, IntoElement, ParentElement as _, SharedString, Styled as _,
    Window, div,
};
use mellow_client_core::DownloadableFile;
use mellow_protocol::PublicUser;

use crate::components::{
    Avatar, AvatarPerson, LiveRegion as _, SettingsListStatus, SettingsListStatusKind, with_type,
};
use crate::i18n::strings;
use crate::messages::picture_for_box;
use crate::theme::tokens::{
    COMPONENT_SERVER_SETTINGS_AVATAR_SIZE, COMPONENT_SERVER_SETTINGS_ROW_GAP,
    COMPONENT_SERVER_SETTINGS_ROW_MIN_HEIGHT, COMPONENT_SERVER_SETTINGS_ROW_PADDING_BLOCK,
    COMPONENT_SERVER_SETTINGS_TEXT_GAP, TYPE_BODY_LG, TYPE_BODY_SM,
};
use crate::theme::{color, length, theme};

/// A list row: its picture, its text column, its actions, over a divider.
pub(super) fn list_row(cx: &App) -> Div {
    div()
        .flex()
        .items_center()
        .gap(length(COMPONENT_SERVER_SETTINGS_ROW_GAP))
        .min_h(length(COMPONENT_SERVER_SETTINGS_ROW_MIN_HEIGHT))
        .py(length(COMPONENT_SERVER_SETTINGS_ROW_PADDING_BLOCK))
        .border_b_1()
        .border_color(color(theme(cx).component_server_settings_row_divider))
}

/// The row's text column, its lines spaced as the web's.
pub(super) fn text_column() -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .gap(length(COMPONENT_SERVER_SETTINGS_TEXT_GAP))
}

/// "Created by Ana", "Banned on …": the quieter line under a row's name.
pub(super) fn secondary_line(text: impl Into<SharedString>, cx: &App) -> Div {
    with_type(div(), TYPE_BODY_SM)
        .text_color(color(theme(cx).component_server_settings_secondary_text))
        .child(text.into())
}

/// The sentence a list says when it has no rows.
pub(super) fn empty_sentence(text: impl Into<SharedString>, cx: &App) -> Div {
    with_type(div(), TYPE_BODY_LG)
        .text_color(color(theme(cx).component_settings_body_text))
        .child(text.into())
}

/// A person's avatar in a list row, playing while `is_row_pointed_at`.
pub(super) fn person_avatar(
    user: &PublicUser,
    is_row_pointed_at: bool,
    window: &Window,
    cx: &App,
) -> impl IntoElement {
    let picture = user.avatar.as_ref().map(|avatar| {
        picture_for_box(
            DownloadableFile::ProfileImage(avatar.id),
            COMPONENT_SERVER_SETTINGS_AVATAR_SIZE,
            COMPONENT_SERVER_SETTINGS_AVATAR_SIZE,
            crate::pictures::chosen_profile_image_animation(avatar, is_row_pointed_at, cx),
            window,
        )
    });
    Avatar::new(
        AvatarPerson::from_user(user, picture),
        COMPONENT_SERVER_SETTINGS_AVATAR_SIZE,
    )
}

/// "Paul can join again": read out after an action, never shown.
pub(super) fn status_notice(id: &'static str, notice: SharedString) -> impl IntoElement {
    div().id(id).live_polite(notice).h_0().overflow_hidden()
}

/// The skeleton while a list loads, or what failed with "Try again" (`SettingsListStatus`).
pub(super) fn list_status(
    id: &'static str,
    has_failed: bool,
    failure_text: &'static str,
    on_retry: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> SettingsListStatus {
    let catalogue = strings(cx);
    SettingsListStatus::new(
        id,
        if has_failed {
            SettingsListStatusKind::Failed {
                failure_text: failure_text.into(),
                retry_label: catalogue.common().try_again().into(),
                on_retry: std::sync::Arc::new(on_retry),
            }
        } else {
            SettingsListStatusKind::Loading {
                label: catalogue.server_settings().loading().into(),
            }
        },
    )
}
