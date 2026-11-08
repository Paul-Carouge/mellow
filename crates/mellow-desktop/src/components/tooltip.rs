//! The tooltip (DESIGN.md §6.34): a floating label on `surface.floating` with the high shadow,
//! shown after `motion.delay.tooltip`. The web relies on the browser's `title` for most buttons;
//! the desktop draws every one.

use std::time::Duration;

use gpui::{
    AnyView, App, AppContext as _, Context, IntoElement, ParentElement as _, Render, SharedString,
    Styled as _, Window, div,
};

use super::{shadow, with_type};
use crate::theme::tokens::{
    COMPONENT_TOOLTIP_DELAY, COMPONENT_TOOLTIP_MAX_WIDTH, COMPONENT_TOOLTIP_PADDING_BLOCK,
    COMPONENT_TOOLTIP_PADDING_INLINE, COMPONENT_TOOLTIP_RADIUS, TYPE_TOOLTIP,
};
use crate::theme::{color, delay, length, theme};

#[derive(Debug, Clone)]
pub struct Tooltip {
    label: SharedString,
}

impl Tooltip {
    #[must_use]
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
        }
    }

    /// For `StatefulInteractiveElement::tooltip`.
    pub fn view(label: SharedString, cx: &mut App) -> AnyView {
        cx.new(|_| Self::new(label)).into()
    }

    /// A label that arrives after the tooltip shows (who reacted, once the instance answers).
    pub fn set_label(&mut self, label: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.label = label.into();
        cx.notify();
    }

    /// The delay token, kept under reduced motion.
    #[must_use]
    pub fn show_delay() -> Duration {
        delay(COMPONENT_TOOLTIP_DELAY)
    }
}

impl Render for Tooltip {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        with_type(div(), TYPE_TOOLTIP)
            .max_w(length(COMPONENT_TOOLTIP_MAX_WIDTH))
            .px(length(COMPONENT_TOOLTIP_PADDING_INLINE))
            .py(length(COMPONENT_TOOLTIP_PADDING_BLOCK))
            .rounded(length(COMPONENT_TOOLTIP_RADIUS))
            .bg(color(tokens.component_tooltip_background))
            .border(length(1.0))
            .border_color(color(tokens.color_border_divider))
            .shadow(shadow(tokens.component_tooltip_shadow))
            .text_color(color(tokens.component_tooltip_text))
            .child(self.label.clone())
    }
}
