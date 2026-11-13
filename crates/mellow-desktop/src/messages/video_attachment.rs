//! A video attached to a message (`MessageAttachments.svelte`, `web-inline-media.md`): a small
//! screen of the media stage, its box fixed before anything loads (the stored size, else 16:9),
//! nothing fetched before play. The web draws the browser's controls over the picture and a band
//! under it; the browser's controls follow no theme, so here the stage keeps the picture whole
//! and the sound card's name row and controls sit under it, on the card's ground, where their
//! colours hold in both themes. Pressing the picture plays or pauses it; "Enlarge" fills the
//! window with the picture and the same controls (the desktop has no picture viewer), Escape or
//! "Close the video" coming back. A video this system cannot decode (a container or codec it
//! lacks, past the size limit) becomes the file card with the "download it" note.

use gpui::{
    App, ElementId, InteractiveElement as _, IntoElement, KeyDownEvent, ParentElement as _,
    RenderOnce, Role, StatefulInteractiveElement as _, Styled as _, Window, anchored, deferred,
    div, point, prelude::FluentBuilder as _, px,
};
use mellow_protocol::Attachment;

use super::attachments::picture_box;
use super::audio_attachment::{media_card, name_row, playback_note, player_controls};
use crate::attachment_audio::{
    AudioPlayback, attachment_audio_view, attachment_video_picture, is_video_played_in_app,
    keep_player_while_shown, toggle_attachment_audio,
};
use crate::attachment_download::download_control;
use crate::components::{
    FocusNextControl, FocusPreviousControl, IconButton, child_id, focus_next_inside,
    focus_previous_inside, icon, remembered_focus_handle,
};
use crate::i18n;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_ATTACHMENT_FILE_ICON_SIZE, COMPONENT_ATTACHMENT_FILE_PADDING,
    COMPONENT_ATTACHMENT_IMAGE_MAX_HEIGHT, COMPONENT_ATTACHMENT_IMAGE_MAX_WIDTH,
    COMPONENT_ATTACHMENT_IMAGE_RADIUS,
};
use crate::theme::{color, length, theme};
use crate::voice::video_view::{VideoFit, video_view};

/// A video's shape before it plays, as on the web: 16:9.
const DEFAULT_VIDEO_ASPECT_RATIO: f32 = 16.0 / 9.0;

/// The video's box: its stored size within the picture maxima, else the widest 16:9 box.
#[must_use]
pub(crate) fn video_box(width: Option<u32>, height: Option<u32>) -> (f32, f32) {
    if width.is_some_and(|width| width > 0) && height.is_some_and(|height| height > 0) {
        return picture_box(width, height);
    }
    let width = COMPONENT_ATTACHMENT_IMAGE_MAX_WIDTH
        .min(COMPONENT_ATTACHMENT_IMAGE_MAX_HEIGHT * DEFAULT_VIDEO_ASPECT_RATIO);
    (width, width / DEFAULT_VIDEO_ASPECT_RATIO)
}

#[derive(IntoElement)]
pub(crate) struct VideoAttachment {
    id: ElementId,
    attachment: Attachment,
}

impl VideoAttachment {
    pub(crate) fn new(id: ElementId, attachment: Attachment) -> Self {
        Self { id, attachment }
    }
}

impl RenderOnce for VideoAttachment {
    #[allow(clippy::too_many_lines)] // The card, its stage and the enlarged view.
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        keep_player_while_shown(&self.id, self.attachment.id, window, cx);
        let id = self.id;
        let attachment = self.attachment;
        let messages = i18n::strings(cx).messages();
        let name = attachment.file_name.clone();
        let mut view = attachment_audio_view(attachment.id, cx);
        if !is_video_played_in_app(&attachment) {
            view.playback = AudioPlayback::CannotPlay;
        }
        let (download, saved) = download_control(&id, &attachment, cx);
        if view.playback == AudioPlayback::CannotPlay {
            return media_card(id, messages.video_player(&name), cx)
                .child(name_row(
                    &attachment,
                    IconName::File,
                    vec![download.into_any_element()],
                    cx,
                ))
                .child(playback_note(messages.cannot_play(), cx))
                .children(saved)
                .into_any_element();
        }

        let tokens = theme(cx);
        let picture = attachment_video_picture(attachment.id, cx);
        let is_playing = view.playback == AudioPlayback::Playing;
        let enlarged = window.use_keyed_state(child_id(&id, "enlarged"), cx, |_, _| false);
        let is_enlarged = *enlarged.read(cx);
        let enlarge_focus = remembered_focus_handle(&child_id(&id, "enlarge"), window, cx);
        let close_focus = remembered_focus_handle(&child_id(&id, "close"), window, cx);
        // The enlarged view's own container, not a stop itself: Tab and Shift+Tab go round its
        // controls as in every modal layer, never to the conversation hidden behind.
        let enlarged_view_focus = window
            .use_keyed_state(child_id(&id, "enlarged-focus"), cx, |_, cx| {
                cx.focus_handle()
            })
            .read(cx)
            .clone();

        let stage = |stage_id: ElementId, cx: &App| {
            let toggled = attachment.clone();
            div()
                .id(stage_id)
                .relative()
                .flex()
                .items_center()
                .justify_center()
                .overflow_hidden()
                .bg(color(theme(cx).component_attachment_video_background))
                .cursor_pointer()
                .on_click(move |_, _, cx| toggle_attachment_audio(&toggled, cx))
                .children(picture.as_ref().map(|picture| {
                    div()
                        .absolute()
                        .size_full()
                        .child(video_view(picture, VideoFit::Contain))
                }))
                // Decorative: the play button below is the one screen readers and Tab reach.
                .when(!is_playing, |stage| {
                    stage.child(icon(
                        IconName::Play,
                        COMPONENT_ATTACHMENT_FILE_ICON_SIZE,
                        color(theme(cx).color_media_text),
                    ))
                })
        };

        let (width, height) = video_box(attachment.width, attachment.height);
        let enlarge = {
            let enlarged = enlarged.clone();
            let close_focus = close_focus.clone();
            IconButton::new(
                child_id(&id, "enlarge"),
                IconName::CornersOut,
                messages.enlarge_video(&name),
            )
            .track_focus(&enlarge_focus)
            .on_click(move |window, cx| {
                enlarged.update(cx, |is_enlarged, cx| {
                    *is_enlarged = true;
                    cx.notify();
                });
                window.focus(&close_focus, cx);
            })
        };

        let enlarged_view = is_enlarged.then(|| {
            let viewport = window.viewport_size();
            let close = {
                let enlarged = enlarged.clone();
                let enlarge_focus = enlarge_focus.clone();
                move |window: &mut Window, cx: &mut App| {
                    enlarged.update(cx, |is_enlarged, cx| {
                        *is_enlarged = false;
                        cx.notify();
                    });
                    window.focus(&enlarge_focus, cx);
                }
            };
            let close_on_escape = close.clone();
            let (next_container, previous_container) =
                (enlarged_view_focus.clone(), enlarged_view_focus.clone());
            let controls = div()
                .flex()
                .items_center()
                .p(length(COMPONENT_ATTACHMENT_FILE_PADDING))
                .bg(color(tokens.component_attachment_file_background))
                .child(div().flex_1().min_w_0().child(player_controls(
                    &child_id(&id, "large"),
                    &attachment,
                    view,
                    cx,
                )))
                .child(
                    IconButton::new(
                        child_id(&id, "close"),
                        IconName::X,
                        i18n::strings(cx).desktop().close_enlarged_video(),
                    )
                    .track_focus(&close_focus)
                    .on_click(close),
                );
            deferred(
                anchored().position(point(px(0.0), px(0.0))).child(
                    div()
                        .id(child_id(&id, "enlarged-view"))
                        .role(Role::Dialog)
                        .aria_label(messages.video_player(&name))
                        .track_focus(&enlarged_view_focus)
                        .tab_group()
                        .on_action(move |_: &FocusNextControl, window, cx| {
                            focus_next_inside(&next_container, window, cx);
                        })
                        .on_action(move |_: &FocusPreviousControl, window, cx| {
                            focus_previous_inside(&previous_container, window, cx);
                        })
                        .occlude()
                        .w(viewport.width)
                        .h(viewport.height)
                        .flex()
                        .flex_col()
                        .bg(color(tokens.color_media_stage))
                        .on_key_down(move |event: &KeyDownEvent, window, cx| {
                            if event.keystroke.key == "escape" {
                                cx.stop_propagation();
                                close_on_escape(window, cx);
                            }
                        })
                        .child(stage(child_id(&id, "large-stage"), cx).flex_1().w_full())
                        .child(controls),
                ),
            )
            .with_priority(1)
        });

        media_card(id.clone(), messages.video_player(&name), cx)
            .child(
                stage(child_id(&id, "stage"), cx)
                    .w_full()
                    .aspect_ratio(width / height)
                    .rounded(length(COMPONENT_ATTACHMENT_IMAGE_RADIUS)),
            )
            .child(name_row(
                &attachment,
                IconName::File,
                vec![enlarge.into_any_element(), download.into_any_element()],
                cx,
            ))
            .child(player_controls(&id, &attachment, view, cx))
            .when(view.playback == AudioPlayback::NoOutput, |card| {
                card.child(playback_note(messages.no_audio_output(), cx))
            })
            .children(saved)
            .children(enlarged_view)
            .into_any_element()
    }
}

impl std::fmt::Debug for VideoAttachment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VideoAttachment")
            .field("id", &self.id)
            .field("attachment", &self.attachment.id)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Context, Render, TestAppContext, WindowAppearance};
    use mellow_protocol::Snowflake;

    use super::*;
    use crate::attachment_audio::MAXIMUM_PLAYED_ATTACHMENT_BYTES;
    use crate::theme::ActiveTheme;
    use crate::theme::tokens::COMPONENT_ATTACHMENT_FILE_MAX_WIDTH;

    struct Card(Attachment);

    impl Render for Card {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .w(px(COMPONENT_ATTACHMENT_FILE_MAX_WIDTH))
                .child(VideoAttachment::new("video".into(), self.0.clone()))
        }
    }

    fn video(content_type: &str, size_bytes: u64) -> Attachment {
        Attachment {
            id: Snowflake::from_raw(44),
            file_name: "clip.mp4".to_owned(),
            content_type: content_type.to_owned(),
            size_bytes,
            width: None,
            height: None,
        }
    }

    #[test]
    fn a_video_box_is_its_stored_shape_or_sixteen_by_nine() {
        assert_eq!(video_box(None, None), (400.0, 225.0));
        assert_eq!(
            video_box(Some(1080), Some(1920)),
            picture_box(Some(1080), Some(1920))
        );
        assert_eq!(video_box(Some(0), Some(10)), (400.0, 225.0));
    }

    fn shown(attachment: Attachment, cx: &mut TestAppContext) -> gpui::VisualTestContext {
        cx.update(|cx| {
            cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
            gpui_component::init(cx);
        });
        let (_card, cx) = cx.add_window_view(|_, _| Card(attachment));
        cx.run_until_parked();
        cx.clone()
    }

    #[gpui::test]
    fn a_video_shows_its_player_until_it_cannot_play_here(cx: &mut TestAppContext) {
        let too_large = video("video/mp4", MAXIMUM_PLAYED_ATTACHMENT_BYTES + 1);
        let mut cx = shown(too_large.clone(), cx);
        let is_played_here = cfg!(any(target_os = "linux", target_os = "macos"));
        assert_eq!(cx.debug_bounds("audio-player").is_some(), is_played_here);
        assert_eq!(cx.debug_bounds("audio-note").is_some(), !is_played_here);

        // Refused before anything is fetched: the card says to download it instead.
        cx.update(|window, cx| {
            toggle_attachment_audio(&too_large, cx);
            window.refresh();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("audio-player").is_none());
        assert!(cx.debug_bounds("audio-note").is_some());
    }

    #[gpui::test]
    fn a_container_this_system_does_not_read_is_offered_as_a_file(cx: &mut TestAppContext) {
        let mut cx = shown(video("video/x-msvideo", 10), cx);
        assert!(cx.debug_bounds("audio-player").is_none());
        assert!(cx.debug_bounds("audio-note").is_some());
    }
}
