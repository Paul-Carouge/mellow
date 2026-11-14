//! The rail's "+", the server header's menu and the dialogs of `server_dialogs`, headless.

use gpui::{Entity, TestAppContext, VisualTestContext};

use super::*;
use crate::components::MenuEntry;
use crate::i18n::Language;
use crate::shell::place_menus::PlaceMenuTarget;
use crate::shell_fixtures::{
    GUILD_RUST, RUST_GENERAL, RUST_TALK, open_shell, sample_state, sample_state_administering_rust,
};

fn labels_of(entries: Vec<MenuEntry>) -> Vec<String> {
    entries
        .into_iter()
        .filter_map(|entry| match entry {
            MenuEntry::Item(item) => Some(item.label.to_string()),
            MenuEntry::Separator => None,
        })
        .collect()
}

fn menu_labels(
    shell: &Entity<DuskShell>,
    target: PlaceMenuTarget,
    cx: &mut VisualTestContext,
) -> Vec<String> {
    shell.update(cx, |shell, cx| {
        shell
            .place_menu_entries(target, cx)
            .map(|(_, entries)| labels_of(entries))
            .unwrap_or_default()
    })
}

fn shown_kind(shell: &Entity<DuskShell>, cx: &mut VisualTestContext) -> Option<&'static str> {
    shell.read_with(cx, |shell, _| {
        shell
            .server_dialogs
            .shown
            .as_ref()
            .map(|shown| match shown.kind {
                OpenDialog::AddServer(_) => "add server",
                OpenDialog::AnotherInstance => "another instance",
                OpenDialog::InvitePeople => "invite people",
                OpenDialog::CreateChannel { .. } => "create channel",
                OpenDialog::DeleteChannel(_) => "delete channel",
                OpenDialog::MoveToCategory { .. } => "move to category",
                OpenDialog::RenameCategory { .. } => "rename category",
            })
    })
}

fn shown_dialog(shell: &Entity<DuskShell>, cx: &mut VisualTestContext) -> Entity<ModalDialog> {
    shell.read_with(cx, |shell, _| {
        shell
            .server_dialogs
            .shown
            .as_ref()
            .map(|shown| shown.dialog.clone())
            .expect("a dialog is shown")
    })
}

#[test]
fn a_template_sends_its_categories_in_the_interface_language() {
    let blank = initial_categories_of(Strings::new(Language::English), TEMPLATE_COUNT - 1);
    assert_eq!(
        blank,
        [InitialCategory {
            name: "Text channels".to_owned(),
            channels: vec![InitialTextChannel {
                name: "general".to_owned(),
                topic: String::new(),
            }],
        }]
    );
    assert_eq!(
        template_channel_names(Strings::new(Language::French), 0),
        "général, photos, sorties"
    );
}

#[gpui::test]
fn the_server_header_offers_inviting_and_creating_to_an_administrator(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state_administering_rust(), 1280.0);
    let header = menu_labels(&shell, PlaceMenuTarget::ServerHeader(GUILD_RUST), cx);
    assert_eq!(
        header.get(..4),
        Some(
            &[
                "Invite people".to_owned(),
                "Server settings".to_owned(),
                "Create channel".to_owned(),
                "Create category".to_owned(),
            ][..]
        ),
        "{header:?}"
    );
    // The strip's icon invites too, but creates nothing: that is the open server's header.
    let strip = menu_labels(&shell, PlaceMenuTarget::Server(GUILD_RUST), cx);
    assert_eq!(strip.first().map(String::as_str), Some("Invite people"));
    assert!(
        !strip.iter().any(|label| label == "Create channel"),
        "{strip:?}"
    );
}

#[gpui::test]
fn a_manager_moves_and_deletes_channels_from_their_menu(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state_administering_rust(), 1280.0);
    // "general" opens its category: it can go down, not up.
    let channel = menu_labels(&shell, PlaceMenuTarget::Channel(RUST_GENERAL), cx);
    for expected in ["Move down", "Move to category…", "Delete channel"] {
        assert!(channel.iter().any(|label| label == expected), "{channel:?}");
    }
    assert!(
        !channel.iter().any(|label| label == "Move up"),
        "{channel:?}"
    );
    let category = menu_labels(&shell, PlaceMenuTarget::Category(RUST_TALK), cx);
    for expected in ["Rename category", "Delete category"] {
        assert!(
            category.iter().any(|label| label == expected),
            "{category:?}"
        );
    }
}

#[gpui::test]
fn a_plain_member_cannot_move_or_delete_channels(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    let channel = menu_labels(&shell, PlaceMenuTarget::Channel(RUST_GENERAL), cx);
    for refused in [
        "Move up",
        "Move down",
        "Move to category…",
        "Delete channel",
    ] {
        assert!(!channel.iter().any(|label| label == refused), "{channel:?}");
    }
    let header = menu_labels(&shell, PlaceMenuTarget::ServerHeader(GUILD_RUST), cx);
    assert!(
        !header.iter().any(|label| label == "Create channel"),
        "{header:?}"
    );
}

#[gpui::test]
fn the_server_header_opens_its_menu_and_closes_it_when_pressed_again(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state_administering_rust(), 1280.0);
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_place(Some(GUILD_RUST), None, window, cx);
        });
    });
    cx.run_until_parked();
    let header = shell
        .read_with(cx, |shell, _| shell.place_menus.server_header_bounds.get())
        .expect("the header's place is recorded");
    cx.simulate_click(header.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(shell.read_with(cx, |shell, _| shell.is_server_header_menu_open()));
    cx.simulate_click(header.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert!(!shell.read_with(cx, |shell, _| shell.is_server_header_menu_open()));
}

#[gpui::test]
fn the_rails_plus_offers_creating_joining_and_another_instance(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| shell.toggle_add_server_menu(0, window, cx));
    });
    cx.run_until_parked();
    assert!(shell.read_with(cx, |shell, _| shell.is_add_server_menu_open()));
    // Pressed again, it closes.
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.note_add_server_press();
            shell.toggle_add_server_menu(0, window, cx);
        });
    });
    cx.run_until_parked();
    assert!(!shell.read_with(cx, |shell, _| shell.is_add_server_menu_open()));
    assert_eq!(
        connect_another_instance_label(Strings::new(Language::French)),
        "Se connecter à une autre instance"
    );
}

#[gpui::test]
fn create_and_join_are_two_pages_of_one_dialog(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state(), 1280.0);
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| shell.open_add_server(false, window, cx));
    });
    cx.run_until_parked();
    assert_eq!(shown_kind(&shell, cx), Some("add server"));
    // Nothing typed yet: no action.
    assert!(!shown_dialog(&shell, cx).read_with(cx, |dialog, _| dialog.is_submit_enabled()));
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| shell.switch_add_server_page(window, cx));
    });
    cx.run_until_parked();
    let is_joining = shell.read_with(cx, |shell, cx| match &shell.server_dialogs.shown {
        Some(ShownDialog {
            kind: OpenDialog::AddServer(body),
            ..
        }) => body.read(cx).is_joining,
        _ => false,
    });
    assert!(is_joining);
    // Cancel closes the dialog.
    let dialog = shown_dialog(&shell, cx);
    cx.update(|window, cx| dialog.update(cx, |dialog, cx| dialog.request_close(window, cx)));
    cx.run_until_parked();
    assert_eq!(shown_kind(&shell, cx), None);
}

#[gpui::test]
fn choosing_category_retitles_create_channel(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state_administering_rust(), 1280.0);
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.open_create_channel(GUILD_RUST, None, ChannelKind::Text, window, cx);
        });
    });
    cx.run_until_parked();
    let text_dialog = shown_dialog(&shell, cx);
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.choose_channel_kind(ChannelKind::Category, window, cx);
        });
    });
    cx.run_until_parked();
    // Drawn again, as the title changed; still one dialog.
    assert_ne!(shown_dialog(&shell, cx), text_dialog);
    assert_eq!(shown_kind(&shell, cx), Some("create channel"));
    let category_dialog = shown_dialog(&shell, cx);
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.choose_channel_kind(ChannelKind::Category, window, cx);
        });
    });
    cx.run_until_parked();
    // Category again changes nothing.
    assert_eq!(shown_dialog(&shell, cx), category_dialog);
}

#[gpui::test]
fn deleting_a_channel_waits_for_its_name(cx: &mut TestAppContext) {
    let (shell, cx) = open_shell(cx, sample_state_administering_rust(), 1280.0);
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.confirm_delete_channel(RUST_GENERAL, window, cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(shown_kind(&shell, cx), Some("delete channel"));
    assert!(!shown_dialog(&shell, cx).read_with(cx, |dialog, _| dialog.is_submit_enabled()));
    // A category loses nothing but its heading: Delete is offered at once.
    cx.update(|window, cx| {
        shell.update(cx, |shell, cx| {
            shell.confirm_delete_channel(RUST_TALK, window, cx);
        });
    });
    cx.run_until_parked();
    assert!(shown_dialog(&shell, cx).read_with(cx, |dialog, _| dialog.is_submit_enabled()));
}

#[gpui::test]
fn a_refused_move_says_why_under_the_header_and_lets_the_next_one_start(cx: &mut TestAppContext) {
    // The test session has no instance: the move is refused as a lost session would be.
    let (shell, cx) = open_shell(cx, sample_state_administering_rust(), 1280.0);
    shell.update(cx, |shell, cx| {
        shell.move_channel_step(RUST_GENERAL, Step::Down, cx);
    });
    cx.run_until_parked();
    shell.read_with(cx, |shell, _| {
        assert!(shell.server_dialogs.reorder_error.is_some());
        assert!(!shell.server_dialogs.is_reordering);
    });
}
