//! `ProgressMeter.svelte` (ADR-0047): the level of a checklist or a progress bar, a track filled
//! from its leading side with the lamp, and its figure. A checklist's level is segmented, one
//! segment per item, so "3 of 5" reads as three lit items. The fill glides to a new level over
//! `motion.duration.slow` and a segment lights over `motion.duration.base`, at once under reduced
//! motion.

use gpui::{
    App, ElementId, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, Role, SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    relative,
};
use gpui_base::motion::{Transition, transition};

use super::{child_id, easing, icon, with_type};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CHECKLIST_MARK_GLYPH_SIZE, COMPONENT_CHECKLIST_METER_HEIGHT,
    COMPONENT_CHECKLIST_METER_SEGMENT_GAP, COMPONENT_POLL_ANSWER_INNER_GAP, MOTION_DURATION_BASE,
    MOTION_DURATION_SLOW, MOTION_EASING_STANDARD, SPACE_3XS, TYPE_BODY,
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
    is_segmented: bool,
    complete_label: Option<SharedString>,
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
            is_segmented: false,
            complete_label: None,
        }
    }

    /// One segment per unit of `maximum` (a checklist's items) instead of one continuous fill.
    #[must_use]
    pub fn segmented(mut self) -> Self {
        self.is_segmented = true;
        self
    }

    /// Replaces the figure with a check and these words ("All done") once the level is full.
    #[must_use]
    pub fn complete_label(mut self, label: Option<SharedString>) -> Self {
        self.complete_label = label;
        self
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
        let fill = color(tokens.component_checklist_meter_fill);
        let track = color(tokens.component_checklist_meter_track);
        let level = div()
            .id(self.id.clone())
            .role(Role::ProgressIndicator)
            .aria_label(self.label)
            .aria_value(self.spoken)
            .aria_numeric_value(f64::from(self.value))
            .aria_min_numeric_value(0.0)
            .aria_max_numeric_value(f64::from(self.maximum))
            .flex_1()
            .min_w_0()
            .h(length(COMPONENT_CHECKLIST_METER_HEIGHT));
        let level = if self.is_segmented {
            let segment_policy = Transition::new(animation_duration(MOTION_DURATION_BASE, cx))
                .easing(easing(MOTION_EASING_STANDARD));
            let segments: Vec<_> = (0..self.maximum.max(1))
                .map(|index| {
                    let target = if index < self.value { fill } else { track };
                    let segment_id = ElementId::NamedChild(
                        std::sync::Arc::new(self.id.clone()),
                        SharedString::from(format!("segment-{index}")),
                    );
                    let tint = transition(segment_id, target, segment_policy.clone(), window, cx);
                    let is_lit = index < self.value;
                    div()
                        .debug_selector(move || {
                            format!(
                                "meter-segment-{index}-{}",
                                if is_lit { "lit" } else { "dim" }
                            )
                        })
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .rounded_full()
                        .bg(tint)
                })
                .collect();
            level
                .flex()
                .gap(length(COMPONENT_CHECKLIST_METER_SEGMENT_GAP))
                .children(segments)
        } else {
            let share = transition(
                child_id(&self.id, "fill"),
                filled_share(self.value, self.maximum),
                Transition::new(animation_duration(MOTION_DURATION_SLOW, cx))
                    .easing(easing(MOTION_EASING_STANDARD)),
                window,
                cx,
            );
            level
                .rounded_full()
                .overflow_hidden()
                .bg(track)
                .child(div().h_full().w(relative(share)).rounded_full().bg(fill))
        };
        let figure = match self.complete_label {
            Some(label) => {
                let done = color(tokens.component_checklist_done_text);
                with_type(div(), TYPE_BODY)
                    .debug_selector(|| "meter-complete".to_owned())
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(length(SPACE_3XS))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(done)
                    .child(icon(
                        IconName::CheckCircle,
                        COMPONENT_CHECKLIST_MARK_GLYPH_SIZE,
                        done,
                    ))
                    .child(label)
            }
            None => with_type(div(), TYPE_BODY)
                .flex_none()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(color(tokens.component_checklist_figure_text))
                .child(self.figure),
        };
        div()
            .flex()
            .items_center()
            .gap(length(COMPONENT_POLL_ANSWER_INNER_GAP))
            .child(level)
            .child(figure)
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
