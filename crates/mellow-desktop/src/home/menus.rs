//! A person's and a group conversation's context menus (`PersonContextMenu.svelte`, the group
//! rows of `ChannelSidebar.svelte`, DESIGN.md §6.33): their items come from client-core's
//! `person_menu_actions` and `group_menu_actions`, read as `menuItemLook` reads them. Home keeps the
//! open menu and draws it over the window, so a member row, a message's author, a friend row and a
//! conversation row all open the same one. A change that fails is said in the shell's notice bar;
//! one that works is read out.
//!
//! shortcut: "Notification settings…" is left out and "Mute" mutes until turned back on, without
//! the web's durations, until the desktop has the per-place notification dialog (step 6).
//! shortcut: no moderation entries (the web's menu has none either); they come with their dialogs
//! (step 6).

use gpui::{App, AppContext as _, ClipboardItem, Context, FocusHandle, Window};
use mellow_client_core::ClientError;
use mellow_client_core::shared_logic::context_menus_people::{
    GroupMenuAction, MenuLine, PersonMenuAction, PersonMenuFacts, group_menu_actions,
    person_menu_actions,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_client_core::shared_logic::notification_settings::{
    MuteEnd, PlaceNotificationSettings, is_mute_active, with_place_settings,
};
use mellow_protocol::{
    PublicUser, RelationshipKind, SendFriendRequest, SetRelationshipKind, Snowflake,
};

use super::friends_pane::Confirmed;
use super::{Home, HomeEvent, conversation_display_name};
use crate::components::{ContextMenu, MenuEntry, MenuItem, MenuItemTone, MenuOrigin};
use crate::i18n::{Strings, strings};
use crate::notifier::notification_settings_of;
use crate::preferences::change_preferences;
use crate::session::now_unix_milliseconds;
use crate::shell::OpenDirectConversation;
use crate::sign_in_form::describe_error;
use crate::theme::icons::IconName;

/// Who a person's menu is about, and where it was opened.
#[derive(Debug, Clone, PartialEq)]
pub struct PersonMenuRequest {
    pub user: PublicUser,
    pub origin: MenuOrigin,
    /// Gets focus back when the menu closes; `None`: whatever has focus when it opens.
    pub opener: Option<FocusHandle>,
    /// A one-to-one conversation's row: Message is pointless there, its ID is copied and it mutes.
    pub conversation_id: Option<Snowflake>,
    /// A member row of this server's side sheet, which shows profiles.
    pub profile_guild_id: Option<Snowflake>,
}

impl PersonMenuRequest {
    /// A menu about `user` opened at `origin`, from anywhere but a conversation or member row.
    #[must_use]
    pub fn new(user: PublicUser, origin: MenuOrigin) -> Self {
        Self {
            user,
            origin,
            opener: None,
            conversation_id: None,
            profile_guild_id: None,
        }
    }
}

/// Opens a person's menu from a view that cannot reach Home (the message list): the shell hands
/// it on.
#[derive(Debug, Clone, PartialEq, gpui::Action)]
#[action(namespace = mellow_home, no_json)]
pub struct OpenPersonMenu {
    pub request: PersonMenuRequest,
}

/// How a person's menu item reads (`menuItemLook`).
fn person_item_look(
    action: PersonMenuAction,
    catalogue: Strings,
) -> (&'static str, IconName, MenuItemTone) {
    use MenuItemTone::{Danger, Normal};
    use PersonMenuAction as Action;
    match action {
        Action::Profile => (catalogue.members().profile(), IconName::At, Normal),
        Action::Message => (
            catalogue.context_menu().message(),
            IconName::ChatCircle,
            Normal,
        ),
        Action::AddFriend => (catalogue.members().add_friend(), IconName::UserPlus, Normal),
        Action::AcceptRequest => (
            catalogue.members().accept_request(),
            IconName::Check,
            Normal,
        ),
        Action::IgnoreRequest => (
            catalogue.context_menu().ignore_request(),
            IconName::X,
            Normal,
        ),
        Action::CancelRequest => (
            catalogue.context_menu().cancel_request(),
            IconName::X,
            Normal,
        ),
        Action::Unblock => (catalogue.friends().unblock(), IconName::Prohibit, Normal),
        // No dialog follows on the desktop yet, so no ellipsis.
        Action::Mute => (
            catalogue.notifications().mute_legend(),
            IconName::BellSlash,
            Normal,
        ),
        Action::Unmute => (catalogue.notifications().unmute(), IconName::Bell, Normal),
        Action::NotificationSettings => (
            catalogue.notifications().settings_menu(),
            IconName::Bell,
            Normal,
        ),
        Action::CopyUserId => (catalogue.user_card().copy_user_id(), IconName::Copy, Normal),
        Action::CopyChannelId => (
            catalogue.context_menu().copy_channel_id(),
            IconName::Copy,
            Normal,
        ),
        Action::RemoveFriend => (
            catalogue.friends().remove_friend(),
            IconName::UserMinus,
            Danger,
        ),
        Action::Block => (catalogue.friends().block(), IconName::Prohibit, Danger),
    }
}

fn group_item_look(
    action: GroupMenuAction,
    catalogue: Strings,
) -> (&'static str, IconName, MenuItemTone) {
    use GroupMenuAction as Action;
    use MenuItemTone::{Danger, Normal};
    match action {
        Action::RenameGroup => (catalogue.groups().rename(), IconName::PencilSimple, Normal),
        Action::Mute => (
            catalogue.notifications().mute_legend(),
            IconName::BellSlash,
            Normal,
        ),
        Action::Unmute => (catalogue.notifications().unmute(), IconName::Bell, Normal),
        Action::NotificationSettings => (
            catalogue.notifications().settings_menu(),
            IconName::Bell,
            Normal,
        ),
        Action::CopyChannelId => (
            catalogue.context_menu().copy_channel_id(),
            IconName::Copy,
            Normal,
        ),
        Action::LeaveGroup => (catalogue.groups().leave(), IconName::SignOut, Danger),
    }
}

/// The menu's entries (`menuEntries`), each action acting through `run`.
fn menu_entries<Action: Copy + 'static>(
    lines: Vec<MenuLine<Action>>,
    look: impl Fn(Action) -> (&'static str, IconName, MenuItemTone),
    run: impl Fn(Action, &mut Window, &mut App) + Clone + 'static,
) -> Vec<MenuEntry> {
    lines
        .into_iter()
        .map(|line| match line {
            MenuLine::Separator => MenuEntry::Separator,
            MenuLine::Action(action) => {
                let (label, icon, tone) = look(action);
                let run = run.clone();
                MenuEntry::Item(
                    MenuItem::new(label, icon, move |window, cx| run(action, window, cx))
                        .tone(tone),
                )
            }
        })
        .collect()
}

/// The conversation's own mute, as its menu shows it.
fn is_conversation_muted(user_id: Option<Snowflake>, conversation_id: Snowflake, cx: &App) -> bool {
    user_id.is_some_and(|user_id| {
        let settings = notification_settings_of(user_id, cx);
        is_mute_active(
            settings
                .places
                .get(&conversation_id)
                .and_then(|place| place.muted_until),
            now_unix_milliseconds(),
        )
    })
}

impl Home {
    fn current_user_id(&self, cx: &App) -> Option<Snowflake> {
        self.session
            .read(cx)
            .state()
            .current_user()
            .map(|user| user.id)
    }

    fn show_menu(
        &mut self,
        label: String,
        entries: Vec<MenuEntry>,
        origin: MenuOrigin,
        opener: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let opener = opener.or_else(|| window.focused(cx));
        let menu = cx.new(|cx| ContextMenu::open(label, entries, origin, opener, window, cx));
        let subscription = cx.subscribe(&menu, |home, _, _: &gpui::DismissEvent, cx| {
            home.menu = None;
            cx.notify();
        });
        self.menu = Some((menu, subscription));
        cx.notify();
    }

    /// A person's menu, with only what the reader may do about them.
    pub fn open_person_menu(
        &mut self,
        request: PersonMenuRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let catalogue = strings(cx);
        let state = self.session.read(cx).state();
        let current_user_id = self.current_user_id(cx);
        let user = request.user.clone();
        let lines = person_menu_actions(PersonMenuFacts {
            is_self: current_user_id == Some(user.id),
            relationship: state
                .relationship(user.id)
                .map(|relationship| relationship.kind),
            can_show_profile: request.profile_guild_id.is_some(),
            can_open_conversation: request.conversation_id.is_none(),
            is_conversation_row: request.conversation_id.is_some(),
            is_conversation_muted: request.conversation_id.is_some_and(|conversation_id| {
                is_conversation_muted(current_user_id, conversation_id, cx)
            }),
        })
        .into_iter()
        // shortcut: the per-place notification dialog comes with step 6.
        .filter(|line| *line != MenuLine::Action(PersonMenuAction::NotificationSettings))
        .collect();
        let home = cx.entity().downgrade();
        let acting = request.clone();
        let entries = menu_entries(
            lines,
            |action| person_item_look(action, catalogue),
            move |action, window, cx| {
                let _ = home.update(cx, |home, cx| {
                    home.run_person_action(action, &acting, window, cx);
                });
            },
        );
        let label = catalogue.context_menu().actions_for(display_name(&user));
        self.show_menu(label, entries, request.origin, request.opener, window, cx);
    }

    pub(super) fn run_person_action(
        &mut self,
        action: PersonMenuAction,
        request: &PersonMenuRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use PersonMenuAction as Action;
        let user = &request.user;
        let person = display_name(user).to_owned();
        let words = strings(cx).friends();
        match action {
            Action::Profile => {
                if let Some(guild_id) = request.profile_guild_id {
                    cx.emit(HomeEvent::ShowMemberProfile {
                        guild_id,
                        user_id: user.id,
                    });
                }
            }
            Action::Message => {
                window.dispatch_action(Box::new(OpenDirectConversation { user_id: user.id }), cx);
            }
            Action::AddFriend => self.send_friend_request_to(user, cx),
            Action::AcceptRequest => self.change_relationship(
                user.id,
                Some(SetRelationshipKind::Friend),
                (words.accepted(&person), words.accept_failed(&person)),
                cx,
            ),
            Action::IgnoreRequest => self.change_relationship(
                user.id,
                None,
                (words.ignored(&person), words.ignore_failed(&person)),
                cx,
            ),
            Action::CancelRequest => self.change_relationship(
                user.id,
                None,
                (words.cancelled(&person), words.cancel_failed(&person)),
                cx,
            ),
            Action::Unblock => self.change_relationship(
                user.id,
                None,
                (words.unblocked(&person), words.unblock_failed(&person)),
                cx,
            ),
            Action::Mute | Action::Unmute => {
                if let Some(conversation_id) = request.conversation_id {
                    self.set_conversation_muted(
                        conversation_id,
                        action == Action::Mute,
                        &person,
                        cx,
                    );
                }
            }
            Action::NotificationSettings => {}
            Action::CopyUserId => {
                let copied = strings(cx).context_menu().user_id_copied();
                self.copy_identifier(user.id, copied, cx);
            }
            Action::CopyChannelId => {
                if let Some(conversation_id) = request.conversation_id {
                    let copied = strings(cx).context_menu().channel_id_copied();
                    self.copy_identifier(conversation_id, copied, cx);
                }
            }
            Action::RemoveFriend => {
                self.ask_to_change_friendship(user, Confirmed::Remove, window, cx);
            }
            Action::Block => self.ask_to_change_friendship(user, Confirmed::Block, window, cx),
        }
    }

    /// Block, asked in the destructive dialog the Friends space uses, from anywhere.
    pub fn ask_to_block(&mut self, user: &PublicUser, window: &mut Window, cx: &mut Context<Self>) {
        self.ask_to_change_friendship(user, Confirmed::Block, window, cx);
    }

    fn ask_to_change_friendship(
        &mut self,
        user: &PublicUser,
        confirmed: Confirmed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.friends_pane.update(cx, |pane, cx| {
            pane.confirm(user, confirmed, false, window, cx);
        });
    }

    /// A group conversation's menu: anyone in it may rename it or leave it (ADR-0026).
    pub fn open_group_menu(
        &mut self,
        group_id: Snowflake,
        origin: MenuOrigin,
        opener: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let catalogue = strings(cx);
        let current_user_id = self.current_user_id(cx);
        let Some(group) = self
            .session
            .read(cx)
            .state()
            .direct_message_channel(group_id)
        else {
            return;
        };
        let name = conversation_display_name(group, current_user_id, catalogue);
        let lines = group_menu_actions(is_conversation_muted(current_user_id, group_id, cx))
            .into_iter()
            // shortcut: the per-place notification dialog comes with step 6.
            .filter(|line| *line != MenuLine::Action(GroupMenuAction::NotificationSettings))
            .collect();
        let home = cx.entity().downgrade();
        let entries = menu_entries(
            lines,
            |action| group_item_look(action, catalogue),
            move |action, window, cx| {
                let _ = home.update(cx, |home, cx| {
                    home.run_group_action(action, group_id, window, cx);
                });
            },
        );
        let label = catalogue.context_menu().actions_for(&name);
        self.show_menu(label, entries, origin, opener, window, cx);
    }

    pub(super) fn run_group_action(
        &mut self,
        action: GroupMenuAction,
        group_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use GroupMenuAction as Action;
        match action {
            Action::RenameGroup => self.ask_to_rename_group(group_id, window, cx),
            Action::LeaveGroup => self.ask_to_leave_group(group_id, window, cx),
            Action::Mute | Action::Unmute => {
                let catalogue = strings(cx);
                let current_user_id = self.current_user_id(cx);
                let name = self
                    .session
                    .read(cx)
                    .state()
                    .direct_message_channel(group_id)
                    .map(|group| conversation_display_name(group, current_user_id, catalogue))
                    .unwrap_or_default();
                self.set_conversation_muted(group_id, action == Action::Mute, &name, cx);
            }
            Action::NotificationSettings => {}
            Action::CopyChannelId => {
                let copied = strings(cx).context_menu().channel_id_copied();
                self.copy_identifier(group_id, copied, cx);
            }
        }
    }

    // ---- What the items do ----

    fn copy_identifier(&mut self, id: Snowflake, copied: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(id.to_string()));
        copied.clone_into(&mut self.announcement);
        cx.notify();
    }

    /// Mutes until turned back on, or unmutes keeping the "Notify me about" choice.
    fn set_conversation_muted(
        &mut self,
        conversation_id: Snowflake,
        is_muted: bool,
        name: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = self.current_user_id(cx) else {
            return;
        };
        change_preferences(cx, move |preferences| {
            let settings = preferences.notification_settings_of(user_id);
            let place = settings
                .places
                .get(&conversation_id)
                .copied()
                .unwrap_or_default();
            let changed = PlaceNotificationSettings {
                muted_until: is_muted.then_some(MuteEnd::Never),
                ..place
            };
            preferences.set_notification_settings(
                user_id,
                with_place_settings(&settings, conversation_id, changed),
            );
        });
        let words = strings(cx).notifications();
        self.announcement = if is_muted {
            words.saved(name)
        } else {
            words.unmuted(name)
        };
        cx.notify();
    }

    /// Add friend: sends a request, or accepts theirs if one was waiting.
    fn send_friend_request_to(&mut self, user: &PublicUser, cx: &mut Context<Self>) {
        let request = SendFriendRequest {
            username: user.username.clone(),
            friend_code: None,
        };
        let failure = strings(cx).members().friend_request_failed().to_owned();
        let home = cx.entity().downgrade();
        self.session.update(cx, |session, cx| {
            session.send_friend_request(request, cx, move |answer, cx| {
                cx.defer(move |cx| {
                    let _ = home.update(cx, |home, cx| {
                        let outcome = answer.map(|relationship| {
                            let words = strings(cx).friends();
                            let person = display_name(&relationship.user);
                            if relationship.kind == RelationshipKind::Friend {
                                words.accepted_theirs(person)
                            } else {
                                words.sent(person)
                            }
                        });
                        home.finish_change(outcome, &failure, cx);
                    });
                });
            });
        });
    }

    /// Accept (`Some(Friend)`), or remove a request or a block (`None`).
    fn change_relationship(
        &mut self,
        user_id: Snowflake,
        kind: Option<SetRelationshipKind>,
        (done, failure): (String, String),
        cx: &mut Context<Self>,
    ) {
        let home = cx.entity().downgrade();
        // Deferred: a signed-out session answers at once, while Home is still being updated.
        let on_done = move |answer: Result<(), ClientError>, cx: &mut App| {
            cx.defer(move |cx| {
                let _ = home.update(cx, |home, cx| {
                    home.finish_change(answer.map(|()| done), &failure, cx);
                });
            });
        };
        self.session.update(cx, |session, cx| match kind {
            Some(kind) => session.set_relationship(user_id, kind, cx, on_done),
            None => session.remove_relationship(user_id, cx, on_done),
        });
    }

    /// Reads out what a change did, or says in the notice bar why it did not happen.
    fn finish_change(
        &mut self,
        outcome: Result<String, ClientError>,
        failure: &str,
        cx: &mut Context<Self>,
    ) {
        match outcome {
            Ok(done) => self.announcement = done,
            Err(error) => {
                let reason =
                    describe_error(&error, failure, crate::strings_pending_i18n::strings(cx));
                cx.emit(HomeEvent::MenuActionFailed(reason));
            }
        }
        cx.notify();
    }
}
