//! The popover headless, outside a call: what it says, where its keys go, what it reports.

use std::sync::Arc;

use gpui::{
    Context, Entity, InputEvent as _, IntoElement, KeyUpEvent, Keystroke, Render, Subscription,
    TestAppContext, VisualTestContext, Window, WindowAppearance, bounds, div, point, px, size,
};
use mellow_media::{StreamFormat, SyntheticAudioDevices, SyntheticSound};
use mellow_protocol::GatewayEvent;

use super::*;
use crate::components::bind_tab_navigation;
use crate::shell_fixtures::{GUILD_RUST, sample_state_administering_rust};
use crate::theme::ActiveTheme;
use crate::voice::install_voice;

struct Host {
    popover: Entity<SoundboardPopover>,
    events: Vec<SoundboardPopoverEvent>,
    _subscription: Subscription,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.popover.clone())
    }
}

fn sound(id: u64, name: &str) -> SoundboardSound {
    SoundboardSound {
        id: Snowflake::from_raw(id),
        name: name.to_owned(),
        emoji: None,
        duration_milliseconds: 900,
    }
}

fn open(
    sounds: Vec<SoundboardSound>,
    cx: &mut TestAppContext,
) -> (Entity<Host>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
        cx.set_global(crate::strings_pending_i18n::ActiveStrings(
            &crate::strings_pending_i18n::ENGLISH,
        ));
        bind_tab_navigation(cx);
    });
    let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
    session.update(cx, |session, _| {
        let mut state = sample_state_administering_rust();
        state.apply_event(
            GatewayEvent::GuildSoundsUpdate {
                guild_id: GUILD_RUST,
                sounds,
            },
            0,
        );
        *session.state_mut() = state;
    });
    let voice = cx.update(|cx| {
        install_voice(
            &session,
            Arc::new(|| {
                let format = StreamFormat {
                    sample_rate: 48_000,
                    channels: 1,
                };
                Box::new(SyntheticAudioDevices::new(
                    SyntheticSound::Silence,
                    format,
                    format,
                ))
            }),
            cx,
        )
    });
    let (host, cx) = cx.add_window_view(|window, cx| {
        let anchor = bounds(point(px(40.0), px(600.0)), size(px(32.0), px(32.0)));
        let popover = cx.new(|cx| {
            SoundboardPopover::open(anchor, session, voice, GUILD_RUST, true, window, cx)
        });
        let subscription = cx.subscribe(&popover, |host: &mut Host, _, event, _| {
            host.events.push(*event);
        });
        Host {
            popover,
            events: Vec::new(),
            _subscription: subscription,
        }
    });
    cx.simulate_resize(size(px(900.0), px(700.0)));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (host, cx)
}

fn popover_of(host: &Entity<Host>, cx: &mut VisualTestContext) -> Entity<SoundboardPopover> {
    host.read_with(cx, |host, _| host.popover.clone())
}

#[gpui::test]
fn arrows_move_between_sounds_and_escape_closes_with_focus_back(cx: &mut TestAppContext) {
    let sounds = (1..=4).map(|id| sound(id, "beep")).collect();
    let (host, cx) = open(sounds, cx);
    let popover = popover_of(&host, cx);
    assert!(cx.update(|window, cx| popover.read(cx).grid_focus.is_focused(window)));

    cx.simulate_keystrokes("right");
    cx.simulate_keystrokes("down");
    assert_eq!(popover.read_with(cx, |popover, _| popover.active_index), 3);
    cx.simulate_keystrokes("left");
    assert_eq!(popover.read_with(cx, |popover, _| popover.active_index), 2);
    // Outside a call the soundboard cannot play: Enter sends nothing.
    cx.simulate_keystrokes("enter");

    cx.simulate_keystrokes("escape");
    assert_eq!(
        host.read_with(cx, |host, _| host.events.clone()),
        vec![SoundboardPopoverEvent::Dismissed {
            should_return_focus: true
        }]
    );
}

#[gpui::test]
fn without_sounds_add_sounds_takes_focus_and_opens_the_settings(cx: &mut TestAppContext) {
    let (host, cx) = open(Vec::new(), cx);
    let popover = popover_of(&host, cx);
    assert!(cx.update(|window, cx| popover.read(cx).add_focus.is_focused(window)));
    // GPUI activates a focused button on the key's release, as browsers do.
    cx.simulate_keystrokes("enter");
    let keystroke = Keystroke::parse("enter").expect("a valid keystroke in a test");
    cx.update(|window, cx| {
        window.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
    });
    cx.run_until_parked();
    assert!(
        host.read_with(cx, |host, _| host.events.clone())
            .contains(&SoundboardPopoverEvent::AddSounds)
    );
}
