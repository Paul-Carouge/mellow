//! `Switch.svelte` and its settings row (DESIGN.md §6.5, §5.9): the ink switch, a check on the knob
//! when on and a cross when off, so the state never relies on colour. By default it is the web's
//! `switchRow`: the label and its description, the switch at the trailing end, a divider under the
//! row, and the whole row flips it. A locked switch stays focusable, so its reason can be read on it.

use std::sync::Arc;

use gpui::{
    AnyElement, App, CursorStyle, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Toggled, Window, div, prelude::FluentBuilder as _,
};
use gpui_base::motion::{Transition, transition};

use super::{
    child_id, colour_transition, easing, focus_ring, icon, is_focus_visible,
    remembered_focus_handle, with_type,
};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_SWITCH_DISABLED_OPACITY, COMPONENT_SWITCH_GLYPH_SIZE, COMPONENT_SWITCH_HEIGHT,
    COMPONENT_SWITCH_KNOB_INSET, COMPONENT_SWITCH_KNOB_SIZE, COMPONENT_SWITCH_WIDTH,
    MOTION_DURATION_FAST, MOTION_EASING_STANDARD, SPACE_MD, SPACE_SM, TYPE_BODY, TYPE_CHANNEL_NAME,
};
use crate::theme::{animation_duration, color, length, theme};

type ChangeHandler = Arc<dyn Fn(bool, &mut Window, &mut App) + 'static>;

/// How much of the row the switch draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SwitchLayout {
    /// A settings row with a divider under it, as the web's stacked `switchRow`s.
    DividedRow,
    /// A row alone in a dialog or a popover, where a divider would separate nothing.
    Row,
    /// The track alone: the host draws the label in its own row (a role's permissions).
    ControlOnly,
}

#[derive(IntoElement)]
pub struct Switch {
    id: ElementId,
    is_on: bool,
    label: SharedString,
    description: Option<SharedString>,
    is_locked: bool,
    layout: SwitchLayout,
    on_change: Option<ChangeHandler>,
}

impl Switch {
    /// `label` is the setting's name, shown on the row and read as the switch's name.
    #[must_use]
    pub fn new(id: impl Into<ElementId>, is_on: bool, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            is_on,
            label: label.into(),
            description: None,
            is_locked: false,
            layout: SwitchLayout::DividedRow,
            on_change: None,
        }
    }

    /// The sentence under the label, read after the name.
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

    /// No divider under the row: a switch alone in a dialog or a popover.
    #[must_use]
    pub fn without_divider(mut self) -> Self {
        self.layout = SwitchLayout::Row;
        self
    }

    /// Only the track, for a host that lays out the label itself; the label and description stay
    /// the switch's accessible name and description.
    #[must_use]
    pub fn control_only(mut self) -> Self {
        self.layout = SwitchLayout::ControlOnly;
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

/// The 40 × 24 track and its knob, sliding over `motion.duration.fast`.
fn track(
    id: &ElementId,
    is_on: bool,
    is_locked: bool,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let tokens = theme(cx);
    let (track, knob, glyph, glyph_icon) = if is_on {
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
    let track = colour_transition(child_id(id, "track"), color(track), window, cx);
    let knob_offset = transition(
        child_id(id, "knob"),
        knob_offset(is_on),
        Transition::new(animation_duration(MOTION_DURATION_FAST, cx))
            .easing(easing(MOTION_EASING_STANDARD)),
        window,
        cx,
    );
    div()
        .relative()
        .flex_none()
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
        )
        .into_any_element()
}

impl RenderOnce for Switch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let focus_handle = remembered_focus_handle(&self.id, window, cx);
        let is_ring_shown = is_focus_visible(&focus_handle, window);
        let is_on = self.is_on;
        let is_locked = self.is_locked;
        let on_change = self.on_change;
        // The ring follows the track's pill, wherever the focus sits (the row or the track).
        let control = div()
            .relative()
            .flex_none()
            .child(track(&self.id, is_on, is_locked, window, cx))
            .when(is_ring_shown, |control| {
                control.child(focus_ring(COMPONENT_SWITCH_HEIGHT / 2.0, cx))
            });
        let label = self.label.clone();
        let description = self.description.clone();

        div()
            .id(self.id.clone())
            .role(Role::Switch)
            .aria_label(self.label)
            .when_some(self.description, |switch, description| {
                switch.aria_description(description)
            })
            .aria_toggled(if is_on { Toggled::True } else { Toggled::False })
            .track_focus(&focus_handle)
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
            .map(|switch| match self.layout {
                SwitchLayout::ControlOnly => switch.flex_none().child(control),
                SwitchLayout::Row | SwitchLayout::DividedRow => switch
                    .debug_selector(|| format!("switch-row-{}", self.id))
                    .flex()
                    .items_center()
                    .gap(length(SPACE_MD))
                    .py(length(SPACE_SM))
                    .when(self.layout == SwitchLayout::DividedRow, |row| {
                        row.border_b_1()
                            .border_color(color(tokens.color_border_divider))
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(
                                with_type(div(), TYPE_CHANNEL_NAME)
                                    .text_color(color(tokens.color_text_heading))
                                    .child(label),
                            )
                            .when_some(description, |text, description| {
                                text.child(
                                    with_type(div(), TYPE_BODY)
                                        .text_color(color(tokens.color_text_muted))
                                        .child(description),
                                )
                            }),
                    )
                    .child(control),
            })
    }
}

debug_without_callbacks!(Switch, id, label);
