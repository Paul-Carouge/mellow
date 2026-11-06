//! `ProgressMeter.svelte` (ADR-0047): the level of a checklist or a progress bar, a track filled
//! from its leading side in the poll's colour, and its figure. The fill glides to a new level over
//! `motion.duration.slow`, at once under reduced motion.

use gpui::{
    App, ElementId, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, Role, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    relative,
};
use gpui_base::motion::{Transition, transition};

use super::{child_id, easing, with_type};
use crate::theme::tokens::{
    COMPONENT_POLL_ANSWER_BORDER_WIDTH, COMPONENT_POLL_ANSWER_INNER_GAP,
    COMPONENT_POLL_MARK_GLYPH_SIZE, MOTION_DURATION_SLOW, MOTION_EASING_STANDARD, TYPE_BODY_SM,
};
use crate::theme::{animation_duration, color, length, theme};

#[derive(Debug, IntoElement)]
pub struct ProgressMeter {
    id: ElementId,
    label: SharedString,
    value: u32,
    maximum: u32,
    figure: SharedString,
    spoken: SharedString,
}

impl ProgressMeter {
    /// `figure` is shown ("3/5", "40 %"); `spoken` is what a screen reader says ("3 of 5 done").
    #[must_use]
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        value: u32,
        maximum: u32,
        figure: impl Into<SharedString>,
        spoken: impl Into<SharedString>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            value,
            maximum,
            figure: figure.into(),
            spoken: spoken.into(),
        }
    }
}

/// The filled share of the track, 0 to 1; an empty maximum shows an empty track.
#[must_use]
pub fn filled_share(value: u32, maximum: u32) -> f32 {
    if maximum == 0 {
        0.0
    } else {
        // Counts stay far below f32's exact integer range.
        #[allow(clippy::cast_precision_loss)]
        let share = value.min(maximum) as f32 / maximum as f32;
        share
    }
}

impl RenderOnce for ProgressMeter {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let share = transition(
            child_id(&self.id, "fill"),
            filled_share(self.value, self.maximum),
            Transition::new(animation_duration(MOTION_DURATION_SLOW, cx))
                .easing(easing(MOTION_EASING_STANDARD)),
            window,
            cx,
        );
        div()
            .flex()
            .items_center()
            .gap(length(COMPONENT_POLL_ANSWER_INNER_GAP))
            .child(
                div()
                    .id(self.id)
                    .role(Role::ProgressIndicator)
                    .aria_label(self.label)
                    .aria_value(self.spoken)
                    .aria_numeric_value(f64::from(self.value))
                    .aria_min_numeric_value(0.0)
                    .aria_max_numeric_value(f64::from(self.maximum))
                    .flex_1()
                    .min_w_0()
                    .h(length(COMPONENT_POLL_MARK_GLYPH_SIZE))
                    .rounded_full()
                    .overflow_hidden()
                    .border(length(COMPONENT_POLL_ANSWER_BORDER_WIDTH))
                    .border_color(color(tokens.component_poll_answer_border))
                    .bg(color(tokens.component_poll_answer_background))
                    .child(
                        div()
                            .h_full()
                            .w(relative(share))
                            .bg(color(tokens.component_poll_bar_mine)),
                    ),
            )
            .child(
                with_type(div(), TYPE_BODY_SM)
                    .flex_none()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(color(tokens.component_poll_percent_text))
                    .child(self.figure),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_share_stays_inside_the_track() {
        assert!((filled_share(3, 4) - 0.75).abs() < f32::EPSILON);
        assert!((filled_share(9, 4) - 1.0).abs() < f32::EPSILON);
        assert!(filled_share(1, 0).abs() < f32::EPSILON);
    }
}
