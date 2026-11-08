//! The GIF picker headless, without Klipy: its sheet is filled by hand (`show_items`), so nothing
//! leaves the machine.

use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, TestAppContext, VisualTestContext, Window, WindowAppearance, div, point, px,
    size,
};
use mellow_client_core::shared_logic::klipy::KlipyMedia;

use super::*;
use crate::components::bind_tab_navigation;
use crate::strings_pending_i18n::{ActiveStrings, ENGLISH};
use crate::theme::ActiveTheme;

struct Harness {
    picker: Entity<GifPicker>,
    events: Vec<GifPickerEvent>,
    _subscription: Subscription,
}

impl Render for Harness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.picker.clone())
    }
}

fn gif(slug: &str) -> KlipyItem {
    let media = |name: &str| KlipyMedia {
        url: format!("https://static.klipy.com/ii/{slug}/{name}"),
        width: 216,
        height: 144,
        is_video: false,
    };
    KlipyItem::Gif(KlipyGif {
        slug: slug.to_owned(),
        title: format!("GIF {slug}"),
        tile: media("sm.webp"),
        picture_tile: Some(media("sm.webp")),
        still: Some(media("sm.jpg")),
        message_url: media("md.webp").url,
    })
}

/// A picker beside a button near the window's bottom right, its sheet holding four GIFs of the
/// same shape: two rows of two in its 414 px.
fn open_picker(cx: &mut TestAppContext) -> (Entity<Harness>, &mut VisualTestContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(ActiveStrings(&ENGLISH));
        cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
        bind_tab_navigation(cx);
    });
    let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
    let settings = GifSettings {
        klipy_app_key: "mock-key".to_owned(),
        customer_id: "0123456789abcdef".to_owned(),
    };
    let (harness, cx) = cx.add_window_view(|window, cx| {
        let anchor = Bounds::new(point(px(800.0), px(560.0)), size(px(32.0), px(32.0)));
        let picker = cx.new(|cx| GifPicker::new(session, settings, None, anchor, window, cx));
        let subscription = cx.subscribe(&picker, |harness: &mut Harness, _, event, _| {
            harness.events.push(event.clone());
        });
        Harness {
            picker,
            events: Vec::new(),
            _subscription: subscription,
        }
    });
    cx.simulate_resize(size(px(900.0), px(600.0)));
    cx.update(|window, _| window.activate_window());
    cx.run_until_parked();
    let picker = picker_of(&harness, cx);
    picker.update(cx, |picker, cx| {
        picker.show_items(vec![gif("a"), gif("b"), gif("c"), gif("d")], cx);
    });
    cx.run_until_parked();
    (harness, cx)
}

fn picker_of(harness: &Entity<Harness>, cx: &mut VisualTestContext) -> Entity<GifPicker> {
    harness.read_with(cx, |harness, _| harness.picker.clone())
}

fn active_index(harness: &Entity<Harness>, cx: &mut VisualTestContext) -> usize {
    picker_of(harness, cx).read_with(cx, |picker, _| picker.active_index)
}

fn take_events(harness: &Entity<Harness>, cx: &mut VisualTestContext) -> Vec<GifPickerEvent> {
    harness.update(cx, |harness, _| std::mem::take(&mut harness.events))
}

#[gpui::test]
fn without_klipy_the_picker_says_it_cannot_be_reached(cx: &mut TestAppContext) {
    let (harness, cx) = open_picker(cx);
    let picker = picker_of(&harness, cx);
    // Before the sheet was filled by hand, the trending request failed at once.
    picker.update(cx, GifPicker::load_first_page);
    cx.run_until_parked();
    assert_eq!(
        picker.read_with(cx, |picker, _| picker.status),
        SheetStatus::Failed(GifFailure::Failed)
    );
}

#[gpui::test]
fn arrows_move_over_the_rows_enter_sends_and_up_from_the_first_row_returns_to_the_search(
    cx: &mut TestAppContext,
) {
    let (harness, cx) = open_picker(cx);
    let picker = picker_of(&harness, cx);
    let search_focus = picker.read_with(cx, |picker, cx| picker.search.read(cx).focus_handle(cx));
    assert!(
        cx.update(|window, _| search_focus.is_focused(window)),
        "the search has focus first"
    );

    cx.simulate_keystrokes("down");
    assert_eq!(active_index(&harness, cx), 0);
    cx.simulate_keystrokes("right");
    assert_eq!(active_index(&harness, cx), 1);
    cx.simulate_keystrokes("down");
    assert_eq!(active_index(&harness, cx), 3);
    cx.simulate_keystrokes("home");
    assert_eq!(active_index(&harness, cx), 0);
    cx.simulate_keystrokes("up");
    assert!(cx.update(|window, _| search_focus.is_focused(window)));

    cx.simulate_keystrokes("down");
    cx.simulate_keystrokes("end");
    cx.simulate_keystrokes("enter");
    assert_eq!(
        take_events(&harness, cx),
        [GifPickerEvent::Pick {
            slug: "d".to_owned(),
            message_url: "https://static.klipy.com/ii/d/md.webp".to_owned(),
            query: String::new(),
        }]
    );
}

#[gpui::test]
fn esc_closes_the_picker_from_the_search_and_from_the_tiles(cx: &mut TestAppContext) {
    let (harness, cx) = open_picker(cx);
    cx.simulate_keystrokes("escape");
    assert_eq!(
        take_events(&harness, cx),
        [GifPickerEvent::Dismissed { by_pointer: false }]
    );
    cx.simulate_keystrokes("down");
    cx.simulate_keystrokes("escape");
    assert_eq!(
        take_events(&harness, cx),
        [GifPickerEvent::Dismissed { by_pointer: false }]
    );
}

fn media(name: &str) -> KlipyMedia {
    KlipyMedia {
        url: format!("https://static.klipy.com/ii/a/{name}"),
        width: 220,
        height: 180,
        is_video: false,
    }
}

#[test]
fn tiles_play_the_animated_picture_and_keep_the_jpeg_still_under_reduced_motion() {
    let gif = KlipyGif {
        slug: "a".to_owned(),
        title: String::new(),
        tile: KlipyMedia {
            is_video: true,
            ..media("sm.mp4")
        },
        picture_tile: Some(media("md.webp")),
        still: Some(media("sm.jpg")),
        message_url: media("md.webp").url,
    };
    assert_eq!(
        tile_picture(&gif, Animation::Animate),
        Some(&media("md.webp"))
    );
    assert_eq!(
        tile_picture(&gif, Animation::StillFrame),
        Some(&media("sm.jpg"))
    );
    let without_still = KlipyGif { still: None, ..gif };
    assert_eq!(
        tile_picture(&without_still, Animation::StillFrame),
        Some(&media("md.webp")),
        "its first frame then"
    );
}

#[test]
fn arrows_and_home_end_name_tile_keys() {
    assert_eq!(tile_key("left"), Some(GifTileKey::Left));
    assert_eq!(tile_key("end"), Some(GifTileKey::End));
    assert_eq!(tile_key("a"), None);
}

#[test]
fn a_refused_star_says_why_in_the_webs_words() {
    let catalogue = Strings::default();
    let limit = ClientError::Api {
        status: 409,
        code: ApiErrorCode::Conflict,
        message: String::new(),
    };
    assert_eq!(
        favourite_failure_sentence(&limit, catalogue, &crate::strings_pending_i18n::ENGLISH),
        catalogue
            .gifs()
            .too_many_favourites(i64::from(MAXIMUM_GIF_FAVOURITES))
    );
    let busy = ClientError::Api {
        status: 429,
        code: ApiErrorCode::RateLimited,
        message: String::new(),
    };
    assert_eq!(
        favourite_failure_sentence(&busy, catalogue, &crate::strings_pending_i18n::ENGLISH),
        catalogue.gifs().favourites_busy()
    );
}

#[gpui::test]
fn the_sheet_follows_the_active_tile_down_and_back_up(cx: &mut TestAppContext) {
    let (harness, cx) = open_picker(cx);
    let picker = picker_of(&harness, cx);
    let slugs: Vec<String> = (0..24).map(|index| format!("g{index}")).collect();
    picker.update(cx, |picker, cx| {
        picker.show_items(slugs.iter().map(|slug| gif(slug)).collect(), cx);
    });
    cx.run_until_parked();
    let scrolled = |cx: &mut VisualTestContext| {
        picker.read_with(cx, |picker, _| -f32::from(picker.scroll.offset().y))
    };

    cx.simulate_keystrokes("down");
    cx.simulate_keystrokes("end");
    cx.run_until_parked();
    assert_eq!(active_index(&harness, cx), 23);
    assert!(scrolled(cx) > 0.0, "the last row is brought into view");

    cx.simulate_keystrokes("home");
    cx.run_until_parked();
    assert!(scrolled(cx).abs() < f32::EPSILON, "and the first one again");
}

#[test]
fn a_row_is_scrolled_into_view_by_the_shortest_distance() {
    // Already visible: the sheet stays.
    assert!((scrolled_to_show(100.0, 200.0, 50.0, 300.0) - 50.0).abs() < f32::EPSILON);
    // Below the visible part: its bottom meets the sheet's bottom.
    assert!((scrolled_to_show(400.0, 500.0, 50.0, 300.0) - 200.0).abs() < f32::EPSILON);
    // Above it: its top meets the sheet's top.
    assert!((scrolled_to_show(10.0, 110.0, 50.0, 300.0) - 10.0).abs() < f32::EPSILON);
}
