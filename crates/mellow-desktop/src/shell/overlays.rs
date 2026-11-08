//! The panels that slide over the sheet (`Drawer.svelte`, DESIGN.md §4.2, ADR-0022 §1): the
//! navigator from the leading edge below 816 px, the member sheet from the trailing edge below
//! 1064 px (across the whole window below 816 px). A scrim keeps the rest out of reach; Esc and the
//! scrim close them; Tab stays inside; focus goes back where it was.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, Context, InteractiveElement as _, IntoElement,
    KeyDownEvent, MouseButton, ParentElement as _, Role, StatefulInteractiveElement as _,
    Styled as _, Window, anchored, deferred, div, point, prelude::FluentBuilder as _, px,
};

use super::DuskShell;
use crate::components::{
    FocusNextControl, FocusPreviousControl, easing, focus_next_inside, focus_previous_inside,
    shadow,
};
use crate::i18n::strings;
use crate::theme::tokens::{
    COMPONENT_NAVIGATOR_STRIP_WIDTH, COMPONENT_SIDE_SHEET_ENTER_OFFSET, COMPONENT_SIDE_SHEET_WIDTH,
    COMPONENT_SIDEBAR_WIDTH, MOTION_DURATION_SLOW, MOTION_EASING_ENTER, SIZE_MOBILE_PANEL_PEEK,
    SIZE_SHEET_INSET,
};
use crate::theme::{animation_duration, color, length, theme, window_width_at_default_zoom};
use crate::window_chrome::TITLE_BAR_BAND_HEIGHT;

/// Which edge a panel slides in from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Edge {
    Leading,
    Trailing,
}

impl DuskShell {
    pub(super) fn render_navigation_drawer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        // The drawer leaves the sheet's peek visible, so on the narrowest windows the channel list
        // gives up width instead of being cut.
        let room = window_width_at_default_zoom(window)
            - SIZE_MOBILE_PANEL_PEEK
            - COMPONENT_NAVIGATOR_STRIP_WIDTH;
        let channels_width = COMPONENT_SIDEBAR_WIDTH.min(room.max(0.0));
        let navigator = self
            .render_navigator(channels_width, true, window, cx)
            .into_any_element();
        let label = strings(cx).shell().servers_and_channels();
        let panel = div()
            .h_full()
            .pt(length(TITLE_BAR_BAND_HEIGHT))
            .child(navigator)
            .into_any_element();
        self.overlay(
            "navigation-drawer",
            label.to_owned(),
            Edge::Leading,
            panel,
            window,
            cx,
        )
    }

    pub(super) fn render_member_overlay(
        &mut self,
        is_full_window: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tokens = theme(cx);
        let sheet = self
            .render_member_sheet(None, window, cx)
            .into_any_element();
        let label = self.member_sheet_label(cx);
        let panel = if is_full_window {
            div()
                .size_full()
                .pt(length(TITLE_BAR_BAND_HEIGHT))
                .bg(color(tokens.component_side_sheet_background))
                .child(sheet)
        } else {
            // Inside the conversation sheet's frame, as in the mockup.
            div()
                .h_full()
                .pt(length(TITLE_BAR_BAND_HEIGHT + 3.0 * SIZE_SHEET_INSET))
                .pb(length(2.0 * SIZE_SHEET_INSET))
                .pr(length(2.0 * SIZE_SHEET_INSET))
                .child(
                    div()
                        .h_full()
                        .w(length(COMPONENT_SIDE_SHEET_WIDTH))
                        .shadow(shadow(tokens.elevation_high))
                        .child(sheet),
                )
        };
        self.overlay(
            "member-overlay",
            label,
            Edge::Trailing,
            panel.into_any_element(),
            window,
            cx,
        )
    }

    fn overlay(
        &mut self,
        id: &'static str,
        label: String,
        edge: Edge,
        panel: AnyElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tokens = theme(cx);
        let viewport = window.viewport_size();
        let duration = animation_duration(MOTION_DURATION_SLOW, cx).max(Duration::from_millis(1));
        let travel = match edge {
            Edge::Leading => -COMPONENT_SIDE_SHEET_ENTER_OFFSET,
            Edge::Trailing => COMPONENT_SIDE_SHEET_ENTER_OFFSET,
        };
        let sliding_panel = div()
            .id(id)
            .role(Role::Dialog)
            .aria_label(label)
            .track_focus(&self.overlay_focus)
            .on_action(cx.listener(|shell, _: &FocusNextControl, window, cx| {
                focus_next_inside(&shell.overlay_focus, window, cx);
            }))
            .on_action(cx.listener(|shell, _: &FocusPreviousControl, window, cx| {
                focus_previous_inside(&shell.overlay_focus, window, cx);
            }))
            .on_key_down(cx.listener(|shell, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && !event.keystroke.modifiers.modified() {
                    cx.stop_propagation();
                    shell.close_navigation_drawer(window, cx);
                    shell.close_member_overlay(window, cx);
                }
            }))
            // Clicks on the panel stay on the panel; only the scrim closes.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .h_full()
            .relative()
            .child(panel)
            .with_animation(
                id,
                Animation::new(duration)
                    .with_easing(|progress| easing(MOTION_EASING_ENTER).sample(progress)),
                move |panel, progress| {
                    panel
                        .left(length(travel * (1.0 - progress)))
                        .opacity(progress)
                },
            );
        let scrim = div()
            .id("overlay-scrim")
            .occlude()
            .w(px(f32::from(viewport.width)))
            .h(px(f32::from(viewport.height)))
            .flex()
            .flex_row()
            .map(|scrim| match edge {
                Edge::Leading => scrim.justify_start(),
                Edge::Trailing => scrim.justify_end(),
            })
            .bg(color(tokens.color_scrim))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|shell, _, window, cx| {
                    shell.close_navigation_drawer(window, cx);
                    shell.close_member_overlay(window, cx);
                }),
            )
            .child(sliding_panel);
        deferred(anchored().position(point(px(0.0), px(0.0))).child(scrim)).with_priority(1)
    }
}
