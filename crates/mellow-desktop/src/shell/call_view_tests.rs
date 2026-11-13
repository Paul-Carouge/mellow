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
