//! `ChoiceOption.svelte` (DESIGN.md §6.55): the radio rows of a dialog's choice (a channel type, a
//! server template). The whole row is its label; a checked row is washed and edged, not only
//! dotted. As native radios: one Tab stop on the checked row, arrows move and choose, wrapping.

use std::sync::Arc;

use gpui::{
    App, ElementId, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Toggled, Window, div, prelude::FluentBuilder as _,
};

use super::{
    child_id, colour_transition, focus_ring, icon, is_focus_visible, remembered_hover, with_type,
};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CHOICE_BORDER_WIDTH, COMPONENT_CHOICE_GAP, COMPONENT_CHOICE_ICON_SIZE,
    COMPONENT_CHOICE_INDICATOR_SIZE, COMPONENT_CHOICE_LIST_GAP, COMPONENT_CHOICE_MIN_HEIGHT,
    COMPONENT_CHOICE_PADDING_BLOCK, COMPONENT_CHOICE_PADDING_INLINE, COMPONENT_CHOICE_RADIUS,
    SIZE_FOCUS_RING_WIDTH, SPACE_XS, TYPE_BODY_SM, TYPE_CHANNEL_NAME,
};
use crate::theme::{color, length, theme};

type SelectHandler = Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>;

/// One row of a choice.
#[derive(Debug, Clone)]
pub struct ChoiceOption {
    pub label: SharedString,
    pub description: Option<SharedString>,
    pub icon: Option<IconName>,
}

impl ChoiceOption {
    #[must_use]
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            description: None,
            icon: None,
        }
    }

    #[must_use]
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    #[must_use]
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }
}

/// The row a key moves the choice to, or `None` for keys a radio group does not handle.
#[must_use]
pub fn next_choice_index(key: &str, current: usize, option_count: usize) -> Option<usize> {
    if option_count == 0 {
        return None;
    }
    match key {
        "down" | "right" => Some((current + 1) % option_count),
        "up" | "left" => Some((current + option_count - 1) % option_count),
        _ => None,
    }
}

#[derive(IntoElement)]
pub struct ChoiceGroup {
    id: ElementId,
    label: SharedString,
    options: Vec<ChoiceOption>,
    checked_index: usize,
    on_select: Option<SelectHandler>,
}

impl ChoiceGroup {
    #[must_use]
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        options: Vec<ChoiceOption>,
        checked_index: usize,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            options,
            checked_index,
            on_select: None,
        }
    }

    #[must_use]
    pub fn on_select(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Arc::new(handler));
        self
    }
}

/// The browser's radio, drawn in the choice's colours: a ring, and the apricot dot when checked.
fn radio_indicator(is_checked: bool, cx: &App) -> impl IntoElement {
    let tokens = theme(cx);
    let edge = if is_checked {
        tokens.component_choice_indicator
    } else {
        tokens.component_choice_border_checked
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(length(COMPONENT_CHOICE_INDICATOR_SIZE))
        .rounded_full()
        .border(length(SIZE_FOCUS_RING_WIDTH))
        .border_color(color(edge))
        .when(is_checked, |indicator| {
            indicator.child(
                div()
                    .size(length(SPACE_XS))
                    .rounded_full()
                    .bg(color(tokens.component_choice_indicator)),
            )
        })
}

impl RenderOnce for ChoiceGroup {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let option_count = self.options.len();
        let row_ids: Vec<ElementId> = (0..option_count)
            .map(|index| {
                ElementId::NamedChild(
                    Arc::new(self.id.clone()),
                    SharedString::from(format!("choice-{index}")),
                )
            })
            .collect();
        let focus_handles: Vec<FocusHandle> = row_ids
            .iter()
            .enumerate()
            .map(|(index, row_id)| {
                window
                    .use_keyed_state(child_id(row_id, "focus"), cx, |_, cx| cx.focus_handle())
                    .read(cx)
                    .clone()
                    .tab_stop(index == self.checked_index)
            })
            .collect();
        let on_select = self.on_select;
        let key_handles = focus_handles.clone();
        let key_on_select = on_select.clone();
        let checked_index = self.checked_index;

        let rows = self
            .options
            .into_iter()
            .zip(row_ids.into_iter().zip(focus_handles))
            .enumerate()
            .map(|(index, (option, (row_id, focus_handle)))| {
                let is_checked = index == checked_index;
                let hover = remembered_hover(&row_id, window, cx);
                let target_fill = if is_checked {
                    color(tokens.component_choice_background_checked)
                } else if *hover.read(cx) {
                    color(tokens.component_choice_background_hover)
                } else {
                    let hover_fill = color(tokens.component_choice_background_hover);
                    gpui::Hsla {
                        a: 0.0,
                        ..hover_fill
                    }
                };
                let fill = colour_transition(child_id(&row_id, "fill"), target_fill, window, cx);
                let is_ring_shown = is_focus_visible(&focus_handle, window);
                let on_select = on_select.clone();
                let description_colour = if is_checked {
                    tokens.component_choice_description_text_checked
                } else {
                    tokens.component_choice_description_text
                };
                div()
                    .id(row_id)
                    .role(Role::RadioButton)
                    .aria_label(option.label.clone())
                    .when_some(option.description.clone(), |row, description| {
                        row.aria_description(description)
                    })
                    .aria_toggled(if is_checked {
                        Toggled::True
                    } else {
                        Toggled::False
                    })
                    .aria_position_in_set(index + 1)
                    .aria_size_of_set(option_count)
                    .track_focus(&focus_handle)
                    .relative()
                    .flex()
                    .items_center()
                    .gap(length(COMPONENT_CHOICE_GAP))
                    .min_h(length(COMPONENT_CHOICE_MIN_HEIGHT))
                    .px(length(COMPONENT_CHOICE_PADDING_INLINE))
                    .py(length(COMPONENT_CHOICE_PADDING_BLOCK))
                    .rounded(length(COMPONENT_CHOICE_RADIUS))
                    .border(length(COMPONENT_CHOICE_BORDER_WIDTH))
                    .border_color(color(if is_checked {
                        tokens.component_choice_border_checked
                    } else {
                        tokens.component_choice_border
                    }))
                    .bg(fill)
                    .cursor_pointer()
                    .on_hover(move |is_hovered, _, cx| {
                        hover.update(cx, |remembered, cx| {
                            *remembered = *is_hovered;
                            cx.notify();
                        });
                    })
                    .on_click(move |_, window, cx| {
                        if let Some(on_select) = on_select.as_ref() {
                            on_select(index, window, cx);
                        }
                    })
                    .child(radio_indicator(is_checked, cx))
                    .when_some(option.icon, |row, choice_icon| {
                        row.child(icon(
                            choice_icon,
                            COMPONENT_CHOICE_ICON_SIZE,
                            color(tokens.component_choice_icon),
                        ))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .min_w_0()
                            .child(
                                with_type(div(), TYPE_CHANNEL_NAME)
                                    .text_color(color(tokens.component_choice_name_text))
                                    .child(option.label),
                            )
                            .when_some(option.description, |text, description| {
                                text.child(
                                    with_type(div(), TYPE_BODY_SM)
                                        .text_color(color(description_colour))
                                        .child(description),
                                )
                            }),
                    )
                    .when(is_ring_shown, |row| {
                        row.child(focus_ring(COMPONENT_CHOICE_RADIUS, cx))
                    })
            });

        div()
            .id(self.id)
            .role(Role::RadioGroup)
            .aria_label(self.label)
            .flex()
            .flex_col()
            .gap(length(COMPONENT_CHOICE_LIST_GAP))
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.modified() {
                    return;
                }
                let Some(next) =
                    next_choice_index(&event.keystroke.key, checked_index, option_count)
                else {
                    return;
                };
                cx.stop_propagation();
                if let Some(handle) = key_handles.get(next) {
                    window.focus(handle, cx);
                }
                if let Some(on_select) = key_on_select.as_ref() {
                    on_select(next, window, cx);
                }
            })
            .children(rows)
    }
}

debug_without_callbacks!(ChoiceGroup, id, label);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_move_the_choice_and_wrap() {
        assert_eq!(next_choice_index("down", 2, 3), Some(0));
        assert_eq!(next_choice_index("up", 0, 3), Some(2));
        assert_eq!(next_choice_index("right", 0, 3), Some(1));
        assert_eq!(next_choice_index("home", 1, 3), None);
    }
}
