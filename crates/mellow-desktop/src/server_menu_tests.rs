//! Drives the real window with pointer and key events: the server menu of the channel list.

use crate::generated_ui::{AppWindow, ShellDialog};
use i_slint_backend_testing::ElementHandle;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize};
use std::cell::Cell;
use std::rc::Rc;

const WIDE_WINDOW: LogicalSize = LogicalSize::new(1440.0, 900.0);
// `sidebar-header-menu-margin` in design/tokens.json.
const MENU_GAP_BELOW_HEADER: f32 = 8.0;
const SERVER_NAME: &str = "Lounge";

fn signed_in_window_with_every_menu_item() -> AppWindow {
    i_slint_backend_testing::init_no_event_loop();
    let window = AppWindow::new().expect("the window builds with the testing backend");
    window.window().set_size(WIDE_WINDOW);
    window.set_is_signed_in(true);
    window.set_server_name(SERVER_NAME.into());
    window.set_can_invite_people(true);
    window.set_can_open_server_settings(true);
    window.set_can_create_channels(true);
    window.set_can_leave_server(true);
    window.show().expect("the window shows");
    window
}

fn center_of(window: &AppWindow, accessible_label: &str) -> LogicalPosition {
    let element = ElementHandle::find_by_accessible_label(window, accessible_label)
        .next()
        .unwrap_or_else(|| panic!("no element labelled {accessible_label:?}"));
    let position = element.absolute_position();
    let size = element.size();
    LogicalPosition::new(
        position.x + size.width / 2.0,
        position.y + size.height / 2.0,
    )
}

/// Where an item of the open server menu is on screen. The menu opens under the header, as in the
/// mockup (`design/mockups/dusk-direction.html`), but the testing API reports popup items relative
/// to the popup itself, so the popup's origin is derived from the header.
fn center_of_menu_item(window: &AppWindow, accessible_label: &str) -> LogicalPosition {
    let header = ElementHandle::find_by_accessible_label(window, SERVER_NAME)
        .next()
        .expect("the server header exists");
    let item = center_of(window, accessible_label);
    LogicalPosition::new(
        header.absolute_position().x + item.x,
        header.absolute_position().y + header.size().height + MENU_GAP_BELOW_HEADER + item.y,
    )
}

fn click_at(window: &AppWindow, position: LogicalPosition) {
    let button = PointerEventButton::Left;
    let slint_window = window.window();
    slint_window.dispatch_event(WindowEvent::PointerMoved { position });
    slint_window.dispatch_event(WindowEvent::PointerPressed { position, button });
    slint_window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

fn open_server_menu(window: &AppWindow) {
    click_at(window, center_of(window, SERVER_NAME));
}

fn count_calls(register: impl FnOnce(Rc<Cell<u32>>)) -> Rc<Cell<u32>> {
    let calls = Rc::new(Cell::new(0));
    register(calls.clone());
    calls
}

#[test]
fn clicking_invite_people_in_the_server_menu_opens_the_invite_dialog() {
    let window = signed_in_window_with_every_menu_item();
    let invite_creations =
        count_calls(|calls| window.on_create_invite(move || calls.set(calls.get() + 1)));

    open_server_menu(&window);
    click_at(&window, center_of_menu_item(&window, "Invite people"));

    assert_eq!(window.get_open_dialog(), ShellDialog::InvitePeople);
    assert_eq!(invite_creations.get(), 1);
}

#[test]
fn clicking_server_settings_in_the_server_menu_asks_to_open_them() {
    let window = signed_in_window_with_every_menu_item();
    let requests =
        count_calls(|calls| window.on_open_server_settings(move || calls.set(calls.get() + 1)));

    open_server_menu(&window);
    click_at(&window, center_of_menu_item(&window, "Server settings"));

    assert_eq!(requests.get(), 1);
}

#[test]
fn clicking_create_channel_in_the_server_menu_opens_the_dialog() {
    let window = signed_in_window_with_every_menu_item();

    open_server_menu(&window);
    click_at(&window, center_of_menu_item(&window, "Create channel"));

    assert_eq!(window.get_open_dialog(), ShellDialog::CreateChannel);
}

#[test]
fn clicking_leave_server_in_the_server_menu_opens_the_confirmation() {
    let window = signed_in_window_with_every_menu_item();

    open_server_menu(&window);
    click_at(&window, center_of_menu_item(&window, "Leave server"));

    assert_eq!(window.get_open_dialog(), ShellDialog::LeaveServer);
}

// Tab first: whether the menu's first item takes focus by itself on opening could not be checked with
// the testing backend (Enter right after opening did nothing there); this covers the activation.
#[test]
fn pressing_enter_on_a_focused_server_menu_item_opens_the_invite_dialog() {
    let window = signed_in_window_with_every_menu_item();

    open_server_menu(&window);
    for key in [Key::Tab, Key::Return] {
        window
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
        window
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
    }

    assert_eq!(window.get_open_dialog(), ShellDialog::InvitePeople);
}
