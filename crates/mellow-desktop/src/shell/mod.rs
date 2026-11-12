//! The signed-in Dusk shell (DESIGN.md §4.2, ADR-0022, `docs/design/desktop-shell-and-navigation.md`):
//! the dusk ground holding the navigator card (server rail and channel list above the user card),
//! the conversation sheet and the member side sheet, with the connection banner above them. It
//! follows the session's client state, keeps where the reader is (`Navigation`) and remembers it
//! with the column widths and member sheets in the preferences. Below their breakpoints the
//! navigator becomes a drawer and the member sheet an overlay. Every length and colour is a token.

mod channel_sidebar;
mod conversation_menus;
mod conversation_sheet;
mod custom_status_dialog;
mod member_profile;
mod member_sheet;
mod message_search_host;
mod overlays;
mod place_menus;
mod place_notifications;
pub mod quick_switcher;
mod regions;
mod resize_handle;
mod server_rail;
mod settings_screens;
mod user_card;

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, FocusHandle, ImageSource,
    InteractiveElement as _, IntoElement, KeyBinding, KeyDownEvent, MouseMoveEvent,
    ParentElement as _, Pixels, Render, Role, StatefulInteractiveElement as _, Styled as _,
    Subscription, Task, WeakEntity, Window, actions, div, prelude::FluentBuilder as _,
};
use mellow_client_core::DownloadableFile;
use mellow_client_core::state::{ClientState, StateChange};
use mellow_protocol::{ProfileImage, Snowflake};

use crate::column_widths::{
    CHANNELS_COLUMN_BOUNDS, ColumnBounds, MEMBERS_COLUMN_BOUNDS, SEARCH_COLUMN_BOUNDS,
    WIDTH_BESIDE_COLUMNS, clamp_column_width, fit_column_widths, largest_column_width,
    stored_column_width,
};
use crate::components::{ContextMenu, ModalDialog, with_tab_navigation};
use crate::connection_banner::ConnectionBanner;
use crate::conversation::{ConversationView, OpenForwardOrigin};
use crate::home::{Home, HomeEvent, OpenPersonMenu};
use crate::local_widgets::error_line;
use crate::message_search::MessageSearchView;
use crate::navigation::{Direction, Navigation, Place};
use crate::pictures::{Animation, DisplaySize, PictureRequest, picture_source};
use crate::preferences::{PreferencesStore, change_preferences, preferences};
use crate::session::{Session, StateChanged};
use crate::theme::tokens::{
    BREAKPOINT_NAVIGATOR, BREAKPOINT_SIDE_SHEET, COMPONENT_NOTICE_BAR_ICON_SIZE,
    COMPONENT_NOTICE_BAR_MIN_HEIGHT, COMPONENT_NOTICE_BAR_PADDING_BLOCK,
    COMPONENT_NOTICE_BAR_PADDING_INLINE, SIZE_SHEET_INSET,
};
use crate::theme::{SANS_FONT_FAMILY, color, length, theme, window_width_at_default_zoom};
use crate::window_chrome::{TITLE_BAR_BAND_HEIGHT, TOP_BAND_HEIGHT};

use self::quick_switcher::{QuickSwitcher, QuickSwitcherEvent};
pub use self::regions::Region;
use self::regions::{NextRegion, PreviousRegion, RegionFocus};

actions!(
    mellow_shell,
    [
        /// Alt+Up: the previous channel or conversation.
        PreviousChannel,
        /// Alt+Down.
        NextChannel,
        /// Alt+Shift+Up: the previous channel with unread messages.
        PreviousUnreadChannel,
        /// Alt+Shift+Down.
        NextUnreadChannel,
        /// Ctrl+Alt+Up (Cmd on macOS): the previous server, Home first.
        PreviousServer,
        /// Ctrl+Alt+Down.
        NextServer,
        /// Ctrl+K: the quick switcher; again closes it.
        ToggleQuickSwitcher,
        /// Ctrl+U: the member sheet.
        ToggleMemberSheet,
        /// Ctrl+, (Cmd+, on macOS, the platform's settings shortcut): the user settings.
        OpenUserSettings,
        /// Ctrl+F: the message search field, or the results with their own field.
        FocusMessageSearch
    ]
);

/// Opens (creating it if needed) the one-to-one conversation with someone and shows it, the
/// composer focused: what a person's "Message" does anywhere in the shell (context menus, profile
/// cards). A failure shows in a notice bar under the connection banner.
#[derive(Debug, Clone, PartialEq, gpui::Action)]
#[action(namespace = mellow_shell, no_json)]
pub struct OpenDirectConversation {
    pub user_id: Snowflake,
}

const SHELL_KEY_CONTEXT: &str = "DuskShell";

/// `Ctrl` in DESIGN.md §7.2 is `Cmd` on macOS.
#[cfg(target_os = "macos")]
macro_rules! secondary {
    ($keys:literal) => {
        concat!("cmd-", $keys)
    };
}
#[cfg(not(target_os = "macos"))]
macro_rules! secondary {
    ($keys:literal) => {
        concat!("ctrl-", $keys)
    };
}

/// The web's navigation shortcuts (DESIGN.md §7.2).
pub fn bind_shell_keys(cx: &mut App) {
    let context = Some(SHELL_KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("alt-up", PreviousChannel, context),
        KeyBinding::new("alt-down", NextChannel, context),
        KeyBinding::new("alt-shift-up", PreviousUnreadChannel, context),
        KeyBinding::new("alt-shift-down", NextUnreadChannel, context),
        KeyBinding::new(secondary!("alt-up"), PreviousServer, context),
        KeyBinding::new(secondary!("alt-down"), NextServer, context),
        KeyBinding::new(secondary!("k"), ToggleQuickSwitcher, context),
        KeyBinding::new(secondary!("u"), ToggleMemberSheet, context),
        KeyBinding::new(secondary!(","), OpenUserSettings, context),
        KeyBinding::new(secondary!("f"), FocusMessageSearch, context),
    ]);
    regions::bind_region_keys(cx, context);
}

/// Which cards fit beside each other at a window width (DESIGN.md §4.2 "Breakpoints").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellLayout {
    /// Below the navigator breakpoint: the sheet alone, full-bleed; the navigator is a drawer and
    /// the member sheet covers the window.
    SheetAlone,
    /// An open member sheet lies over the sheet.
    NavigatorAndSheet,
    /// From the side sheet breakpoint, an open member sheet docks beside the sheet.
    NavigatorSheetAndSideSheet,
}

impl ShellLayout {
    /// `width` is in logical pixels at 100 % zoom, so zooming in collapses the layout as the web
    /// client's rem-based media queries do (DESIGN.md §7.5).
    #[must_use]
    pub fn for_window_width(width: f32, is_side_sheet_open: bool) -> Self {
        if width < BREAKPOINT_NAVIGATOR {
            Self::SheetAlone
        } else if is_side_sheet_open && width >= BREAKPOINT_SIDE_SHEET {
            Self::NavigatorSheetAndSideSheet
        } else {
            Self::NavigatorAndSheet
        }
    }
}

/// A column the reader resizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResizableColumn {
    Channels,
    Members,
    /// The docked search results, which take the members' place.
    Search,
}

impl ResizableColumn {
    fn bounds(self) -> ColumnBounds {
        match self {
            Self::Channels => CHANNELS_COLUMN_BOUNDS,
            Self::Members => MEMBERS_COLUMN_BOUNDS,
            Self::Search => SEARCH_COLUMN_BOUNDS,
        }
    }

    /// The channel list's trailing edge widens it rightwards; the side sheet's leading edge does
    /// the opposite.
    fn grows_rightwards(self) -> bool {
        self == Self::Channels
    }
}

/// A handle being dragged: written to the preferences only when the gesture ends, so Esc puts the
/// remembered width back.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ColumnDrag {
    column: ResizableColumn,
    start_x: f32,
    start_width: f32,
    width: f32,
}

/// The widths laid out this frame, and what each handle may reach.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ColumnLayout {
    channels: f32,
    /// The docked side column's width: the members or the search results.
    side: Option<f32>,
    channels_bounds: ColumnBounds,
    side_bounds: ColumnBounds,
}

pub struct DuskShell {
    session: Entity<Session>,
    banner: Entity<ConnectionBanner>,
    /// The message list and composer of the open channel or conversation.
    conversation: Entity<ConversationView>,
    /// The Friends space, the people picker and the group dialogs.
    home: Entity<Home>,
    /// Message search: the header's field and the results.
    message_search: Entity<MessageSearchView>,
    /// Why the last `OpenDirectConversation` or menu action failed, until the next place opens.
    conversation_error: Option<String>,
    navigation: Navigation,
    /// Kept while the shell lives, so collapsing survives the drawer closing (as on the web).
    collapsed_category_ids: HashSet<Snowflake>,
    is_navigation_drawer_open: bool,
    /// The member sheet over the conversation, below the side sheet breakpoint.
    is_member_overlay_open: bool,
    column_drag: Option<ColumnDrag>,
    quick_switcher: Option<(Entity<QuickSwitcher>, Subscription)>,
    status_menu: Option<(Entity<ContextMenu>, Subscription)>,
    custom_status_dialog: Option<(Entity<ModalDialog>, Subscription)>,
    /// The menus of servers, channels, categories and the chat header, and their dialogs.
    place_menus: place_menus::PlaceMenus,
    /// The server or channel settings over the shell.
    open_settings: Option<settings_screens::OpenSettings>,
    /// The profile a member row opened in the side sheet.
    member_profile: member_profile::MemberProfileState,
    /// Where the user card is drawn, for the status menu to open across it.
    user_card_bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// "User ID copied" on the user card, until `copied_label_timer` ends it.
    is_user_id_copied: bool,
    copied_label_timer: Option<Task<()>>,
    /// What had focus before a drawer or overlay opened: it gets it back when it closes.
    overlay_opener: Option<FocusHandle>,
    overlay_focus: FocusHandle,
    member_sheet_focus: FocusHandle,
    status_button_focus: FocusHandle,
    root_focus: FocusHandle,
    /// F6 between regions, and one Tab stop per navigator list.
    regions: RegionFocus,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for DuskShell {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DuskShell")
            .field("place", &self.navigation.place())
            .field("is_navigation_drawer_open", &self.is_navigation_drawer_open)
            .field("is_member_overlay_open", &self.is_member_overlay_open)
            .finish_non_exhaustive()
    }
}

impl DuskShell {
    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let banner = cx.new(|cx| ConnectionBanner::new(session.clone(), cx));
        let conversation = cx.new(|cx| ConversationView::new(session.clone(), window, cx));
        let home = cx.new(|cx| Home::new(session.clone(), window, cx));
        let message_search = cx.new(|cx| MessageSearchView::new(session.clone(), window, cx));
        let subscriptions = vec![
            cx.subscribe_in(&home, window, Self::follow_home),
            cx.subscribe_in(&message_search, window, Self::follow_message_search),
            cx.observe(&message_search, |_, _, cx| cx.notify()),
            cx.subscribe(&session, |shell, _, changes: &StateChanged, cx| {
                shell.follow_state(&changes.0, cx);
            }),
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.observe(&banner, |_, _, cx| cx.notify()),
            // The preferences may still be read when the first `Ready` comes.
            cx.observe_global::<PreferencesStore>(|shell, cx| shell.follow_state(&[], cx)),
        ];
        let root_focus = cx.focus_handle();
        window.focus(&root_focus, cx);
        let mut shell = Self {
            session,
            banner,
            conversation,
            home,
            message_search,
            conversation_error: None,
            navigation: Navigation::default(),
            collapsed_category_ids: HashSet::new(),
            is_navigation_drawer_open: false,
            is_member_overlay_open: false,
            column_drag: None,
            quick_switcher: None,
            status_menu: None,
            custom_status_dialog: None,
            place_menus: place_menus::PlaceMenus::default(),
            open_settings: None,
            member_profile: member_profile::MemberProfileState::new(cx),
            user_card_bounds: Rc::new(Cell::new(None)),
            is_user_id_copied: false,
            copied_label_timer: None,
            overlay_opener: None,
            overlay_focus: cx.focus_handle(),
            member_sheet_focus: cx.focus_handle(),
            status_button_focus: cx.focus_handle().tab_stop(true),
            root_focus,
            regions: RegionFocus::new(cx),
            _subscriptions: subscriptions,
        };
        shell.follow_state(&[], cx);
        shell
    }

    #[must_use]
    pub fn navigation(&self) -> &Navigation {
        &self.navigation
    }

    #[must_use]
    pub fn is_navigation_drawer_open(&self) -> bool {
        self.is_navigation_drawer_open
    }

    #[must_use]
    pub fn is_member_overlay_open(&self) -> bool {
        self.is_member_overlay_open
    }

    /// The open channel's or conversation's messages and composer.
    #[must_use]
    pub fn conversation(&self) -> &Entity<ConversationView> {
        &self.conversation
    }

    /// The Friends space, the people picker and the group dialogs.
    #[must_use]
    pub fn home(&self) -> &Entity<Home> {
        &self.home
    }

    #[must_use]
    pub fn is_quick_switcher_open(&self) -> bool {
        self.quick_switcher.is_some()
    }

    fn state<'a>(&self, cx: &'a App) -> &'a ClientState {
        self.session.read(cx).state()
    }

    /// The state changed: the place gives way to what remains, and the open server's members are
    /// asked for again after a `Ready`.
    fn follow_state(&mut self, changes: &[StateChange], cx: &mut Context<Self>) {
        let Some(remembered) = preferences(cx) else {
            return;
        };
        let state = self.session.read(cx).state();
        if self.navigation.follow_state(state, remembered) {
            self.after_navigation(cx);
        } else if changes.contains(&StateChange::Guilds) {
            self.load_members_of_open_server(cx);
        }
        self.show_place_in_conversation(cx);
        cx.notify();
    }

    /// The place changed: remembered for the next launch, and the members asked for.
    fn after_navigation(&mut self, cx: &mut Context<Self>) {
        self.conversation_error = None;
        if let Some(place) = self.navigation.place() {
            change_preferences(cx, move |preferences| {
                preferences.last_place = Some(place.remembered());
                if let Place::Server {
                    guild_id,
                    channel_id: Some(channel_id),
                } = place
                {
                    preferences.remember_channel(guild_id, channel_id);
                }
            });
        }
        self.load_members_of_open_server(cx);
        self.show_place_in_conversation(cx);
        cx.notify();
    }

    /// The conversation view follows the place, with its names in the current language.
    fn show_place_in_conversation(&mut self, cx: &mut Context<Self>) {
        let place = self.open_conversation(cx);
        self.conversation
            .update(cx, |conversation, cx| conversation.set_place(place, cx));
    }

    fn load_members_of_open_server(&mut self, cx: &mut Context<Self>) {
        if let Some(guild_id) = self.navigation.selected_guild_id() {
            self.session
                .update(cx, |session, cx| session.load_members(guild_id, false, cx));
        }
    }

    /// Opens a server (`None`: Home), at `channel_id` when given; the drawer closes.
    pub fn open_place(
        &mut self,
        guild_id: Option<Snowflake>,
        channel_id: Option<Snowflake>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = self.session.read(cx).state();
        self.navigation.select_guild(state, guild_id, channel_id);
        self.after_navigation(cx);
        self.close_navigation_drawer(window, cx);
    }

    /// Opens a channel or conversation; the drawer closes.
    pub fn open_channel(
        &mut self,
        channel_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = self.session.read(cx).state();
        self.navigation.select_channel(state, channel_id);
        self.after_navigation(cx);
        self.close_navigation_drawer(window, cx);
    }

    fn step_channel(
        &mut self,
        direction: Direction,
        only_unread: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The settings sheet covers the shell: its shortcuts wait until it closes.
        if self.is_settings_screen_open() {
            return;
        }
        let state = self.state(cx);
        if let Some(channel_id) = self
            .navigation
            .adjacent_channel(state, direction, only_unread)
        {
            self.open_channel(channel_id, window, cx);
        }
    }

    fn step_server(&mut self, direction: Direction, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_settings_screen_open() {
            return;
        }
        let state = self.state(cx);
        if let Some(guild_id) = self.navigation.adjacent_server(state, direction) {
            self.open_place(guild_id, None, window, cx);
        }
    }

    fn toggle_category(&mut self, category_id: Snowflake, cx: &mut Context<Self>) {
        if !self.collapsed_category_ids.remove(&category_id) {
            self.collapsed_category_ids.insert(category_id);
        }
        cx.notify();
    }

    // ---- The member sheet ----

    /// Whose members the side sheet shows: the open server, with a channel open, or the open
    /// group conversation (`GroupMembersSheet.svelte`).
    fn member_sheet_place_id(&self, cx: &App) -> Option<Snowflake> {
        let state = self.state(cx);
        match self.navigation.place()? {
            Place::Server {
                guild_id,
                channel_id: Some(_),
            } => Some(guild_id),
            Place::Home {
                conversation_id: Some(conversation_id),
            } => state
                .direct_message_channel(conversation_id)
                .filter(|conversation| conversation.owner_id.is_some())
                .map(|group| group.id),
            _ => None,
        }
    }

    /// Whether the member sheet is left open for this server (remembered per server).
    fn is_member_sheet_docked_open(&self, cx: &App) -> bool {
        self.member_sheet_place_id(cx).is_some_and(|place_id| {
            preferences(cx)
                .is_some_and(|preferences| preferences.member_sheet_place_ids.contains(&place_id))
        })
    }

    /// Whether the header's toggle shows the sheet as open, docked or over the conversation.
    fn is_member_sheet_shown(&self, window: &Window, cx: &App) -> bool {
        if self.message_search.read(cx).is_panel_open() {
            false
        } else if window_width_at_default_zoom(window) >= BREAKPOINT_SIDE_SHEET {
            self.is_member_sheet_docked_open(cx)
        } else {
            self.is_member_overlay_open
        }
    }

    /// The header's toggle and Ctrl+U: docked where it fits, over the sheet otherwise.
    fn toggle_member_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(place_id) = self.member_sheet_place_id(cx) else {
            return;
        };
        if self.quick_switcher.is_some() || self.is_settings_screen_open() {
            return;
        }
        if self.message_search.read(cx).is_panel_open() {
            // The members take the side sheet back from the results, which stay for the next search.
            self.hide_search_panel(window, cx);
            if self.is_member_sheet_shown(window, cx) {
                cx.notify();
                return;
            }
        }
        if window_width_at_default_zoom(window) >= BREAKPOINT_SIDE_SHEET {
            let is_open = !self.is_member_sheet_docked_open(cx);
            change_preferences(cx, move |preferences| {
                preferences.remember_member_sheet(place_id, is_open);
            });
            if !is_open && self.member_sheet_focus.contains_focused(window, cx) {
                window.focus(&self.root_focus, cx);
            }
        } else if self.is_member_overlay_open {
            self.close_member_overlay(window, cx);
        } else {
            // Closed first, so the overlay records the drawer's opener, not the drawer.
            self.close_navigation_drawer(window, cx);
            self.open_overlay(window, cx);
            self.is_member_overlay_open = true;
        }
        cx.notify();
    }

    fn close_member_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_member_overlay_open {
            self.close_member_overlay(window, cx);
        } else if let Some(place_id) = self.member_sheet_place_id(cx) {
            change_preferences(cx, move |preferences| {
                preferences.remember_member_sheet(place_id, false);
            });
            // The close button is gone with the sheet.
            window.focus(&self.root_focus, cx);
        }
        cx.notify();
    }

    // ---- Overlays: the navigation drawer and the member overlay ----

    fn open_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.overlay_opener = window.focused(cx);
        window.focus(&self.overlay_focus, cx);
    }

    fn give_focus_back_after_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let opener = self
            .overlay_opener
            .take()
            .unwrap_or_else(|| self.root_focus.clone());
        window.focus(&opener, cx);
    }

    fn open_navigation_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.is_member_overlay_open = false;
        self.open_overlay(window, cx);
        self.is_navigation_drawer_open = true;
        cx.notify();
    }

    fn close_navigation_drawer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_navigation_drawer_open {
            self.is_navigation_drawer_open = false;
            self.give_focus_back_after_overlay(window, cx);
            cx.notify();
        }
    }

    fn close_member_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_member_overlay_open {
            self.is_member_overlay_open = false;
            self.give_focus_back_after_overlay(window, cx);
            cx.notify();
        }
    }

    /// Puts away a drawer or member overlay the window or the place no longer has room for. Its
    /// opener may be gone with the layout too (the narrow header's "Open navigation"), so focus
    /// goes to the shell, where shortcuts and Tab keep working.
    fn close_obsolete_layers(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let window_width = window_width_at_default_zoom(window);
        let layout =
            ShellLayout::for_window_width(window_width, self.is_member_sheet_docked_open(cx));
        let is_member_overlay_obsolete =
            window_width >= BREAKPOINT_SIDE_SHEET || self.member_sheet_place_id(cx).is_none();
        if self.is_navigation_drawer_open && layout != ShellLayout::SheetAlone {
            self.overlay_opener = None;
            self.close_navigation_drawer(window, cx);
        }
        if self.is_member_overlay_open && is_member_overlay_obsolete {
            self.overlay_opener = None;
            self.close_member_overlay(window, cx);
        }
    }

    // ---- The quick switcher ----

    /// Ctrl+K and the header's "Go to…": opens it, or closes it when open.
    pub fn toggle_quick_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((switcher, _)) = self.quick_switcher.take() {
            switcher.update(cx, |switcher, cx| switcher.give_focus_back(window, cx));
            cx.notify();
            return;
        }
        if self.is_member_overlay_open || self.is_settings_screen_open() {
            return;
        }
        // Closed first, so the switcher records the drawer's opener, not the drawer.
        self.close_navigation_drawer(window, cx);
        let switcher = cx.new(|cx| QuickSwitcher::new(self.session.clone(), window, cx));
        let subscription = cx.subscribe_in(&switcher, window, Self::follow_quick_switcher);
        self.quick_switcher = Some((switcher, subscription));
        cx.notify();
    }

    fn follow_quick_switcher(
        &mut self,
        switcher: &Entity<QuickSwitcher>,
        event: &QuickSwitcherEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            QuickSwitcherEvent::Dismissed => {
                self.quick_switcher = None;
                cx.notify();
            }
            QuickSwitcherEvent::Open {
                guild_id,
                channel_id,
            } => {
                self.quick_switcher = None;
                self.open_place(*guild_id, *channel_id, window, cx);
                // After a result, the conversation's composer takes focus rather than the opener.
                window.focus(&self.root_focus, cx);
                if self.navigation.selected_channel_id().is_some() {
                    self.conversation.update(cx, |conversation, cx| {
                        conversation.focus_composer(window, cx);
                    });
                }
            }
            QuickSwitcherEvent::OpenConversationWith { user_id, failure } => {
                let switcher = switcher.downgrade();
                let user_id = *user_id;
                let failure = failure.clone();
                self.session.update(cx, |session, cx| {
                    session.open_direct_message(
                        user_id,
                        failure,
                        cx,
                        move |answer, cx| match answer {
                            Ok(conversation_id) => {
                                open_conversation_chosen_in_switcher(
                                    &switcher,
                                    conversation_id,
                                    cx,
                                );
                            }
                            Err(message) => {
                                let _ = switcher.update(cx, |switcher, cx| {
                                    switcher.show_error(message, cx);
                                });
                            }
                        },
                    );
                });
            }
        }
    }

    // ---- Home ----

    fn follow_home(
        &mut self,
        _: &Entity<Home>,
        event: &HomeEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            HomeEvent::OpenConversation(conversation_id) => {
                self.show_conversation(*conversation_id, window, cx);
            }
            HomeEvent::ShowFriends => {
                self.open_place(None, None, window, cx);
                let pane = self.home.read(cx).friends_pane().clone();
                pane.update(cx, |pane, cx| pane.focus_start(window, cx));
            }
            HomeEvent::ShowMemberProfile { guild_id, user_id } => {
                self.show_member_profile(*guild_id, *user_id, window, cx);
            }
            HomeEvent::MenuActionFailed(reason) => {
                self.conversation_error = Some(reason.clone());
                cx.notify();
            }
        }
    }

    /// Opens a conversation just found or created, the composer focused.
    fn show_conversation(
        &mut self,
        conversation_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_channel(conversation_id, window, cx);
        self.close_member_overlay(window, cx);
        if self.navigation.selected_channel_id() == Some(conversation_id) {
            self.conversation.update(cx, |conversation, cx| {
                conversation.focus_composer(window, cx);
            });
        }
    }

    /// The `OpenDirectConversation` action: opens or creates the conversation with `user_id`.
    pub fn open_direct_conversation_with(
        &mut self,
        user_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let failure = crate::i18n::strings(cx)
            .members()
            .open_conversation_failed()
            .to_owned();
        let shell = cx.entity().downgrade();
        let window_handle = window.window_handle();
        self.conversation_error = None;
        self.session.update(cx, |session, cx| {
            session.open_direct_message(user_id, failure, cx, move |answer, cx| {
                // The answer comes while the session is being updated, and showing the
                // conversation reads it: wait until that update is over.
                cx.defer(move |cx| {
                    let _ = window_handle.update(cx, |_, window, cx| {
                        let _ = shell.update(cx, |shell, cx| match answer {
                            Ok(conversation_id) => {
                                shell.show_conversation(conversation_id, window, cx);
                            }
                            Err(message) => {
                                shell.conversation_error = Some(message);
                                cx.notify();
                            }
                        });
                    });
                });
            });
        });
    }

    /// The Home list's "+" and its empty state's "New message": the people picker.
    pub(crate) fn open_new_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.home
            .update(cx, |home, cx| home.open_people_picker(None, window, cx));
    }

    // ---- Column widths ----

    /// The widths to lay out at this window width: the reader's (or the drag's), fitted so the
    /// conversation keeps its minimum.
    fn column_layout(
        &self,
        window_width: f32,
        side_column: Option<ResizableColumn>,
        cx: &App,
    ) -> ColumnLayout {
        let stored = preferences(cx);
        let width_of =
            |column: ResizableColumn| match self.column_drag.filter(|drag| drag.column == column) {
                Some(drag) => drag.width,
                None => stored_column_width(
                    stored.and_then(|preferences| match column {
                        ResizableColumn::Channels => preferences.channels_column_width,
                        ResizableColumn::Members => preferences.members_column_width,
                        ResizableColumn::Search => preferences.search_column_width,
                    }),
                    column.bounds(),
                ),
            };
        let room = window_width - WIDTH_BESIDE_COLUMNS;
        let side_bounds = side_column.map_or(MEMBERS_COLUMN_BOUNDS, ResizableColumn::bounds);
        let (channels, side) = fit_column_widths(
            room,
            (width_of(ResizableColumn::Channels), CHANNELS_COLUMN_BOUNDS),
            side_column.map(|column| (width_of(column), side_bounds)),
        );
        let side_minimum = if side_column.is_some() {
            side_bounds.minimum
        } else {
            0.0
        };
        ColumnLayout {
            channels,
            side,
            channels_bounds: ColumnBounds {
                maximum: largest_column_width(CHANNELS_COLUMN_BOUNDS, room - side_minimum),
                ..CHANNELS_COLUMN_BOUNDS
            },
            side_bounds: ColumnBounds {
                maximum: largest_column_width(side_bounds, room - channels),
                ..side_bounds
            },
        }
    }

    fn remember_column_width(column: ResizableColumn, width: f32, cx: &mut App) {
        change_preferences(cx, move |preferences| match column {
            ResizableColumn::Channels => preferences.channels_column_width = Some(width),
            ResizableColumn::Members => preferences.members_column_width = Some(width),
            ResizableColumn::Search => preferences.search_column_width = Some(width),
        });
    }

    fn follow_column_drag(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mut drag) = self.column_drag else {
            return;
        };
        if event.pressed_button.is_none() {
            self.end_column_drag(cx);
            return;
        }
        let window_width = window_width_at_default_zoom(window);
        let layout = self.column_layout(window_width, self.docked_side_column(window, cx), cx);
        let bounds = match drag.column {
            ResizableColumn::Channels => layout.channels_bounds,
            ResizableColumn::Members | ResizableColumn::Search => layout.side_bounds,
        };
        let towards_wider = if drag.column.grows_rightwards() {
            1.0
        } else {
            -1.0
        };
        let moved = logical_x(event.position.x, window) - drag.start_x;
        drag.width = clamp_column_width(
            drag.start_width + towards_wider * moved,
            bounds,
            bounds.maximum,
        );
        self.column_drag = Some(drag);
        cx.notify();
    }

    fn end_column_drag(&mut self, cx: &mut Context<Self>) {
        if let Some(drag) = self.column_drag.take() {
            Self::remember_column_width(drag.column, drag.width, cx);
            cx.notify();
        }
    }

    // ---- Keys ----

    /// Esc closes the topmost layer the shell owns (menus, dialogs and the switcher handle their
    /// own first), then a docked member sheet that has focus. Esc during a drag puts the
    /// remembered width back. Esc in the composer marks the channel read (the conversation view).
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key != "escape" || event.keystroke.modifiers.modified() {
            return;
        }
        if self.column_drag.take().is_some() {
            cx.stop_propagation();
            cx.notify();
        } else if self.is_member_overlay_open {
            cx.stop_propagation();
            self.close_member_overlay(window, cx);
        } else if self.is_navigation_drawer_open {
            cx.stop_propagation();
            self.close_navigation_drawer(window, cx);
        } else if self.member_sheet_focus.contains_focused(window, cx) {
            cx.stop_propagation();
            self.close_member_sheet(window, cx);
        }
    }

    // ---- Rendering ----

    /// A notice bar, read out when it appears, for a sign-out the instance did not confirm and a
    /// conversation that did not open; both show when both happened.
    fn notice_bar(&self, cx: &App) -> Option<impl IntoElement + use<>> {
        let tokens = theme(cx);
        let errors: Vec<String> = self
            .session
            .read(cx)
            .sign_out_error()
            .map(str::to_owned)
            .into_iter()
            .chain(self.conversation_error.clone())
            .collect();
        if errors.is_empty() {
            return None;
        }
        Some(
            div()
                .id("shell-notice-bar")
                .debug_selector(|| "shell-notice-bar".to_owned())
                .role(Role::Alert)
                .aria_label(errors.join(" "))
                .flex()
                .flex_col()
                .items_center()
                .min_h(length(COMPONENT_NOTICE_BAR_MIN_HEIGHT))
                .px(length(COMPONENT_NOTICE_BAR_PADDING_INLINE))
                .py(length(COMPONENT_NOTICE_BAR_PADDING_BLOCK))
                .bg(color(tokens.color_danger_wash))
                .children(errors.into_iter().map(|error| {
                    error_line(
                        error,
                        tokens.color_text_danger,
                        COMPONENT_NOTICE_BAR_ICON_SIZE,
                    )
                })),
        )
    }
}

/// The conversation the switcher asked for now exists: it opens through the switcher's `Open`
/// event, whose handler has the window to put focus on the conversation (the switcher's input
/// goes away with it). Nothing opens when the switcher was closed meanwhile.
fn open_conversation_chosen_in_switcher(
    switcher: &WeakEntity<QuickSwitcher>,
    conversation_id: Snowflake,
    cx: &mut App,
) {
    let _ = switcher.update(cx, |_, cx| {
        cx.emit(QuickSwitcherEvent::Open {
            guild_id: None,
            channel_id: Some(conversation_id),
        });
    });
}

/// A window x position in logical pixels at 100 % zoom, as the column widths are.
fn logical_x(x: Pixels, window: &Window) -> f32 {
    let viewport_width = f32::from(window.viewport_size().width);
    if viewport_width <= 0.0 {
        return f32::from(x);
    }
    f32::from(x) * window_width_at_default_zoom(window) / viewport_width
}

/// A picture drawn `size` logical pixels wide, as its still frame.
/// shortcut: animated avatars and icons play on hover on the web (`isPictureAnimationWanted`);
/// the desktop shows their still frame until the picture-animation setting lands.
fn picture(file: DownloadableFile, logical_side: f32, window: &Window) -> ImageSource {
    let device_side =
        f32::from(length(logical_side).to_pixels(window.rem_size())) * window.scale_factor();
    // A picture side is a few hundred device pixels: the cast keeps whole pixels, at least one.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let side = device_side.ceil().max(1.0) as u32;
    picture_source(PictureRequest {
        file: file.into(),
        animation: Animation::StillFrame,
        display_size: DisplaySize {
            width: side,
            height: side,
        },
    })
}

pub(crate) fn profile_picture(image: ProfileImage, size: f32, window: &Window) -> ImageSource {
    picture(DownloadableFile::ProfileImage(image.id), size, window)
}

fn icon_picture(image: ProfileImage, size: f32, window: &Window) -> ImageSource {
    picture(DownloadableFile::Icon(image.id), size, window)
}

impl Render for DuskShell {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let window_width = window_width_at_default_zoom(window);
        // Another server, or Home, starts an empty search (the field needs the window).
        let scope_guild_id = self.navigation.selected_guild_id();
        self.message_search.update(cx, |search, cx| {
            search.set_scope(scope_guild_id, window, cx);
        });
        let side_column = self.docked_side_column(window, cx);
        let layout = ShellLayout::for_window_width(window_width, side_column.is_some());
        // A drawer or overlay left open while the window widens would come back on the next
        // shrink; Home, or a server without channels, has no members to show.
        let is_drawer_obsolete =
            self.is_navigation_drawer_open && layout != ShellLayout::SheetAlone;
        let is_member_overlay_obsolete = self.is_member_overlay_open
            && (window_width >= BREAKPOINT_SIDE_SHEET || self.member_sheet_place_id(cx).is_none());
        if is_drawer_obsolete || is_member_overlay_obsolete {
            cx.defer_in(window, |shell, window, cx| {
                shell.close_obsolete_layers(window, cx);
            });
        }
        let is_full_bleed = layout == ShellLayout::SheetAlone;
        let is_side_docked = layout == ShellLayout::NavigatorSheetAndSideSheet;
        let side_column = side_column.filter(|_| is_side_docked);
        let columns = self.column_layout(window_width, side_column, cx);
        let is_search_overlay_open = self.is_search_overlay_open(window, cx);
        let inset = length(SIZE_SHEET_INSET);
        // The web shows twice the inset at the top and at the trailing end when nothing is docked
        // there (`Shell.svelte`); the owner validated that rhythm.
        let trailing_padding = if is_side_docked {
            SIZE_SHEET_INSET
        } else {
            2.0 * SIZE_SHEET_INSET
        };
        let banner = self.banner.read(cx);
        let notice_bar = self.notice_bar(cx);
        let is_notice_shown =
            banner.trouble().is_some() || banner.is_back_online_shown() || notice_bar.is_some();
        // The notices sit below the macOS title band, and the cards keep their gap below them.
        let top_padding = if is_full_bleed {
            0.0
        } else if is_notice_shown {
            2.0 * SIZE_SHEET_INSET
        } else {
            TOP_BAND_HEIGHT - TITLE_BAR_BAND_HEIGHT
        };

        let cards = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_row()
            .pt(length(top_padding))
            .when(!is_full_bleed, |ground| {
                ground
                    .pb(inset)
                    .pl(inset)
                    .pr(length(trailing_padding))
                    .gap(inset)
            })
            .when(!is_full_bleed, |ground| {
                ground.child(
                    div()
                        .relative()
                        .flex()
                        .flex_none()
                        .child(self.render_navigator(columns.channels, false, window, cx))
                        .child(self.render_resize_handle(
                            ResizableColumn::Channels,
                            columns.channels,
                            columns.channels_bounds,
                            window,
                            cx,
                        )),
                )
            })
            .child(self.render_conversation_sheet(layout, window, cx))
            .when_some(side_column.zip(columns.side), |ground, (column, width)| {
                let sheet = if column == ResizableColumn::Search {
                    self.message_search.update(cx, |search, cx| {
                        search.render_panel(true, Some(width), window, cx)
                    })
                } else {
                    self.render_member_sheet(Some(width), window, cx)
                        .into_any_element()
                };
                ground.child(div().relative().flex().flex_none().child(sheet).child(
                    self.render_resize_handle(column, width, columns.side_bounds, window, cx),
                ))
            });

        let root = div()
            .size_full()
            .relative()
            .flex()
            .flex_col()
            .bg(color(tokens.color_surface_rail))
            .font_family(SANS_FONT_FAMILY)
            .text_color(color(tokens.color_text_normal))
            .key_context(SHELL_KEY_CONTEXT)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_action(cx.listener(|shell, _: &PreviousChannel, window, cx| {
                shell.step_channel(Direction::Previous, false, window, cx);
            }))
            .on_action(cx.listener(|shell, _: &NextChannel, window, cx| {
                shell.step_channel(Direction::Next, false, window, cx);
            }))
            .on_action(cx.listener(|shell, _: &PreviousUnreadChannel, window, cx| {
                shell.step_channel(Direction::Previous, true, window, cx);
            }))
            .on_action(cx.listener(|shell, _: &NextUnreadChannel, window, cx| {
                shell.step_channel(Direction::Next, true, window, cx);
            }))
            .on_action(cx.listener(|shell, _: &PreviousServer, window, cx| {
                shell.step_server(Direction::Previous, window, cx);
            }))
            .on_action(cx.listener(|shell, _: &NextServer, window, cx| {
                shell.step_server(Direction::Next, window, cx);
            }))
            .on_action(cx.listener(|shell, _: &ToggleQuickSwitcher, window, cx| {
                shell.toggle_quick_switcher(window, cx);
            }))
            .on_action(cx.listener(|shell, _: &ToggleMemberSheet, window, cx| {
                shell.toggle_member_sheet(window, cx);
            }))
            .on_action(cx.listener(|shell, _: &OpenUserSettings, window, cx| {
                shell.open_user_settings(window, cx);
            }))
            .on_action(cx.listener(|shell, _: &FocusMessageSearch, window, cx| {
                shell.focus_message_search(window, cx);
            }))
            .on_action(
                cx.listener(|shell, action: &OpenDirectConversation, window, cx| {
                    shell.open_direct_conversation_with(action.user_id, window, cx);
                }),
            )
            .on_action(
                cx.listener(|shell, action: &OpenForwardOrigin, window, cx| {
                    shell.open_channel(action.channel_id, window, cx);
                    let (channel_id, message_id) = (action.channel_id, action.message_id);
                    shell.conversation.update(cx, |conversation, cx| {
                        conversation.reach_after_opening(channel_id, message_id, cx);
                    });
                }),
            )
            .on_action(cx.listener(|shell, action: &OpenPersonMenu, window, cx| {
                let request = action.request.clone();
                shell
                    .home
                    .update(cx, |home, cx| home.open_person_menu(request, window, cx));
            }))
            .on_action(cx.listener(|shell, _: &NextRegion, window, cx| {
                shell.move_to_region(Direction::Next, window, cx);
            }))
            .on_action(cx.listener(|shell, _: &PreviousRegion, window, cx| {
                shell.move_to_region(Direction::Previous, window, cx);
            }))
            .on_mouse_move(cx.listener(Self::follow_column_drag))
            .capture_any_mouse_up(cx.listener(|shell, _, _, cx| shell.end_column_drag(cx)))
            .when(TITLE_BAR_BAND_HEIGHT > 0.0, |root| {
                root.child(div().flex_none().h(length(TITLE_BAR_BAND_HEIGHT)))
            })
            .child(self.banner.clone())
            .children(notice_bar)
            .child(cards)
            .when(self.is_navigation_drawer_open, |root| {
                root.child(self.render_navigation_drawer(window, cx))
            })
            .when(self.is_member_overlay_open, |root| {
                root.child(self.render_member_overlay(is_full_bleed, window, cx))
            })
            .when(is_search_overlay_open, |root| {
                root.child(self.render_search_overlay(is_full_bleed, window, cx))
            })
            .children(self.render_settings_screen())
            .when_some(self.quick_switcher.as_ref(), |root, (switcher, _)| {
                root.child(switcher.clone())
            })
            .when_some(self.status_menu.as_ref(), |root, (menu, _)| {
                root.child(menu.clone())
            })
            .child(self.home.clone())
            .when_some(self.custom_status_dialog.as_ref(), |root, (dialog, _)| {
                root.child(dialog.clone())
            })
            .children(self.render_place_menus(window, cx));
        if self.regions.is_focus_pending() && self.is_navigation_drawer_open {
            cx.defer_in(window, |shell, window, cx| {
                shell.focus_pending_region(window, cx);
            });
        }
        with_tab_navigation(root, &self.root_focus)
    }
}

#[cfg(test)]
mod tests;
