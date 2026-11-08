//! Tabs (`FriendsPane.svelte`, `GifPicker.svelte`, DESIGN.md §6.6): a tab list with the apricot
//! indicator under the chosen tab. One Tab stop for the list; Left and Right move and choose
//! (wrapping), Home and End jump to the ends, as the web's `moveTabFocus`.

use std::sync::Arc;

use gpui::{
    App, ElementId, FocusHandle, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _,
};

use super::{
    child_id, colour_transition, focus_ring, is_focus_visible, remembered_hover, with_type,
};
use crate::theme::tokens::{
    COMPONENT_TABS_GAP, COMPONENT_TABS_HEIGHT, COMPONENT_TABS_INDICATOR_HEIGHT, RADIUS_XS,
    TYPE_BODY,
};
use crate::theme::{color, length, theme};

type SelectHandler = Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>;

/// The tab a key moves to from `current`, or `None` for keys the tab list does not handle.
#[must_use]
pub fn next_tab_index(key: &str, current: usize, tab_count: usize) -> Option<usize> {
    if tab_count == 0 {
        return None;
    }
    match key {
        "right" => Some((current + 1) % tab_count),
        "left" => Some((current + tab_count - 1) % tab_count),
        "home" => Some(0),
        "end" => Some(tab_count - 1),
        _ => None,
    }
}

#[derive(IntoElement)]
pub struct Tabs {
    id: ElementId,
    label: SharedString,
    tab_labels: Vec<SharedString>,
    /// Read out instead of the visible text when it says less ("👍, 3 reactions" for "👍 3").
    accessible_labels: Option<Vec<SharedString>>,
    selected_index: usize,
    on_select: Option<SelectHandler>,
}

impl Tabs {
    /// `label` names the tab list for screen readers.
    #[must_use]
    pub fn new(
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        tab_labels: Vec<SharedString>,
        selected_index: usize,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            tab_labels,
            accessible_labels: None,
            selected_index,
            on_select: None,
        }
    }

    /// Screen reader names, one per tab, when the visible text is not enough.
    #[must_use]
    pub fn accessible_labels(mut self, labels: Vec<SharedString>) -> Self {
        self.accessible_labels = Some(labels);
        self
    }

    #[must_use]
    pub fn on_select(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_select = Some(Arc::new(handler));
        self
    }
}

impl RenderOnce for Tabs {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let tab_count = self.tab_labels.len();
        let focus_handles: Vec<FocusHandle> = (0..tab_count)
            .map(|index| {
                let roving_tab_id = ElementId::NamedChild(
                    Arc::new(self.id.clone()),
                    SharedString::from(format!("tab-{index}")),
                );
                window
                    .use_keyed_state(child_id(&roving_tab_id, "focus"), cx, |_, cx| {
                        cx.focus_handle()
                    })
                    .read(cx)
                    .clone()
                    // Roving: only the chosen tab is a Tab stop.
                    .tab_stop(index == self.selected_index)
            })
            .collect();
        let on_select = self.on_select;
        let key_handles = focus_handles.clone();
        let key_on_select = on_select.clone();

        let tab_list_id = self.id.clone();
        let accessible_labels = self.accessible_labels.unwrap_or_default();
        let tabs = self
            .tab_labels
            .into_iter()
            .zip(focus_handles)
            .enumerate()
            .map(|(index, (tab_label, focus_handle))| {
                let tab_id = ElementId::NamedChild(
                    Arc::new(self.id.clone()),
                    SharedString::from(format!("tab-{index}")),
                );
                let is_selected = index == self.selected_index;
                let hover = remembered_hover(&tab_id, window, cx);
                let target_text = if is_selected {
                    tokens.component_tabs_text_selected
                } else if *hover.read(cx) {
                    tokens.component_tabs_text_hover
                } else {
                    tokens.component_tabs_text
                };
                let text =
                    colour_transition(child_id(&tab_id, "text"), color(target_text), window, cx);
                let is_ring_shown = is_focus_visible(&focus_handle, window);
                let on_select = on_select.clone();
                with_type(div(), TYPE_BODY)
                    .id(tab_id)
                    .role(Role::Tab)
                    .aria_label(
                        accessible_labels
                            .get(index)
                            .cloned()
                            .unwrap_or_else(|| tab_label.clone()),
                    )
                    .aria_selected(is_selected)
                    .aria_position_in_set(index + 1)
                    .aria_size_of_set(tab_count)
                    .track_focus(&focus_handle)
                    .relative()
                    .flex()
                    .items_center()
                    .h(length(COMPONENT_TABS_HEIGHT))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(text)
                    .cursor_pointer()
                    .border_b(length(COMPONENT_TABS_INDICATOR_HEIGHT))
                    .border_color(if is_selected {
                        color(tokens.component_tabs_indicator)
                    } else {
                        gpui::transparent_black()
                    })
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
                    .child(tab_label)
                    .when(is_ring_shown, |tab| tab.child(focus_ring(RADIUS_XS, cx)))
            });

        let selected_index = self.selected_index;
        div()
            .id(tab_list_id)
            .role(Role::TabList)
            .aria_label(self.label)
            .flex()
            .flex_wrap()
            .items_center()
            .gap(length(COMPONENT_TABS_GAP))
            .border_b_1()
            .border_color(color(tokens.component_tabs_divider))
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.modifiers.modified() {
                    return;
                }
                let Some(next) = next_tab_index(&event.keystroke.key, selected_index, tab_count)
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
            .children(tabs)
    }
}

debug_without_callbacks!(Tabs, id, label);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_wrap_and_home_end_jump() {
        assert_eq!(next_tab_index("right", 2, 3), Some(0));
        assert_eq!(next_tab_index("left", 0, 3), Some(2));
        assert_eq!(next_tab_index("home", 2, 3), Some(0));
        assert_eq!(next_tab_index("end", 0, 3), Some(2));
        assert_eq!(next_tab_index("down", 0, 3), None);
        assert_eq!(next_tab_index("right", 0, 0), None);
    }
}
