//! The empty Dusk shell (DESIGN.md §4.2, ADR-0022): the dusk ground holding the navigator card
//! (server rail and channel column above the user card), the conversation sheet and the side
//! sheet. Every length and colour is a token; the panes are placeholders until their features come.

use gpui::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, Role,
    StatefulInteractiveElement as _, Styled, Window, div, prelude::FluentBuilder as _,
};

use crate::i18n::{Strings, strings};
use crate::theme::tokens::{
    COMPONENT_CHAT_HEADER_HEIGHT, COMPONENT_COMPOSER_MARGIN_BOTTOM,
    COMPONENT_COMPOSER_MARGIN_INLINE, COMPONENT_COMPOSER_MIN_HEIGHT, COMPONENT_COMPOSER_RADIUS,
    COMPONENT_NAVIGATOR_RADIUS, COMPONENT_NAVIGATOR_STRIP_WIDTH, COMPONENT_RAIL_ITEM_GAP,
    COMPONENT_RAIL_PADDING_TOP, COMPONENT_SHEET_RADIUS, COMPONENT_SIDE_SHEET_RADIUS,
    COMPONENT_SIDE_SHEET_WIDTH, COMPONENT_SIDEBAR_HEADER_HEIGHT, COMPONENT_SIDEBAR_WIDTH,
    COMPONENT_USER_PANEL_HEIGHT, COMPONENT_USER_PANEL_MARGIN, COMPONENT_USER_PANEL_RADIUS,
    SIZE_SHEET_INSET, ThemeTokens,
};
use crate::theme::{SANS_FONT_FAMILY, color, length, theme, window_width_at_default_zoom};
use crate::window_chrome::{TITLE_BAR_BAND_HEIGHT, TOP_BAND_HEIGHT};

/// Which cards fit beside each other at a window width (DESIGN.md §4.2 "Breakpoints").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellLayout {
    /// Below the navigator breakpoint: the sheet alone, full-bleed.
    SheetAlone,
    NavigatorAndSheet,
    /// From the side sheet breakpoint, an open side sheet docks beside the sheet.
    NavigatorSheetAndSideSheet,
}

impl ShellLayout {
    /// `width` is in logical pixels at 100 % zoom, so zooming in collapses the layout as the web
    /// client's rem-based media queries do (DESIGN.md §7.5).
    // shortcut: below its breakpoint the side sheet is hidden and below 816 the navigator is not
    // drawn; their overlay and drawer forms come with the navigation step (ADR-0051 order of work).
    #[must_use]
    pub fn for_window_width(width: f32, is_side_sheet_open: bool) -> Self {
        use crate::theme::tokens::{BREAKPOINT_NAVIGATOR, BREAKPOINT_SIDE_SHEET};
        if width < BREAKPOINT_NAVIGATOR {
            Self::SheetAlone
        } else if is_side_sheet_open && width >= BREAKPOINT_SIDE_SHEET {
            Self::NavigatorSheetAndSideSheet
        } else {
            Self::NavigatorAndSheet
        }
    }
}

#[derive(Debug)]
pub struct DuskShell {
    is_side_sheet_open: bool,
}

impl Default for DuskShell {
    fn default() -> Self {
        // Open so the skeleton shows every card; it becomes the remembered per-server choice.
        Self {
            is_side_sheet_open: true,
        }
    }
}

impl Render for DuskShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let layout = ShellLayout::for_window_width(
            window_width_at_default_zoom(window),
            self.is_side_sheet_open,
        );
        let is_full_bleed = layout == ShellLayout::SheetAlone;
        let inset = length(SIZE_SHEET_INSET);
        // The web shows twice the inset at the top and at the trailing end when nothing is docked
        // there (`Shell.svelte`); the owner validated that rhythm.
        let trailing_padding = if layout == ShellLayout::NavigatorSheetAndSideSheet {
            SIZE_SHEET_INSET
        } else {
            2.0 * SIZE_SHEET_INSET
        };

        div()
            .size_full()
            .flex()
            .flex_row()
            .bg(color(tokens.color_surface_rail))
            .font_family(SANS_FONT_FAMILY)
            .text_color(color(tokens.color_text_normal))
            .pt(length(if is_full_bleed {
                TITLE_BAR_BAND_HEIGHT
            } else {
                TOP_BAND_HEIGHT
            }))
            .when(!is_full_bleed, |ground| {
                ground
                    .pb(inset)
                    .pl(inset)
                    .pr(length(trailing_padding))
                    .gap(inset)
            })
            .when(!is_full_bleed, |ground| {
                ground.child(navigator(tokens, strings(cx)))
            })
            .child(sheet(tokens, is_full_bleed))
            .when(
                layout == ShellLayout::NavigatorSheetAndSideSheet,
                |ground| ground.child(side_sheet(tokens)),
            )
    }
}

fn navigator(tokens: &ThemeTokens, strings: Strings) -> impl IntoElement {
    let server_rail = div()
        .debug_selector(|| "server-rail".to_owned())
        .id("server-rail")
        .role(Role::Navigation)
        .aria_label(strings.rail().servers())
        .flex_none()
        .w(length(COMPONENT_NAVIGATOR_STRIP_WIDTH))
        .pt(length(COMPONENT_RAIL_PADDING_TOP))
        .flex()
        .flex_col()
        .items_center()
        .gap(length(COMPONENT_RAIL_ITEM_GAP));
    let channel_column = div()
        .debug_selector(|| "channel-column".to_owned())
        .flex_1()
        .flex()
        .flex_col()
        .child(div().flex_none().h(length(COMPONENT_SIDEBAR_HEADER_HEIGHT)));
    let user_card = div()
        .debug_selector(|| "user-card".to_owned())
        .flex_none()
        .h(length(COMPONENT_USER_PANEL_HEIGHT))
        .m(length(COMPONENT_USER_PANEL_MARGIN))
        .rounded(length(COMPONENT_USER_PANEL_RADIUS))
        .bg(color(tokens.component_user_panel_background));

    div()
        .debug_selector(|| "navigator".to_owned())
        .id("navigator")
        .role(Role::Navigation)
        .aria_label(strings.shell().servers_and_channels())
        .flex_none()
        .w(length(
            COMPONENT_NAVIGATOR_STRIP_WIDTH + COMPONENT_SIDEBAR_WIDTH,
        ))
        .flex()
        .flex_col()
        .bg(color(tokens.component_navigator_background))
        .rounded(length(COMPONENT_NAVIGATOR_RADIUS))
        .overflow_hidden()
        .child(
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_row()
                .child(server_rail)
                .child(channel_column),
        )
        .child(user_card)
}

fn sheet(tokens: &ThemeTokens, is_full_bleed: bool) -> impl IntoElement {
    let composer = div()
        .debug_selector(|| "composer".to_owned())
        .flex_none()
        .h(length(COMPONENT_COMPOSER_MIN_HEIGHT))
        .mx(length(COMPONENT_COMPOSER_MARGIN_INLINE))
        .mb(length(COMPONENT_COMPOSER_MARGIN_BOTTOM))
        .rounded(length(COMPONENT_COMPOSER_RADIUS))
        .bg(color(tokens.component_composer_background));

    div()
        .debug_selector(|| "sheet".to_owned())
        .id("sheet")
        .role(Role::Main)
        .flex_1()
        .min_w_0()
        .flex()
        .flex_col()
        .bg(color(tokens.color_surface_chat))
        .when(!is_full_bleed, |sheet| {
            sheet.rounded(length(COMPONENT_SHEET_RADIUS))
        })
        .overflow_hidden()
        .child(div().flex_none().h(length(COMPONENT_CHAT_HEADER_HEIGHT)))
        .child(div().flex_1().min_h_0())
        .child(composer)
}

fn side_sheet(tokens: &ThemeTokens) -> impl IntoElement {
    div()
        .debug_selector(|| "side-sheet".to_owned())
        .id("side-sheet")
        .role(Role::Complementary)
        .flex_none()
        .w(length(COMPONENT_SIDE_SHEET_WIDTH))
        .bg(color(tokens.component_side_sheet_background))
        .rounded(length(COMPONENT_SIDE_SHEET_RADIUS))
}

#[cfg(test)]
mod tests {
    use gpui::{Bounds, Pixels, TestAppContext, WindowAppearance, px, size};

    use super::*;
    use crate::theme::ActiveTheme;
    use crate::theme::tokens::{BREAKPOINT_NAVIGATOR, BREAKPOINT_SIDE_SHEET, SIZE_DOCK_HEIGHT};

    #[test]
    fn the_layout_follows_the_generated_breakpoints() {
        let below = |breakpoint: f32| breakpoint - 1.0;
        assert_eq!(
            ShellLayout::for_window_width(below(BREAKPOINT_NAVIGATOR), true),
            ShellLayout::SheetAlone
        );
        assert_eq!(
            ShellLayout::for_window_width(BREAKPOINT_NAVIGATOR, true),
            ShellLayout::NavigatorAndSheet
        );
        assert_eq!(
            ShellLayout::for_window_width(below(BREAKPOINT_SIDE_SHEET), true),
            ShellLayout::NavigatorAndSheet
        );
        assert_eq!(
            ShellLayout::for_window_width(BREAKPOINT_SIDE_SHEET, true),
            ShellLayout::NavigatorSheetAndSideSheet
        );
        assert_eq!(
            ShellLayout::for_window_width(BREAKPOINT_SIDE_SHEET, false),
            ShellLayout::NavigatorAndSheet
        );
    }

    fn bounds_of(cx: &mut gpui::VisualTestContext, selector: &'static str) -> Bounds<Pixels> {
        cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("`{selector}` was not drawn"))
    }

    #[gpui::test]
    fn the_shell_draws_its_cards_with_the_token_layout(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Dark)));
        let (_shell, cx) = cx.add_window_view(|_, _| DuskShell::default());
        cx.simulate_resize(size(px(1280.0), px(800.0)));
        cx.run_until_parked();

        let navigator = bounds_of(cx, "navigator");
        let sheet = bounds_of(cx, "sheet");
        let side_sheet = bounds_of(cx, "side-sheet");
        let inset = px(SIZE_SHEET_INSET);

        assert_eq!(navigator.origin.x, inset);
        assert_eq!(navigator.origin.y, px(TOP_BAND_HEIGHT));
        assert_eq!(
            navigator.size.width,
            px(COMPONENT_NAVIGATOR_STRIP_WIDTH + COMPONENT_SIDEBAR_WIDTH)
        );
        assert_eq!(sheet.origin.x, navigator.right() + inset);
        assert_eq!(side_sheet.origin.x, sheet.right() + inset);
        assert_eq!(side_sheet.size.width, px(COMPONENT_SIDE_SHEET_WIDTH));
        assert_eq!(side_sheet.right(), px(1280.0) - inset);
        assert_eq!(
            bounds_of(cx, "server-rail").size.width,
            px(COMPONENT_NAVIGATOR_STRIP_WIDTH)
        );

        // ADR-0022 §4: the user card and the composer's base row share their height and bottom.
        let user_card = bounds_of(cx, "user-card");
        let composer = bounds_of(cx, "composer");
        assert_eq!(user_card.size.height, px(SIZE_DOCK_HEIGHT));
        assert_eq!(composer.size.height, user_card.size.height);
        assert_eq!(composer.bottom(), user_card.bottom());
        assert_eq!(navigator.bottom(), px(800.0) - inset);
    }

    #[gpui::test]
    fn a_narrow_window_shows_the_sheet_alone_and_full_bleed(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_global(ActiveTheme::for_appearance(WindowAppearance::Light)));
        let (_shell, cx) = cx.add_window_view(|_, _| DuskShell::default());
        cx.simulate_resize(size(px(BREAKPOINT_NAVIGATOR - 1.0), px(600.0)));
        cx.run_until_parked();

        assert!(cx.debug_bounds("navigator").is_none());
        assert!(cx.debug_bounds("side-sheet").is_none());
        let sheet = bounds_of(cx, "sheet");
        assert_eq!(sheet.origin.x, px(0.0));
        assert_eq!(sheet.origin.y, px(TITLE_BAR_BAND_HEIGHT));
        assert_eq!(sheet.size.width, px(BREAKPOINT_NAVIGATOR - 1.0));
    }
}
