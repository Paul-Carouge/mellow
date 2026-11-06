//! Mellow's Dusk primitives for GPUI (DESIGN.md §5–§6, ADR-0051): the web client's components
//! (`web/src/components`) redrawn from the generated tokens, with the same sizes, colours per state,
//! focus ring, motion and keyboard behaviour, and AccessKit roles and names.
//!
//! gpui-component brings the text input; everything visible is drawn here so it looks like Dusk,
//! not like gpui-component's own theme.

/// `Debug` for primitives holding callbacks, views or pictures, which have none: the primitive's
/// name and its id or label are what a log line or a failing test needs.
macro_rules! debug_without_callbacks {
    ($type:ty $(, $field:ident)*) => {
        impl std::fmt::Debug for $type {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter
                    .debug_struct(stringify!($type))
                    $(.field(stringify!($field), &self.$field))*
                    .finish_non_exhaustive()
            }
        }
    };
}

mod avatar;
mod badge;
mod button;
mod choice;
mod dialog;
pub mod gallery;
#[cfg(test)]
mod keyboard_tests;
mod menu;
mod progress;
mod range_field;
mod settings_list_status;
mod status;
mod switch;
mod tabs;
mod text_field;
mod tooltip;
mod unsaved_bar;

use std::sync::Arc;

use gpui::{
    App, BoxShadow, ElementId, FocusHandle, FontWeight, Hsla, InteractiveElement, IntoElement,
    KeyBinding, SharedString, Styled, Svg, Window, actions, div, point, svg,
};
use gpui_base::motion::{Easing, Transition, transition};

pub use avatar::{Avatar, AvatarPerson, GroupAvatar};
pub use badge::{MentionBadge, RoleChip, RoleChips};
pub use button::{Button, ButtonSize, ButtonVariant, IconButton, IconButtonTone};
pub use choice::{ChoiceGroup, ChoiceOption, next_choice_index};
pub use dialog::{ModalDialog, ModalDialogEvent};
pub use menu::{
    ContextMenu, MenuAlignment, MenuAnchor, MenuEntry, MenuItem, MenuItemTone, MenuOrigin,
    MenuPlacement, menu_top_next_to_anchor, next_menu_item_index, place_menu,
    popover_position_beside_anchor, typeahead_menu_item_index,
};
pub use progress::ProgressMeter;
pub use range_field::{RangeBounds, RangeField};
pub use settings_list_status::{SettingsListStatus, SettingsListStatusKind};
pub use status::{ShownStatus, StatusIndicator};
pub use switch::Switch;
pub use tabs::{Tabs, next_tab_index};
pub use text_field::{PasswordToggle, TextField, sync_text_input_colours};
pub use tooltip::Tooltip;
pub use unsaved_bar::{UnsavedChangesBar, UnsavedChangesLabels, shake_offset};

use crate::theme::icons::IconName;
use crate::theme::tokens::{
    MOTION_DURATION_FAST, MOTION_EASING_STANDARD, SIZE_FOCUS_RING_OFFSET, SIZE_FOCUS_RING_WIDTH,
    ShadowLayer, TypeToken,
};
use crate::theme::{animation_duration, color, length};

/// What a primitive runs when it is activated: a click, Enter or Space.
pub type ActivationHandler = Arc<dyn Fn(&mut Window, &mut App) + 'static>;

/// Applies a composite type token (DESIGN.md §2.4). GPUI has no letter spacing yet, so the four
/// tracked display tokens lose their tracking (a documented gap).
pub fn with_type<E: Styled>(element: E, token: TypeToken) -> E {
    let family = token.font_family.first().copied().unwrap_or_default();
    element
        .font_family(family)
        .text_size(length(token.font_size))
        .line_height(length(token.font_size * token.line_height))
        .font_weight(FontWeight(f32::from(token.font_weight)))
}

/// A shadow token as GPUI box shadows.
#[must_use]
pub fn shadow(layers: &[ShadowLayer]) -> Vec<BoxShadow> {
    layers
        .iter()
        .map(|layer| BoxShadow {
            color: color(layer.color),
            offset: point(
                length(layer.offset_x).to_pixels(DEFAULT_REM),
                length(layer.offset_y).to_pixels(DEFAULT_REM),
            ),
            blur_radius: length(layer.blur).to_pixels(DEFAULT_REM),
            spread_radius: length(layer.spread).to_pixels(DEFAULT_REM),
            inset: false,
        })
        .collect()
}

// shortcut: shadows are fixed pixels while every other length is in rems, so a zoomed window keeps
// 100 % shadows; upgrade when zoom lands (DESIGN.md §7.5) by reading the window's rem size.
const DEFAULT_REM: gpui::Pixels = gpui::px(16.0);

/// A Phosphor icon tinted like text, as the web's `Icon.svelte` (DESIGN.md §2.9).
#[must_use]
pub fn icon(name: IconName, size: f32, tint: Hsla) -> Svg {
    svg()
        .path(name.asset_path())
        .flex_none()
        .size(length(size))
        .text_color(tint)
}

/// Keyboard focus only, as CSS `:focus-visible`: a click does not draw the ring.
#[must_use]
pub fn is_focus_visible(focus_handle: &FocusHandle, window: &Window) -> bool {
    focus_handle.is_focused(window) && window.last_input_was_keyboard()
}

/// The focus ring (DESIGN.md §7.2): 2 px of `color.focus`, 2 px away from the element, following
/// its radius concentrically. A child of the element, so the element must not clip its overflow.
#[must_use]
pub fn focus_ring(element_radius: f32, cx: &App) -> impl IntoElement {
    let distance = SIZE_FOCUS_RING_WIDTH + SIZE_FOCUS_RING_OFFSET;
    let outside = -length(distance);
    div()
        .absolute()
        .top(outside)
        .left(outside)
        .right(outside)
        .bottom(outside)
        .rounded(length(element_radius + distance))
        .border(length(SIZE_FOCUS_RING_WIDTH))
        .border_color(color(crate::theme::theme(cx).color_focus))
}

/// The ring drawn inside an element, for rows a scrolling list would clip (`focus-ring-inset`).
#[must_use]
pub fn inset_focus_ring(element_radius: f32, cx: &App) -> impl IntoElement {
    div()
        .absolute()
        .inset_0()
        .rounded(length(element_radius))
        .border(length(SIZE_FOCUS_RING_WIDTH))
        .border_color(color(crate::theme::theme(cx).color_focus))
}

/// A token's cubic Bézier curve as an easing.
#[must_use]
pub fn easing(curve: [f32; 4]) -> Easing {
    let [x1, y1, x2, y2] = curve;
    Easing::CubicBezier { x1, y1, x2, y2 }
}

/// `transition-colors duration-fast ease-standard`: a colour moving towards its new value over
/// 120 ms, at once under reduced motion. Frames are requested only while it moves.
pub fn colour_transition(id: ElementId, target: Hsla, window: &mut Window, cx: &mut App) -> Hsla {
    let policy = Transition::new(animation_duration(MOTION_DURATION_FAST, cx))
        .easing(easing(MOTION_EASING_STANDARD));
    transition(id, target, policy, window, cx)
}

/// A child id under a primitive's own id, for the state it keeps between frames.
#[must_use]
pub fn child_id(parent: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(parent.clone()), SharedString::new_static(name))
}

/// Whether the pointer is over the element, remembered between frames (set by `on_hover`).
pub fn remembered_hover(id: &ElementId, window: &mut Window, cx: &mut App) -> gpui::Entity<bool> {
    window.use_keyed_state(child_id(id, "hover"), cx, |_, _| false)
}

/// A focus handle that lives as long as the element keeps being drawn.
pub fn remembered_focus_handle(id: &ElementId, window: &mut Window, cx: &mut App) -> FocusHandle {
    window
        .use_keyed_state(child_id(id, "focus"), cx, |_, cx| cx.focus_handle())
        .read(cx)
        .clone()
        .tab_stop(true)
}

actions!(
    mellow_components,
    [
        /// Tab: the next control, as browsers do.
        FocusNextControl,
        /// Shift+Tab: the previous control.
        FocusPreviousControl
    ]
);

/// Tab and Shift+Tab move between controls; GPUI leaves those keys to the app.
pub fn bind_tab_navigation(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", FocusNextControl, None),
        KeyBinding::new("shift-tab", FocusPreviousControl, None),
    ]);
}

/// Handles Tab and Shift+Tab on a window's root element. Actions travel from the focused element
/// up, so the root must track a focus handle that holds focus while nothing else does.
pub fn with_tab_navigation<E: InteractiveElement>(element: E, root_focus: &FocusHandle) -> E {
    element
        .track_focus(root_focus)
        .on_action(|_: &FocusNextControl, window, cx| window.focus_next(cx))
        .on_action(|_: &FocusPreviousControl, window, cx| window.focus_prev(cx))
}
