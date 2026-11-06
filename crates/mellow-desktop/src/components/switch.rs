//! `Switch.svelte` (DESIGN.md §6.5): the ink switch, a check on the knob when on and a cross when
//! off, so the state never relies on colour. A locked switch stays focusable, so its row's reason
//! can be read on it.

use std::sync::Arc;

use gpui::{
    App, CursorStyle, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, Role, SharedString, StatefulInteractiveElement as _, Styled as _, Toggled, Window,
    div, prelude::FluentBuilder as _,
};
use gpui_base::motion::{Transition, transition};

use super::{
    child_id, colour_transition, easing, focus_ring, icon, is_focus_visible,
    remembered_focus_handle,
};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_SWITCH_DISABLED_OPACITY, COMPONENT_SWITCH_GLYPH_SIZE, COMPONENT_SWITCH_HEIGHT,
    COMPONENT_SWITCH_KNOB_INSET, COMPONENT_SWITCH_KNOB_SIZE, COMPONENT_SWITCH_WIDTH,
    MOTION_DURATION_FAST, MOTION_EASING_STANDARD, RADIUS_SM, SIZE_HIT_WEB_MIN,
};
use crate::theme::{animation_duration, color, length, theme};

type ChangeHandler = Arc<dyn Fn(bool, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    is_on: bool,
    label: SharedString,
    description: Option<SharedString>,
    is_locked: bool,
    on_change: Option<ChangeHandler>,
}

impl Switch {
    /// `label` is the setting's name, which the row also shows.
    #[must_use]
    pub fn new(id: impl Into<ElementId>, is_on: bool, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            is_on,
            label: label.into(),
            description: None,
            is_locked: false,
            on_change: None,
        }
    }

    #[must_use]
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    #[must_use]
    pub fn locked(mut self, is_locked: bool) -> Self {
        self.is_locked = is_locked;
        self
    }

    /// Receives the new state.
    #[must_use]
    pub fn on_change(mut self, handler: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Arc::new(handler));
        self
    }
}

/// Where the knob's leading edge sits: the inset when off, the far inset when on.
pub(crate) fn knob_offset(is_on: bool) -> f32 {
    if is_on {
        COMPONENT_SWITCH_WIDTH - COMPONENT_SWITCH_KNOB_SIZE - COMPONENT_SWITCH_KNOB_INSET
    } else {
        COMPONENT_SWITCH_KNOB_INSET
    }
}

impl RenderOnce for Switch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let focus_handle = remembered_focus_handle(&self.id, window, cx);
        let (track, knob, glyph, glyph_icon) = if self.is_on {
            (
                tokens.component_switch_track_on,
                tokens.component_switch_knob_on,
                tokens.component_switch_glyph_on,
                IconName::Check,
            )
        } else {
            (
                tokens.component_switch_track_off,
                tokens.component_switch_knob_off,
                tokens.component_switch_glyph_off,
                IconName::X,
            )
        };
        let track = colour_transition(child_id(&self.id, "track"), color(track), window, cx);
        let knob_offset = transition(
            child_id(&self.id, "knob"),
            knob_offset(self.is_on),
            Transition::new(animation_duration(MOTION_DURATION_FAST, cx))
                .easing(easing(MOTION_EASING_STANDARD)),
            window,
            cx,
        );
        let is_ring_shown = is_focus_visible(&focus_handle, window);
        let is_on = self.is_on;
        let is_locked = self.is_locked;
        let on_change = self.on_change;

        div()
            .id(self.id)
            .role(Role::Switch)
            .aria_label(self.label)
            .when_some(self.description, |switch, description| {
                switch.aria_description(description)
            })
            .aria_toggled(if is_on { Toggled::True } else { Toggled::False })
            .track_focus(&focus_handle)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(length(SIZE_HIT_WEB_MIN))
            .rounded(length(RADIUS_SM))
            .cursor(if is_locked {
                CursorStyle::OperationNotAllowed
            } else {
                CursorStyle::PointingHand
            })
            .on_click(move |_, window, cx| {
                if let Some(on_change) = on_change.as_ref().filter(|_| !is_locked) {
                    on_change(!is_on, window, cx);
                }
            })
            .child(
                div()
                    .relative()
                    .w(length(COMPONENT_SWITCH_WIDTH))
                    .h(length(COMPONENT_SWITCH_HEIGHT))
                    .rounded_full()
                    .bg(track)
                    .when(is_locked, |track| {
                        track.opacity(COMPONENT_SWITCH_DISABLED_OPACITY)
                    })
                    .child(
                        div()
                            .absolute()
                            .top(length(COMPONENT_SWITCH_KNOB_INSET))
                            .left(length(knob_offset))
                            .size(length(COMPONENT_SWITCH_KNOB_SIZE))
                            .rounded_full()
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(color(knob))
                            .child(icon(glyph_icon, COMPONENT_SWITCH_GLYPH_SIZE, color(glyph))),
                    ),
            )
            .when(is_ring_shown, |switch| {
                switch.child(focus_ring(RADIUS_SM, cx))
            })
    }
}

debug_without_callbacks!(Switch, id, label);
