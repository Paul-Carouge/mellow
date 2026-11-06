//! Headless keyboard behaviour of the primitives: what a keyboard-only person relies on.

use gpui::{
    AppContext as _, Context, DismissEvent, Entity, FocusHandle, InputEvent as _,
    InteractiveElement as _, IntoElement, KeyUpEvent, Keystroke, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, TestAppContext, VisualTestContext, Window,
    WindowAppearance, div,
};

use super::{
    Button, ContextMenu, MenuAlignment, MenuAnchor, MenuEntry, MenuItem, MenuOrigin, ModalDialog,
    ModalDialogEvent, Switch, Tabs, bind_tab_navigation, with_tab_navigation,
};
use crate::theme::ActiveTheme;
use crate::theme::icons::IconName;

fn set_up(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark));
        bind_tab_navigation(cx);
    });
}

/// The window takes key focus with its root focused, as the app's windows do when they open.
fn activate(cx: &mut VisualTestContext, root: &FocusHandle) {
    cx.update(|window, cx| {
        window.activate_window();
        window.focus(root, cx);
    });
    cx.run_until_parked();
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

struct SwitchHarness {
    root: FocusHandle,
    is_on: bool,
    is_locked: bool,
}

impl Render for SwitchHarness {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        with_tab_navigation(div().size_full(), &self.root).child(
            Switch::new("switch", self.is_on, "Notifications")
                .locked(self.is_locked)
                .on_change(move |is_on, _, cx| {
                    entity.update(cx, |harness, cx| {
                        harness.is_on = is_on;
                        cx.notify();
                    });
                }),
        )
    }
}

#[gpui::test]
fn space_and_enter_flip_a_switch_and_a_locked_one_stays(cx: &mut TestAppContext) {
    set_up(cx);
    let (harness, cx) = cx.add_window_view(|_, cx| SwitchHarness {
        root: cx.focus_handle(),
        is_on: true,
        is_locked: false,
    });
    let root = harness.read_with(cx, |harness, _| harness.root.clone());
    activate(cx, &root);
    press(cx, "tab");
    press(cx, "space");
    assert!(!harness.read_with(cx, |harness, _| harness.is_on));
    press(cx, "enter");
    assert!(harness.read_with(cx, |harness, _| harness.is_on));

    harness.update(cx, |harness, cx| {
        harness.is_locked = true;
        cx.notify();
    });
    cx.run_until_parked();
    press(cx, "space");
    assert!(
        harness.read_with(cx, |harness, _| harness.is_on),
        "a locked switch keeps its state"
    );
}

struct TabsHarness {
    root: FocusHandle,
    selected_index: usize,
}

impl Render for TabsHarness {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let entity = cx.entity();
        with_tab_navigation(div().size_full(), &self.root).child(
            Tabs::new(
                "tabs",
                "Friends",
                vec!["Online".into(), "All".into(), "Pending".into()],
                self.selected_index,
            )
            .on_select(move |index, _, cx| {
                entity.update(cx, |harness, cx| {
                    harness.selected_index = index;
                    cx.notify();
                });
            }),
        )
    }
}

#[gpui::test]
fn arrows_choose_tabs_wrapping_and_home_end_jump(cx: &mut TestAppContext) {
    set_up(cx);
    let (harness, cx) = cx.add_window_view(|_, cx| TabsHarness {
        root: cx.focus_handle(),
        selected_index: 0,
    });
    let root = harness.read_with(cx, |harness, _| harness.root.clone());
    activate(cx, &root);
    let selected = |cx: &mut VisualTestContext| harness.read_with(cx, |h, _| h.selected_index);

    press(cx, "tab");
    press(cx, "right");
    assert_eq!(selected(cx), 1);
    press(cx, "right");
    press(cx, "right");
    assert_eq!(selected(cx), 0, "Right wraps to the first tab");
    press(cx, "left");
    assert_eq!(selected(cx), 2, "Left wraps to the last tab");
    press(cx, "home");
    assert_eq!(selected(cx), 0);
    press(cx, "end");
    assert_eq!(selected(cx), 2);
}

struct MenuHarness {
    root: FocusHandle,
    opener: FocusHandle,
    menu: Option<(Entity<ContextMenu>, Subscription)>,
    chosen: Vec<SharedString>,
}

impl MenuHarness {
    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let entity = cx.entity();
        let entries = ["Copier", "Épingler", "Éditer", "Supprimer"]
            .into_iter()
            .map(|label| {
                let entity = entity.clone();
                MenuEntry::Item(MenuItem::new(label, IconName::Copy, move |_, cx| {
                    entity.update(cx, |harness, _| harness.chosen.push(label.into()));
                }))
            })
            .collect();
        let origin = MenuOrigin::Anchor {
            bounds: MenuAnchor {
                top: 10.0,
                bottom: 40.0,
                left: 10.0,
                right: 100.0,
            },
            alignment: MenuAlignment::Start,
        };
        let opener = Some(self.opener.clone());
        let menu = cx.new(|cx| ContextMenu::open("Message", entries, origin, opener, window, cx));
        let subscription = cx.subscribe(&menu, |harness, _, _: &DismissEvent, cx| {
            harness.menu = None;
            cx.notify();
        });
        self.menu = Some((menu, subscription));
        cx.notify();
    }

    fn highlighted(&self, cx: &gpui::App) -> Option<usize> {
        self.menu
            .as_ref()
            .and_then(|(menu, _)| menu.read(cx).highlighted_index())
    }
}

impl Render for MenuHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        with_tab_navigation(div().size_full(), &self.root)
            .child(div().id("opener").track_focus(&self.opener).size_10())
            .children(self.menu.iter().map(|(menu, _)| menu.clone()))
    }
}

#[gpui::test]
fn a_menu_wraps_jumps_types_ahead_chooses_and_gives_focus_back(cx: &mut TestAppContext) {
    set_up(cx);
    let (harness, cx) = cx.add_window_view(|_, cx| MenuHarness {
        root: cx.focus_handle(),
        opener: cx.focus_handle(),
        menu: None,
        chosen: Vec::new(),
    });
    let opener = harness.read_with(cx, |harness, _| harness.opener.clone());
    cx.update(|window, cx| window.focus(&opener, cx));
    cx.update(|window, cx| harness.update(cx, |harness, cx| harness.open(window, cx)));
    cx.run_until_parked();
    let highlighted = |cx: &mut VisualTestContext| harness.read_with(cx, MenuHarness::highlighted);

    assert_eq!(
        highlighted(cx),
        Some(0),
        "opened from a button: the first item"
    );
    press(cx, "up");
    assert_eq!(highlighted(cx), Some(3), "Up wraps to the last item");
    press(cx, "down");
    assert_eq!(highlighted(cx), Some(0), "Down wraps to the first item");
    press(cx, "end");
    assert_eq!(highlighted(cx), Some(3));
    press(cx, "home");
    assert_eq!(highlighted(cx), Some(0));
    press(cx, "e");
    assert_eq!(highlighted(cx), Some(1), "type-ahead ignores the accent");
    press(cx, "e");
    assert_eq!(highlighted(cx), Some(2), "typing it again cycles");

    press(cx, "enter");
    harness.read_with(cx, |harness, _| {
        assert!(harness.menu.is_none(), "choosing closes the menu");
        assert_eq!(harness.chosen, vec![SharedString::from("Éditer")]);
    });
    assert!(cx.update(|window, _| opener.is_focused(window)));

    cx.update(|window, cx| harness.update(cx, |harness, cx| harness.open(window, cx)));
    cx.run_until_parked();
    press(cx, "escape");
    harness.read_with(cx, |harness, _| {
        assert!(harness.menu.is_none(), "Esc closes the menu");
        assert_eq!(harness.chosen.len(), 1, "without choosing");
    });
    assert!(cx.update(|window, _| opener.is_focused(window)));
}

struct DialogHarness {
    root: FocusHandle,
    opener: FocusHandle,
    dialog: Option<(Entity<ModalDialog>, Subscription)>,
    submissions: usize,
}

impl DialogHarness {
    fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dialog = cx.new(|cx| {
            ModalDialog::new("Delete #general?", "Cancel", window, cx).submit(
                "Delete channel",
                "Deleting…",
                true,
            )
        });
        let subscription =
            cx.subscribe(
                &dialog,
                |harness, _, event: &ModalDialogEvent, cx| match event {
                    ModalDialogEvent::Closed => {
                        harness.dialog = None;
                        cx.notify();
                    }
                    ModalDialogEvent::Submitted => harness.submissions += 1,
                },
            );
        self.dialog = Some((dialog, subscription));
        cx.notify();
    }
}

impl Render for DialogHarness {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        with_tab_navigation(div().size_full(), &self.root)
            .child(Button::new("opener", "Delete").track_focus(&self.opener))
            .child(Button::new("outside", "Outside"))
            .children(self.dialog.iter().map(|(dialog, _)| dialog.clone()))
    }
}

#[gpui::test]
fn a_dialog_traps_tab_closes_on_esc_unless_busy_and_gives_focus_back(cx: &mut TestAppContext) {
    set_up(cx);
    let (harness, cx) = cx.add_window_view(|_, cx| DialogHarness {
        root: cx.focus_handle(),
        opener: cx.focus_handle(),
        dialog: None,
        submissions: 0,
    });
    cx.run_until_parked();
    let opener = harness.read_with(cx, |harness, _| harness.opener.clone());
    cx.update(|window, cx| window.focus(&opener, cx));
    cx.update(|window, cx| harness.update(cx, |harness, cx| harness.open(window, cx)));
    cx.run_until_parked();
    let dialog = harness.read_with(cx, |harness, _| {
        harness.dialog.as_ref().map(|(dialog, _)| dialog.clone())
    });
    let Some(dialog) = dialog else {
        panic!("the dialog opened");
    };
    let is_inside = |cx: &mut VisualTestContext| {
        let container = dialog.read_with(cx, gpui::Focusable::focus_handle);
        cx.update(|window, cx| container.contains_focused(window, cx))
    };

    assert!(is_inside(cx), "a destructive dialog starts on Cancel");
    for _ in 0..5 {
        press(cx, "tab");
        assert!(is_inside(cx), "Tab never leaves the dialog");
    }
    for _ in 0..5 {
        press(cx, "shift-tab");
        assert!(is_inside(cx), "Shift+Tab never leaves the dialog");
    }

    dialog.update(cx, |dialog, cx| dialog.set_busy(true, cx));
    press(cx, "escape");
    assert!(
        harness.read_with(cx, |harness, _| harness.dialog.is_some()),
        "a busy dialog stays open"
    );
    dialog.update(cx, |dialog, cx| dialog.set_busy(false, cx));
    press(cx, "escape");
    assert!(harness.read_with(cx, |harness, _| harness.dialog.is_none()));
    assert!(
        cx.update(|window, _| opener.is_focused(window)),
        "focus is back on the opener"
    );
}

#[gpui::test]
fn enter_on_the_dialogs_action_submits_it_once(cx: &mut TestAppContext) {
    set_up(cx);
    let (harness, cx) = cx.add_window_view(|_, cx| DialogHarness {
        root: cx.focus_handle(),
        opener: cx.focus_handle(),
        dialog: None,
        submissions: 0,
    });
    cx.run_until_parked();
    cx.update(|window, cx| harness.update(cx, |harness, cx| harness.open(window, cx)));
    cx.run_until_parked();
    // Cancel holds focus first; Tab reaches the action.
    press(cx, "tab");
    press(cx, "enter");
    assert_eq!(harness.read_with(cx, |harness, _| harness.submissions), 1);
}
