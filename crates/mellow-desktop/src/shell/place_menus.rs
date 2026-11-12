//! The menus of places (`ServerRail.svelte`, `ChannelSidebar.svelte`, `ChatPane.svelte`,
//! `notification-preferences.svelte.ts`, DESIGN.md §6.33, docs/design/desktop-place-menus.md): a
//! server's icon in the rail, a channel row, a category header, and the chat header's "More
//! options" in a narrow window. Their items come from client-core's `context_menus_places`; this
//! module puts them into words and carries them out. A right-click opens a menu at the pointer;
//! the Menu key and Shift+F10 open the focused row's next to it. Focus goes back to the row.
//!
//! Mutes and "Notify me about" are stored per account in the preferences, the web's shape, so the
//! notifier, the Dock badge and the rows follow them.
//!
//! shortcut: the actions whose screens the desktop has not yet (invite people, category settings,
//! create, move, rename and delete channels and categories: M3 step 6; joining voice: step 7) are
//! left out of the menus, through facts that say the reader cannot; voice rows have no menu until
//! voice comes. Add each with its screen. Server and channel settings show once the desktop has a
//! page the reader may open (`settings::server`, `settings::channel`).

use crate::components::LiveRegion as _;
use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    AppContext as _, Bounds, Context, DismissEvent, Div, Entity, FocusHandle,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, Pixels,
    SharedString, Stateful, Subscription, WeakEntity, Window, div,
};
use mellow_client_core::shared_logic::context_menus_places::{
    CategoryMenuAction, CategoryMenuFacts, ChannelMenuAction, ChannelMenuFacts,
    ChatHeaderMenuAction, ChatHeaderMenuFacts, MenuLine, NotificationMenuAction, ServerMenuAction,
    ServerMenuFacts, ServerMenuPlace, category_menu_actions, channel_menu_actions,
    chat_header_menu_actions, server_menu_actions, server_menu_facts_of,
};
use mellow_client_core::shared_logic::notification_settings::{
    NotificationSettings, PlaceNotificationSettings, is_mute_active, with_place_settings,
};
use mellow_client_core::shared_logic::notifications::{NotificationTarget, notification_target_id};
use mellow_protocol::{ChannelKind, Snowflake};

use super::DuskShell;
use super::place_notifications::{InitialGroup, PlaceNotificationsBody};
use super::regions::Region;
use crate::components::{
    ContextMenu, MenuAlignment, MenuAnchor, MenuEntry, MenuItem, MenuItemTone, MenuOrigin,
    ModalDialog, ModalDialogEvent,
};
use crate::i18n::strings;
use crate::notifier::notification_settings_of;
use crate::preferences::change_preferences;
use crate::session::now_unix_milliseconds;
use crate::theme::icons::IconName;

/// Go to's shortcut, for screen readers (`aria-keyshortcuts`).
const GO_TO_KEY_SHORTCUTS: &str = if cfg!(target_os = "macos") {
    "Meta+K"
} else {
    "Control+K"
};

/// What a menu is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlaceMenuTarget {
    Server(Snowflake),
    Channel(Snowflake),
    Category(Snowflake),
    ChatHeader,
}

/// The open menu and the dialogs its items open, with what was last said to screen readers.
#[derive(Default)]
pub(super) struct PlaceMenus {
    menu: Option<(Entity<ContextMenu>, Subscription, PlaceMenuTarget)>,
    /// Where the chat header's "More options" is drawn, for its menu to open under it.
    pub(super) more_options_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Whether its menu was open when the button was pressed: the press closes the menu before the
    /// click, which must then not reopen it (the web's `createPopoverToggle`).
    was_chat_header_menu_open_at_press: Option<bool>,
    notifications_dialog: Option<(Entity<ModalDialog>, Subscription)>,
    leave_dialog: Option<(Entity<ModalDialog>, Subscription)>,
    /// "Server ID copied", "#general unmuted.": read out, never shown.
    announcement: SharedString,
}

/// The target's own mute (`isTargetMuted`): a channel of a muted server is not "muted" itself.
pub(super) fn is_target_muted(
    settings: &NotificationSettings,
    target_id: Snowflake,
    now: u64,
) -> bool {
    is_mute_active(
        settings
            .places
            .get(&target_id)
            .and_then(|place| place.muted_until),
        now,
    )
}

/// A row that opens a place's menu: where it sits in its list, for the keyboard's origin.
#[derive(Debug, Clone, Copy)]
pub(super) struct MenuRow {
    pub(super) target: PlaceMenuTarget,
    pub(super) region: Region,
    pub(super) child_index: usize,
}

/// Adds the menu's openers to `row`: a right-click at the pointer, the Menu key and Shift+F10
/// next to the row; `focus` gets focus back when the menu closes.
pub(super) fn with_place_menu(
    row: Stateful<Div>,
    menu_row: MenuRow,
    focus: FocusHandle,
    cx: &mut Context<DuskShell>,
) -> Stateful<Div> {
    let pointer_focus = focus.clone();
    row.on_mouse_down(
        MouseButton::Right,
        cx.listener(move |shell, event: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            let origin = MenuOrigin::Point {
                x: f32::from(event.position.x),
                y: f32::from(event.position.y),
            };
            shell.open_place_menu(
                menu_row.target,
                origin,
                Some(pointer_focus.clone()),
                window,
                cx,
            );
        }),
    )
    .on_key_down(cx.listener(move |shell, event: &KeyDownEvent, window, cx| {
        if !crate::components::is_context_menu_keystroke(&event.keystroke) {
            return;
        }
        cx.stop_propagation();
        let origin = shell.keyboard_menu_origin(menu_row, window);
        shell.open_place_menu(menu_row.target, origin, Some(focus.clone()), window, cx);
    }))
}

/// One item that runs `run` on the shell once the menu closed.
fn entry(
    shell: &WeakEntity<DuskShell>,
    label: impl Into<SharedString>,
    icon: IconName,
    run: impl Fn(&mut DuskShell, &mut Window, &mut Context<DuskShell>) + 'static,
) -> MenuItem {
    let shell = shell.clone();
    MenuItem::new(label, icon, move |window, cx| {
        let _ = shell.update(cx, |shell, cx| run(shell, window, cx));
    })
}

fn entries<Action: Copy>(
    lines: Vec<MenuLine<Action>>,
    item_of: impl Fn(Action) -> Option<MenuItem>,
) -> Vec<MenuEntry> {
    lines
        .into_iter()
        .filter_map(|line| match line {
            MenuLine::Separator => Some(MenuEntry::Separator),
            MenuLine::Action(action) => item_of(action).map(MenuEntry::Item),
        })
        .collect()
}

impl DuskShell {
    /// The signed-in reader's notification choices.
    pub(super) fn notification_settings(&self, cx: &gpui::App) -> NotificationSettings {
        self.state(cx)
            .current_user()
            .map(|user| notification_settings_of(user.id, cx))
            .unwrap_or_default()
    }

    /// Next to the focused row: beside a rail icon rather than over the next ones, under a
    /// channel row. At the pointer when the row is not laid out.
    fn keyboard_menu_origin(&self, row: MenuRow, window: &Window) -> MenuOrigin {
        let list = if row.region == Region::ServerRail {
            &self.regions.rail
        } else {
            &self.regions.channel_list
        };
        match list.scroll.bounds_for_item(row.child_index) {
            // Its top-right corner as the anchor: the menu opens beside it, first item focused.
            Some(bounds) if row.region == Region::ServerRail => MenuOrigin::Anchor {
                bounds: MenuAnchor {
                    top: f32::from(bounds.top()),
                    bottom: f32::from(bounds.top()),
                    left: f32::from(bounds.right()),
                    right: f32::from(bounds.right()),
                },
                alignment: MenuAlignment::Start,
            },
            Some(bounds) => MenuOrigin::Anchor {
                bounds: MenuAnchor {
                    top: f32::from(bounds.top()),
                    bottom: f32::from(bounds.bottom()),
                    left: f32::from(bounds.left()),
                    right: f32::from(bounds.right()),
                },
                alignment: MenuAlignment::Start,
            },
            None => {
                let pointer = window.mouse_position();
                MenuOrigin::Point {
                    x: f32::from(pointer.x),
                    y: f32::from(pointer.y),
                }
            }
        }
    }

    /// Opens the menu of `target`, unless it offers nothing.
    pub(super) fn open_place_menu(
        &mut self,
        target: PlaceMenuTarget,
        origin: MenuOrigin,
        opener: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((label, entries)) = self.place_menu_entries(target, cx) else {
            return;
        };
        if entries.is_empty() {
            return;
        }
        let opener = opener.or_else(|| window.focused(cx));
        let menu = cx.new(|cx| ContextMenu::open(label, entries, origin, opener, window, cx));
        let subscription = cx.subscribe(&menu, |shell, _, _: &DismissEvent, cx| {
            shell.place_menus.menu = None;
            cx.notify();
        });
        self.place_menus.menu = Some((menu, subscription, target));
        cx.notify();
    }

    /// Whether `target` still exists: a server left or a channel gone closes its menu.
    fn has_place(&self, target: PlaceMenuTarget, cx: &gpui::App) -> bool {
        let state = self.state(cx);
        match target {
            PlaceMenuTarget::Server(guild_id) => state.guild(guild_id).is_some(),
            PlaceMenuTarget::Channel(channel_id) | PlaceMenuTarget::Category(channel_id) => {
                state.my_channel_permissions(channel_id).is_some()
            }
            PlaceMenuTarget::ChatHeader => self.navigation.selected_channel_id().is_some(),
        }
    }

    /// The menu's accessible name and items.
    fn place_menu_entries(
        &self,
        target: PlaceMenuTarget,
        cx: &mut Context<Self>,
    ) -> Option<(String, Vec<MenuEntry>)> {
        match target {
            PlaceMenuTarget::Server(guild_id) => self.server_menu_entries(guild_id, cx),
            PlaceMenuTarget::Channel(channel_id) => self.channel_menu_entries(channel_id, cx),
            PlaceMenuTarget::Category(category_id) => self.category_menu_entries(category_id, cx),
            PlaceMenuTarget::ChatHeader => self.chat_header_menu_entries(cx),
        }
    }

    /// A server's icon in the rail (the strip's menu).
    fn server_menu_entries(
        &self,
        guild_id: Snowflake,
        cx: &mut Context<Self>,
    ) -> Option<(String, Vec<MenuEntry>)> {
        let words = strings(cx);
        let shell = cx.entity().downgrade();
        let state = self.state(cx);
        let settings = self.notification_settings(cx);
        let now = now_unix_milliseconds();
        let guild = state.guild(guild_id)?;
        let name = guild.name.clone();
        let facts = ServerMenuFacts {
            // Their screens come with server management (see the module's shortcut).
            can_invite_people: false,
            can_open_settings: crate::settings::server::has_server_settings_page_for_reader(guild),
            can_create_channels: false,
            ..server_menu_facts_of(
                guild,
                state.current_user().map(|user| user.id),
                is_target_muted(&settings, guild_id, now),
            )
        };
        let notification_target = NotificationTarget::Server { guild_id };
        let items = entries(
            server_menu_actions(facts, ServerMenuPlace::Strip),
            |action| match action {
                ServerMenuAction::CopyServerId => Some(entry(
                    &shell,
                    words.context_menu().copy_server_id(),
                    IconName::Copy,
                    move |shell, _, cx| {
                        let copied = strings(cx).context_menu().server_id_copied();
                        shell.copy_id(guild_id, copied, cx);
                    },
                )),
                ServerMenuAction::LeaveServer => Some(
                    entry(
                        &shell,
                        words.sidebar().leave_server(),
                        IconName::SignOut,
                        move |shell, window, cx| {
                            shell.confirm_leave_server(guild_id, window, cx);
                        },
                    )
                    .tone(MenuItemTone::Danger),
                ),
                ServerMenuAction::Notification(action) => Some(notification_item(
                    &shell,
                    action,
                    notification_target,
                    name.clone(),
                    words,
                )),
                ServerMenuAction::ServerSettings => Some(entry(
                    &shell,
                    words.sidebar().server_settings(),
                    IconName::GearSix,
                    move |shell, window, cx| shell.open_server_settings(guild_id, None, window, cx),
                )),
                ServerMenuAction::InvitePeople
                | ServerMenuAction::CreateChannel
                | ServerMenuAction::CreateCategory => None,
            },
        );
        Some((words.context_menu().actions_for(&guild.name), items))
    }

    /// A text channel's row.
    fn channel_menu_entries(
        &self,
        channel_id: Snowflake,
        cx: &mut Context<Self>,
    ) -> Option<(String, Vec<MenuEntry>)> {
        let words = strings(cx);
        let shell = cx.entity().downgrade();
        let state = self.state(cx);
        let settings = self.notification_settings(cx);
        let now = now_unix_milliseconds();
        let (guild, channel) = state.guilds().iter().find_map(|guild| {
            guild
                .channels
                .iter()
                .find(|channel| channel.id == channel_id)
                .map(|channel| (guild, channel))
        })?;
        if channel.kind != ChannelKind::Text {
            return None;
        }
        let guild_id = guild.id;
        let shown_name = format!("#{}", channel.name);
        let facts = ChannelMenuFacts {
            can_edit_channel: crate::settings::channel::has_channel_settings_page_for_reader(
                state, channel_id,
            ),
            is_muted: is_target_muted(&settings, channel_id, now),
            can_move_up: false,
            can_move_down: false,
            can_move_to_category: false,
            can_delete: false,
            voice: None,
        };
        let notification_target = NotificationTarget::Channel {
            guild_id,
            channel_id,
        };
        let items = entries(channel_menu_actions(facts), |action| match action {
            ChannelMenuAction::CopyChannelId => Some(entry(
                &shell,
                words.context_menu().copy_channel_id(),
                IconName::Copy,
                move |shell, _, cx| {
                    let copied = strings(cx).context_menu().channel_id_copied();
                    shell.copy_id(channel_id, copied, cx);
                },
            )),
            ChannelMenuAction::Notification(action) => Some(notification_item(
                &shell,
                action,
                notification_target,
                shown_name.clone(),
                words,
            )),
            ChannelMenuAction::EditChannel => Some(entry(
                &shell,
                words.context_menu().edit_channel(),
                IconName::GearSix,
                move |shell, window, cx| shell.open_channel_settings(channel_id, window, cx),
            )),
            ChannelMenuAction::JoinVoice
            | ChannelMenuAction::LeaveVoice
            | ChannelMenuAction::MoveUp
            | ChannelMenuAction::MoveDown
            | ChannelMenuAction::MoveToCategory
            | ChannelMenuAction::DeleteChannel => None,
        });
        Some((words.context_menu().actions_for(&shown_name), items))
    }

    /// A category's header.
    fn category_menu_entries(
        &self,
        category_id: Snowflake,
        cx: &mut Context<Self>,
    ) -> Option<(String, Vec<MenuEntry>)> {
        let words = strings(cx);
        let shell = cx.entity().downgrade();
        let state = self.state(cx);
        let category = state.guild_channel(category_id)?;
        let facts = CategoryMenuFacts {
            // Renaming, moving and deleting come with server management.
            can_manage: false,
            can_move_up: false,
            can_move_down: false,
        };
        let items = entries(category_menu_actions(facts), |action| match action {
            CategoryMenuAction::CopyCategoryId => Some(entry(
                &shell,
                words.context_menu().copy_category_id(),
                IconName::Copy,
                move |shell, _, cx| {
                    let copied = strings(cx).context_menu().category_id_copied();
                    shell.copy_id(category_id, copied, cx);
                },
            )),
            CategoryMenuAction::RenameCategory
            | CategoryMenuAction::MoveUp
            | CategoryMenuAction::MoveDown
            | CategoryMenuAction::DeleteCategory => None,
        });
        Some((words.context_menu().actions_for(&category.name), items))
    }

    /// The chat header's "More options".
    fn chat_header_menu_entries(&self, cx: &mut Context<Self>) -> Option<(String, Vec<MenuEntry>)> {
        let words = strings(cx);
        let shell = cx.entity().downgrade();
        let state = self.state(cx);
        let channel_id = self.navigation.selected_channel_id()?;
        let group_id = state
            .direct_message_channel(channel_id)
            .filter(|conversation| conversation.owner_id.is_some())
            .map(|conversation| conversation.id);
        let facts = ChatHeaderMenuFacts {
            is_group: group_id.is_some(),
            can_edit_channel: crate::settings::channel::has_channel_settings_page_for_reader(
                state, channel_id,
            ),
        };
        let items = entries(chat_header_menu_actions(facts), |action| match action {
            ChatHeaderMenuAction::GoTo => Some(
                entry(
                    &shell,
                    words.chat().go_to(),
                    IconName::Compass,
                    DuskShell::toggle_quick_switcher,
                )
                .key_shortcuts(GO_TO_KEY_SHORTCUTS),
            ),
            ChatHeaderMenuAction::PinnedMessages => Some(entry(
                &shell,
                words.pins().pinned_messages(),
                IconName::PushPin,
                |shell, window, cx| {
                    shell.conversation.update(cx, |conversation, cx| {
                        if !conversation.is_pins_panel_open() {
                            conversation.toggle_pins_panel(window, cx);
                        }
                    });
                },
            )),
            ChatHeaderMenuAction::AddFriendsToGroup => group_id.map(|group_id| {
                entry(
                    &shell,
                    words.groups().add_friends_to_group(),
                    IconName::UserPlus,
                    move |shell, window, cx| {
                        shell.home.update(cx, |home, cx| {
                            home.open_people_picker(Some(group_id), window, cx);
                        });
                    },
                )
            }),
            ChatHeaderMenuAction::ChannelSettings => Some(entry(
                &shell,
                words.chat().channel_settings(),
                IconName::GearSix,
                move |shell, window, cx| shell.open_channel_settings(channel_id, window, cx),
            )),
        });
        Some((words.chat().more_options().to_owned(), items))
    }

    /// The chat header's "More options" in a narrow window, under its button; pressed again, it
    /// closes.
    pub(super) fn toggle_chat_header_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let was_open = self
            .place_menus
            .was_chat_header_menu_open_at_press
            .take()
            .unwrap_or_else(|| self.is_chat_header_menu_open());
        if was_open {
            self.place_menus.menu = None;
            cx.notify();
            return;
        }
        let Some(bounds) = self.place_menus.more_options_bounds.get() else {
            return;
        };
        let origin = MenuOrigin::Anchor {
            bounds: MenuAnchor {
                top: f32::from(bounds.top()),
                bottom: f32::from(bounds.bottom()),
                left: f32::from(bounds.left()),
                right: f32::from(bounds.right()),
            },
            alignment: MenuAlignment::End,
        };
        self.open_place_menu(PlaceMenuTarget::ChatHeader, origin, None, window, cx);
    }

    pub(super) fn is_chat_header_menu_open(&self) -> bool {
        matches!(
            self.place_menus.menu,
            Some((_, _, PlaceMenuTarget::ChatHeader))
        )
    }

    pub(super) fn note_more_options_press(&mut self) {
        self.place_menus.was_chat_header_menu_open_at_press = Some(self.is_chat_header_menu_open());
    }

    fn copy_id(&mut self, id: Snowflake, copied: &'static str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(id.to_string()));
        self.place_menus.announcement = copied.into();
        cx.notify();
    }

    /// "Unmute" in a menu: at once, keeping the "Notify me about" choice.
    fn unmute(&mut self, target: NotificationTarget, place_name: &str, cx: &mut Context<Self>) {
        let Some(user_id) = self.state(cx).current_user().map(|user| user.id) else {
            return;
        };
        let target_id = notification_target_id(target);
        change_preferences(cx, move |preferences| {
            let settings = preferences.notification_settings_of(user_id);
            let place = settings.places.get(&target_id).copied().unwrap_or_default();
            let unmuted = with_place_settings(
                &settings,
                target_id,
                PlaceNotificationSettings {
                    muted_until: None,
                    ..place
                },
            );
            preferences.set_notification_settings(user_id, unmuted);
        });
        self.place_menus.announcement = strings(cx).notifications().unmuted(place_name).into();
        cx.notify();
    }

    /// "Mute…" and "Notification settings…": the place's dialog, focus in the matching group.
    fn open_place_notifications(
        &mut self,
        target: NotificationTarget,
        place_name: String,
        initial_group: InitialGroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = self.state(cx).current_user().map(|user| user.id) else {
            return;
        };
        let words = strings(cx);
        let settings = notification_settings_of(user_id, cx);
        let body = cx.new(|_| {
            PlaceNotificationsBody::new(target, &settings, initial_group, now_unix_milliseconds())
        });
        let dialog = cx.new(|cx| {
            ModalDialog::new(
                words.notifications().dialog_title(&place_name),
                words.common().cancel(),
                window,
                cx,
            )
            .submit(words.common().save(), words.common().save(), false)
            .body(body.clone().into())
        });
        let subscription = cx.subscribe_in(
            &dialog,
            window,
            move |shell, dialog, event: &ModalDialogEvent, window, cx| match event {
                ModalDialogEvent::Submitted => {
                    let body = body.read(cx);
                    let now = now_unix_milliseconds();
                    let saved = body.saved_into(&notification_settings_of(user_id, cx), now);
                    change_preferences(cx, move |preferences| {
                        preferences.set_notification_settings(user_id, saved);
                    });
                    shell.place_menus.announcement =
                        strings(cx).notifications().saved(&place_name).into();
                    dialog.update(cx, |dialog, cx| dialog.request_close(window, cx));
                }
                ModalDialogEvent::Closed => {
                    shell.place_menus.notifications_dialog = None;
                    cx.notify();
                }
            },
        );
        self.place_menus.notifications_dialog = Some((dialog, subscription));
        cx.notify();
    }

    /// "Leave server", the destructive pattern of DESIGN.md §8: Cancel takes focus, a failure
    /// stays in the dialog, and focus goes to the composer once the server is gone.
    fn confirm_leave_server(
        &mut self,
        guild_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self
            .state(cx)
            .guild(guild_id)
            .map(|guild| guild.name.clone())
        else {
            return;
        };
        let words = strings(cx).leave_server();
        let dialog = cx.new(|cx| {
            ModalDialog::new(
                words.title(&name),
                strings(cx).common().cancel(),
                window,
                cx,
            )
            .subtitle(words.subtitle(&name))
            .submit(words.submit(), words.leaving(), true)
        });
        let window_handle = window.window_handle();
        let subscription = cx.subscribe(
            &dialog,
            move |shell, dialog, event: &ModalDialogEvent, cx| match event {
                ModalDialogEvent::Submitted => {
                    dialog.update(cx, |dialog, cx| {
                        dialog.set_busy(true, cx);
                        dialog.set_error(None, cx);
                    });
                    let shell_handle = cx.entity().downgrade();
                    shell.session.update(cx, |session, cx| {
                        session.leave_server(
                            guild_id,
                            words.failed().to_owned(),
                            cx,
                            move |answer, cx| match answer {
                                Ok(()) => {
                                    let _ = window_handle.update(cx, |_, window, cx| {
                                        let _ = shell_handle.update(cx, |shell, cx| {
                                            shell.place_menus.leave_dialog = None;
                                            shell.conversation.update(cx, |conversation, cx| {
                                                conversation.focus_composer(window, cx);
                                            });
                                            cx.notify();
                                        });
                                    });
                                }
                                Err(message) => dialog.update(cx, |dialog, cx| {
                                    dialog.set_busy(false, cx);
                                    dialog.set_error(Some(message.into()), cx);
                                }),
                            },
                        );
                    });
                }
                ModalDialogEvent::Closed => {
                    shell.place_menus.leave_dialog = None;
                    cx.notify();
                }
            },
        );
        self.place_menus.leave_dialog = Some((dialog, subscription));
        cx.notify();
    }

    /// The open menu (closed when its place went away), its dialogs, and the status line screen
    /// readers hear.
    pub(super) fn render_place_menus(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<gpui::AnyElement> {
        if let Some((_, _, target)) = &self.place_menus.menu
            && !self.has_place(*target, cx)
        {
            cx.defer_in(window, |shell, _, cx| {
                shell.place_menus.menu = None;
                cx.notify();
            });
        }
        let menus = &self.place_menus;
        menus
            .menu
            .as_ref()
            .map(|(menu, _, _)| menu.clone().into_any_element())
            .into_iter()
            .chain(
                menus
                    .notifications_dialog
                    .as_ref()
                    .map(|(dialog, _)| dialog.clone().into_any_element()),
            )
            .chain(
                menus
                    .leave_dialog
                    .as_ref()
                    .map(|(dialog, _)| dialog.clone().into_any_element()),
            )
            .chain(std::iter::once(
                div()
                    .id("place-menu-announcement")
                    .live_polite(menus.announcement.clone())
                    .into_any_element(),
            ))
            .collect()
    }
}

/// "Mute…", "Unmute" or "Notification settings…" for `target`, named `place_name` in what it says.
fn notification_item(
    shell: &WeakEntity<DuskShell>,
    action: NotificationMenuAction,
    target: NotificationTarget,
    place_name: String,
    words: crate::i18n::Strings,
) -> MenuItem {
    let notifications = words.notifications();
    match action {
        NotificationMenuAction::Unmute => entry(
            shell,
            notifications.unmute(),
            IconName::Bell,
            move |shell, _, cx| {
                shell.unmute(target, &place_name, cx);
            },
        ),
        NotificationMenuAction::Mute | NotificationMenuAction::NotificationSettings => {
            let (label, icon, group) = if action == NotificationMenuAction::Mute {
                (
                    notifications.mute_menu(),
                    IconName::BellSlash,
                    InitialGroup::Mute,
                )
            } else {
                (
                    notifications.settings_menu(),
                    IconName::Bell,
                    InitialGroup::Level,
                )
            };
            entry(shell, label, icon, move |shell, window, cx| {
                shell.open_place_notifications(target, place_name.clone(), group, window, cx);
            })
        }
    }
}

#[cfg(test)]
#[path = "place_menus_tests.rs"]
mod tests;
