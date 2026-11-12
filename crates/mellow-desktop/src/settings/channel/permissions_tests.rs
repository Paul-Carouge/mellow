//! The channel Permissions page headless: what it offers, unsaved edits, their rebase on the
//! gateway's changes and the end of an edit by its echo.

use gpui::{AppContext as _, TestAppContext, VisualTestContext, WindowAppearance};
use mellow_client_core::state::ClientState;
use mellow_protocol::{GatewayEvent, PermissionOverwrite, Role};

use super::*;
use crate::shell_fixtures::{
    GUILD_RUST, MODERATORS, RUST_GENERAL, ZOE, sample_state_administering_rust,
};

/// The sample world where Rust Lyon has its `@everyone` role and the reader administers it.
fn administered_state() -> ClientState {
    let mut state = sample_state_administering_rust();
    let mut guild = state
        .guild(GUILD_RUST)
        .cloned()
        .expect("the sample world has Rust Lyon");
    guild.roles.insert(
        0,
        Role {
            id: GUILD_RUST,
            name: "@everyone".to_owned(),
            color: 0,
            position: 0,
            permissions: Permissions::VIEW_CHANNEL | Permissions::SEND_MESSAGES,
            is_displayed_separately: false,
            is_mentionable: false,
            icon: None,
            unicode_emoji: None,
        },
    );
    state.apply_event(GatewayEvent::GuildCreate(Box::new(guild)), 0);
    state
}

fn open_page(
    cx: &mut TestAppContext,
    state: ClientState,
) -> (
    Entity<Session>,
    Entity<ChannelPermissionsPage>,
    &mut VisualTestContext,
) {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.set_global(crate::strings_pending_i18n::ActiveStrings(
            &crate::strings_pending_i18n::ENGLISH,
        ));
        cx.set_global(crate::theme::ActiveTheme::for_appearance(
            WindowAppearance::Dark,
        ));
    });
    let session = cx.new(|_| Session::signed_out_kept_in_memory_only());
    session.update(cx, |session, _| *session.state_mut() = state);
    let page_session = session.clone();
    let (page, cx) = cx.add_window_view(|window, cx| {
        ChannelPermissionsPage::new(page_session, RUST_GENERAL, window, cx)
    });
    cx.run_until_parked();
    (session, page, cx)
}

/// Replaces the overwrites of #general as the gateway would.
fn set_overwrites(
    session: &Entity<Session>,
    overwrites: Vec<PermissionOverwrite>,
    cx: &mut VisualTestContext,
) {
    session.update(cx, |session, cx| {
        let mut channel = session
            .state()
            .guild_channel(RUST_GENERAL)
            .cloned()
            .expect("the sample world has #general");
        channel.permission_overwrites = overwrites;
        session
            .state_mut()
            .apply_event(GatewayEvent::ChannelUpdate(channel), 0);
        cx.notify();
    });
    cx.run_until_parked();
}

fn overwrite(
    id: Snowflake,
    kind: OverwriteKind,
    allow: Permissions,
    deny: Permissions,
) -> PermissionOverwrite {
    PermissionOverwrite {
        id,
        kind,
        allow,
        deny,
    }
}

fn editor(page: &Entity<ChannelPermissionsPage>, cx: &mut VisualTestContext) -> EditorView {
    page.read_with(cx, ChannelPermissionsPage::view)
        .and_then(|view| view.editor)
        .expect("a target is shown")
}

fn state_of(editor: &EditorView, bit: Permissions) -> OverwriteState {
    editor
        .groups
        .iter()
        .flat_map(|group| &group.permissions)
        .find(|permission| permission.bit == bit)
        .map(|permission| permission.current)
        .expect("the permission is in the channel's editor")
}

fn has_changes(page: &Entity<ChannelPermissionsPage>, cx: &mut VisualTestContext) -> bool {
    page.read_with(cx, ChannelPermissionsPage::has_changes)
}

#[gpui::test]
fn everyone_is_shown_and_a_changed_state_waits_for_the_echo(cx: &mut TestAppContext) {
    let (session, page, cx) = open_page(cx, administered_state());
    let shown = editor(&page, cx);
    assert_eq!(shown.row.target.id, GUILD_RUST);
    assert!(shown.is_everyone && shown.is_editable && !shown.can_remove);
    // A text channel's editor leaves out the voice permissions.
    assert!(
        shown
            .groups
            .iter()
            .flat_map(|group| &group.permissions)
            .all(|permission| permission.bit != Permissions::SPEAK)
    );

    page.update(cx, |page, cx| {
        page.change(Permissions::SEND_MESSAGES, OverwriteState::Deny, cx);
    });
    assert!(has_changes(&page, cx));
    assert_eq!(
        state_of(&editor(&page, cx), Permissions::SEND_MESSAGES),
        OverwriteState::Deny
    );

    // The saved overwrite comes back through the gateway: nothing is left to save.
    set_overwrites(
        &session,
        vec![overwrite(
            GUILD_RUST,
            OverwriteKind::Role,
            Permissions::empty(),
            Permissions::SEND_MESSAGES,
        )],
        cx,
    );
    assert!(!has_changes(&page, cx));
    assert!(page.read_with(cx, |page, _| page.edit.is_none()));
}

#[gpui::test]
fn a_change_made_meanwhile_by_someone_else_is_kept_under_the_edit(cx: &mut TestAppContext) {
    let (session, page, cx) = open_page(cx, administered_state());
    page.update(cx, |page, cx| {
        page.change(Permissions::SEND_MESSAGES, OverwriteState::Deny, cx);
    });
    set_overwrites(
        &session,
        vec![overwrite(
            GUILD_RUST,
            OverwriteKind::Role,
            Permissions::ADD_REACTIONS,
            Permissions::empty(),
        )],
        cx,
    );
    assert!(has_changes(&page, cx));
    let shown = editor(&page, cx);
    assert_eq!(
        state_of(&shown, Permissions::SEND_MESSAGES),
        OverwriteState::Deny
    );
    assert_eq!(
        state_of(&shown, Permissions::ADD_REACTIONS),
        OverwriteState::Allow
    );
    page.update(cx, ChannelPermissionsPage::reset);
    assert!(!has_changes(&page, cx));
}

#[gpui::test]
fn another_target_waits_while_an_edit_is_unsaved(cx: &mut TestAppContext) {
    let (session, page, cx) = open_page(cx, administered_state());
    set_overwrites(
        &session,
        vec![overwrite(
            ZOE,
            OverwriteKind::Member,
            Permissions::empty(),
            Permissions::SEND_MESSAGES,
        )],
        cx,
    );
    page.update(cx, |page, cx| {
        page.change(Permissions::ADD_REACTIONS, OverwriteState::Allow, cx);
        page.select(ZOE, cx);
    });
    assert_eq!(editor(&page, cx).row.target.id, GUILD_RUST);
    assert_eq!(page.read_with(cx, |page, _| page.local_attention), 1);

    page.update(cx, |page, cx| {
        page.reset(cx);
        page.select(ZOE, cx);
    });
    let shown = editor(&page, cx);
    assert_eq!(shown.row.target.id, ZOE);
    assert!(shown.can_remove, "a member's overwrite can be removed");
    assert_eq!(
        state_of(&shown, Permissions::SEND_MESSAGES),
        OverwriteState::Deny
    );
}

#[gpui::test]
fn a_role_at_or_above_the_readers_highest_is_locked(cx: &mut TestAppContext) {
    let (session, page, cx) = open_page(cx, administered_state());
    set_overwrites(
        &session,
        vec![overwrite(
            MODERATORS,
            OverwriteKind::Role,
            Permissions::SEND_MESSAGES,
            Permissions::empty(),
        )],
        cx,
    );
    page.update(cx, |page, cx| page.select(MODERATORS, cx));
    let shown = editor(&page, cx);
    assert_eq!(shown.row.target.id, MODERATORS);
    assert!(shown.row.is_locked && !shown.is_editable && !shown.can_remove);
    page.update(cx, |page, cx| {
        page.change(Permissions::SEND_MESSAGES, OverwriteState::Deny, cx);
    });
    assert!(!has_changes(&page, cx), "a locked target takes no edit");
}

#[gpui::test]
fn an_added_target_is_waited_for_then_shown(cx: &mut TestAppContext) {
    let (session, page, cx) = open_page(cx, administered_state());
    page.update(cx, |page, cx| {
        page.selected_target_id = Some(ZOE);
        page.added_target_id = Some(ZOE);
        cx.notify();
    });
    assert!(
        page.read_with(cx, ChannelPermissionsPage::view)
            .is_some_and(|view| view.editor.is_none()),
        "nothing else is edited while the added target is on its way"
    );
    set_overwrites(
        &session,
        vec![overwrite(
            ZOE,
            OverwriteKind::Member,
            Permissions::empty(),
            Permissions::empty(),
        )],
        cx,
    );
    assert_eq!(editor(&page, cx).row.target.id, ZOE);
    assert!(page.read_with(cx, |page, _| page.added_target_id.is_none()));
}

#[gpui::test]
fn permissions_are_offered_to_a_reader_who_manages_roles(cx: &mut TestAppContext) {
    let (session, _page, cx) = open_page(cx, administered_state());
    let offered = session.read_with(cx, |session, _| {
        super::super::offered_sections(session.state(), RUST_GENERAL)
    });
    assert!(offered.contains(
        &mellow_client_core::shared_logic::channel_settings::ChannelSettingsSection::Permissions
    ));
}

#[gpui::test]
fn the_add_dialog_offers_members_and_roles_the_reader_may_target(cx: &mut TestAppContext) {
    let (_session, page, cx) = open_page(cx, administered_state());
    cx.update(|window, cx| {
        page.update(cx, |page, cx| page.open_add_dialog(window, cx));
    });
    cx.run_until_parked();
    let dialog = page
        .read_with(cx, |page, _| page.add_dialog.clone())
        .expect("the dialog opens");
    let candidates = dialog.read_with(cx, AddOverwriteTarget::candidates);
    assert!(
        candidates.roles.is_empty(),
        "Moderators sits above the reader's highest role"
    );
    let super::super::add_overwrite_target::MemberCandidates::Ready(members) = candidates.members
    else {
        panic!("the sample world has its members");
    };
    let member_ids: Vec<Snowflake> = members
        .iter()
        .map(|candidate| candidate.target.id)
        .collect();
    assert!(member_ids.contains(&ZOE));

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    assert!(page.read_with(cx, |page, _| page.add_dialog.is_none()));
}

#[gpui::test]
fn a_change_made_while_saving_is_still_to_save(cx: &mut TestAppContext) {
    let (_session, page, cx) = open_page(cx, administered_state());
    page.update(cx, |page, cx| {
        page.change(Permissions::SEND_MESSAGES, OverwriteState::Deny, cx);
    });
    let sent = page
        .read_with(cx, ChannelPermissionsPage::selected_draft)
        .map(|(_, draft)| draft)
        .expect("an edit is shown");
    page.update(cx, |page, cx| {
        page.busy = Some(Busy::Save);
        page.change(Permissions::ADD_REACTIONS, OverwriteState::Allow, cx);
        page.finish_save(sent, Ok(()), cx);
    });
    assert!(
        has_changes(&page, cx),
        "the change made meanwhile was never sent"
    );

    // Saved with nothing changed meanwhile: the bar waits for the echo.
    let sent = page
        .read_with(cx, ChannelPermissionsPage::selected_draft)
        .map(|(_, draft)| draft)
        .expect("an edit is shown");
    page.update(cx, |page, cx| page.finish_save(sent, Ok(()), cx));
    assert!(!has_changes(&page, cx));
}

#[gpui::test]
fn an_edit_the_reader_lost_the_right_to_make_is_dropped(cx: &mut TestAppContext) {
    let (session, page, cx) = open_page(cx, administered_state());
    page.update(cx, |page, cx| {
        page.change(Permissions::SEND_MESSAGES, OverwriteState::Deny, cx);
    });
    assert!(has_changes(&page, cx));
    session.update(cx, |session, cx| {
        let mut guild = session
            .state()
            .guild(GUILD_RUST)
            .cloned()
            .expect("the sample world has Rust Lyon");
        guild.my_permissions = Permissions::VIEW_CHANNEL | Permissions::MANAGE_ROLES;
        session
            .state_mut()
            .apply_event(GatewayEvent::GuildCreate(Box::new(guild)), 0);
        cx.notify();
    });
    cx.run_until_parked();
    assert!(!has_changes(&page, cx), "the server would refuse it");
    let shown = editor(&page, cx);
    let send_messages = shown
        .groups
        .iter()
        .flat_map(|group| &group.permissions)
        .find(|permission| permission.bit == Permissions::SEND_MESSAGES)
        .expect("a text channel offers Send Messages");
    assert!(send_messages.is_locked && shown.is_editable);
    page.update(cx, |page, cx| {
        page.change(Permissions::SEND_MESSAGES, OverwriteState::Deny, cx);
    });
    assert!(!has_changes(&page, cx), "a locked permission takes no edit");
}

#[gpui::test]
fn an_added_target_already_brought_by_the_gateway_is_not_waited_for(cx: &mut TestAppContext) {
    let (session, page, cx) = open_page(cx, administered_state());
    cx.update(|window, cx| {
        page.update(cx, |page, cx| page.open_add_dialog(window, cx));
    });
    cx.run_until_parked();
    let dialog = page
        .read_with(cx, |page, _| page.add_dialog.clone())
        .expect("the dialog opens");
    let zoe = OverwriteTarget {
        kind: OverwriteKind::Member,
        id: ZOE,
    };
    let zoe_overwrite = overwrite(
        ZOE,
        OverwriteKind::Member,
        Permissions::empty(),
        Permissions::empty(),
    );
    // The echo comes before the REST answer.
    set_overwrites(&session, vec![zoe_overwrite], cx);
    dialog.update(cx, |_, cx| cx.emit(AddOverwriteTargetEvent::Added(zoe)));
    cx.run_until_parked();
    assert!(page.read_with(cx, |page, _| page.added_target_id.is_none()
        && page.add_dialog.is_none()));
    assert_eq!(editor(&page, cx).row.target.id, ZOE);

    // Someone else removes it: `@everyone` takes its place.
    set_overwrites(&session, Vec::new(), cx);
    assert_eq!(editor(&page, cx).row.target.id, GUILD_RUST);
}

#[gpui::test]
fn arrows_choose_a_state_from_the_keyboard(cx: &mut TestAppContext) {
    let (_session, page, cx) = open_page(cx, administered_state());
    let first = editor(&page, cx)
        .groups
        .first()
        .and_then(|group| group.permissions.first())
        .map(|permission| (permission.bit, permission.current))
        .expect("the editor has permissions");
    assert_eq!(first.1, OverwriteState::Inherit);
    // Tab through the add button and the targets to the first permission's chosen state.
    let mut tabs = 0;
    while page.read_with(cx, |page, _| page.edit.is_none()) {
        assert!(tabs < 10, "a state takes focus from the keyboard");
        cx.update(Window::focus_next);
        cx.run_until_parked();
        cx.simulate_keystrokes("right");
        tabs += 1;
    }
    assert_eq!(
        state_of(&editor(&page, cx), first.0),
        OverwriteState::Allow,
        "right from Inherit chooses Allow"
    );
    cx.simulate_keystrokes("right");
    assert_eq!(
        state_of(&editor(&page, cx), first.0),
        OverwriteState::Deny,
        "the arrows wrap around"
    );
}
