//! The viewer driven headless: opening from a message, previous and next, the loading notice's
//! delay on the test clock, actual size, keys and focus coming back on close. No picture session
//! runs, so every picture stays loading.

use std::time::Duration;

use gpui::{
    Context, FocusHandle, InputEvent as _, IntoElement, KeyUpEvent, Keystroke, Modifiers, Render,
    TestAppContext, VisualTestContext, Window, WindowAppearance, div, px,
};
use mellow_protocol::{Attachment, Snowflake};

use super::*;
use crate::components::{bind_tab_navigation, with_tab_navigation};
use crate::messages::MessageAttachments;
use crate::theme::ActiveTheme;

const SENT_AT: u64 = 1_762_180_320_000;

fn picture(id: u64, name: &str, width: u32, height: u32) -> Attachment {
    Attachment {
        id: Snowflake::from_raw(id),
        file_name: name.to_owned(),
        content_type: "image/png".to_owned(),
        size_bytes: 1_200_000,
        width: Some(width),
        height: Some(height),
    }
}

fn message_pictures() -> Vec<Attachment> {
    vec![
        picture(1, "capture.png", 2880, 1800),
        picture(2, "chat.gif", 480, 270),
        // A file card between them: not part of the viewer's set.
        Attachment {
            id: Snowflake::from_raw(3),
            file_name: "notes.txt".to_owned(),
            content_type: "text/plain".to_owned(),
            size_bytes: 10,
            width: None,
            height: None,
        },
        picture(4, "schéma.webp", 1200, 900),
    ]
}

struct Harness {
    root: FocusHandle,
    attachments: Vec<Attachment>,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        with_tab_navigation(div().size_full(), &self.root)
            .child(
                MessageAttachments::new("message", self.attachments.clone())
                    .sent_by("Ana", SENT_AT),
            )
            .children(open_media_viewer_view(cx))
    }
}

fn set_up(cx: &mut TestAppContext) -> VisualTestContext {
    set_up_with(message_pictures(), cx)
}

fn set_up_with(attachments: Vec<Attachment>, cx: &mut TestAppContext) -> VisualTestContext {
    cx.update(|cx| {
        cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
        gpui_component::init(cx);
        bind_tab_navigation(cx);
    });
    let (harness, cx) = cx.add_window_view(|_, cx| Harness {
        root: cx.focus_handle(),
        attachments,
    });
    let root = harness.read_with(cx, |harness, _| harness.root.clone());
    cx.update(|window, cx| {
        window.activate_window();
        window.focus(&root, cx);
    });
    cx.run_until_parked();
    cx.clone()
}

fn press(cx: &mut VisualTestContext, key: &'static str) {
    cx.simulate_keystrokes(key);
    let keystroke = Keystroke::parse(key).expect("a valid keystroke in a test");
    cx.update(|window, cx| {
        window.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
    });
    cx.run_until_parked();
}

fn click_centre_of(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), Modifiers::default());
    cx.run_until_parked();
}

/// Past the entrance animation, drawn again.
fn settle(cx: &mut VisualTestContext) {
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
}

fn viewer(cx: &mut VisualTestContext) -> Option<Entity<MediaViewer>> {
    cx.update(|_, cx| open_media_viewer_view(cx))
}

fn shown_name(cx: &mut VisualTestContext) -> String {
    let viewer = viewer(cx).expect("the viewer is open");
    viewer.read_with(cx, |viewer, _| {
        viewer
            .shown()
            .map(|shown| shown.file_name.clone())
            .unwrap_or_default()
    })
}

#[gpui::test]
fn a_click_opens_the_message_pictures_and_previous_and_next_wrap(cx: &mut TestAppContext) {
    let mut cx = set_up(cx);
    click_centre_of(&mut cx, "attachment-picture-2");
    assert!(cx.debug_bounds("media-viewer").is_some());
    assert_eq!(shown_name(&mut cx), "chat.gif");
    let count = viewer(&mut cx)
        .expect("open")
        .read_with(&cx, |viewer, _| viewer.set.media.len());
    assert_eq!(count, 3, "the file card is not one of the viewer's items");

    press(&mut cx, "right");
    assert_eq!(shown_name(&mut cx), "schéma.webp");
    press(&mut cx, "right");
    assert_eq!(
        shown_name(&mut cx),
        "capture.png",
        "after the last comes the first"
    );
    press(&mut cx, "left");
    assert_eq!(shown_name(&mut cx), "schéma.webp");
}

#[gpui::test]
fn the_loading_notice_waits_340_ms(cx: &mut TestAppContext) {
    let mut cx = set_up(cx);
    click_centre_of(&mut cx, "attachment-picture-2");
    assert!(cx.debug_bounds("media-viewer-notice").is_none());
    cx.executor().advance_clock(Duration::from_millis(339));
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("media-viewer-notice").is_none(),
        "a picture arriving within the delay never flashes the notice"
    );
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    assert!(cx.debug_bounds("media-viewer-notice").is_some());

    // Another picture starts the wait again.
    press(&mut cx, "right");
    assert!(cx.debug_bounds("media-viewer-notice").is_none());
}

#[gpui::test]
fn a_picture_larger_than_the_room_toggles_to_its_own_size(cx: &mut TestAppContext) {
    let mut cx = set_up(cx);
    click_centre_of(&mut cx, "attachment-picture-1");
    settle(&mut cx);
    let fitted = cx
        .debug_bounds("media-viewer-picture")
        .expect("the picture is drawn");
    assert!(fitted.size.width < px(2880.0));

    click_centre_of(&mut cx, "media-viewer-picture");
    let actual = cx
        .debug_bounds("media-viewer-picture")
        .expect("the picture is drawn");
    assert_eq!(actual.size.width, px(2880.0));
    assert_eq!(actual.size.height, px(1800.0));
    // The middle clicked stays under the pointer.
    let middle = actual.origin + gpui::point(actual.size.width / 2.0, actual.size.height / 2.0);
    assert!((f32::from(middle.x) - f32::from(fitted.center().x)).abs() < 2.0);
    assert!((f32::from(middle.y) - f32::from(fitted.center().y)).abs() < 2.0);

    // Back to fit; a small picture is never a toggle.
    click_centre_of(&mut cx, "media-viewer-picture");
    let refitted = cx.debug_bounds("media-viewer-picture").expect("drawn");
    assert_eq!(refitted.size, fitted.size);
    press(&mut cx, "right");
    click_centre_of(&mut cx, "media-viewer-picture");
    let small = cx.debug_bounds("media-viewer-picture").expect("drawn");
    assert_eq!(small.size.width, px(480.0));
    assert!(
        viewer(&mut cx).is_some(),
        "a click on the picture never closes"
    );
}

#[gpui::test]
fn escape_closes_and_focus_goes_back_to_the_picture(cx: &mut TestAppContext) {
    let mut cx = set_up(cx);
    // From the keyboard: Tab to the first picture, Enter opens it.
    press(&mut cx, "tab");
    press(&mut cx, "enter");
    assert_eq!(shown_name(&mut cx), "capture.png");
    // Tab stays inside the viewer.
    for _ in 0..12 {
        press(&mut cx, "tab");
        let is_inside = cx.update(|window, cx| {
            open_media_viewer_view(cx)
                .is_some_and(|viewer| viewer.read(cx).container_focus.contains_focused(window, cx))
        });
        assert!(is_inside, "Tab never leaves the viewer");
    }
    press(&mut cx, "escape");
    assert!(viewer(&mut cx).is_none());
    assert!(cx.debug_bounds("media-viewer").is_none());
    // Enter on the focused picture opens it again: focus came back to it.
    press(&mut cx, "enter");
    assert_eq!(shown_name(&mut cx), "capture.png");
}

#[gpui::test]
fn a_click_on_the_room_closes_it(cx: &mut TestAppContext) {
    let mut cx = set_up(cx);
    click_centre_of(&mut cx, "attachment-picture-2");
    let room = cx.debug_bounds("media-viewer").expect("open");
    cx.simulate_click(
        room.origin + gpui::point(px(4.0), px(4.0)),
        Modifiers::default(),
    );
    cx.run_until_parked();
    assert!(viewer(&mut cx).is_none());
}

/// Videos open in the viewer only where this system plays them: Windows decodes no video
/// attachment yet (ROADMAP M3, "the Media Foundation H.264 decoder is not yet wired into
/// `VideoStream`"), so a video there is a file card and stays out of the viewer's set.
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod playable_videos {
    use super::*;
    use crate::attachment_audio::{AudioPlayback, attachment_audio_view};

    fn video_and_picture() -> Vec<Attachment> {
        vec![
            Attachment {
                id: Snowflake::from_raw(7),
                file_name: "clip.mp4".to_owned(),
                content_type: "video/mp4".to_owned(),
                size_bytes: 2_000_000,
                width: Some(1280),
                height: Some(720),
            },
            picture(8, "après.png", 480, 270),
        ]
    }

    fn open_on(attachment_id: u64, attachments: &[Attachment], cx: &mut VisualTestContext) {
        let set = ViewerSet::of_message(attachments, "Ana", SENT_AT);
        cx.update(|window, cx| {
            open_media_viewer(
                &set,
                Snowflake::from_raw(attachment_id),
                None,
                false,
                window,
                cx,
            );
        });
        cx.run_until_parked();
    }

    fn playback_of(attachment_id: u64, cx: &mut VisualTestContext) -> AudioPlayback {
        cx.update(|_, cx| attachment_audio_view(Snowflake::from_raw(attachment_id), cx).playback)
    }

    #[gpui::test]
    fn a_video_still_loading_never_plays_once_the_viewer_moved_on(cx: &mut TestAppContext) {
        let attachments = video_and_picture();
        let mut cx = set_up_with(attachments.clone(), cx);
        open_on(7, &attachments, &mut cx);
        cx.update(|_, cx| {
            crate::attachment_audio::pretend_fetching_for_tests(Snowflake::from_raw(7), cx);
        });
        // "Next picture" (the focused video's own Right seeks).
        let open = viewer(&mut cx).expect("open");
        open.update_in(&mut cx, |viewer, window, cx| viewer.step(true, window, cx));
        cx.run_until_parked();
        assert_eq!(playback_of(7, &mut cx), AudioPlayback::Idle, "next picture");

        open.update_in(&mut cx, |viewer, window, cx| viewer.step(false, window, cx));
        cx.run_until_parked();
        cx.update(|_, cx| {
            crate::attachment_audio::pretend_fetching_for_tests(Snowflake::from_raw(7), cx);
        });
        press(&mut cx, "escape");
        assert!(viewer(&mut cx).is_none());
        assert_eq!(playback_of(7, &mut cx), AudioPlayback::Idle, "closed");
    }

    #[gpui::test]
    fn after_a_click_on_the_video_a_click_on_the_room_closes(cx: &mut TestAppContext) {
        let attachments = video_and_picture();
        let mut cx = set_up_with(attachments.clone(), cx);
        open_on(7, &attachments, &mut cx);
        settle(&mut cx);
        click_centre_of(&mut cx, "video-screen");
        assert!(viewer(&mut cx).is_some(), "the video is not the room");
        let room = cx.debug_bounds("media-viewer").expect("open");
        cx.simulate_click(
            room.origin + gpui::point(px(4.0), px(4.0)),
            Modifiers::default(),
        );
        cx.run_until_parked();
        assert!(viewer(&mut cx).is_none());
    }
}

#[gpui::test]
fn try_again_is_reached_with_tab(cx: &mut TestAppContext) {
    let mut cx = set_up(cx);
    cx.update(|_, cx| {
        crate::pictures::fail_attachment_pictures_for_tests(Snowflake::from_raw(2), cx);
    });
    click_centre_of(&mut cx, "attachment-picture-2");
    assert!(cx.debug_bounds("media-viewer-notice").is_some());
    let is_retry_focused = |cx: &mut VisualTestContext| {
        cx.update(|window, cx| {
            open_media_viewer_view(cx)
                .is_some_and(|viewer| viewer.read(cx).retry_focus.is_focused(window))
        })
    };
    let mut is_reached = false;
    for _ in 0..12 {
        press(&mut cx, "tab");
        if is_retry_focused(&mut cx) {
            is_reached = true;
            break;
        }
    }
    assert!(is_reached, "\"Try again\" is a Tab stop");
}
