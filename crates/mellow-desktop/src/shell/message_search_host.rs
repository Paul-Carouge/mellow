//! Where the shell puts message search (`Shell.svelte`, DESIGN.md 5.12): the field in the chat
//! header and the results docked in the side sheet's place where they fit, a magnifier and the
//! results over the conversation otherwise; Ctrl+F; and a chosen result opened in its channel.

use gpui::{App, Context, Entity, IntoElement, ParentElement as _, Styled as _, Window, div};
use mellow_protocol::Snowflake;

use super::{DuskShell, ResizableColumn, ShellLayout};
use crate::column_widths::{CHANNELS_COLUMN_BOUNDS, SEARCH_COLUMN_BOUNDS, WIDTH_BESIDE_COLUMNS};
use crate::components::shadow;
use crate::i18n::strings;
use crate::message_search::{FieldPlacement, MessageSearchEvent, MessageSearchView};
use crate::theme::tokens::{BREAKPOINT_SIDE_SHEET, SIZE_SHEET_INSET};
use crate::theme::{color, length, theme, window_width_at_default_zoom};
use crate::window_chrome::TITLE_BAR_BAND_HEIGHT;

/// Whether the results dock beside the conversation at this window width: past the side sheet's
/// breakpoint, with room for the channel list, the conversation and the results at their minimum.
fn has_room_for_search_sheet(window_width: f32) -> bool {
    // Past the side sheet's breakpoint too: the sum is the larger today, and both must hold.
    window_width
        >= BREAKPOINT_SIDE_SHEET.max(
            WIDTH_BESIDE_COLUMNS + CHANNELS_COLUMN_BOUNDS.minimum + SEARCH_COLUMN_BOUNDS.minimum,
        )
}

impl DuskShell {
    #[must_use]
    pub fn message_search(&self) -> &Entity<MessageSearchView> {
        &self.message_search
    }

    pub(super) fn has_room_for_search_sheet(window: &Window) -> bool {
        has_room_for_search_sheet(window_width_at_default_zoom(window))
    }

    /// The column docked beside the conversation: the results while open where they fit, else
    /// the members when left open and the window has room.
    pub(super) fn docked_side_column(&self, window: &Window, cx: &App) -> Option<ResizableColumn> {
        let window_width = window_width_at_default_zoom(window);
        if self.message_search.read(cx).is_panel_open() {
            return has_room_for_search_sheet(window_width).then_some(ResizableColumn::Search);
        }
        (ShellLayout::for_window_width(window_width, self.is_member_sheet_docked_open(cx))
            == ShellLayout::NavigatorSheetAndSideSheet)
            .then_some(ResizableColumn::Members)
    }

    pub(super) fn is_search_overlay_open(&self, window: &Window, cx: &App) -> bool {
        self.message_search.read(cx).is_panel_open() && !Self::has_room_for_search_sheet(window)
    }

    /// The header's field where the results dock, else its magnifier opening them over the
    /// conversation.
    pub(super) fn render_search_control(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if Self::has_room_for_search_sheet(window) {
            return self.message_search.update(cx, |search, cx| {
                search.render_field(FieldPlacement::Header, window, cx)
            });
        }
        let entity = cx.entity();
        let is_open = self.message_search.read(cx).is_panel_open();
        crate::components::IconButton::new(
            "open-search",
            crate::theme::icons::IconName::MagnifyingGlass,
            strings(cx).search().open_search(),
        )
        .sizes(
            crate::theme::tokens::COMPONENT_CHAT_HEADER_BUTTON_SIZE,
            crate::theme::tokens::COMPONENT_CHAT_HEADER_ICON_SIZE,
        )
        .expanded(is_open)
        .key_shortcuts(if cfg!(target_os = "macos") {
            "Meta+F"
        } else {
            "Control+F"
        })
        .on_click(move |window, cx| {
            entity.update(cx, |shell, cx| shell.open_search_overlay(window, cx));
        })
        .into_any_element()
    }

    /// Ctrl+F (Cmd+F), as on Discord: the header's field, or the results with their own field.
    pub(super) fn focus_message_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quick_switcher.is_some()
            || self.is_member_overlay_open
            || self.is_settings_screen_open()
        {
            return;
        }
        if Self::has_room_for_search_sheet(window) {
            self.message_search
                .update(cx, |search, cx| search.focus_field(window, cx));
        } else if self.is_search_overlay_open(window, cx) {
            self.message_search
                .update(cx, |search, cx| search.focus_field(window, cx));
        } else {
            self.open_search_overlay(window, cx);
        }
    }

    /// The results over the conversation, their field focused; focus comes back when they close.
    pub(super) fn open_search_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Closed first, so the overlay records the drawer's opener, not the drawer.
        self.close_navigation_drawer(window, cx);
        self.close_member_overlay(window, cx);
        self.open_overlay(window, cx);
        self.message_search.update(cx, |search, cx| {
            search.show_panel(cx);
            search.focus_field(window, cx);
        });
        cx.notify();
    }

    /// The results give the side sheet back; over the conversation, focus goes back too.
    pub(super) fn hide_search_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let was_overlay = self.is_search_overlay_open(window, cx);
        self.message_search
            .update(cx, MessageSearchView::hide_panel);
        if was_overlay {
            self.give_focus_back_after_overlay(window, cx);
        }
        cx.notify();
    }

    /// Esc or the scrim over the narrow window's results.
    pub(super) fn close_search_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_search_overlay_open(window, cx) {
            self.hide_search_panel(window, cx);
        }
    }

    pub(super) fn follow_message_search(
        &mut self,
        _: &Entity<MessageSearchView>,
        event: &MessageSearchEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            MessageSearchEvent::OpenResult {
                channel_id,
                message_id,
            } => self.open_search_result(*channel_id, *message_id, window, cx),
            MessageSearchEvent::OpenQuickSwitcher => {
                self.close_search_overlay(window, cx);
                if self.quick_switcher.is_none() {
                    self.toggle_quick_switcher(window, cx);
                }
            }
            MessageSearchEvent::Closed => {
                if self.overlay_opener.is_some() && !Self::has_room_for_search_sheet(window) {
                    self.give_focus_back_after_overlay(window, cx);
                } else {
                    self.conversation.update(cx, |conversation, cx| {
                        conversation.focus_composer(window, cx);
                    });
                }
                cx.notify();
            }
        }
    }

    /// Opens the result's channel at the message, older pages loading until it is there
    /// (DESIGN.md 5.12); a channel gone since the search is said in the results, which stay.
    fn open_search_result(
        &mut self,
        channel_id: Snowflake,
        message_id: Snowflake,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let scope_guild_id = self.message_search.read(cx).search().scope_guild_id();
        let state = self.session.read(cx).state();
        let is_reachable = match scope_guild_id {
            Some(guild_id) => state.guild(guild_id).is_some_and(|guild| {
                guild
                    .channels
                    .iter()
                    .any(|channel| channel.id == channel_id)
            }),
            None => state.direct_message_channel(channel_id).is_some(),
        };
        if !is_reachable {
            let search_strings = strings(cx).search();
            let notice = if scope_guild_id.is_none() {
                search_strings.conversation_gone()
            } else {
                search_strings.jump_missing()
            };
            self.message_search
                .update(cx, |search, cx| search.show_notice(notice.into(), cx));
            return;
        }
        // Over the conversation, the results step aside for the message.
        self.close_search_overlay(window, cx);
        match scope_guild_id {
            Some(guild_id) => self.open_place(Some(guild_id), Some(channel_id), window, cx),
            None => self.open_channel(channel_id, window, cx),
        }
        self.conversation.update(cx, |conversation, cx| {
            conversation.reach_after_opening(channel_id, message_id, cx);
        });
    }

    pub(super) fn render_search_overlay(
        &mut self,
        is_full_window: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tokens = theme(cx);
        let results = self.message_search.update(cx, |search, cx| {
            search.render_panel(false, None, window, cx)
        });
        let label = strings(cx).search().results().to_owned();
        let panel = if is_full_window {
            div()
                .size_full()
                .pt(length(TITLE_BAR_BAND_HEIGHT))
                .bg(color(tokens.component_side_sheet_background))
                .child(results)
        } else {
            // Inside the conversation sheet's frame, as the member overlay.
            div()
                .h_full()
                .pt(length(TITLE_BAR_BAND_HEIGHT + 3.0 * SIZE_SHEET_INSET))
                .pb(length(2.0 * SIZE_SHEET_INSET))
                .pr(length(2.0 * SIZE_SHEET_INSET))
                .child(
                    div()
                        .h_full()
                        .w(length(SEARCH_COLUMN_BOUNDS.minimum))
                        .max_w(length(
                            window_width_at_default_zoom(window) - 4.0 * SIZE_SHEET_INSET,
                        ))
                        .shadow(shadow(tokens.elevation_high))
                        .child(results),
                )
        };
        self.overlay_with_trailing_edge(
            "search-overlay",
            label,
            panel.into_any_element(),
            window,
            cx,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_results_dock_only_where_the_channel_list_conversation_and_results_fit() {
        assert!(!has_room_for_search_sheet(BREAKPOINT_SIDE_SHEET - 1.0));
        let enough =
            WIDTH_BESIDE_COLUMNS + CHANNELS_COLUMN_BOUNDS.minimum + SEARCH_COLUMN_BOUNDS.minimum;
        assert!(has_room_for_search_sheet(enough.max(BREAKPOINT_SIDE_SHEET)));
        assert!(!has_room_for_search_sheet(enough - 1.0));
    }
}
