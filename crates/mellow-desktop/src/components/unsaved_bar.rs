//! `UnsavedChangesBar.svelte` (DESIGN.md §5.8): rises at the bottom of a settings page while a form
//! differs from what is stored. Leaving then is refused: the bar shakes (not under reduced motion),
//! turns red-edged and asks to save or reset first.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, App, ElementId, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _,
};

use super::{ActivationHandler, Button, ButtonVariant, child_id, easing, icon, shadow, with_type};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_SETTINGS_ERROR_ICON_GAP, COMPONENT_SETTINGS_ERROR_ICON_SIZE,
    COMPONENT_UNSAVED_BAR_BORDER_ATTENTION_WIDTH, COMPONENT_UNSAVED_BAR_BORDER_WIDTH,
    COMPONENT_UNSAVED_BAR_BUTTON_GAP, COMPONENT_UNSAVED_BAR_MARGIN_BOTTOM,
    COMPONENT_UNSAVED_BAR_PADDING_BLOCK, COMPONENT_UNSAVED_BAR_PADDING_LEADING,
    COMPONENT_UNSAVED_BAR_PADDING_TRAILING, COMPONENT_UNSAVED_BAR_RADIUS,
    COMPONENT_UNSAVED_BAR_SHAKE_DISTANCE, COMPONENT_UNSAVED_BAR_SHAKE_STEP_DURATION,
    COMPONENT_UNSAVED_BAR_SHAKE_SWING_COUNT, COMPONENT_UNSAVED_BAR_SLIDE_DURATION,
    MOTION_EASING_ENTER, SPACE_XS, TYPE_BODY,
};
use crate::theme::{color, length, theme};

/// The `mw-rise` distance the bar comes up from.
const RISE_DISTANCE: f32 = SPACE_XS;

/// The bar's horizontal offset at `progress` (0 to 1) of a shake: swings alternating sides, each
/// smaller than the last, from and back to rest (the web's keyframes, linearly between them).
#[must_use]
pub fn shake_offset(progress: f32, distance: f32, swing_count: usize) -> f32 {
    if swing_count == 0 {
        return 0.0;
    }
    // Counts here are a handful of swings.
    #[allow(clippy::cast_precision_loss)]
    let swings = swing_count as f32;
    let keyframe = |index: usize| -> f32 {
        if index == 0 || index > swing_count {
            return 0.0;
        }
        let swing = index - 1;
        #[allow(clippy::cast_precision_loss)]
        let fade = 1.0 - swing as f32 / swings;
        let side = if swing.is_multiple_of(2) { 1.0 } else { -1.0 };
        side * distance * fade
    };
    let segments = swing_count + 1;
    #[allow(clippy::cast_precision_loss)]
    let position = progress.clamp(0.0, 1.0) * segments as f32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let segment = (position.floor() as usize).min(segments - 1);
    #[allow(clippy::cast_precision_loss)]
    let within = position - segment as f32;
    keyframe(segment) + (keyframe(segment + 1) - keyframe(segment)) * within
}

/// The bar's words: all of them come from the interface strings of the host.
#[derive(Debug, Clone)]
pub struct UnsavedChangesLabels {
    pub careful: SharedString,
    pub save_or_reset: SharedString,
    pub reset: SharedString,
    pub save_changes: SharedString,
    pub saving: SharedString,
}

#[derive(IntoElement)]
pub struct UnsavedChangesBar {
    id: ElementId,
    /// Raised by each refused attempt to leave; 0 until the first.
    attention: u32,
    is_saving: bool,
    is_save_enabled: bool,
    error_message: Option<SharedString>,
    labels: UnsavedChangesLabels,
    on_reset: ActivationHandler,
    on_save: ActivationHandler,
}

impl UnsavedChangesBar {
    #[must_use]
    pub fn new(
        id: impl Into<ElementId>,
        labels: UnsavedChangesLabels,
        on_reset: ActivationHandler,
        on_save: ActivationHandler,
    ) -> Self {
        Self {
            id: id.into(),
            attention: 0,
            is_saving: false,
            is_save_enabled: true,
            error_message: None,
            labels,
            on_reset,
            on_save,
        }
    }

    #[must_use]
    pub fn attention(mut self, attention: u32) -> Self {
        self.attention = attention;
        self
    }

    #[must_use]
    pub fn saving(mut self, is_saving: bool) -> Self {
        self.is_saving = is_saving;
        self
    }

    #[must_use]
    pub fn save_enabled(mut self, is_save_enabled: bool) -> Self {
        self.is_save_enabled = is_save_enabled;
        self
    }

    #[must_use]
    pub fn error_message(mut self, error_message: impl Into<SharedString>) -> Self {
        self.error_message = Some(error_message.into());
        self
    }
}

impl RenderOnce for UnsavedChangesBar {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let is_asking = self.attention > 0;
        let message = match self.error_message {
            Some(error) => with_type(div(), TYPE_BODY)
                .flex()
                .gap(length(COMPONENT_SETTINGS_ERROR_ICON_GAP))
                .text_color(color(tokens.component_settings_error_text))
                .child(icon(
                    IconName::WarningCircle,
                    COMPONENT_SETTINGS_ERROR_ICON_SIZE,
                    color(tokens.component_settings_error_icon),
                ))
                .child(error),
            None => with_type(div(), TYPE_BODY)
                .text_color(color(tokens.component_unsaved_bar_text))
                .child(if is_asking {
                    self.labels.save_or_reset.clone()
                } else {
                    self.labels.careful.clone()
                }),
        };
        let on_reset = self.on_reset;
        let on_save = self.on_save;
        let bar = div()
            .id(child_id(&self.id, "bar"))
            .role(Role::Alert)
            .relative()
            .flex()
            .flex_wrap()
            .items_center()
            .justify_end()
            .gap(length(COMPONENT_UNSAVED_BAR_BUTTON_GAP))
            .py(length(COMPONENT_UNSAVED_BAR_PADDING_BLOCK))
            .pl(length(COMPONENT_UNSAVED_BAR_PADDING_LEADING))
            .pr(length(COMPONENT_UNSAVED_BAR_PADDING_TRAILING))
            .rounded(length(COMPONENT_UNSAVED_BAR_RADIUS))
            .bg(color(tokens.component_unsaved_bar_background))
            .shadow(shadow(tokens.elevation_high))
            .when(is_asking, |bar| {
                bar.border(length(COMPONENT_UNSAVED_BAR_BORDER_ATTENTION_WIDTH))
                    .border_color(color(tokens.component_unsaved_bar_border_attention))
            })
            .when(!is_asking, |bar| {
                bar.border(length(COMPONENT_UNSAVED_BAR_BORDER_WIDTH))
                    .border_color(color(tokens.component_unsaved_bar_border))
            })
            .child(div().flex_1().min_w_0().child(message))
            .child(
                Button::new(child_id(&self.id, "reset"), self.labels.reset)
                    .variant(ButtonVariant::Ghost)
                    .disabled(self.is_saving)
                    .on_click(move |window, cx| on_reset(window, cx)),
            )
            .child(
                Button::new(
                    child_id(&self.id, "save"),
                    if self.is_saving {
                        self.labels.saving
                    } else {
                        self.labels.save_changes
                    },
                )
                .variant(ButtonVariant::Primary)
                .disabled(self.is_saving || !self.is_save_enabled)
                .on_click(move |window, cx| on_save(window, cx)),
            );

        // GPUI skips both one-shot animations under reduced motion and shows the end state.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let swing_count = COMPONENT_UNSAVED_BAR_SHAKE_SWING_COUNT as usize;
        let shake_duration =
            Duration::from_millis(COMPONENT_UNSAVED_BAR_SHAKE_STEP_DURATION.0 * swing_count as u64);
        let shaken = bar.with_animation(
            // A new id per refusal replays the shake.
            ElementId::NamedChild(
                std::sync::Arc::new(self.id.clone()),
                SharedString::from(format!("shake-{}", self.attention)),
            ),
            Animation::new(shake_duration),
            move |bar, progress| {
                let offset = if is_asking {
                    shake_offset(progress, COMPONENT_UNSAVED_BAR_SHAKE_DISTANCE, swing_count)
                } else {
                    0.0
                };
                bar.left(length(offset))
            },
        );
        let rising = div().relative().child(shaken).with_animation(
            child_id(&self.id, "rise"),
            Animation::new(Duration::from_millis(
                COMPONENT_UNSAVED_BAR_SLIDE_DURATION.0,
            ))
            .with_easing(move |progress| easing(MOTION_EASING_ENTER).sample(progress)),
            |bar, progress| {
                bar.top(length(RISE_DISTANCE * (1.0 - progress)))
                    .opacity(progress)
            },
        );
        div()
            .id(self.id)
            .pb(length(COMPONENT_UNSAVED_BAR_MARGIN_BOTTOM))
            .child(rising)
    }
}

debug_without_callbacks!(UnsavedChangesBar, id);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shake_swings_both_sides_smaller_each_time_and_comes_back_to_rest() {
        let swings = 6;
        let distance = 6.0;
        assert!(shake_offset(0.0, distance, swings).abs() < 1e-6);
        assert!(shake_offset(1.0, distance, swings).abs() < 1e-6);
        let keyframe = |index: usize| {
            #[allow(clippy::cast_precision_loss)]
            let progress = index as f32 / 7.0;
            shake_offset(progress, distance, swings)
        };
        assert!((keyframe(1) - distance).abs() < 1e-4);
        assert!(keyframe(2) < 0.0);
        assert!(keyframe(3) > 0.0 && keyframe(3) < keyframe(1));
        assert!(shake_offset(0.5, distance, 0).abs() < f32::EPSILON);
    }
}
