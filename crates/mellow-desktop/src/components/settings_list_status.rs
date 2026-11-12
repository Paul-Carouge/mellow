//! `SettingsListStatus.svelte`: what a settings list shows before it has rows, static skeleton rows
//! while it loads (no shimmer, so nothing moves under reduced motion either), or what failed with
//! a retry button.

use crate::components::LiveRegion as _;
use gpui::{
    App, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, RenderOnce,
    SharedString, Styled as _, Window, div, relative,
};

use super::{ActivationHandler, Button, child_id, icon, with_type};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_SERVER_SETTINGS_AVATAR_SIZE, COMPONENT_SERVER_SETTINGS_ROW_GAP,
    COMPONENT_SERVER_SETTINGS_ROW_MIN_HEIGHT, COMPONENT_SERVER_SETTINGS_SKELETON_BAR_HEIGHT,
    COMPONENT_SETTINGS_ACTION_GAP, COMPONENT_SETTINGS_ERROR_ICON_GAP,
    COMPONENT_SETTINGS_ERROR_ICON_SIZE, TYPE_BODY,
};
use crate::theme::{color, length, theme};

/// The web's `w-2/5`, `w-1/3`, `w-1/2`, `w-1/4`: uneven bars read as text, not as a table.
const SKELETON_TEXT_SHARES: [f32; 4] = [0.4, 1.0 / 3.0, 0.5, 0.25];

#[derive(Clone)]
pub enum SettingsListStatusKind {
    /// `label` is said instead of the skeleton ("Loading…").
    Loading { label: SharedString },
    Failed {
        failure_text: SharedString,
        retry_label: SharedString,
        on_retry: ActivationHandler,
    },
}

#[derive(IntoElement)]
pub struct SettingsListStatus {
    id: ElementId,
    kind: SettingsListStatusKind,
}

impl SettingsListStatus {
    #[must_use]
    pub fn new(id: impl Into<ElementId>, kind: SettingsListStatusKind) -> Self {
        Self {
            id: id.into(),
            kind,
        }
    }
}

impl RenderOnce for SettingsListStatus {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        match self.kind {
            SettingsListStatusKind::Loading { label } => {
                let fill = color(tokens.component_server_settings_skeleton_fill);
                div()
                    .id(self.id)
                    .live_polite(label)
                    .flex()
                    .flex_col()
                    .children(SKELETON_TEXT_SHARES.iter().map(|share| {
                        div()
                            .flex()
                            .items_center()
                            .gap(length(COMPONENT_SERVER_SETTINGS_ROW_GAP))
                            .min_h(length(COMPONENT_SERVER_SETTINGS_ROW_MIN_HEIGHT))
                            .border_b_1()
                            .border_color(color(tokens.component_server_settings_row_divider))
                            .child(
                                div()
                                    .flex_none()
                                    .size(length(COMPONENT_SERVER_SETTINGS_AVATAR_SIZE))
                                    .rounded_full()
                                    .bg(fill),
                            )
                            .child(
                                div()
                                    .h(length(COMPONENT_SERVER_SETTINGS_SKELETON_BAR_HEIGHT))
                                    .w(relative(*share))
                                    .rounded_full()
                                    .bg(fill),
                            )
                    }))
                    .into_any_element()
            }
            SettingsListStatusKind::Failed {
                failure_text,
                retry_label,
                on_retry,
            } => {
                let error = color(tokens.component_settings_error_text);
                let retry_id = child_id(&self.id, "retry");
                div()
                    .id(self.id)
                    .live_assertive(failure_text.clone())
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(length(COMPONENT_SETTINGS_ACTION_GAP))
                    .child(
                        with_type(div(), TYPE_BODY)
                            .flex()
                            .gap(length(COMPONENT_SETTINGS_ERROR_ICON_GAP))
                            .text_color(error)
                            .child(icon(
                                IconName::WarningCircle,
                                COMPONENT_SETTINGS_ERROR_ICON_SIZE,
                                color(tokens.component_settings_error_icon),
                            ))
                            .child(failure_text),
                    )
                    .child(
                        Button::new(retry_id, retry_label)
                            .on_click(move |window, cx| on_retry(window, cx)),
                    )
                    .into_any_element()
            }
        }
    }
}

debug_without_callbacks!(SettingsListStatusKind);
debug_without_callbacks!(SettingsListStatus, id);
