//! `TextField.svelte` (DESIGN.md §6.3): the label above, the field with a 3:1 outline, an optional
//! helper and error below. The editing itself (IME, selection, clipboard, undo) is gpui-component's
//! single-line input, drawn without its own frame inside the Dusk field.

use gpui::{
    App, ElementId, Entity, Focusable as _, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Toggled, Window, div, prelude::FluentBuilder as _,
};
use gpui_component::input::{Input, InputState};

use super::{
    child_id, focus_ring, icon, inset_focus_ring, is_focus_visible, remembered_hover, with_type,
};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_TEXT_FIELD_BORDER_WIDTH, COMPONENT_TEXT_FIELD_HEIGHT,
    COMPONENT_TEXT_FIELD_HELPER_GAP, COMPONENT_TEXT_FIELD_ICON_SIZE,
    COMPONENT_TEXT_FIELD_LABEL_GAP, COMPONENT_TEXT_FIELD_PADDING_INLINE,
    COMPONENT_TEXT_FIELD_RADIUS, SPACE_2XS, SPACE_XS, TYPE_BODY, TYPE_BODY_LG, TYPE_CATEGORY,
    ThemeTokens,
};
use crate::theme::{MONOSPACE_FONT_FAMILY, color, length, theme};

/// The password toggle, named from the host's interface strings ("Show password"); it reports
/// whether the password is shown, as the web's `aria-pressed`.
#[derive(Debug, Clone)]
pub struct PasswordToggle {
    pub label: SharedString,
}

#[derive(IntoElement)]
pub struct TextField {
    id: ElementId,
    label: SharedString,
    state: Entity<InputState>,
    helper: Option<SharedString>,
    error: Option<SharedString>,
    leading_icon: Option<IconName>,
    password_toggle: Option<PasswordToggle>,
    is_monospace: bool,
    is_read_only: bool,
    is_label_hidden: bool,
}

impl TextField {
    /// The host owns `state` (`InputState::new(window, cx)`, with its placeholder and value).
    #[must_use]
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        state: &Entity<InputState>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            state: state.clone(),
            helper: None,
            error: None,
            leading_icon: None,
            password_toggle: None,
            is_monospace: false,
            is_read_only: false,
            is_label_hidden: false,
        }
    }

    #[must_use]
    pub fn helper(mut self, helper: impl Into<SharedString>) -> Self {
        self.helper = Some(helper.into());
        self
    }

    /// Shown under the field and announced with it.
    #[must_use]
    pub fn error(mut self, error: impl Into<SharedString>) -> Self {
        self.error = Some(error.into());
        self
    }

    #[must_use]
    pub fn leading_icon(mut self, leading_icon: IconName) -> Self {
        self.leading_icon = Some(leading_icon);
        self
    }

    /// A password field: masked, with a show/hide toggle. Create the state `.masked(true)`.
    #[must_use]
    pub fn password(mut self, toggle: PasswordToggle) -> Self {
        self.password_toggle = Some(toggle);
        self
    }

    #[must_use]
    pub fn monospace(mut self) -> Self {
        self.is_monospace = true;
        self
    }

    /// Named for screen readers only: rows of a list whose heading names them all (a poll's
    /// answers), each with its placeholder in view.
    #[must_use]
    pub fn label_hidden(mut self) -> Self {
        self.is_label_hidden = true;
        self
    }

    /// Focusable and selectable, but not editable (the web's `readonly`, while a form saves).
    #[must_use]
    pub fn read_only(mut self, is_read_only: bool) -> Self {
        self.is_read_only = is_read_only;
        self
    }
}

/// Points gpui-component's text input colours at Dusk's (caret, selection, text, placeholder),
/// since its input paints them from its own theme. Called whenever the Dusk theme changes.
pub fn sync_text_input_colours(tokens: &ThemeTokens, cx: &mut App) {
    gpui_component::Theme::update(cx, |input_theme| {
        input_theme.colors.foreground = color(tokens.component_text_field_text);
        input_theme.colors.muted_foreground = color(tokens.component_text_field_placeholder);
        input_theme.colors.caret = color(tokens.component_text_field_text);
        input_theme.colors.selection = color(tokens.component_message_selection_background);
        input_theme.colors.border = color(tokens.component_text_field_border);
    });
}

impl RenderOnce for TextField {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let input_focus = self.state.read(cx).focus_handle(cx);
        // Any focus draws the ring around a field, a click included (DESIGN.md §7.2).
        let is_field_focused = input_focus.is_focused(window);
        let border = if self.error.is_some() {
            tokens.color_text_danger
        } else {
            tokens.component_text_field_border
        };
        let description = match (&self.helper, &self.error) {
            (Some(helper), Some(error)) => Some(SharedString::from(format!("{helper} {error}"))),
            (Some(text), None) | (None, Some(text)) => Some(text.clone()),
            (None, None) => None,
        };
        let is_masked = self.state.read(cx).presentation().is_masked();
        let toggle_state = self.state.clone();
        let toggle_id = child_id(&self.id, "password-toggle");

        let field = div()
            .relative()
            .flex()
            .items_center()
            .gap(length(SPACE_XS))
            .h(length(COMPONENT_TEXT_FIELD_HEIGHT))
            .px(length(COMPONENT_TEXT_FIELD_PADDING_INLINE))
            .rounded(length(COMPONENT_TEXT_FIELD_RADIUS))
            .border(length(COMPONENT_TEXT_FIELD_BORDER_WIDTH))
            .border_color(color(border))
            .bg(color(tokens.component_text_field_background))
            .when_some(self.leading_icon, |field, leading| {
                field.child(icon(
                    leading,
                    COMPONENT_TEXT_FIELD_ICON_SIZE,
                    color(tokens.component_text_field_icon),
                ))
            })
            .child(
                div().flex_1().min_w_0().child(
                    // The value in `type.body-lg` from the field's own padding, as the web's
                    // `<input>`: the input's own text size and inline padding are replaced.
                    with_type(Input::new(&self.state), TYPE_BODY_LG)
                        .px_0()
                        .text_color(color(tokens.component_text_field_text))
                        .when(self.is_monospace, |input| {
                            input.font_family(MONOSPACE_FONT_FAMILY)
                        })
                        .appearance(false)
                        .readonly(self.is_read_only)
                        .aria_label(self.label.clone())
                        .when_some(description, |input, description| {
                            input.aria_label(SharedString::from(format!(
                                "{}, {description}",
                                self.label
                            )))
                        }),
                ),
            )
            .when_some(self.password_toggle, |field, toggle| {
                let toggle_focus = window
                    .use_keyed_state(child_id(&toggle_id, "focus"), cx, |_, cx| cx.focus_handle())
                    .read(cx)
                    .clone()
                    .tab_stop(true);
                let is_ring_shown = is_focus_visible(&toggle_focus, window);
                let toggle_hover = remembered_hover(&toggle_id, window, cx);
                let toggle_icon = color(if *toggle_hover.read(cx) {
                    tokens.component_text_field_icon_hover
                } else {
                    tokens.component_text_field_icon
                });
                field.child(
                    div()
                        .id(toggle_id)
                        .role(Role::Button)
                        .aria_label(toggle.label)
                        .aria_toggled(if is_masked {
                            Toggled::False
                        } else {
                            Toggled::True
                        })
                        .track_focus(&toggle_focus)
                        .relative()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        // As tall as the field, flush with its trailing edge.
                        .size(length(COMPONENT_TEXT_FIELD_HEIGHT))
                        .mr(-length(COMPONENT_TEXT_FIELD_PADDING_INLINE))
                        .cursor_pointer()
                        .on_hover(move |is_hovered, _, cx| {
                            toggle_hover.update(cx, |remembered, cx| {
                                *remembered = *is_hovered;
                                cx.notify();
                            });
                        })
                        .on_click(move |_, window, cx| {
                            toggle_state.update(cx, |state, cx| {
                                let is_masked = state.presentation().is_masked();
                                state.set_masked(!is_masked, window, cx);
                            });
                        })
                        .child(icon(
                            if is_masked {
                                IconName::Eye
                            } else {
                                IconName::EyeSlash
                            },
                            COMPONENT_TEXT_FIELD_ICON_SIZE,
                            toggle_icon,
                        ))
                        .when(is_ring_shown, |toggle| {
                            toggle.child(inset_focus_ring(0.0, cx))
                        }),
                )
            })
            .when(is_field_focused, |field| {
                field.child(focus_ring(COMPONENT_TEXT_FIELD_RADIUS, cx))
            });

        div()
            .id(self.id)
            .flex()
            .flex_col()
            .gap(length(COMPONENT_TEXT_FIELD_LABEL_GAP))
            .when(!self.is_label_hidden, |column| {
                column.child(
                    with_type(div(), TYPE_CATEGORY)
                        .text_color(color(tokens.component_text_field_label_text))
                        .child(self.label),
                )
            })
            .child(field)
            .when_some(self.helper, |column, helper| {
                column.child(
                    with_type(div(), TYPE_BODY)
                        .mt(length(
                            COMPONENT_TEXT_FIELD_HELPER_GAP - COMPONENT_TEXT_FIELD_LABEL_GAP,
                        ))
                        .text_color(color(tokens.component_text_field_helper_text))
                        .child(helper),
                )
            })
            .when_some(self.error, |column, error| {
                column.child(
                    with_type(div(), TYPE_BODY)
                        .mt(length(
                            COMPONENT_TEXT_FIELD_HELPER_GAP - COMPONENT_TEXT_FIELD_LABEL_GAP,
                        ))
                        .flex()
                        .gap(length(SPACE_2XS))
                        .text_color(color(tokens.color_text_danger))
                        .child(icon(
                            IconName::WarningCircle,
                            COMPONENT_TEXT_FIELD_ICON_SIZE,
                            color(tokens.color_text_danger),
                        ))
                        .child(error),
                )
            })
    }
}

debug_without_callbacks!(TextField, id, label);
