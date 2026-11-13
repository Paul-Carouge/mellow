//! The pictures of `MessageAttachments.svelte` (DESIGN.md §5.4 "Attachments"): a previewable
//! picture inline, in a box its stored size fixes before it loads; a sound with its player
//! (`audio_attachment`); anything else as a card with its name and size.
//!
//! A video plays in its own card (`video_attachment`); any other file has a download button.
//!
//! shortcut: no picture viewer yet; a picture is not enlarged or saved from the list.

use gpui::{
    AnyElement, App, ElementId, InteractiveElement as _, IntoElement, ObjectFit,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, StyledImage as _, Window, div, img,
};
use mellow_client_core::DownloadableFile;
use mellow_client_core::shared_logic::format::{FileSize, FileSizeUnit, human_file_size};
use mellow_media::PLAYABLE_ATTACHMENT_VIDEO_TYPES;
use mellow_protocol::Attachment;

use super::audio_attachment::{AudioAttachment, media_card, name_row};
use super::time::locale;
use super::video_attachment::VideoAttachment;
use super::{picture_animation, picture_for_box};
use crate::attachment_audio::is_played_in_app;
use crate::attachment_download::download_control;
use crate::components::{Tooltip, child_id, icon, with_type};
use crate::i18n::catalogue::KbOrMbOrGb;
use crate::i18n::{self, Strings};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_ATTACHMENT_FILE_BORDER_WIDTH, COMPONENT_ATTACHMENT_FILE_GAP,
    COMPONENT_ATTACHMENT_FILE_ICON_SIZE, COMPONENT_ATTACHMENT_FILE_MAX_WIDTH,
    COMPONENT_ATTACHMENT_FILE_PADDING, COMPONENT_ATTACHMENT_FILE_RADIUS, COMPONENT_ATTACHMENT_GAP,
    COMPONENT_ATTACHMENT_IMAGE_MAX_HEIGHT, COMPONENT_ATTACHMENT_IMAGE_MAX_WIDTH,
    COMPONENT_ATTACHMENT_IMAGE_RADIUS, TYPE_BODY_LG, TYPE_BODY_SM, ThemeTokens,
};
use crate::theme::{color, length, theme};

/// Shown inline: the instance serves only these as pictures (ADR-0020); it stores most
/// compressed pictures as AVIF (ADR-0033).
const PREVIEWABLE_PICTURE_TYPES: [&str; 5] = [
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
];
const AUDIO_TYPES: [&str; 6] = [
    "audio/mpeg",
    "audio/aac",
    "audio/mp4",
    "audio/ogg",
    "audio/wav",
    "audio/flac",
];

/// The box a picture shows in (`imagePreviewWidth`): its own size, never above its pixels nor the
/// preview maxima, keeping its shape; the largest box when the instance gave no size.
#[must_use]
pub(crate) fn picture_box(width: Option<u32>, height: Option<u32>) -> (f32, f32) {
    match (width, height) {
        (Some(width), Some(height)) if width > 0 && height > 0 => {
            #[allow(clippy::cast_precision_loss)] // Picture sides are far below 2^24.
            let (width, height) = (width as f32, height as f32);
            let shown_width = width
                .min(COMPONENT_ATTACHMENT_IMAGE_MAX_WIDTH)
                .min(COMPONENT_ATTACHMENT_IMAGE_MAX_HEIGHT * width / height);
            (shown_width, shown_width * height / width)
        }
        _ => (
            COMPONENT_ATTACHMENT_IMAGE_MAX_WIDTH,
            COMPONENT_ATTACHMENT_IMAGE_MAX_HEIGHT,
        ),
    }
}

/// "512 bytes", "2.4 MB", « 2,4 Mo ».
#[must_use]
pub(crate) fn file_size_label(bytes: u64, strings: Strings) -> String {
    match human_file_size(locale(strings), bytes) {
        FileSize::Bytes(bytes) => strings
            .file_size()
            .bytes(i64::try_from(bytes).unwrap_or(i64::MAX)),
        FileSize::InUnit { value, unit } => strings.file_size().in_unit(
            &value,
            match unit {
                FileSizeUnit::Kilobytes => KbOrMbOrGb::Kb,
                FileSizeUnit::Megabytes => KbOrMbOrGb::Mb,
                FileSizeUnit::Gigabytes => KbOrMbOrGb::Gb,
            },
        ),
    }
}

#[derive(IntoElement)]
pub struct MessageAttachments {
    id: ElementId,
    attachments: Vec<Attachment>,
    is_pending: bool,
}

impl MessageAttachments {
    #[must_use]
    pub fn new(id: impl Into<ElementId>, attachments: Vec<Attachment>) -> Self {
        Self {
            id: id.into(),
            attachments,
            is_pending: false,
        }
    }

    /// A message not sent yet: its files show as cards, since nothing can be fetched yet.
    #[must_use]
    pub fn pending(mut self, is_pending: bool) -> Self {
        self.is_pending = is_pending;
        self
    }
}

/// A file the app neither shows nor plays: its card with the download button.
fn downloadable_file_card(id: ElementId, attachment: &Attachment, cx: &App) -> AnyElement {
    let glyph = if AUDIO_TYPES.contains(&attachment.content_type.as_str()) {
        IconName::FileAudio
    } else {
        IconName::File
    };
    let (download, saved) = download_control(&id, attachment, cx);
    media_card(id, attachment.file_name.clone(), cx)
        .child(name_row(
            attachment,
            glyph,
            vec![download.into_any_element()],
            cx,
        ))
        .children(saved)
        .into_any_element()
}

/// A file not sent yet, or a picture the instance could not serve: name and size only.
fn file_card(attachment: &Attachment, tokens: &ThemeTokens, strings: Strings) -> AnyElement {
    let is_audio = AUDIO_TYPES.contains(&attachment.content_type.as_str());
    div()
        .flex()
        .items_center()
        .gap(length(COMPONENT_ATTACHMENT_FILE_GAP))
        .w_full()
        .max_w(length(COMPONENT_ATTACHMENT_FILE_MAX_WIDTH))
        .p(length(COMPONENT_ATTACHMENT_FILE_PADDING))
        .rounded(length(COMPONENT_ATTACHMENT_FILE_RADIUS))
        .border(length(COMPONENT_ATTACHMENT_FILE_BORDER_WIDTH))
        .border_color(color(tokens.component_attachment_file_border))
        .bg(color(tokens.component_attachment_file_background))
        .child(icon(
            if is_audio {
                IconName::FileAudio
            } else {
                IconName::File
            },
            COMPONENT_ATTACHMENT_FILE_ICON_SIZE,
            color(tokens.component_attachment_file_icon),
        ))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    with_type(div(), TYPE_BODY_LG)
                        .truncate()
                        .text_color(color(tokens.component_attachment_file_name_text))
                        .child(attachment.file_name.clone()),
                )
                .child(
                    with_type(div(), TYPE_BODY_SM)
                        .text_color(color(tokens.component_attachment_file_size_text))
                        .child(file_size_label(attachment.size_bytes, strings)),
                ),
        )
        .into_any_element()
}

impl RenderOnce for MessageAttachments {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let strings = i18n::strings(cx);
        let animation = picture_animation(cx);
        let items: Vec<AnyElement> = self
            .attachments
            .iter()
            .map(|attachment| {
                let item_id = ElementId::NamedChild(
                    std::sync::Arc::new(self.id.clone()),
                    attachment.id.to_string().into(),
                );
                if self.is_pending {
                    return file_card(attachment, tokens, strings);
                }
                if is_played_in_app(attachment) {
                    return AudioAttachment::new(item_id, attachment.clone()).into_any_element();
                }
                if PLAYABLE_ATTACHMENT_VIDEO_TYPES.contains(&attachment.content_type.as_str()) {
                    return VideoAttachment::new(item_id, attachment.clone()).into_any_element();
                }
                if !PREVIEWABLE_PICTURE_TYPES.contains(&attachment.content_type.as_str()) {
                    return downloadable_file_card(item_id, attachment, cx);
                }
                let (width, height) = picture_box(attachment.width, attachment.height);
                let source = picture_for_box(
                    DownloadableFile::AttachmentPreview(attachment.id),
                    width,
                    height,
                    animation,
                    window,
                );
                let fallback = attachment.clone();
                let name = SharedString::from(attachment.file_name.clone());
                div()
                    .id(ElementId::NamedChild(
                        std::sync::Arc::new(self.id.clone()),
                        attachment.id.to_string().into(),
                    ))
                    .role(Role::Image)
                    .aria_label(name.clone())
                    .max_w_full()
                    .w(length(width))
                    // Scales down with a narrow column instead of cropping, as the web's `h-auto`.
                    .aspect_ratio(width / height)
                    .overflow_hidden()
                    .rounded(length(COMPONENT_ATTACHMENT_IMAGE_RADIUS))
                    .bg(color(tokens.component_attachment_image_placeholder))
                    .tooltip(move |_, cx| Tooltip::view(name.clone(), cx))
                    .tooltip_show_delay(Tooltip::show_delay())
                    .child(
                        img(source)
                            .size_full()
                            .object_fit(ObjectFit::Cover)
                            // A picture the instance could not serve shows as its file card.
                            .with_fallback(move || {
                                FallbackCard(fallback.clone()).into_any_element()
                            }),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .id(child_id(&self.id, "list"))
            .role(Role::List)
            .aria_label(
                strings
                    .messages()
                    .attachment_count(i64::try_from(self.attachments.len()).unwrap_or(i64::MAX)),
            )
            .flex()
            .flex_col()
            .items_start()
            .gap(length(COMPONENT_ATTACHMENT_GAP))
            .pt(length(COMPONENT_ATTACHMENT_GAP))
            .children(items)
    }
}

/// The file card in place of a picture that failed, drawn with the theme at that moment.
#[derive(IntoElement)]
struct FallbackCard(Attachment);

impl RenderOnce for FallbackCard {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        file_card(&self.0, theme(cx), i18n::strings(cx))
    }
}

impl std::fmt::Debug for MessageAttachments {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MessageAttachments")
            .field("id", &self.id)
            .field("attachments", &self.attachments.len())
            .field("is_pending", &self.is_pending)
            .finish()
    }
}

impl std::fmt::Debug for FallbackCard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_tuple("FallbackCard")
            .field(&self.0.id)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    #[test]
    fn a_picture_box_keeps_its_shape_within_the_maxima_and_its_own_pixels() {
        assert_eq!(picture_box(Some(200), Some(100)), (200.0, 100.0));
        assert_eq!(picture_box(Some(4000), Some(1000)), (400.0, 100.0));
        assert_eq!(picture_box(Some(1000), Some(4000)), (75.0, 300.0));
        assert_eq!(
            picture_box(None, Some(10)),
            (
                COMPONENT_ATTACHMENT_IMAGE_MAX_WIDTH,
                COMPONENT_ATTACHMENT_IMAGE_MAX_HEIGHT
            )
        );
        assert_eq!(
            picture_box(Some(0), Some(10)),
            (
                COMPONENT_ATTACHMENT_IMAGE_MAX_WIDTH,
                COMPONENT_ATTACHMENT_IMAGE_MAX_HEIGHT
            )
        );
    }

    #[test]
    fn sizes_are_worded_in_the_readers_language() {
        assert_eq!(
            file_size_label(512, Strings::new(Language::English)),
            "512 bytes"
        );
        assert_eq!(
            file_size_label(2_516_582, Strings::new(Language::French)),
            "2,4\u{a0}Mo"
        );
    }
}
