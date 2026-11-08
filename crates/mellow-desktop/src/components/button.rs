//! `Button.svelte`, `button-variants.ts` and `IconButton.svelte` (DESIGN.md §6.1, §6.2).

use std::sync::Arc;

use gpui::{
    App, ElementId, FocusHandle, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    RenderOnce, Role, SharedString, StatefulInteractiveElement as _, Styled as _, Toggled, Window,
    div, prelude::FluentBuilder as _,
};

use super::{
    ActivationHandler, Tooltip, child_id, colour_transition, focus_ring, icon, is_focus_visible,
    remembered_focus_handle, remembered_hover, with_type,
};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_BUTTON_DISABLED_OPACITY, COMPONENT_BUTTON_HEIGHT, COMPONENT_BUTTON_HEIGHT_LARGE,
    COMPONENT_BUTTON_HEIGHT_SMALL, COMPONENT_BUTTON_PADDING_INLINE, COMPONENT_BUTTON_RADIUS,
    RADIUS_SM, SIZE_HIT_WEB_MIN, TYPE_BUTTON, ThemeTokens,
};
use crate::theme::{color, length, theme};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonVariant {
    Primary,
    Secondary,
    Danger,
    /// Cancel and other quiet actions beside a primary one.
    Ghost,
}

/// 40 px is the desktop default; 44 px is the web's touch size; 32 px fits dense rows (§6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonSize {
    Small,
    Default,
    Large,
}

impl ButtonSize {
    fn height(self) -> f32 {
        match self {
            Self::Small => COMPONENT_BUTTON_HEIGHT_SMALL,
            Self::Default => COMPONENT_BUTTON_HEIGHT,
            Self::Large => COMPONENT_BUTTON_HEIGHT_LARGE,
        }
    }
}

/// Fill, fill under the pointer, fill while pressed and label colour of a button.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ButtonColours {
    pub rest: Hsla,
    pub hover: Hsla,
    pub pressed: Hsla,
    pub label: Hsla,
}

/// `buttonVariantClasses`: a disabled or busy filled button takes the disabled wash instead of a
/// dimmed fill, which made apricot read as brown; a ghost has no fill to swap, so it dims.
pub(crate) fn button_colours(
    variant: ButtonVariant,
    is_disabled: bool,
    tokens: &ThemeTokens,
) -> ButtonColours {
    let transparent = Hsla::transparent_black();
    if is_disabled && variant != ButtonVariant::Ghost {
        let wash = color(tokens.component_button_disabled_background);
        return ButtonColours {
            rest: wash,
            hover: wash,
            pressed: wash,
            label: color(tokens.component_button_disabled_text),
        };
    }
    let (rest, hover, pressed, label) = match variant {
        ButtonVariant::Primary => (
            tokens.component_button_primary_background,
            tokens.component_button_primary_background_hover,
            tokens.component_button_primary_background_pressed,
            tokens.component_button_primary_text,
        ),
        ButtonVariant::Secondary => (
            tokens.component_button_secondary_background,
            tokens.component_button_secondary_background_hover,
            tokens.component_button_secondary_background_hover,
            tokens.component_button_secondary_text,
        ),
        ButtonVariant::Danger => (
            tokens.component_button_danger_background,
            tokens.component_button_danger_background_hover,
            tokens.component_button_danger_background_hover,
            tokens.component_button_danger_text,
        ),
        ButtonVariant::Ghost => {
            let hover = if is_disabled {
                transparent
            } else {
                color(tokens.color_state_hover)
            };
            return ButtonColours {
                // The hover colour at no opacity, so the fade does not pass through black.
                rest: Hsla { a: 0.0, ..hover },
                hover,
                pressed: hover,
                label: color(tokens.color_text_normal),
            };
        }
    };
    ButtonColours {
        rest: color(rest),
        hover: color(hover),
        pressed: color(pressed),
        label: color(label),
    }
}

/// A text button. Disabled and busy buttons stay focusable (`aria-disabled`), so focus is not lost
/// while an action runs; activating them does nothing.
#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: SharedString,
    accessible_label: Option<SharedString>,
    variant: ButtonVariant,
    size: ButtonSize,
    is_disabled: bool,
    is_full_width: bool,
    focus_handle: Option<FocusHandle>,
    on_click: Option<ActivationHandler>,
}

impl Button {
    #[must_use]
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            accessible_label: None,
            variant: ButtonVariant::Secondary,
            size: ButtonSize::Default,
            is_disabled: false,
            is_full_width: false,
            focus_handle: None,
            on_click: None,
        }
    }

    #[must_use]
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    #[must_use]
    pub fn size(mut self, size: ButtonSize) -> Self {
        self.size = size;
        self
    }

    #[must_use]
    pub fn disabled(mut self, is_disabled: bool) -> Self {
        self.is_disabled = is_disabled;
        self
    }

    #[must_use]
    pub fn full_width(mut self) -> Self {
        self.is_full_width = true;
        self
    }

    /// The name read out instead of the visible text, when that text alone does not say whom
    /// the button acts on ("Unblock" on a row).
    #[must_use]
    pub fn accessible_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessible_label = Some(label.into());
        self
    }

    /// A handle the host focuses itself (a dialog's first control).
    #[must_use]
    pub fn track_focus(mut self, focus_handle: &FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle.clone());
        self
    }

    #[must_use]
    pub fn on_click(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Arc::new(handler));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let colours = button_colours(self.variant, self.is_disabled, tokens);
        let focus_handle = match self.focus_handle {
            Some(focus_handle) => focus_handle.tab_stop(true),
            None => remembered_focus_handle(&self.id, window, cx),
        };
        let hover = remembered_hover(&self.id, window, cx);
        let is_hovered = *hover.read(cx);
        let background = colour_transition(
            child_id(&self.id, "background"),
            if is_hovered {
                colours.hover
            } else {
                colours.rest
            },
            window,
            cx,
        );
        let is_ring_shown = is_focus_visible(&focus_handle, window);
        let is_disabled = self.is_disabled;
        let on_click = self.on_click;

        with_type(div(), TYPE_BUTTON)
            .id(self.id)
            .role(Role::Button)
            .aria_label(self.accessible_label.unwrap_or_else(|| self.label.clone()))
            .track_focus(&focus_handle)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(length(self.size.height()))
            .min_w(length(SIZE_HIT_WEB_MIN))
            .px(length(COMPONENT_BUTTON_PADDING_INLINE))
            .rounded(length(COMPONENT_BUTTON_RADIUS))
            .bg(background)
            .text_color(colours.label)
            .when(self.is_full_width, gpui::Styled::w_full)
            .when(
                is_disabled && self.variant == ButtonVariant::Ghost,
                |button| button.opacity(COMPONENT_BUTTON_DISABLED_OPACITY),
            )
            .when(!is_disabled, |button| {
                button
                    .cursor_pointer()
                    .active(move |style| style.bg(colours.pressed))
            })
            .on_hover(move |is_hovered, _, cx| {
                hover.update(cx, |remembered, cx| {
                    *remembered = *is_hovered;
                    cx.notify();
                });
            })
            .on_click(move |_, window, cx| {
                if let Some(on_click) = on_click.as_ref().filter(|_| !is_disabled) {
                    on_click(window, cx);
                }
            })
            .child(self.label)
            .when(is_ring_shown, |button| {
                button.child(focus_ring(COMPONENT_BUTTON_RADIUS, cx))
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconButtonTone {
    Standard,
    /// Delete, leave: the icon turns red under the pointer.
    Danger,
}

/// An icon, an accessible label and a tooltip (DESIGN.md §6.2); a toggle reports its state.
#[derive(IntoElement)]
pub struct IconButton {
    id: ElementId,
    icon: IconName,
    label: SharedString,
    size: f32,
    icon_size: f32,
    tone: IconButtonTone,
    toggled: Option<bool>,
    is_disabled: bool,
    /// The host's handle, to move focus here; otherwise one kept while it is drawn.
    focus_handle: Option<FocusHandle>,
    on_click: Option<ActivationHandler>,
}

/// Header, hover bar and user card buttons are 32 px with 20 px icons (DESIGN.md §6.2).
const DEFAULT_ICON_BUTTON_SIZE: f32 = COMPONENT_BUTTON_HEIGHT_SMALL;
const DEFAULT_ICON_BUTTON_ICON_SIZE: f32 = crate::theme::tokens::COMPONENT_TEXT_FIELD_ICON_SIZE;

impl IconButton {
    #[must_use]
    pub fn new(id: impl Into<ElementId>, icon: IconName, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon,
            label: label.into(),
            size: DEFAULT_ICON_BUTTON_SIZE,
            icon_size: DEFAULT_ICON_BUTTON_ICON_SIZE,
            tone: IconButtonTone::Standard,
            toggled: None,
            is_disabled: false,
            focus_handle: None,
            on_click: None,
        }
    }

    /// The host's sizes, from its own tokens.
    #[must_use]
    pub fn sizes(mut self, button_size: f32, icon_size: f32) -> Self {
        self.size = button_size;
        self.icon_size = icon_size;
        self
    }

    #[must_use]
    pub fn tone(mut self, tone: IconButtonTone) -> Self {
        self.tone = tone;
        self
    }

    /// Makes it a toggle button (mute, deafen) showing whether it is on.
    #[must_use]
    pub fn toggled(mut self, is_on: bool) -> Self {
        self.toggled = Some(is_on);
        self
    }

    #[must_use]
    pub fn disabled(mut self, is_disabled: bool) -> Self {
        self.is_disabled = is_disabled;
        self
    }

    #[must_use]
    pub fn on_click(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Arc::new(handler));
        self
    }

    /// A handle the host focuses itself (a tab stop, as the remembered one is).
    #[must_use]
    pub fn track_focus(mut self, focus_handle: &FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle.clone());
        self
    }
}

impl RenderOnce for IconButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let focus_handle = self
            .focus_handle
            .clone()
            .unwrap_or_else(|| remembered_focus_handle(&self.id, window, cx));
        let hover = remembered_hover(&self.id, window, cx);
        let is_hovered = *hover.read(cx) && !self.is_disabled;
        let is_on = self.toggled == Some(true);
        let hover_fill = color(tokens.color_state_hover);
        let target_fill = if is_on {
            color(tokens.color_state_selected)
        } else if is_hovered {
            hover_fill
        } else {
            Hsla {
                a: 0.0,
                ..hover_fill
            }
        };
        let target_icon = match (self.tone, is_hovered || is_on) {
            (IconButtonTone::Danger, true) => color(tokens.color_text_danger),
            (IconButtonTone::Standard, true) => color(tokens.color_text_normal),
            (_, false) => color(tokens.color_text_secondary),
        };
        let fill = colour_transition(child_id(&self.id, "background"), target_fill, window, cx);
        let icon_tint = colour_transition(child_id(&self.id, "icon"), target_icon, window, cx);
        let is_ring_shown = is_focus_visible(&focus_handle, window);
        let label = self.label.clone();
        let is_disabled = self.is_disabled;
        let on_click = self.on_click;

        div()
            .id(self.id)
            .role(Role::Button)
            .aria_label(self.label)
            .when_some(self.toggled, |button, is_on| {
                button.aria_toggled(if is_on { Toggled::True } else { Toggled::False })
            })
            .track_focus(&focus_handle)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(length(self.size))
            .rounded(length(RADIUS_SM))
            .bg(fill)
            .when(is_disabled, |button| {
                button.opacity(COMPONENT_BUTTON_DISABLED_OPACITY)
            })
            .when(!is_disabled, gpui::Styled::cursor_pointer)
            .tooltip(move |_, cx| Tooltip::view(label.clone(), cx))
            .tooltip_show_delay(Tooltip::show_delay())
            .on_hover(move |is_hovered, _, cx| {
                hover.update(cx, |remembered, cx| {
                    *remembered = *is_hovered;
                    cx.notify();
                });
            })
            .on_click(move |_, window, cx| {
                if let Some(on_click) = on_click.as_ref().filter(|_| !is_disabled) {
                    on_click(window, cx);
                }
            })
            .child(icon(self.icon, self.icon_size, icon_tint))
            .when(is_ring_shown, |button| {
                button.child(focus_ring(RADIUS_SM, cx))
            })
    }
}

debug_without_callbacks!(Button, id, label);
debug_without_callbacks!(IconButton, id, label);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::tokens::{DARK_THEME, LIGHT_THEME};

    #[test]
    fn a_disabled_filled_button_takes_the_wash_not_a_dimmed_fill() {
        for tokens in [&DARK_THEME, &LIGHT_THEME] {
            for variant in [
                ButtonVariant::Primary,
                ButtonVariant::Secondary,
                ButtonVariant::Danger,
            ] {
                let colours = button_colours(variant, true, tokens);
                assert_eq!(
                    colours.rest,
                    color(tokens.component_button_disabled_background)
                );
                assert_eq!(colours.hover, colours.rest, "no hover while disabled");
                assert_eq!(colours.label, color(tokens.component_button_disabled_text));
            }
        }
    }

    #[test]
    fn primary_warms_under_the_pointer_and_deepens_when_pressed() {
        let colours = button_colours(ButtonVariant::Primary, false, &DARK_THEME);
        assert_eq!(
            colours.rest,
            color(DARK_THEME.component_button_primary_background)
        );
        assert_eq!(
            colours.hover,
            color(DARK_THEME.component_button_primary_background_hover)
        );
        assert_eq!(
            colours.pressed,
            color(DARK_THEME.component_button_primary_background_pressed)
        );
    }

    #[test]
    fn a_ghost_button_fills_only_under_the_pointer() {
        let colours = button_colours(ButtonVariant::Ghost, false, &LIGHT_THEME);
        assert!(colours.rest.a.abs() < f32::EPSILON);
        assert_eq!(colours.hover, color(LIGHT_THEME.color_state_hover));
        assert_eq!(colours.label, color(LIGHT_THEME.color_text_normal));
    }
}
