//! The attachment tray inside the composer, above the field (`AttachmentTray.svelte`, DESIGN.md
//! §5.5, §6.27): one card per file with its preview, its name cut in the middle, its state, Retry
//! when it failed and Remove.

use std::path::Path;

use gpui::{
    App, ElementId, Entity, InteractiveElement as _, IntoElement, ObjectFit, ParentElement as _,
    Role, SharedString, StatefulInteractiveElement as _, Styled as _, StyledImage as _, div, img,
    prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::format::middle_truncate;

use super::{Composer, TrayItem, TrayItemStatus, written_file_size};
use crate::components::{IconButton, IconButtonTone, icon, shadow, with_type};
use crate::i18n::Strings;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_ACTION_BAR_BUTTON_SIZE, COMPONENT_ACTION_BAR_ICON_SIZE, COMPONENT_ACTION_BAR_RADIUS,
    COMPONENT_COMPOSER_TRAY_GAP, COMPONENT_COMPOSER_TRAY_ITEM_BORDER_WIDTH,
    COMPONENT_COMPOSER_TRAY_ITEM_FILE_ICON_SIZE, COMPONENT_COMPOSER_TRAY_ITEM_GAP,
    COMPONENT_COMPOSER_TRAY_ITEM_PADDING, COMPONENT_COMPOSER_TRAY_ITEM_PREVIEW_HEIGHT,
    COMPONENT_COMPOSER_TRAY_ITEM_RADIUS, COMPONENT_COMPOSER_TRAY_ITEM_REMOVE_OFFSET,
    COMPONENT_COMPOSER_TRAY_ITEM_UPLOADING_OPACITY, COMPONENT_COMPOSER_TRAY_ITEM_WIDTH,
    COMPONENT_COMPOSER_TRAY_PADDING, COMPONENT_MESSAGE_STATUS_ICON_SIZE, SIZE_HIT_DESKTOP_MIN,
    TYPE_BODY, TYPE_BODY_SM,
};
use crate::theme::{color, length, theme};

/// An item's width fits about this many characters of its name.
const FILE_NAME_MAXIMUM_CHARACTERS: usize = 16;

/// What GPUI's own image loader draws: the tray previews these from the picked file itself.
const PREVIEWABLE_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

fn is_previewable(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            PREVIEWABLE_EXTENSIONS
                .iter()
                .any(|previewable| extension.eq_ignore_ascii_case(previewable))
        })
}

#[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
pub(super) fn tray(
    items: &[TrayItem],
    composer: &Entity<Composer>,
    catalogue: Strings,
    cx: &App,
) -> impl IntoElement {
    let tokens = theme(cx);
    let labels = catalogue.attachment_tray();
    div()
        .id("composer-tray")
        .role(Role::List)
        .aria_label(labels.files_to_send())
        .flex()
        .gap(length(COMPONENT_COMPOSER_TRAY_GAP))
        .overflow_x_scroll()
        .p(length(COMPONENT_COMPOSER_TRAY_PADDING))
        .border_b(length(COMPONENT_COMPOSER_TRAY_ITEM_BORDER_WIDTH))
        .border_color(color(tokens.component_composer_tray_divider))
        .children(items.iter().map(|item| {
            let key = item.key;
            let is_uploading = item.status == TrayItemStatus::Uploading;
            let preview = div()
                .flex()
                .items_center()
                .justify_center()
                .h(length(COMPONENT_COMPOSER_TRAY_ITEM_PREVIEW_HEIGHT))
                .overflow_hidden()
                .rounded(length(COMPONENT_COMPOSER_TRAY_ITEM_RADIUS))
                .bg(color(
                    tokens.component_composer_tray_item_preview_background,
                ))
                .when(is_uploading, |preview| {
                    preview.opacity(COMPONENT_COMPOSER_TRAY_ITEM_UPLOADING_OPACITY)
                })
                .child(if is_previewable(&item.path) {
                    img(item.path.clone())
                        .size_full()
                        .object_fit(ObjectFit::Contain)
                        .into_any_element()
                } else {
                    icon(
                        IconName::File,
                        COMPONENT_COMPOSER_TRAY_ITEM_FILE_ICON_SIZE,
                        color(tokens.component_composer_tray_item_file_icon),
                    )
                    .into_any_element()
                });
            let status = match &item.status {
                TrayItemStatus::Failed(reason) => {
                    let retry_target = composer.clone();
                    div()
                        .flex()
                        .flex_col()
                        .gap(length(COMPONENT_COMPOSER_TRAY_ITEM_GAP))
                        .child(
                            with_type(div(), TYPE_BODY_SM)
                                .flex()
                                .gap(length(COMPONENT_COMPOSER_TRAY_ITEM_GAP))
                                .text_color(color(tokens.component_composer_tray_item_error_text))
                                .child(icon(
                                    IconName::WarningCircle,
                                    COMPONENT_MESSAGE_STATUS_ICON_SIZE,
                                    color(tokens.component_composer_tray_item_error_text),
                                ))
                                .child(div().min_w_0().child(reason.clone())),
                        )
                        .child(
                            // The message row's Retry link: a failed file never blocks Send.
                            with_type(div(), TYPE_BODY_SM)
                                .id(ElementId::NamedInteger("tray-retry".into(), key))
                                .role(Role::Button)
                                .aria_label(labels.retry(&item.file_name))
                                .min_h(length(SIZE_HIT_DESKTOP_MIN))
                                .flex()
                                .items_center()
                                .cursor_pointer()
                                .text_color(color(tokens.component_message_editor_hint_link))
                                .hover(gpui::Styled::underline)
                                .on_click(move |_, _, cx| {
                                    retry_target.update(cx, |composer, cx| {
                                        composer.retry_upload(key, cx);
                                    });
                                })
                                .child(catalogue.messages().retry()),
                        )
                }
                TrayItemStatus::Uploading => with_type(div(), TYPE_BODY_SM)
                    .text_color(color(tokens.component_composer_tray_item_status_text))
                    .child(labels.uploading()),
                TrayItemStatus::Uploaded(_) => with_type(div(), TYPE_BODY_SM)
                    .text_color(color(tokens.component_composer_tray_item_status_text))
                    .child(written_file_size(item.size_bytes, catalogue)),
            };
            let remove_target = composer.clone();
            div()
                .id(ElementId::NamedInteger("tray-item".into(), key))
                .role(Role::ListItem)
                .aria_label(SharedString::from(item.file_name.clone()))
                .relative()
                .flex()
                .flex_col()
                .flex_none()
                .gap(length(COMPONENT_COMPOSER_TRAY_ITEM_GAP))
                .w(length(COMPONENT_COMPOSER_TRAY_ITEM_WIDTH))
                .p(length(COMPONENT_COMPOSER_TRAY_ITEM_PADDING))
                .rounded(length(COMPONENT_COMPOSER_TRAY_ITEM_RADIUS))
                .border(length(COMPONENT_COMPOSER_TRAY_ITEM_BORDER_WIDTH))
                .border_color(if matches!(item.status, TrayItemStatus::Failed(_)) {
                    color(tokens.component_composer_tray_item_error_border)
                } else {
                    gpui::transparent_black()
                })
                .bg(color(tokens.component_composer_tray_item_background))
                .child(preview)
                .child(
                    with_type(div(), TYPE_BODY)
                        .truncate()
                        .text_color(color(tokens.component_composer_tray_item_name_text))
                        .child(middle_truncate(
                            &item.file_name,
                            FILE_NAME_MAXIMUM_CHARACTERS,
                        )),
                )
                .child(status)
                .child(
                    div()
                        .absolute()
                        .top(length(COMPONENT_COMPOSER_TRAY_ITEM_REMOVE_OFFSET))
                        .right(length(COMPONENT_COMPOSER_TRAY_ITEM_REMOVE_OFFSET))
                        .flex()
                        .rounded(length(COMPONENT_ACTION_BAR_RADIUS))
                        .border(length(COMPONENT_COMPOSER_TRAY_ITEM_BORDER_WIDTH))
                        .border_color(color(tokens.component_action_bar_border))
                        .bg(color(tokens.component_action_bar_background))
                        .shadow(shadow(tokens.component_action_bar_shadow))
                        .child(
                            IconButton::new(
                                ElementId::NamedInteger("tray-remove".into(), key),
                                IconName::Trash,
                                labels.remove(&item.file_name),
                            )
                            .sizes(
                                COMPONENT_ACTION_BAR_BUTTON_SIZE,
                                COMPONENT_ACTION_BAR_ICON_SIZE,
                            )
                            .tone(IconButtonTone::Danger)
                            .on_click(move |window, cx| {
                                remove_target.update(cx, |composer, cx| {
                                    composer.remove_tray_item(key, window, cx);
                                });
                            }),
                        ),
                )
        }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn previews_only_the_pictures_gpui_draws_whatever_the_case() {
        assert!(is_previewable(Path::new("/tmp/photo.PNG")));
        assert!(is_previewable(Path::new("cat.webp")));
        assert!(!is_previewable(Path::new("clip.mov")));
        assert!(!is_previewable(Path::new("photo.avif")));
        assert!(!is_previewable(Path::new("README")));
    }
}
