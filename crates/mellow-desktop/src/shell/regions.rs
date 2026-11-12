//! Moving between the shell's regions and inside its lists by keyboard (DESIGN.md §7.2):
//! `F6` / `Shift+F6` go round server rail → channel list → message list → composer → side sheet,
//! skipping what is not there; in a narrow window entering the rail or the channel list opens the
//! drawer. The rail and the channel list are one Tab stop each (the focused row, else the
//! selected one, else the first); `Up`, `Down`, `Home` and `End` move inside them.
//!
//! shortcut: the member list keeps a Tab stop per row; its roving focus comes with the profiles
//! it opens (M3 step 4).

use gpui::{
    App, Context, ElementId, FocusHandle, KeyBinding, KeyDownEvent, ScrollHandle, Window, actions,
};

use super::{DuskShell, ShellLayout};
use crate::navigation::Direction;
use crate::theme::window_width_at_default_zoom;

actions!(
    mellow_shell,
    [
        /// F6: focus the next region.
        NextRegion,
        /// Shift+F6: focus the previous region.
        PreviousRegion
    ]
);

pub(super) fn bind_region_keys(cx: &mut App, context: Option<&str>) {
    cx.bind_keys([
        KeyBinding::new("f6", NextRegion, context),
        KeyBinding::new("shift-f6", PreviousRegion, context),
    ]);
}

/// The regions, in `F6` order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    ServerRail,
    ChannelList,
    MessageList,
    Composer,
    SideSheet,
}

const REGION_ORDER: [Region; 5] = [
    Region::ServerRail,
    Region::ChannelList,
    Region::MessageList,
    Region::Composer,
    Region::SideSheet,
];

/// The region `F6` (or `Shift+F6`) goes to from `current`, among those `is_available` keeps;
/// from outside every region, the first (or the last).
fn region_after(
    current: Option<Region>,
    direction: Direction,
    is_available: impl Fn(Region) -> bool,
) -> Option<Region> {
    let count = REGION_ORDER.len();
    let start = current.and_then(|region| REGION_ORDER.iter().position(|each| *each == region));
    (1..=count)
        .map(|step| {
            let index = match (start, direction) {
                (Some(start), Direction::Next) => (start + step) % count,
                (Some(start), Direction::Previous) => (start + count - step) % count,
                (None, Direction::Next) => step - 1,
                (None, Direction::Previous) => count - step,
            };
            REGION_ORDER[index]
        })
        .find(|region| Some(*region) != current && is_available(*region))
}

/// A focusable row of the rail or the channel list, as drawn last.
#[derive(Debug, Clone)]
pub(super) struct RovingRow {
    id: ElementId,
    focus: FocusHandle,
    is_tab_stop: bool,
    /// Its place among the list's children, to scroll it into view.
    child_index: usize,
}

/// Picks a list's one Tab stop while its rows are built: the row focused in the last frame, else
/// the selected one, else the first.
#[derive(Debug)]
pub(super) struct RovingTabStop {
    focused_id: Option<ElementId>,
    /// The caller has a selection, though maybe not among the rows drawn.
    has_selected_row: bool,
    /// False while the last frame drew no selected row: the first row stands in for it.
    may_use_selected_row: bool,
    has_drawn_selected_row: bool,
    rows: Vec<RovingRow>,
}

impl RovingTabStop {
    pub(super) fn new(
        previous_rows: &[RovingRow],
        has_selected_row: bool,
        may_use_selected_row: bool,
        window: &Window,
    ) -> Self {
        Self {
            focused_id: previous_rows
                .iter()
                .find(|row| row.focus.is_focused(window))
                .map(|row| row.id.clone()),
            has_selected_row,
            may_use_selected_row,
            has_drawn_selected_row: false,
            rows: Vec::new(),
        }
    }

    /// The row's focus handle, a Tab stop only when it is the list's.
    pub(super) fn row(
        &mut self,
        id: &ElementId,
        focus: FocusHandle,
        is_selected: bool,
        child_index: usize,
    ) -> FocusHandle {
        self.has_drawn_selected_row |= is_selected;
        let is_tab_stop = match &self.focused_id {
            Some(focused_id) => focused_id == id,
            None if self.has_selected_row && self.may_use_selected_row => is_selected,
            None => self.rows.is_empty(),
        };
        let focus = focus.tab_stop(is_tab_stop);
        self.rows.push(RovingRow {
            id: id.clone(),
            focus: focus.clone(),
            is_tab_stop,
            child_index,
        });
        focus
    }
}

/// A navigator list's focus: the list itself (for `F6`), its scroll and its rows as drawn last.
#[derive(Debug)]
pub(super) struct ListFocus {
    pub(super) focus: FocusHandle,
    pub(super) scroll: ScrollHandle,
    rows: Vec<RovingRow>,
    /// The last frame had a selection but drew no selected row (state ahead of navigation): the
    /// first row is the fallback until the selection is drawn, so the list is not redrawn forever.
    has_missed_selected_row: bool,
}

impl ListFocus {
    fn new(cx: &mut App) -> Self {
        Self {
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            rows: Vec::new(),
            has_missed_selected_row: false,
        }
    }

    pub(super) fn tab_stop_chooser(
        &self,
        has_selected_row: bool,
        window: &Window,
    ) -> RovingTabStop {
        RovingTabStop::new(
            &self.rows,
            has_selected_row,
            !self.has_missed_selected_row,
            window,
        )
    }

    /// Keeps the rows just built. When none became the Tab stop (the focused or selected row was
    /// not drawn) or the fallback changes, the list is drawn again, once, with the new fallback.
    pub(super) fn keep_rows(&mut self, chosen: RovingTabStop, cx: &mut Context<DuskShell>) {
        let has_tab_stop = chosen.rows.is_empty() || chosen.rows.iter().any(|row| row.is_tab_stop);
        let has_missed_selected_row = chosen.has_selected_row && !chosen.has_drawn_selected_row;
        let is_fallback_changed = has_missed_selected_row != self.has_missed_selected_row;
        self.rows = chosen.rows;
        self.has_missed_selected_row = has_missed_selected_row;
        if !has_tab_stop || is_fallback_changed {
            cx.notify();
        }
    }

    /// The row an arrow key moves to, scrolled into view.
    fn row_after_key(&self, key: &str, window: &Window) -> Option<FocusHandle> {
        let current = self
            .rows
            .iter()
            .position(|row| row.focus.is_focused(window))?;
        let row = &self.rows[row_after_key(key, current, self.rows.len())?];
        self.scroll.scroll_to_item(row.child_index);
        Some(row.focus.clone())
    }

    /// The list's Tab stop, scrolled into view; the list itself when it has no rows.
    fn tab_stop(&self) -> FocusHandle {
        match self.rows.iter().find(|row| row.is_tab_stop) {
            Some(row) => {
                self.scroll.scroll_to_item(row.child_index);
                row.focus.clone()
            }
            None => self.focus.clone(),
        }
    }
}

/// The navigator lists' focus, and the region `F6` opened the drawer for.
#[derive(Debug)]
pub(super) struct RegionFocus {
    pub(super) rail: ListFocus,
    pub(super) channel_list: ListFocus,
    pending: Option<Region>,
}

impl RegionFocus {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            rail: ListFocus::new(cx),
            channel_list: ListFocus::new(cx),
            pending: None,
        }
    }

    pub(super) fn is_focus_pending(&self) -> bool {
        self.pending.is_some()
    }

    fn list(&self, region: Region) -> &ListFocus {
        if region == Region::ServerRail {
            &self.rail
        } else {
            &self.channel_list
        }
    }
}

/// The row an arrow key or `Home`/`End` moves to from `current` among `count` rows; lists do not
/// wrap.
fn row_after_key(key: &str, current: usize, count: usize) -> Option<usize> {
    let last = count.checked_sub(1)?;
    match key {
        "up" => current.checked_sub(1),
        "down" => (current < last).then_some(current + 1),
        "home" => Some(0),
        "end" => Some(last),
        _ => None,
    }
}

impl DuskShell {
    /// `Up`, `Down`, `Home` and `End` on a row of the rail or the channel list.
    pub(super) fn move_in_list(
        &mut self,
        region: Region,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        if let Some(row) = self
            .regions
            .list(region)
            .row_after_key(&event.keystroke.key, window)
        {
            cx.stop_propagation();
            window.focus(&row, cx);
            cx.notify();
        }
    }

    /// The region holding focus, if any.
    pub fn focused_region(&self, window: &Window, cx: &App) -> Option<Region> {
        let conversation = self.conversation.read(cx);
        if self.regions.rail.focus.contains_focused(window, cx) {
            Some(Region::ServerRail)
        } else if self.regions.channel_list.focus.contains_focused(window, cx) {
            Some(Region::ChannelList)
        } else if conversation
            .message_list_focus()
            .contains_focused(window, cx)
        {
            Some(Region::MessageList)
        } else if conversation
            .composer()
            .read(cx)
            .field_focus(cx)
            .contains_focused(window, cx)
        {
            Some(Region::Composer)
        } else if self.member_sheet_focus.contains_focused(window, cx) {
            Some(Region::SideSheet)
        } else {
            None
        }
    }

    fn is_region_available(&self, region: Region, window: &Window, cx: &App) -> bool {
        let has_place = self.navigation.place().is_some();
        let has_conversation = self.navigation.selected_channel_id().is_some();
        match region {
            // In a narrow window they open in the drawer.
            Region::ServerRail | Region::ChannelList => has_place,
            Region::MessageList => has_conversation,
            Region::Composer => {
                has_conversation && self.conversation.read(cx).composer().read(cx).has_field()
            }
            Region::SideSheet => {
                let layout = ShellLayout::for_window_width(
                    window_width_at_default_zoom(window),
                    self.is_member_sheet_docked_open(cx),
                );
                self.is_member_overlay_open || layout == ShellLayout::NavigatorSheetAndSideSheet
            }
        }
    }

    /// `F6` and `Shift+F6`. Menus, dialogs, the quick switcher and the settings keep focus while
    /// open.
    pub(super) fn move_to_region(
        &mut self,
        direction: Direction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.quick_switcher.is_some()
            || self.status_menu.is_some()
            || self.is_settings_screen_open()
        {
            return;
        }
        let current = self.focused_region(window, cx);
        let Some(region) = region_after(current, direction, |region| {
            self.is_region_available(region, window, cx)
        }) else {
            return;
        };
        self.focus_region(region, window, cx);
    }

    fn focus_region(&mut self, region: Region, window: &mut Window, cx: &mut Context<Self>) {
        let is_navigator_region = matches!(region, Region::ServerRail | Region::ChannelList);
        // Leaving a layer for a region outside it: it closes, focus goes where F6 points.
        if self.is_member_overlay_open && region != Region::SideSheet {
            self.overlay_opener = None;
            self.close_member_overlay(window, cx);
        }
        if self.is_navigation_drawer_open && !is_navigator_region {
            self.overlay_opener = None;
            self.close_navigation_drawer(window, cx);
        }
        match region {
            Region::ServerRail | Region::ChannelList => {
                let is_navigator_hidden =
                    ShellLayout::for_window_width(window_width_at_default_zoom(window), false)
                        == ShellLayout::SheetAlone
                        && !self.is_navigation_drawer_open;
                if is_navigator_hidden {
                    // Its rows exist once the drawer is drawn.
                    self.open_navigation_drawer(window, cx);
                    self.regions.pending = Some(region);
                } else {
                    window.focus(&self.regions.list(region).tab_stop(), cx);
                }
            }
            Region::MessageList => self.conversation.update(cx, |conversation, cx| {
                conversation.focus_message_list(window, cx);
            }),
            Region::Composer => self.conversation.update(cx, |conversation, cx| {
                conversation.focus_composer(window, cx);
            }),
            Region::SideSheet => window.focus(&self.member_sheet_focus, cx),
        }
        cx.notify();
    }

    /// After a render that drew the drawer: the region F6 opened it for takes focus.
    pub(super) fn focus_pending_region(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(region) = self.regions.pending.take() {
            window.focus(&self.regions.list(region).tab_stop(), cx);
            cx.notify();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f6_goes_round_the_regions_in_order_skipping_missing_ones() {
        let without_side_sheet = |region: Region| region != Region::SideSheet;
        assert_eq!(
            region_after(None, Direction::Next, without_side_sheet),
            Some(Region::ServerRail)
        );
        assert_eq!(
            region_after(Some(Region::Composer), Direction::Next, without_side_sheet),
            Some(Region::ServerRail)
        );
        assert_eq!(
            region_after(
                Some(Region::ServerRail),
                Direction::Previous,
                without_side_sheet
            ),
            Some(Region::Composer)
        );
        assert_eq!(
            region_after(None, Direction::Previous, |_| true),
            Some(Region::SideSheet)
        );
        assert_eq!(
            region_after(Some(Region::ChannelList), Direction::Next, |region| {
                region == Region::ChannelList
            }),
            None
        );
    }

    #[test]
    fn arrows_move_inside_a_list_without_wrapping_and_home_and_end_reach_its_ends() {
        assert_eq!(row_after_key("down", 0, 3), Some(1));
        assert_eq!(row_after_key("down", 2, 3), None);
        assert_eq!(row_after_key("up", 0, 3), None);
        assert_eq!(row_after_key("up", 2, 3), Some(1));
        assert_eq!(row_after_key("home", 2, 3), Some(0));
        assert_eq!(row_after_key("end", 0, 3), Some(2));
        assert_eq!(row_after_key("left", 1, 3), None);
        assert_eq!(row_after_key("end", 0, 0), None);
    }
}
