//! The few controls the sign-in screens and the connection banner need, drawn from the tokens.
//! shortcut: kept small and local until the primitives track lands Mellow's shared components;
//! swap these for those then.

use gpui::{
    Entity, FontWeight, IntoElement, ParentElement as _, SharedString, Styled, div,
    prelude::FluentBuilder as _, relative, svg,
};
use gpui_base::Button;
use gpui_base::input::{Input, InputState};

use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_BUTTON_HEIGHT_LARGE, COMPONENT_BUTTON_PADDING_INLINE, COMPONENT_BUTTON_RADIUS,
    COMPONENT_TEXT_FIELD_BORDER_WIDTH, COMPONENT_TEXT_FIELD_HEIGHT,
    COMPONENT_TEXT_FIELD_HELPER_GAP, COMPONENT_TEXT_FIELD_ICON_SIZE,
    COMPONENT_TEXT_FIELD_LABEL_GAP, COMPONENT_TEXT_FIELD_PADDING_INLINE,
    COMPONENT_TEXT_FIELD_RADIUS, SIZE_HIT_DESKTOP_MIN, SPACE_2XS, TYPE_BODY, TYPE_BODY_LG,
    TYPE_BUTTON, TYPE_CATEGORY, ThemeTokens, TypeToken,
};
use crate::theme::{MONOSPACE_FONT_FAMILY, color, length};

/// A type token's size, weight and line height on any element.
pub fn text_style<E: Styled>(element: E, token: TypeToken) -> E {
    element
        .text_size(length(token.font_size))
        .font_weight(FontWeight(f32::from(token.font_weight)))
        .line_height(relative(token.line_height))
}

/// A labelled field (the web's `TextField.svelte`): label, box, helper, then its error.
#[derive(Debug)]
pub struct TextField<'a> {
    pub label: &'static str,
    pub input: &'a Entity<InputState>,
    pub helper: Option<SharedString>,
    pub error: Option<&'static str>,
    pub is_focused: bool,
    pub is_monospace: bool,
}

impl TextField<'_> {
    pub fn render(self, tokens: &ThemeTokens) -> impl IntoElement {
        let border = if self.is_focused {
            tokens.component_focus_ring_color
        } else if self.error.is_some() {
            tokens.color_text_danger
        } else {
            tokens.component_text_field_border
        };
        div()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_TEXT_FIELD_LABEL_GAP))
            .child(
                text_style(div(), TYPE_CATEGORY)
                    .text_color(color(tokens.component_text_field_label_text))
                    .child(self.label),
            )
            .child(
                text_style(div(), TYPE_BODY_LG)
                    .flex()
                    .items_center()
                    .h(length(COMPONENT_TEXT_FIELD_HEIGHT))
                    .px(length(COMPONENT_TEXT_FIELD_PADDING_INLINE))
                    .rounded(length(COMPONENT_TEXT_FIELD_RADIUS))
                    .border(length(COMPONENT_TEXT_FIELD_BORDER_WIDTH))
                    .border_color(color(border))
                    .bg(color(tokens.component_text_field_background))
                    .text_color(color(tokens.component_text_field_text))
                    .when(self.is_monospace, |field| {
                        field.font_family(MONOSPACE_FONT_FAMILY)
                    })
                    .child(div().flex_1().min_w_0().child(Input::new(self.input))),
            )
            .when_some(self.helper, |field, helper| {
                field.child(
                    text_style(div(), TYPE_BODY)
                        .mt(length(
                            COMPONENT_TEXT_FIELD_HELPER_GAP - COMPONENT_TEXT_FIELD_LABEL_GAP,
                        ))
                        .text_color(color(tokens.component_text_field_helper_text))
                        .child(helper),
                )
            })
            .when_some(self.error, |field, error| {
                field.child(error_line(
                    error,
                    tokens.color_text_danger,
                    COMPONENT_TEXT_FIELD_ICON_SIZE,
                ))
            })
    }
}

/// An icon and a sentence in one colour, as the web's field and form errors.
pub fn error_line(text: impl Into<SharedString>, ink: u32, icon_size: f32) -> impl IntoElement {
    text_style(div(), TYPE_BODY)
        .flex()
        .gap(length(SPACE_2XS))
        .text_color(color(ink))
        .child(
            svg()
                .path(IconName::WarningCircle.asset_path())
                .flex_none()
                .size(length(icon_size))
                .text_color(color(ink)),
        )
        .child(div().flex_1().min_w_0().child(text.into()))
}

/// The primary, full-width button of a form; busy or disabled it takes the disabled wash.
pub fn primary_button(
    id: &'static str,
    label: impl Into<SharedString>,
    is_disabled: bool,
    tokens: &ThemeTokens,
) -> Button {
    let (background, text) = if is_disabled {
        (
            tokens.component_button_disabled_background,
            tokens.component_button_disabled_text,
        )
    } else {
        (
            tokens.component_button_primary_background,
            tokens.component_button_primary_text,
        )
    };
    text_style(Button::new(id), TYPE_BUTTON)
        .disabled(is_disabled)
        .w_full()
        .h(length(COMPONENT_BUTTON_HEIGHT_LARGE))
        .px(length(COMPONENT_BUTTON_PADDING_INLINE))
        .flex()
        .items_center()
        .justify_center()
        .rounded(length(COMPONENT_BUTTON_RADIUS))
        .bg(color(background))
        .text_color(color(text))
        .child(label.into())
}

/// A text button in the link colour, as the card's "Create an account".
pub fn link_button(id: &'static str, label: &'static str, tokens: &ThemeTokens) -> Button {
    text_style(Button::new(id), TYPE_BODY)
        .min_h(length(SIZE_HIT_DESKTOP_MIN))
        .flex()
        .items_center()
        .text_color(color(tokens.component_auth_card_link_text))
        .child(label)
}
