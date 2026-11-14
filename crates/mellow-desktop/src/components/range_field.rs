//! `RangeField.svelte` (DESIGN.md §6.7): a labelled slider, the label and the value on one line,
//! the track under them 44 px tall so it is easy to grab. Arrows, Page Up/Down, Home and End work
//! as the browser's range input; the value is read out as its text.

use std::sync::Arc;

use gpui::{
    App, Bounds, ElementId, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _, Pixels, RenderOnce, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, canvas, div,
    prelude::FluentBuilder as _,
};

use super::{child_id, focus_ring, is_focus_visible, remembered_focus_handle, with_type};
use crate::theme::tokens::{
    COMPONENT_BUTTON_DISABLED_OPACITY, RADIUS_SM, SIZE_HIT_WEB_MIN, SPACE_2XS, SPACE_MD, SPACE_XS,
    TYPE_BODY_SM, TYPE_CATEGORY,
};
use crate::theme::{color, length, theme};

type ChangeHandler = Arc<dyn Fn(f32, &mut Window, &mut App) + 'static>;
type ValueText = Arc<dyn Fn(f32) -> SharedString + 'static>;

// shortcut: the web draws the browser's own range input, so no slider tokens exist; the track and
// thumb take spacing tokens. Add `component.slider.track-*` and `thumb-*` tokens with the next
// token pass (design research §10 item 11).
/// The web's range input height (DESIGN.md §6.7), with a mouse too: the slider's drawn box, which
/// keeps the label, track and value spaced as on the web, not a touch minimum.
const SLIDER_HEIGHT: f32 = SIZE_HIT_WEB_MIN;
const TRACK_HEIGHT: f32 = SPACE_2XS;
const THUMB_SIZE: f32 = SPACE_MD;
/// Page Up and Page Down move a tenth of the range, as browsers do.
const PAGE_SHARE_OF_RANGE: f32 = 0.1;

/// The value bounds of a slider.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RangeBounds {
    pub minimum: f32,
    pub maximum: f32,
    pub step: f32,
}

impl RangeBounds {
    /// Snapped to the step from the minimum and kept inside the bounds.
    #[must_use]
    pub fn snapped(self, value: f32) -> f32 {
        let steps = ((value - self.minimum) / self.step).round();
        (self.minimum + steps * self.step).clamp(self.minimum, self.maximum)
    }

    /// The value a key moves to, or `None` for keys a slider does not handle.
    #[must_use]
    pub fn value_after_key(self, key: &str, value: f32) -> Option<f32> {
        let page = ((self.maximum - self.minimum) * PAGE_SHARE_OF_RANGE).max(self.step);
        let next = match key {
            "right" | "up" => value + self.step,
            "left" | "down" => value - self.step,
            "pageup" => value + page,
            "pagedown" => value - page,
            "home" => self.minimum,
            "end" => self.maximum,
            _ => return None,
        };
        Some(self.snapped(next))
    }

    /// The value under a pointer at `fraction` (0 to 1) of the track.
    #[must_use]
    pub fn value_at_fraction(self, fraction: f32) -> f32 {
        self.snapped(self.minimum + fraction.clamp(0.0, 1.0) * (self.maximum - self.minimum))
    }

    fn fraction_of(self, value: f32) -> f32 {
        let span = self.maximum - self.minimum;
        if span <= 0.0 {
            0.0
        } else {
            ((value - self.minimum) / span).clamp(0.0, 1.0)
        }
    }
}

#[derive(IntoElement)]
pub struct RangeField {
    id: ElementId,
    label: SharedString,
    value: f32,
    bounds: RangeBounds,
    value_text: ValueText,
    on_change: Option<ChangeHandler>,
    is_label_shown: bool,
    is_disabled: bool,
}

impl RangeField {
    #[must_use]
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        value: f32,
        bounds: RangeBounds,
        value_text: impl Fn(f32) -> SharedString + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            value,
            bounds,
            value_text: Arc::new(value_text),
            on_change: None,
            is_label_shown: true,
            is_disabled: false,
        }
    }

    /// The track alone, its label and value read out but not drawn (a player's seek bar).
    #[must_use]
    pub fn label_hidden(mut self) -> Self {
        self.is_label_shown = false;
        self
    }

    /// Dimmed, out of the Tab order and unmoved by the pointer, as a disabled range input (a
    /// zoom with nothing to zoom into).
    #[must_use]
    pub fn disabled(mut self, is_disabled: bool) -> Self {
        self.is_disabled = is_disabled;
        self
    }

    #[must_use]
    pub fn on_change(mut self, handler: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Arc::new(handler));
        self
    }
}

fn fraction_at(position_x: Pixels, track: Bounds<Pixels>) -> f32 {
    let width = f32::from(track.size.width);
    if width <= 0.0 {
        0.0
    } else {
        f32::from(position_x - track.origin.x) / width
    }
}

impl RenderOnce for RangeField {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let focus_handle = remembered_focus_handle(&self.id, window, cx);
        let track_bounds = window.use_keyed_state(child_id(&self.id, "track"), cx, |_, _| {
            Bounds::<Pixels>::default()
        });
        let is_dragging = window.use_keyed_state(child_id(&self.id, "drag"), cx, |_, _| false);
        let value_text = (self.value_text)(self.value);
        let fraction = self.bounds.fraction_of(self.value);
        let is_ring_shown = is_focus_visible(&focus_handle, window);
        let accent = color(tokens.component_slider_accent);
        let (bounds, value) = (self.bounds, self.value);
        let is_disabled = self.is_disabled;
        let on_change = self.on_change.filter(|_| !is_disabled);
        let (key_change, down_change, drag_change) =
            (on_change.clone(), on_change.clone(), on_change);
        let (down_track, paint_track) = (track_bounds.clone(), track_bounds);
        let (down_drag, paint_drag) = (is_dragging.clone(), is_dragging);
        let thumb_left = gpui::relative(fraction);

        let track = div()
            .id(child_id(&self.id, "slider"))
            .role(Role::Slider)
            .aria_label(self.label.clone())
            .aria_value(value_text.clone())
            .aria_numeric_value(f64::from(value))
            .aria_min_numeric_value(f64::from(bounds.minimum))
            .aria_max_numeric_value(f64::from(bounds.maximum))
            .aria_numeric_value_step(f64::from(bounds.step))
            .when(!is_disabled, |track| {
                track.track_focus(&focus_handle).cursor_pointer()
            })
            .when(is_disabled, |track| {
                track.opacity(COMPONENT_BUTTON_DISABLED_OPACITY)
            })
            .relative()
            .h(length(SLIDER_HEIGHT))
            .w_full()
            .rounded(length(RADIUS_SM))
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.modified() {
                    return;
                }
                if let Some(next) = bounds.value_after_key(&event.keystroke.key, value) {
                    cx.stop_propagation();
                    if let Some(on_change) = key_change.as_ref() {
                        on_change(next, window, cx);
                    }
                }
            })
            .on_mouse_down(
                MouseButton::Left,
                move |event: &MouseDownEvent, window, cx| {
                    down_drag.update(cx, |dragging, _| *dragging = true);
                    let fraction = fraction_at(event.position.x, *down_track.read(cx));
                    if let Some(on_change) = down_change.as_ref() {
                        on_change(bounds.value_at_fraction(fraction), window, cx);
                    }
                    // The next paint registers the window-level drag listeners.
                    window.refresh();
                },
            )
            .child(
                // Measures the track for the pointer and, while dragging, follows the pointer
                // across the whole window: element listeners stop once it leaves the track.
                canvas(
                    move |bounds, _, cx| paint_track.update(cx, |stored, _| *stored = bounds),
                    move |track, (), window, cx| {
                        if !*paint_drag.read(cx) {
                            return;
                        }
                        let (move_drag, up_drag) = (paint_drag.clone(), paint_drag);
                        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                            if !phase.bubble() || !*move_drag.read(cx) {
                                return;
                            }
                            if event.pressed_button != Some(MouseButton::Left) {
                                move_drag.update(cx, |dragging, _| *dragging = false);
                                return;
                            }
                            let fraction = fraction_at(event.position.x, track);
                            if let Some(on_change) = drag_change.as_ref() {
                                on_change(bounds.value_at_fraction(fraction), window, cx);
                            }
                        });
                        window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                            if phase.bubble() {
                                up_drag.update(cx, |dragging, _| *dragging = false);
                            }
                        });
                    },
                )
                .absolute()
                .inset_0(),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(length((SLIDER_HEIGHT - TRACK_HEIGHT) / 2.0))
                    .h(length(TRACK_HEIGHT))
                    .rounded_full()
                    .bg(color(tokens.color_border_strong))
                    .child(
                        div()
                            .h_full()
                            .w(gpui::relative(fraction))
                            .rounded_full()
                            .bg(accent),
                    ),
            )
            .child(
                // Centred on its value: the thumb's own half width is taken back.
                div()
                    .absolute()
                    .top(length((SLIDER_HEIGHT - THUMB_SIZE) / 2.0))
                    .left(thumb_left)
                    .ml(-length(THUMB_SIZE / 2.0))
                    .size(length(THUMB_SIZE))
                    .rounded_full()
                    .bg(accent),
            )
            .when(is_ring_shown, |track| {
                track.child(focus_ring(RADIUS_SM, cx))
            });

        if !self.is_label_shown {
            return div().flex().flex_col().min_w_0().child(track);
        }
        div()
            .flex()
            .flex_col()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .justify_between()
                    .gap(length(SPACE_XS))
                    .child(
                        with_type(div(), TYPE_CATEGORY)
                            .text_color(color(tokens.component_text_field_label_text))
                            .child(self.label),
                    )
                    .child(
                        with_type(div(), TYPE_BODY_SM)
                            .flex_none()
                            .text_color(color(tokens.component_slider_value_text))
                            .child(value_text),
                    ),
            )
            .child(track)
    }
}

debug_without_callbacks!(RangeField, id, label);

#[cfg(test)]
mod tests {
    use super::*;

    const VOLUME: RangeBounds = RangeBounds {
        minimum: 0.0,
        maximum: 200.0,
        step: 1.0,
    };

    #[test]
    fn keys_move_by_a_step_a_page_or_to_the_ends_and_stay_inside() {
        assert_eq!(VOLUME.value_after_key("right", 100.0), Some(101.0));
        assert_eq!(VOLUME.value_after_key("down", 100.0), Some(99.0));
        assert_eq!(VOLUME.value_after_key("pageup", 100.0), Some(120.0));
        assert_eq!(VOLUME.value_after_key("pagedown", 10.0), Some(0.0));
        assert_eq!(VOLUME.value_after_key("home", 100.0), Some(0.0));
        assert_eq!(VOLUME.value_after_key("end", 100.0), Some(200.0));
        assert_eq!(VOLUME.value_after_key("up", 200.0), Some(200.0));
        assert_eq!(VOLUME.value_after_key("a", 100.0), None);
    }

    #[test]
    fn the_pointer_picks_a_snapped_value_along_the_track() {
        let quarters = RangeBounds {
            minimum: 0.0,
            maximum: 1.0,
            step: 0.25,
        };
        assert!((quarters.value_at_fraction(0.3) - 0.25).abs() < f32::EPSILON);
        assert!((quarters.value_at_fraction(1.4) - 1.0).abs() < f32::EPSILON);
        assert!(quarters.value_at_fraction(-1.0).abs() < f32::EPSILON);
    }
}
