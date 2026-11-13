//! The attachment tray inside the composer, above the field (`AttachmentTray.svelte`, DESIGN.md
//! §5.5, §6.27): one card per file with its preview, its name cut in the middle, its state, Retry
//! when it failed and Remove.

use std::sync::Arc;

use gpui::{
    App, ElementId, Entity, ImageSource, InteractiveElement as _, IntoElement, ObjectFit,
    ParentElement as _, RenderImage, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, StyledImage as _, Window, div, img, prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::format::middle_truncate;

use super::{Composer, TrayItem, TrayItemStatus, written_file_size};
use crate::components::{
    IconButton, IconButtonTone, focus_ring, icon, is_focus_visible, remembered_focus_handle,
    shadow, with_type,
};
use crate::i18n::Strings;
use crate::pictures::{Animation, DisplaySize, decode_frames};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_ACTION_BAR_BUTTON_SIZE, COMPONENT_ACTION_BAR_ICON_SIZE, COMPONENT_ACTION_BAR_RADIUS,
    COMPONENT_COMPOSER_TRAY_GAP, COMPONENT_COMPOSER_TRAY_ITEM_BORDER_WIDTH,
    COMPONENT_COMPOSER_TRAY_ITEM_FILE_ICON_SIZE, COMPONENT_COMPOSER_TRAY_ITEM_GAP,
    COMPONENT_COMPOSER_TRAY_ITEM_PADDING, COMPONENT_COMPOSER_TRAY_ITEM_PREVIEW_HEIGHT,
    COMPONENT_COMPOSER_TRAY_ITEM_RADIUS, COMPONENT_COMPOSER_TRAY_ITEM_REMOVE_OFFSET,
    COMPONENT_COMPOSER_TRAY_ITEM_UPLOADING_OPACITY, COMPONENT_COMPOSER_TRAY_ITEM_WIDTH,
    COMPONENT_COMPOSER_TRAY_PADDING, COMPONENT_MESSAGE_STATUS_ICON_SIZE, RADIUS_SM,
    SIZE_HIT_DESKTOP_MIN, TYPE_BODY, TYPE_BODY_SM,
};
use crate::theme::{color, length, theme};

/// An item's width fits about this many characters of its name.
const FILE_NAME_MAXIMUM_CHARACTERS: usize = 16;

/// Previews decode at twice their layout size: sharp on high-density screens, small anyway.
const PREVIEW_SCALE: f32 = 2.0;

/// The file's first frame scaled down to the preview box, with the picture pipeline's decoding
/// limits; `None` for anything that is not a picture it decodes. Runs off the UI thread.
pub(super) fn decode_preview(bytes: &[u8]) -> Option<Arc<RenderImage>> {
    let width = (COMPONENT_COMPOSER_TRAY_ITEM_WIDTH - 2.0 * COMPONENT_COMPOSER_TRAY_ITEM_PADDING)
        * PREVIEW_SCALE;
    let height = COMPONENT_COMPOSER_TRAY_ITEM_PREVIEW_HEIGHT * PREVIEW_SCALE;
    // Token sizes are small positive numbers.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let display = DisplaySize {
        width: width.ceil() as u32,
        height: height.ceil() as u32,
    };
    decode_frames(bytes, Animation::StillFrame, display)
        .ok()
        .map(|frames| Arc::new(RenderImage::new(frames)))
}

#[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
pub(super) fn tray(
    items: &[TrayItem],
    composer: &Entity<Composer>,
    catalogue: Strings,
    window: &mut Window,
    cx: &mut App,
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
            let is_uploading = matches!(
                item.status,
                TrayItemStatus::Uploading | TrayItemStatus::Compressing(_)
            );
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
                .child(if let Some(preview) = &item.preview {
                    img(ImageSource::Render(Arc::clone(preview)))
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
                    let retry_id = ElementId::NamedInteger("tray-retry".into(), key);
                    let retry_focus = remembered_focus_handle(&retry_id, window, cx);
                    let is_retry_ring_shown = is_focus_visible(&retry_focus, window);
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
                                .id(retry_id)
                                .role(Role::Button)
                                .aria_label(labels.retry(&item.file_name))
                                .track_focus(&retry_focus)
                                .relative()
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
                                .child(catalogue.messages().retry())
                                .when(is_retry_ring_shown, |retry| {
                                    retry.child(focus_ring(RADIUS_SM, cx))
                                }),
                        )
                }
                TrayItemStatus::Compressing(percent) => {
                    let original_target = composer.clone();
                    let original_id = ElementId::NamedInteger("tray-send-original".into(), key);
                    let original_focus = remembered_focus_handle(&original_id, window, cx);
                    let is_original_ring_shown = is_focus_visible(&original_focus, window);
                    div()
                        .flex()
                        .flex_col()
                        .gap(length(COMPONENT_COMPOSER_TRAY_ITEM_GAP))
                        .child(
                            with_type(div(), TYPE_BODY_SM)
                                .text_color(color(tokens.component_composer_tray_item_status_text))
                                .child(labels.compressing(i64::from(*percent))),
                        )
                        .child(
                            // Compressing a long clip can take a while: the original may go
                            // instead, as the web's Retry-styled link offers.
                            with_type(div(), TYPE_BODY_SM)
                                .id(original_id)
                                .role(Role::Button)
                                .aria_label(labels.send_original_of(&item.file_name))
                                .track_focus(&original_focus)
                                .relative()
                                .min_h(length(SIZE_HIT_DESKTOP_MIN))
                                .flex()
                                .items_center()
                                .cursor_pointer()
                                .text_color(color(tokens.component_message_editor_hint_link))
                                .hover(gpui::Styled::underline)
                                .on_click(move |_, _, cx| {
                                    original_target.update(cx, |composer, cx| {
                                        composer.send_original(key, cx);
                                    });
                                })
                                .child(labels.send_original())
                                .when(is_original_ring_shown, |link| {
                                    link.child(focus_ring(RADIUS_SM, cx))
                                }),
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
    use image::{ImageEncoder as _, codecs::png::PngEncoder};

    use super::*;

    #[test]
    fn previews_pictures_scaled_down_and_nothing_else() {
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(
                &vec![0; 2_000 * 2_000 * 3],
                2_000,
                2_000,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        let preview = decode_preview(&png).expect("a PNG previews");
        let decoded = preview.size(0);
        assert!(
            decoded.width.0 <= 300 && decoded.height.0 <= 300,
            "{decoded:?}"
        );

        assert!(decode_preview(b"%PDF-1.7 not a picture").is_none());
    }
}
