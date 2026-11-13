//! The call view in the shell, drawn from synthetic voice states and pictures: no server, no
//! engine (a call still starting), so only the view's own rules run.

use std::sync::Arc;

use gpui::{
    InputEvent as _, KeyUpEvent, Keystroke, TestAppContext, VisualTestContext, WindowAppearance,
    px, size,
};
use mellow_client_core::shared_logic::call_view::CallLayout;
use mellow_media::{StreamFormat, SyntheticAudioDevices, SyntheticSound};
use mellow_protocol::{GatewayEvent, VideoCodec, VideoTileSize, VoiceState};

use super::*;
use crate::shell::DuskShell;
use crate::shell_fixtures::{ANA, GUILD_RUST, ME, RUST_VOICE, ZOE, sample_state};
use crate::voice::install_voice;
use crate::voice::video_view::tests::synthetic_frame;

fn voice_state(
    user_id: Snowflake,
    video: Option<CallVideo>,
    codec: Option<VideoCodec>,
) -> VoiceState {
    VoiceState {
        user_id,
        guild_id: GUILD_RUST,
        channel_id: RUST_VOICE,
        self_mute: user_id == ZOE,
        self_deaf: false,
        suppressed: false,
        join_sound_id: None,
        self_video: video == Some(CallVideo::Camera),
        self_stream: video == Some(CallVideo::Screen),
        video_codec: codec,
    }
}

fn press(cx: &mut VisualTestContext, key: &'static str) {
    cx.simulate_keystrokes(key);
    let keystroke = Keystroke::parse(key).expect("a valid keystroke in a test");
    cx.update(|window, cx| {
        window.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
    });
    cx.run_until_parked();
}

/// A shell with the app's voice, in a call in Rust's Lounge with Ana (camera) and Zoé (a screen
/// shared in a codec nobody decodes).
fn shell_in_a_call(
    cx: &mut TestAppContext,
) -> (Entity<DuskShell>, Entity<Voice>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(crate::strings_pending_i18n::ActiveStrings(
            &crate::strings_pending_i18n::ENGLISH,
        ));
        cx.set_global(crate::theme::ActiveTheme::for_appearance(
            WindowAppearance::Dark,
        ));
        crate::components::bind_tab_navigation(cx);
        crate::shell::bind_shell_keys(cx);
    });
    let mut state = sample_state();
    for voice_state in [
        voice_state(ME, None, None),
        voice_state(ANA, Some(CallVideo::Camera), Some(VideoCodec::Vp8)),
        voice_state(ZOE, Some(CallVideo::Screen), Some(VideoCodec::Unknown)),
    ] {
        state.apply_event(GatewayEvent::VoiceStateUpdate { voice_state }, 0);
    }
    let session = cx.new(|_| crate::session::Session::signed_out_kept_in_memory_only());
    session.update(cx, |session, _| *session.state_mut() = state);
    let format = StreamFormat {
        sample_rate: 48_000,
        channels: 1,
    };
    let voice = cx.update(|cx| {
        install_voice(
            &session,
            Arc::new(move || {
                Box::new(SyntheticAudioDevices::new(
                    SyntheticSound::Silence,
                    format,
                    format,
                ))
            }),
            cx,
        )
    });
    let (shell, cx) = cx.add_window_view(|window, cx| DuskShell::new(session, window, cx));
    cx.simulate_resize(size(px(1_400.0), px(800.0)));
    cx.update(|window, _| window.activate_window());
    voice.update(cx, |voice, cx| {
        voice.pretend_call_for_tests(GUILD_RUST, RUST_VOICE);
        voice.open_call_view(cx);
    });
    cx.run_until_parked();
    (shell, voice, cx)
}

fn call_view(shell: &Entity<DuskShell>, cx: &mut VisualTestContext) -> Entity<CallView> {
    shell.read_with(cx, |shell, _| {
        shell
            .call_view
            .as_ref()
            .map(|(view, _)| view.clone())
            .expect("the call view is open")
    })
}

#[gpui::test]
fn the_call_view_replaces_the_sheet_with_a_tile_per_person_and_asks_for_their_pictures(
    cx: &mut TestAppContext,
) {
    let (shell, voice, cx) = shell_in_a_call(cx);
    assert!(
        cx.debug_bounds("chat-header").is_none(),
        "the sheet steps aside"
    );
    for user_id in [ME, ANA, ZOE] {
        assert!(
            cx.debug_bounds(Box::leak(
                format!("call-tile-{}", user_id.raw()).into_boxed_str()
            ))
            .is_some(),
            "{user_id:?} has a tile"
        );
    }
    // Three tiles: medium pictures, yours aside, at their drawn size.
    let watched = voice.read_with(cx, |voice, _| voice.watched_cameras_for_tests());
    let watched_ids: Vec<Snowflake> = watched.iter().map(|shown| shown.user_id).collect();
    assert_eq!(watched_ids, [ANA, ZOE]);
    assert!(
        watched
            .iter()
            .all(|shown| shown.tile_size == VideoTileSize::Medium)
    );
    assert!(
        watched
            .iter()
            .all(|shown| shown.display_width > shown.display_height)
    );

    // A decoded picture of Ana's camera shows on her tile; Zoé's undecodable screen does not.
    assert!(cx.debug_bounds("call-picture-1001").is_none());
    let pictures = voice.read_with(cx, |voice, _| voice.video_pictures());
    pictures.update(cx, |pictures, cx| {
        pictures.show(vec![(ANA, synthetic_frame(64, 36))], cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("call-picture-1001").is_some());

    // Putting Ana forward: the speaker view, her picture large and the others small.
    let view = call_view(&shell, cx);
    view.update(cx, |view, cx| view.toggle_picked(ANA, cx));
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |view, _| view.layout()),
        CallLayout::Speaker
    );
    let watched = voice.read_with(cx, |voice, _| voice.watched_cameras_for_tests());
    assert_eq!(
        watched
            .iter()
            .map(|shown| (shown.user_id, shown.tile_size))
            .collect::<Vec<_>>(),
        [(ANA, VideoTileSize::Large), (ZOE, VideoTileSize::Small)]
    );
}

#[gpui::test]
fn a_watched_screen_takes_the_view_and_escape_goes_back_step_by_step(cx: &mut TestAppContext) {
    let (shell, voice, cx) = shell_in_a_call(cx);
    voice.update(cx, |voice, cx| voice.watch_stream(Some(ZOE), cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("stream-view").is_some());
    // The call view asks for no camera while a screen is watched.
    assert_eq!(
        voice.read_with(cx, |voice, _| voice.watched_cameras_for_tests()),
        []
    );

    press(cx, "escape");
    assert!(cx.debug_bounds("stream-view").is_none(), "back to the call");
    assert!(cx.debug_bounds("call-tile-1001").is_some());
    assert_eq!(
        voice.read_with(cx, |voice, _| voice.watched_stream_user_id()),
        None
    );

    press(cx, "escape");
    assert!(shell.read_with(cx, |shell, _| shell.call_view.is_none()));
    assert!(cx.debug_bounds("chat-header").is_some(), "back to the chat");
    assert!(!voice.read_with(cx, |voice, _| voice.is_call_view_open()));
}

#[gpui::test]
fn the_call_view_closes_when_the_call_ends(cx: &mut TestAppContext) {
    let (shell, voice, cx) = shell_in_a_call(cx);
    voice.update(cx, Voice::leave);
    cx.run_until_parked();
    assert!(shell.read_with(cx, |shell, _| shell.call_view.is_none()));
    assert!(cx.debug_bounds("chat-header").is_some());
}

fn toggle_fullscreen(view: &Entity<CallView>, cx: &mut VisualTestContext) {
    cx.update(|window, cx| view.update(cx, |view, cx| view.toggle_fullscreen(window, cx)));
    cx.run_until_parked();
}

fn is_window_fullscreen(cx: &mut VisualTestContext) -> bool {
    cx.update(|window, _| window.is_fullscreen())
}

#[gpui::test]
fn full_screen_shows_the_featured_tile_alone_and_escape_leaves_it_before_closing(
    cx: &mut TestAppContext,
) {
    let (shell, voice, cx) = shell_in_a_call(cx);
    let view = call_view(&shell, cx);
    view.update(cx, |view, cx| view.toggle_picked(ANA, cx));
    toggle_fullscreen(&view, cx);
    assert!(is_window_fullscreen(cx));
    assert!(cx.debug_bounds("call-tile-1001").is_some());
    assert!(
        cx.debug_bounds("call-tile-1002").is_none(),
        "the strip is hidden"
    );
    assert!(
        cx.debug_bounds("chat-header").is_none(),
        "nothing of the shell is drawn underneath"
    );
    assert_eq!(
        voice
            .read_with(cx, |voice, _| voice.watched_cameras_for_tests())
            .iter()
            .map(|shown| shown.user_id)
            .collect::<Vec<_>>(),
        [ANA]
    );

    press(cx, "escape");
    assert!(!view.read_with(cx, |view, _| view.is_fullscreen()));
    assert!(!is_window_fullscreen(cx));
    assert!(shell.read_with(cx, |shell, _| shell.call_view.is_some()));

    press(cx, "escape");
    assert!(shell.read_with(cx, |shell, _| shell.call_view.is_none()));
}

#[gpui::test]
fn a_window_already_in_full_screen_stays_so_when_the_view_leaves_it(cx: &mut TestAppContext) {
    let (shell, _voice, cx) = shell_in_a_call(cx);
    cx.update(|window, _| window.toggle_fullscreen());
    let view = call_view(&shell, cx);
    toggle_fullscreen(&view, cx);
    toggle_fullscreen(&view, cx);
    assert!(!view.read_with(cx, |view, _| view.is_fullscreen()));
    assert!(is_window_fullscreen(cx), "the user's own full screen");

    toggle_fullscreen(&view, cx);
    press(cx, "escape");
    press(cx, "escape");
    assert!(shell.read_with(cx, |shell, _| shell.call_view.is_none()));
    assert!(is_window_fullscreen(cx));
}

#[gpui::test]
fn a_watched_screen_that_goes_away_in_full_screen_leaves_full_screen(cx: &mut TestAppContext) {
    let (shell, voice, cx) = shell_in_a_call(cx);
    voice.update(cx, |voice, cx| voice.watch_stream(Some(ZOE), cx));
    cx.run_until_parked();
    let view = call_view(&shell, cx);
    toggle_fullscreen(&view, cx);
    assert!(is_window_fullscreen(cx));

    voice.update(cx, |voice, cx| voice.watch_stream(None, cx));
    cx.run_until_parked();
    assert!(!view.read_with(cx, |view, _| view.is_fullscreen()));
    assert!(!is_window_fullscreen(cx));
    assert_eq!(
        view.read_with(cx, |view, _| view.layout()),
        CallLayout::Grid,
        "the layout full screen forced is undone"
    );
    assert!(
        cx.debug_bounds("chat-header").is_none(),
        "the call view stays"
    );
}

#[gpui::test]
fn undecodable_and_paused_pictures_say_so(cx: &mut TestAppContext) {
    let (_shell, voice, cx) = shell_in_a_call(cx);
    // Zoé shares in a codec this app does not decode; Ana's camera plays.
    assert!(cx.debug_bounds("call-sentence-1002").is_some());
    assert!(cx.debug_bounds("call-sentence-1001").is_none());
    assert!(cx.debug_bounds("call-paused-1001").is_none());

    voice.update(cx, |voice, cx| {
        voice.note_video_paused_for_tests(ANA, true, cx);
    });
    cx.run_until_parked();
    assert!(cx.debug_bounds("call-paused-1001").is_some());

    voice.update(cx, |voice, cx| voice.watch_stream(Some(ZOE), cx));
    cx.run_until_parked();
    assert!(cx.debug_bounds("stream-sentence").is_some());
}

#[gpui::test]
fn the_last_pictures_go_when_the_call_ends(cx: &mut TestAppContext) {
    let (_shell, voice, cx) = shell_in_a_call(cx);
    let pictures = voice.read_with(cx, |voice, _| voice.video_pictures());
    pictures.update(cx, |pictures, cx| {
        pictures.show(vec![(ANA, synthetic_frame(64, 36))], cx);
        pictures.show_own_camera(Some(synthetic_frame(64, 36)), cx);
    });
    voice.update(cx, Voice::leave);
    cx.run_until_parked();
    pictures.read_with(cx, |pictures, _| {
        assert!(pictures.picture(ANA).is_none());
        assert!(pictures.own_camera().is_none());
    });
}

/// Your voice state says `video`, as the instance sends it back.
fn set_own_video(shell: &Entity<DuskShell>, video: Option<CallVideo>, cx: &mut VisualTestContext) {
    let session = shell.read_with(cx, |shell, _| shell.session.clone());
    session.update(cx, |session, cx| {
        session.state_mut().apply_event(
            GatewayEvent::VoiceStateUpdate {
                voice_state: voice_state(ME, video, Some(VideoCodec::Vp8)),
            },
            0,
        );
        cx.notify();
    });
    cx.run_until_parked();
}

#[gpui::test]
fn your_tile_shows_your_camera_never_your_shared_screen(cx: &mut TestAppContext) {
    let (shell, voice, cx) = shell_in_a_call(cx);
    let own_picture: &'static str = format!("call-picture-{}", ME.raw()).leak();
    let pictures = voice.read_with(cx, |voice, _| voice.video_pictures());
    pictures.update(cx, |pictures, cx| {
        pictures.show_own_camera(Some(synthetic_frame(64, 36)), cx);
    });
    set_own_video(&shell, Some(CallVideo::Camera), cx);
    assert!(cx.debug_bounds(own_picture).is_some());

    set_own_video(&shell, Some(CallVideo::Screen), cx);
    assert!(cx.debug_bounds(own_picture).is_none());
}

#[gpui::test]
fn the_camera_and_share_controls_follow_what_can_start(cx: &mut TestAppContext) {
    // A call without its engine yet: the camera cannot open; a share follows the system's picker
    // (none on Windows), as the voice panel's share row does.
    let (_shell, voice, cx) = shell_in_a_call(cx);
    assert!(cx.debug_bounds("call-control-microphone").is_some());
    assert!(cx.debug_bounds("call-control-camera").is_none());
    let can_share_screen = voice.read_with(cx, |voice, _| voice.can_share_screen());
    assert_eq!(
        cx.debug_bounds("call-control-screen-share").is_some(),
        can_share_screen
    );
}

#[test]
fn people_are_listed_in_join_order_with_you_added_before_your_state_comes_back() {
    let states = [
        voice_state(ANA, Some(CallVideo::Camera), None),
        voice_state(ZOE, None, None),
    ];
    let people = call_people(&states, Some(ME));
    assert_eq!(
        people
            .iter()
            .map(|person| person.user_id)
            .collect::<Vec<_>>(),
        [ANA, ZOE, ME]
    );
    assert_eq!(call_people(&states, Some(ANA)).len(), 2);
}
