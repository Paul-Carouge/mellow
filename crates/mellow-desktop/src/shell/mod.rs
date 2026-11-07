//! The signed-in Dusk shell (DESIGN.md §4.2, ADR-0022, `docs/design/desktop-shell-and-navigation.md`):
//! the dusk ground holding the navigator card (server rail and channel list above the user card),
//! the conversation sheet and the member side sheet, with the connection banner above them. It
//! follows the session's client state, keeps where the reader is (`Navigation`) and remembers it
//! with the column widths and member sheets in the preferences. Below their breakpoints the
//! navigator becomes a drawer and the member sheet an overlay. Every length and colour is a token.

mod channel_sidebar;
mod conversation_sheet;
mod member_sheet;
mod overlays;
pub mod quick_switcher;
mod resize_handle;
mod server_rail;
mod user_card;

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

use gpui::{
    App, AppContext as _, Bounds, Context, Entity, FocusHandle, ImageSource,
    InteractiveElement as _, IntoElement, KeyBinding, KeyDownEvent, MouseMoveEvent,
    ParentElement as _, Pixels, Render, Styled as _, Subscription, Task, Window, actions, div,
    prelude::FluentBuilder as _,
};
use mellow_client_core::DownloadableFile;
use mellow_client_core::state::{ClientState, StateChange};
use mellow_protocol::{ProfileImage, Snowflake};

use crate::column_widths::{
    CHANNELS_COLUMN_BOUNDS, ColumnBounds, MEMBERS_COLUMN_BOUNDS, WIDTH_BESIDE_COLUMNS,
    clamp_column_width, fit_column_widths, largest_column_width, stored_column_width,
};
use crate::components::{ContextMenu, with_tab_navigation};
use crate::connection_banner::ConnectionBanner;
use crate::local_widgets::error_line;
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
        ToggleMemberSheet
    ]
);

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
    ]);
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
}

impl ResizableColumn {
    fn bounds(self) -> ColumnBounds {
        match self {
            Self::Channels => CHANNELS_COLUMN_BOUNDS,
            Self::Members => MEMBERS_COLUMN_BOUNDS,
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
    members: Option<f32>,
    channels_bounds: ColumnBounds,
    members_bounds: ColumnBounds,
}

pub struct DuskShell {
    session: Entity<Session>,
    banner: Entity<ConnectionBanner>,
    navigation: Navigation,
    /// Kept while the shell lives, so collapsing survives the drawer closing (as on the web).
    collapsed_category_ids: HashSet<Snowflake>,
    is_navigation_drawer_open: bool,
    /// The member sheet over the conversation, below the side sheet breakpoint.
    is_member_overlay_open: bool,
    column_drag: Option<ColumnDrag>,
    quick_switcher: Option<(Entity<QuickSwitcher>, Subscription)>,
    status_menu: Option<(Entity<ContextMenu>, Subscription)>,
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
        let subscriptions = vec![
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
            navigation: Navigation::default(),
            collapsed_category_ids: HashSet::new(),
            is_navigation_drawer_open: false,
            is_member_overlay_open: false,
            column_drag: None,
            quick_switcher: None,
            status_menu: None,
            user_card_bounds: Rc::new(Cell::new(None)),
            is_user_id_copied: false,
            copied_label_timer: None,
            overlay_opener: None,
            overlay_focus: cx.focus_handle(),
            member_sheet_focus: cx.focus_handle(),
            status_button_focus: cx.focus_handle().tab_stop(true),
            root_focus,
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
        cx.notify();
    }

    /// The place changed: remembered for the next launch, and the members asked for.
    fn after_navigation(&mut self, cx: &mut Context<Self>) {
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
        cx.notify();
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
        let state = self.state(cx);
        if let Some(channel_id) = self
            .navigation
            .adjacent_channel(state, direction, only_unread)
        {
            self.open_channel(channel_id, window, cx);
        }
    }

    fn step_server(&mut self, direction: Direction, window: &mut Window, cx: &mut Context<Self>) {
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

    /// The server whose members the side sheet shows: the open one, with a channel open.
    /// shortcut: a group conversation's member sheet (`GroupMembersSheet.svelte`) comes with the
    /// conversations step (M3 step 4).
    fn member_sheet_place_id(&self) -> Option<Snowflake> {
        match self.navigation.place()? {
            Place::Server {
                guild_id,
                channel_id: Some(_),
            } => Some(guild_id),
            _ => None,
        }
    }

    /// Whether the member sheet is left open for this server (remembered per server).
    fn is_member_sheet_docked_open(&self, cx: &App) -> bool {
        self.member_sheet_place_id().is_some_and(|place_id| {
            preferences(cx)
                .is_some_and(|preferences| preferences.member_sheet_place_ids.contains(&place_id))
        })
    }

    /// Whether the header's toggle shows the sheet as open, docked or over the conversation.
    fn is_member_sheet_shown(&self, window: &Window, cx: &App) -> bool {
        if window_width_at_default_zoom(window) >= BREAKPOINT_SIDE_SHEET {
            self.is_member_sheet_docked_open(cx)
        } else {
            self.is_member_overlay_open
        }
    }

    /// The header's toggle and Ctrl+U: docked where it fits, over the sheet otherwise.
    fn toggle_member_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(place_id) = self.member_sheet_place_id() else {
            return;
        };
        if self.quick_switcher.is_some() {
            return;
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
            self.is_navigation_drawer_open = false;
            self.open_overlay(window, cx);
            self.is_member_overlay_open = true;
        }
        cx.notify();
    }

    fn close_member_sheet(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_member_overlay_open {
            self.close_member_overlay(window, cx);
        } else if let Some(place_id) = self.member_sheet_place_id() {
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

    // ---- The quick switcher ----

    /// Ctrl+K and the header's "Go to…": opens it, or closes it when open.
    pub fn toggle_quick_switcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((switcher, _)) = self.quick_switcher.take() {
            switcher.update(cx, |switcher, cx| switcher.give_focus_back(window, cx));
            cx.notify();
            return;
        }
        if self.is_member_overlay_open {
            return;
        }
        self.is_navigation_drawer_open = false;
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
                // After a result, the conversation takes focus rather than the opener.
                window.focus(&self.root_focus, cx);
            }
            QuickSwitcherEvent::OpenConversationWith { user_id, failure } => {
                let shell = cx.entity().downgrade();
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
                                let _ = shell.update(cx, |shell, cx| {
                                    shell.quick_switcher = None;
                                    let state = shell.session.read(cx).state();
                                    shell.navigation.select_guild(
                                        state,
                                        None,
                                        Some(conversation_id),
                                    );
                                    shell.after_navigation(cx);
                                });
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

    // ---- Column widths ----

    /// The widths to lay out at this window width: the reader's (or the drag's), fitted so the
    /// conversation keeps its minimum.
    fn column_layout(&self, window_width: f32, is_members_docked: bool, cx: &App) -> ColumnLayout {
        let stored = preferences(cx);
        let width_of =
            |column: ResizableColumn| match self.column_drag.filter(|drag| drag.column == column) {
                Some(drag) => drag.width,
                None => stored_column_width(
                    stored.and_then(|preferences| match column {
                        ResizableColumn::Channels => preferences.channels_column_width,
                        ResizableColumn::Members => preferences.members_column_width,
                    }),
                    column.bounds(),
                ),
            };
        let room = window_width - WIDTH_BESIDE_COLUMNS;
        let (channels, members) = fit_column_widths(
            room,
            (width_of(ResizableColumn::Channels), CHANNELS_COLUMN_BOUNDS),
            is_members_docked.then(|| (width_of(ResizableColumn::Members), MEMBERS_COLUMN_BOUNDS)),
        );
        let members_minimum = if is_members_docked {
            MEMBERS_COLUMN_BOUNDS.minimum
        } else {
            0.0
        };
        ColumnLayout {
            channels,
            members,
            channels_bounds: ColumnBounds {
                maximum: largest_column_width(CHANNELS_COLUMN_BOUNDS, room - members_minimum),
                ..CHANNELS_COLUMN_BOUNDS
            },
            members_bounds: ColumnBounds {
                maximum: largest_column_width(MEMBERS_COLUMN_BOUNDS, room - channels),
                ..MEMBERS_COLUMN_BOUNDS
            },
        }
    }

    fn remember_column_width(column: ResizableColumn, width: f32, cx: &mut App) {
        change_preferences(cx, move |preferences| match column {
            ResizableColumn::Channels => preferences.channels_column_width = Some(width),
            ResizableColumn::Members => preferences.members_column_width = Some(width),
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
        let layout = self.column_layout(window_width, drag.column == ResizableColumn::Members, cx);
        let bounds = match drag.column {
            ResizableColumn::Channels => layout.channels_bounds,
            ResizableColumn::Members => layout.members_bounds,
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
    /// remembered width back.
    /// shortcut: Esc does not mark the channel as read yet (DESIGN.md §7.2); that comes with the
    /// message list.
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

    /// A notice bar for a sign-out the instance did not confirm.
    fn sign_out_error_bar(&self, cx: &App) -> Option<impl IntoElement + use<>> {
        let tokens = theme(cx);
        let error = self.session.read(cx).sign_out_error()?.to_owned();
        Some(
            div()
                .flex()
                .justify_center()
                .min_h(length(COMPONENT_NOTICE_BAR_MIN_HEIGHT))
                .px(length(COMPONENT_NOTICE_BAR_PADDING_INLINE))
                .py(length(COMPONENT_NOTICE_BAR_PADDING_BLOCK))
                .bg(color(tokens.color_danger_wash))
                .child(error_line(
                    error,
                    tokens.color_text_danger,
                    COMPONENT_NOTICE_BAR_ICON_SIZE,
                )),
        )
    }
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
        file,
        animation: Animation::StillFrame,
        display_size: DisplaySize {
            width: side,
            height: side,
        },
    })
}

fn profile_picture(image: ProfileImage, size: f32, window: &Window) -> ImageSource {
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
        let layout =
            ShellLayout::for_window_width(window_width, self.is_member_sheet_docked_open(cx));
        // A drawer or overlay left open while the window widens would come back on the next
        // shrink; Home, or a server without channels, has no members to show.
        if layout != ShellLayout::SheetAlone && self.is_navigation_drawer_open {
            self.is_navigation_drawer_open = false;
        }
        if window_width >= BREAKPOINT_SIDE_SHEET || self.member_sheet_place_id().is_none() {
            self.is_member_overlay_open = false;
        }
        let is_full_bleed = layout == ShellLayout::SheetAlone;
        let is_members_docked = layout == ShellLayout::NavigatorSheetAndSideSheet;
        let columns = self.column_layout(window_width, is_members_docked, cx);
        let inset = length(SIZE_SHEET_INSET);
        // The web shows twice the inset at the top and at the trailing end when nothing is docked
        // there (`Shell.svelte`); the owner validated that rhythm.
        let trailing_padding = if is_members_docked {
            SIZE_SHEET_INSET
        } else {
            2.0 * SIZE_SHEET_INSET
        };
        let banner = self.banner.read(cx);
        let sign_out_error_bar = self.sign_out_error_bar(cx);
        let is_notice_shown = banner.trouble().is_some()
            || banner.is_back_online_shown()
            || sign_out_error_bar.is_some();
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
            .when_some(
                columns.members.filter(|_| is_members_docked),
                |ground, width| {
                    ground.child(
                        div()
                            .relative()
                            .flex()
                            .flex_none()
                            .child(self.render_member_sheet(Some(width), window, cx))
                            .child(self.render_resize_handle(
                                ResizableColumn::Members,
                                width,
                                columns.members_bounds,
                                window,
                                cx,
                            )),
                    )
                },
            );

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
            .on_mouse_move(cx.listener(Self::follow_column_drag))
            .capture_any_mouse_up(cx.listener(|shell, _, _, cx| shell.end_column_drag(cx)))
            .when(TITLE_BAR_BAND_HEIGHT > 0.0, |root| {
                root.child(div().flex_none().h(length(TITLE_BAR_BAND_HEIGHT)))
            })
            .child(self.banner.clone())
            .children(sign_out_error_bar)
            .child(cards)
            .when(self.is_navigation_drawer_open, |root| {
                root.child(self.render_navigation_drawer(window, cx))
            })
            .when(self.is_member_overlay_open, |root| {
                root.child(self.render_member_overlay(is_full_bleed, window, cx))
            })
            .when_some(self.quick_switcher.as_ref(), |root, (switcher, _)| {
                root.child(switcher.clone())
            })
            .when_some(self.status_menu.as_ref(), |root, (menu, _)| {
                root.child(menu.clone())
            });
        with_tab_navigation(root, &self.root_focus)
    }
}

#[cfg(test)]
mod tests;
