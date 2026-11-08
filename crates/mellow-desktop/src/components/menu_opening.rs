//! How a row opens its context menu (`context-menu-trigger.ts`, DESIGN.md §6.33): a right-click
//! opens it at the pointer; Shift+F10 or the Menu key on the focused row opens it beside the row,
//! whose place in the window is remembered while it is drawn.

use gpui::{
    App, Bounds, ElementId, Entity, IntoElement, Keystroke, Pixels, Point, Styled as _, Window,
    canvas,
};

use super::child_id;
use super::menu::{MenuAlignment, MenuAnchor, MenuOrigin};

/// Shift+F10, or the Menu key, as on every desktop (WAI-ARIA, Discord): `isContextMenuKey`.
#[must_use]
pub fn is_context_menu_keystroke(keystroke: &Keystroke) -> bool {
    let modifiers = &keystroke.modifiers;
    if modifiers.control || modifiers.alt || modifiers.platform {
        return false;
    }
    keystroke.key == "menu" || (keystroke.key == "f10" && modifiers.shift)
}

/// A menu opened by a pointer at `position`.
#[must_use]
pub fn menu_origin_at_pointer(position: Point<Pixels>) -> MenuOrigin {
    MenuOrigin::Point {
        x: f32::from(position.x),
        y: f32::from(position.y),
    }
}

/// A menu lined up with an element drawn at `bounds`.
#[must_use]
pub fn menu_origin_beside(bounds: Bounds<Pixels>, alignment: MenuAlignment) -> MenuOrigin {
    MenuOrigin::Anchor {
        bounds: MenuAnchor {
            top: f32::from(bounds.top()),
            bottom: f32::from(bounds.bottom()),
            left: f32::from(bounds.left()),
            right: f32::from(bounds.right()),
        },
        alignment,
    }
}

/// Where the element `id` was drawn last, written by [`bounds_recorder`].
pub fn remembered_bounds(
    id: &ElementId,
    window: &mut Window,
    cx: &mut App,
) -> Entity<Option<Bounds<Pixels>>> {
    window.use_keyed_state(child_id(id, "bounds"), cx, |_, _| None)
}

/// Fills its `relative` parent and writes the parent's bounds into `bounds` each frame.
#[must_use]
pub fn bounds_recorder(bounds: Entity<Option<Bounds<Pixels>>>) -> impl IntoElement {
    canvas(
        move |drawn, _, cx| bounds.update(cx, |remembered, _| *remembered = Some(drawn)),
        |_, (), _, _| {},
    )
    .absolute()
    .size_full()
}

/// The keyboard's origin for a row: beside it once drawn, else nowhere (the key does nothing).
pub fn menu_origin_beside_remembered(
    bounds: &Entity<Option<Bounds<Pixels>>>,
    cx: &App,
) -> Option<MenuOrigin> {
    bounds
        .read(cx)
        .map(|bounds| menu_origin_beside(bounds, MenuAlignment::Start))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shift_f10_and_the_menu_key_open_a_menu_but_not_with_ctrl_alt_or_cmd() {
        let key = |text: &'static str| Keystroke::parse(text).ok();
        for opening in ["shift-f10", "menu"] {
            assert!(key(opening).is_some_and(|keystroke| is_context_menu_keystroke(&keystroke)));
        }
        for other in ["f10", "ctrl-shift-f10", "alt-menu", "cmd-menu", "enter"] {
            assert!(!key(other).is_some_and(|keystroke| is_context_menu_keystroke(&keystroke)));
        }
    }
}
