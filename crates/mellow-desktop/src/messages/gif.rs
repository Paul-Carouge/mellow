//! `MessageGif.svelte` (ADR-0042, docs/design/web-gif-picker.md): a message whose whole text is one
//! Klipy media address shows the GIF itself. The box is always `component.gif.message-height`
//! high, whatever the picture and the column, so the list never moves when it loads; its width
//! follows the picture's shape once decoded (a square until then), and a picture wider than the
//! column is letterboxed. It plays as custom emoji do; while reduced motion keeps pictures still it
//! shows the first frame and a "GIF ▶" key that plays this one. A GIF Klipy no longer serves
//! gives the link back (`fallback`).
//!
//! shortcut: Klipy's MP4 and `WebM` addresses keep their link; video playback comes with step 9.

use gpui::{
    AnyElement, App, ElementId, InteractiveElement as _, IntoElement, ObjectFit,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, StyledImage as _, Toggled, Window, div, img, prelude::FluentBuilder as _,
};

use mellow_client_core::shared_logic::klipy::message_gif_media_of;

use super::{picture_animation, picture_request_for_box};
use crate::components::{
    Tooltip, child_id, icon, is_focus_visible, remembered_focus_handle, with_type,
};
use crate::i18n::strings;
use crate::pictures::{Animation, PictureFile, PictureStatus, picture_source, picture_status};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_GIF_MESSAGE_HEIGHT, COMPONENT_GIF_MESSAGE_MAX_WIDTH, COMPONENT_GIF_MESSAGE_RADIUS,
    COMPONENT_GIF_PLAY_BUTTON_SIZE, COMPONENT_GIF_STAR_ICON_SIZE, COMPONENT_GIF_STAR_INSET,
    SIZE_FOCUS_RING_OFFSET, SIZE_FOCUS_RING_WIDTH, SPACE_3XS, SPACE_XS, TYPE_BODY_SM,
};
use crate::theme::{color, length, theme};

/// The GIF a message's text is, when the instance has GIFs (only then does its key let Klipy's
/// media in, as the web's policy does): a picture address; a clip keeps its link (step 9).
#[must_use]
pub fn message_gif_url(content: &str, are_gifs_shown: bool) -> Option<SharedString> {
    are_gifs_shown
        .then(|| message_gif_media_of(content))
        .flatten()
        .filter(|media| !media.is_video)
        .map(|media| media.url.into())
}

/// The box's width for a picture of `width` × `height`: its shape at the box's height, at most the
/// GIF's maximum width (the column bounds it further); a square before the picture is known.
#[must_use]
pub fn message_gif_width(picture: Option<(u32, u32)>) -> f32 {
    let Some((width, height)) = picture.filter(|(width, height)| *width > 0 && *height > 0) else {
        return COMPONENT_GIF_MESSAGE_HEIGHT;
    };
    // Pictures are a few hundred pixels a side: exact in f32.
    #[allow(clippy::cast_precision_loss)]
    let aspect_ratio = width as f32 / height as f32;
    (COMPONENT_GIF_MESSAGE_HEIGHT * aspect_ratio).min(COMPONENT_GIF_MESSAGE_MAX_WIDTH)
}

/// A message's GIF, or `fallback` (its link) once the picture failed.
#[derive(IntoElement)]
pub struct MessageGif {
    id: ElementId,
    url: SharedString,
    fallback: AnyElement,
}

impl MessageGif {
    /// `url` passed `message_gif_media_of`; `fallback` is the message's text, shown when the GIF
    /// cannot be.
    #[must_use]
    pub fn new(id: impl Into<ElementId>, url: SharedString, fallback: AnyElement) -> Self {
        Self {
            id: id.into(),
            url,
            fallback,
        }
    }
}

impl RenderOnce for MessageGif {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = self.id;
        let is_kept_still = picture_animation(cx) == Animation::StillFrame;
        let is_playing_by_choice =
            window.use_keyed_state(child_id(&id, "playing"), cx, |_, _| false);
        let animation = if is_kept_still && !*is_playing_by_choice.read(cx) {
            Animation::StillFrame
        } else {
            Animation::Animate
        };
        // Decoded to cover a square of the box's height: enough for any shape it is drawn at.
        let request = picture_request_for_box(
            PictureFile::Klipy(self.url),
            COMPONENT_GIF_MESSAGE_HEIGHT,
            COMPONENT_GIF_MESSAGE_HEIGHT,
            animation,
            window,
        );
        let picture_size = match picture_status(request.clone(), cx) {
            PictureStatus::Failed => return self.fallback,
            PictureStatus::Ready { width, height } => Some((width, height)),
            PictureStatus::Loading => None,
        };
        let tokens = theme(cx);
        let label = strings(cx).gifs().message_gif();
        let play_key = is_kept_still.then(|| {
            let is_playing = *is_playing_by_choice.read(cx);
            play_key(child_id(&id, "play"), is_playing, window, cx).on_click(move |_, _, cx| {
                is_playing_by_choice.update(cx, |is_playing, cx| {
                    *is_playing = !*is_playing;
                    cx.notify();
                });
            })
        });
        div()
            .id(id)
            .role(Role::Image)
            .aria_label(label)
            .relative()
            .flex_none()
            .h(length(COMPONENT_GIF_MESSAGE_HEIGHT))
            .w(length(message_gif_width(picture_size)))
            .max_w_full()
            .overflow_hidden()
            .rounded(length(COMPONENT_GIF_MESSAGE_RADIUS))
            .bg(color(tokens.component_gif_message_placeholder))
            .child(
                img(picture_source(request))
                    .size_full()
                    .object_fit(ObjectFit::Contain),
            )
            .children(play_key)
            .into_any_element()
    }
}

impl std::fmt::Debug for MessageGif {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MessageGif")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// The "GIF ▶" key over a GIF kept still: plays this one, or keeps it still again.
fn play_key(
    id: ElementId,
    is_playing: bool,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Stateful<gpui::Div> {
    let tokens = theme(cx);
    let label = strings(cx).gifs().play_gif();
    let focus_handle = remembered_focus_handle(&id, window, cx);
    let ink = color(tokens.component_gif_play_button_text);
    let ring_distance = SIZE_FOCUS_RING_WIDTH + SIZE_FOCUS_RING_OFFSET;
    let is_ring_shown = is_focus_visible(&focus_handle, window);
    with_type(div(), TYPE_BODY_SM)
        .id(id)
        .role(Role::Button)
        .aria_label(label)
        .aria_toggled(if is_playing {
            Toggled::True
        } else {
            Toggled::False
        })
        .track_focus(&focus_handle)
        .absolute()
        .left(length(COMPONENT_GIF_STAR_INSET))
        .bottom(length(COMPONENT_GIF_STAR_INSET))
        .flex()
        .items_center()
        .gap(length(SPACE_3XS))
        .h(length(COMPONENT_GIF_PLAY_BUTTON_SIZE))
        .px(length(SPACE_XS))
        .rounded_full()
        .bg(color(tokens.component_gif_play_button_background))
        .text_color(ink)
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .cursor_pointer()
        .tooltip(move |_, cx| Tooltip::view(label.into(), cx))
        .tooltip_show_delay(Tooltip::show_delay())
        .child(icon(IconName::Gif, COMPONENT_GIF_STAR_ICON_SIZE, ink))
        .when(!is_playing, |key| {
            key.child(icon(IconName::Play, COMPONENT_GIF_STAR_ICON_SIZE, ink))
        })
        // The stage's sky ring, which keeps its contrast over any picture.
        .when(is_ring_shown, |key| {
            let outside = -length(ring_distance);
            key.child(
                div()
                    .absolute()
                    .top(outside)
                    .left(outside)
                    .right(outside)
                    .bottom(outside)
                    .rounded_full()
                    .border(length(SIZE_FOCUS_RING_WIDTH))
                    .border_color(color(tokens.component_gif_tile_focus_ring)),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_klipy_picture_address_is_a_gif_and_only_on_an_instance_with_gifs() {
        let address = "https://static.klipy.com/ii/a/b/md.webp";
        assert_eq!(message_gif_url(address, true), Some(address.into()));
        assert_eq!(message_gif_url(address, false), None);
        assert_eq!(
            message_gif_url("https://static.klipy.com/ii/a/b/md.mp4", true),
            None
        );
        assert_eq!(message_gif_url("look at this", true), None);
    }

    #[test]
    fn the_box_keeps_its_height_and_takes_the_pictures_shape_up_to_its_maximum_width() {
        let height = COMPONENT_GIF_MESSAGE_HEIGHT;
        assert!(
            (message_gif_width(None) - height).abs() < f32::EPSILON,
            "a square until loaded"
        );
        assert!(
            (message_gif_width(Some((200, 100)))
                - (height * 2.0).min(COMPONENT_GIF_MESSAGE_MAX_WIDTH))
            .abs()
                < 1e-3
        );
        assert!((message_gif_width(Some((100, 200))) - height / 2.0).abs() < 1e-3);
        assert!(
            (message_gif_width(Some((1000, 100))) - COMPONENT_GIF_MESSAGE_MAX_WIDTH).abs() < 1e-3
        );
        assert!((message_gif_width(Some((0, 100))) - height).abs() < f32::EPSILON);
    }
}
