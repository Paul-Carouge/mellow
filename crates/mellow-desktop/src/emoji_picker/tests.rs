//! The picker headless: what its keys reach and what it reports.

use gpui::{
    Context, Entity, InputEvent as _, IntoElement, KeyUpEvent, Keystroke, Render, Subscription,
    TestAppContext, VisualTestContext, Window, WindowAppearance, bounds, div, point, px, size,
};
use mellow_client_core::shared_logic::emoji_catalogue::{EMOJI_CATALOGUE, EmojiCategory};

use super::*;
use crate::components::bind_tab_navigation;
use crate::theme::ActiveTheme;

#[test]
fn sections_are_laid_out_as_a_heading_then_rows_of_columns() {
    let sections = picker_sections("", &["👍".to_owned()], None);
    let (lines, starts, headings) = lay_out(&sections, 8);
    assert_eq!(lines[0], PickerLine::Heading(0));
    assert_eq!(
        lines[1],
        PickerLine::Cells {
            section: 0,
            start: 0,
            end: 1
        }
    );
    assert_eq!(lines[2], PickerLine::Heading(1));
    assert_eq!((starts[1], headings[1]), (1, 2));
    let people = EMOJI_CATALOGUE
        .iter()
        .filter(|entry| entry.category == EmojiCategory::People)
        .count();
    // The people's rows, the last one short, then the next heading.
    assert_eq!(headings[2], 3 + people.div_ceil(8));
    assert_eq!(EMOJI_PICKER_COLUMNS, 8);
}

/// Shows the picker and keeps what it reported.
struct Host {
    picker: Entity<EmojiPicker>,
    events: Vec<EmojiPickerEvent>,
    _subscription: Subscription,
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.picker.clone())
    }
}

fn open(cx: &mut TestAppContext) -> (Entity<Host>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
        bind_tab_navigation(cx);
        bind_emoji_picker_keys(cx);
    });
    let (host, cx) = cx.add_window_view(|window, cx| {
        let anchor = bounds(point(px(600.0), px(500.0)), size(px(32.0), px(32.0)));
        let picker =
            cx.new(|cx| EmojiPicker::open(anchor, vec!["🔥".to_owned()], None, window, cx));
        let subscription = cx.subscribe(&picker, |host: &mut Host, _, event, _| {
            host.events.push(event.clone());
        });
        Host {
            picker,
            events: Vec::new(),
            _subscription: subscription,
        }
    });
    cx.simulate_resize(size(px(900.0), px(700.0)));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    (host, cx)
}

/// A full key press: GPUI activates a focused control on the release, as browsers do.
fn press(cx: &mut VisualTestContext, key: &'static str) {
    cx.simulate_keystrokes(key);
    let keystroke = Keystroke::parse(key).expect("a valid keystroke in a test");
    cx.update(|window, cx| {
        window.dispatch_event(KeyUpEvent { keystroke }.to_platform_input(), cx);
    });
    cx.run_until_parked();
}

fn events(host: &Entity<Host>, cx: &mut VisualTestContext) -> Vec<EmojiPickerEvent> {
    host.read_with(cx, |host, _| host.events.clone())
}

fn picker_of(host: &Entity<Host>, cx: &mut VisualTestContext) -> Entity<EmojiPicker> {
    host.read_with(cx, |host, _| host.picker.clone())
}

#[gpui::test]
fn down_from_the_search_then_arrows_and_enter_pick_an_emoji(cx: &mut TestAppContext) {
    let (host, cx) = open(cx);
    press(cx, "down");
    press(cx, "right");
    press(cx, "down");
    press(cx, "enter");
    // Recent (🔥) is one cell; Right reaches the first person, Down the ninth.
    let picker = picker_of(&host, cx);
    let expected = picker.read_with(cx, |picker, _| picker.emoji_at(9).cloned());
    assert_eq!(
        events(&host, cx),
        expected
            .map(EmojiPickerEvent::Picked)
            .into_iter()
            .collect::<Vec<_>>()
    );
}

#[gpui::test]
fn a_search_keeps_only_results_and_enter_in_the_field_picks_the_first(cx: &mut TestAppContext) {
    let (host, cx) = open(cx);
    cx.simulate_input("coeur rouge");
    cx.run_until_parked();
    let picker = picker_of(&host, cx);
    let kinds: Vec<PickerSectionKind> = picker.read_with(cx, |picker, _| {
        picker.sections.iter().map(|section| section.kind).collect()
    });
    assert_eq!(kinds, [PickerSectionKind::Results]);
    press(cx, "enter");
    assert_eq!(
        events(&host, cx),
        [EmojiPickerEvent::Picked(PickerEmoji::Unicode(
            "❤️".to_owned()
        ))]
    );
}

#[gpui::test]
fn up_from_the_first_row_goes_back_to_the_search_and_esc_closes(cx: &mut TestAppContext) {
    let (host, cx) = open(cx);
    press(cx, "down");
    press(cx, "up");
    let picker = picker_of(&host, cx);
    let is_search_focused = cx.update(|window, cx| {
        picker
            .read(cx)
            .search
            .read(cx)
            .focus_handle(cx)
            .is_focused(window)
    });
    assert!(is_search_focused);
    press(cx, "escape");
    assert_eq!(
        events(&host, cx),
        [EmojiPickerEvent::Dismissed {
            should_return_focus: true
        }]
    );
}

#[gpui::test]
fn page_down_and_a_category_move_the_emojis_tab_stop_to_the_sections_start(
    cx: &mut TestAppContext,
) {
    let (host, cx) = open(cx);
    press(cx, "down");
    press(cx, "pagedown");
    let picker = picker_of(&host, cx);
    // Past the one recent emoji, the people start.
    assert_eq!(picker.read_with(cx, |picker, _| picker.active_index), 1);
    cx.update(|_, cx| picker.update(cx, |picker, cx| picker.show_section(2, cx)));
    let (active, current, start) = picker.read_with(cx, |picker, _| {
        (
            picker.active_index,
            picker.current_section,
            picker.section_starts[2],
        )
    });
    assert_eq!((active, current), (start, 2));
}
