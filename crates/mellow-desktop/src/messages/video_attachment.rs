//! A video attached to a message (`MessageAttachments.svelte`, `web-inline-media.md`): a small
//! screen of the media stage, its box fixed before anything loads (the stored size, else 16:9),
//! nothing fetched before play, with its controls laid over the picture's bottom edge as the web's
//! browser controls are (`video_player`). Its name, size, "Enlarge" and download sit in the card's
//! row under it. "Enlarge" opens the media viewer on the video, going on playing; F on the focused
//! video does the same with the window full screen. A video this system cannot decode (a container
//! or codec it lacks, past the size limit) becomes the file card with the "download it" note.

use std::rc::Rc;

use gpui::{
    App, ElementId, FocusHandle, IntoElement, ParentElement as _, RenderOnce, Styled as _, Window,
    prelude::FluentBuilder as _,
};
use mellow_protocol::Attachment;

use super::attachments::picture_box;
use super::audio_attachment::{media_card, name_row, playback_note};
use super::video_player::{VideoScreen, play_video};
use crate::attachment_audio::{
    AudioPlayback, attachment_audio_view, is_video_played_in_app, keep_player_while_shown,
};
use crate::attachment_download::download_control;
use crate::components::{IconButton, child_id, remembered_focus_handle};
use crate::i18n;
use crate::media_viewer::{ViewerSet, open_media_viewer};
use crate::theme::icons::IconName;
use crate::theme::length;
use crate::theme::tokens::{
    COMPONENT_ATTACHMENT_IMAGE_MAX_HEIGHT, COMPONENT_ATTACHMENT_IMAGE_MAX_WIDTH,
    COMPONENT_ATTACHMENT_IMAGE_RADIUS,
};

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
    /// The message's pictures and videos, to open the viewer on; none, no "Enlarge".
    viewer_set: Option<ViewerSet>,
}

impl VideoAttachment {
    pub(crate) fn new(id: ElementId, attachment: Attachment) -> Self {
        Self {
            id,
            attachment,
            viewer_set: None,
        }
    }

    #[must_use]
    pub(crate) fn viewer(mut self, viewer_set: Option<ViewerSet>) -> Self {
        self.viewer_set = viewer_set;
        self
    }
}

impl RenderOnce for VideoAttachment {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        keep_player_while_shown(&self.id, self.attachment.id, window, cx);
        let id = self.id;
        let attachment = self.attachment;
        let messages = i18n::strings(cx).messages();
        let name = attachment.file_name.clone();
        let mut playback = attachment_audio_view(attachment.id, cx).playback;
        if !is_video_played_in_app(&attachment) {
            playback = AudioPlayback::CannotPlay;
        }
        let (download, saved) = download_control(&id, &attachment, cx);
        if playback == AudioPlayback::CannotPlay {
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

        let screen_id = child_id(&id, "stage");
        let screen_focus = remembered_focus_handle(&screen_id, window, cx);
        // Opens the viewer on this video, playing, focus coming back to `opener`.
        let open_viewer = self.viewer_set.map(|set| {
            let opened = attachment.clone();
            Rc::new(
                move |opener: FocusHandle,
                      is_fullscreen: bool,
                      window: &mut Window,
                      cx: &mut App| {
                    open_media_viewer(&set, opened.id, Some(opener), is_fullscreen, window, cx);
                    play_video(&opened, cx);
                },
            )
        });
        let enlarge = open_viewer.clone().map(|open_viewer| {
            let enlarge_id = child_id(&id, "enlarge");
            let enlarge_focus = remembered_focus_handle(&enlarge_id, window, cx);
            let opener = enlarge_focus.clone();
            IconButton::new(
                enlarge_id,
                IconName::CornersOut,
                messages.enlarge_video(&name),
            )
            .track_focus(&enlarge_focus)
            .on_click(move |window, cx| open_viewer(opener.clone(), false, window, cx))
            .into_any_element()
        });
        let screen = VideoScreen::new(screen_id, attachment.clone())
            .track_focus(&screen_focus)
            .when_some(open_viewer, |screen, open_viewer| {
                let opener = screen_focus.clone();
                screen.on_fullscreen(None, move |window, cx| {
                    open_viewer(opener.clone(), true, window, cx);
                })
            });

        let (width, height) = video_box(attachment.width, attachment.height);
        media_card(id.clone(), messages.video_player(&name), cx)
            .child(
                screen
                    .w_full()
                    .aspect_ratio(width / height)
                    .rounded(length(COMPONENT_ATTACHMENT_IMAGE_RADIUS)),
            )
            .child(name_row(
                &attachment,
                IconName::File,
                enlarge
                    .into_iter()
                    .chain([download.into_any_element()])
                    .collect(),
                cx,
            ))
            .when(playback == AudioPlayback::NoOutput, |card| {
                card.child(playback_note(messages.no_audio_output(), cx))
            })
            .children(saved)
            .into_any_element()
    }
}

impl std::fmt::Debug for VideoAttachment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("VideoAttachment")
            .field("id", &self.id)
            .field("attachment", &self.attachment.id)
            .field("has_viewer", &self.viewer_set.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Context, Render, TestAppContext, WindowAppearance, div};
    use mellow_protocol::Snowflake;

    use super::*;
    use crate::attachment_audio::{MAXIMUM_PLAYED_ATTACHMENT_BYTES, toggle_attachment_audio};
    use crate::theme::ActiveTheme;
    use crate::theme::tokens::COMPONENT_ATTACHMENT_FILE_MAX_WIDTH;
    use gpui::px;

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
        assert_eq!(cx.debug_bounds("video-controls").is_some(), is_played_here);
        assert_eq!(cx.debug_bounds("audio-note").is_some(), !is_played_here);

        // Refused before anything is fetched: the card says to download it instead.
        cx.update(|window, cx| {
            toggle_attachment_audio(&too_large, cx);
            window.refresh();
        });
        cx.run_until_parked();
        assert!(cx.debug_bounds("video-controls").is_none());
        assert!(cx.debug_bounds("audio-note").is_some());
    }

    #[gpui::test]
    fn a_container_this_system_does_not_read_is_offered_as_a_file(cx: &mut TestAppContext) {
        let mut cx = shown(video("video/x-msvideo", 10), cx);
        assert!(cx.debug_bounds("video-controls").is_none());
        assert!(cx.debug_bounds("audio-note").is_some());
    }
}
