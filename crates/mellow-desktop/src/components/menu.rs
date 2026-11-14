//! `ContextMenu.svelte`, `menu-keyboard.ts`, `menu-placement.ts` and `popover-placement.ts`
//! (DESIGN.md §6.33): the one menu, placed at the pointer or next to the element that opened it,
//! kept inside the window, dropping in (or lifting in when above). Arrows wrap, Home and End jump,
//! a letter moves to the next item starting with it (accents ignored), Esc and a click outside
//! close it, and focus goes back to the opener.

use std::sync::Arc;
use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, App, Context, DismissEvent, ElementId, EventEmitter, FocusHandle,
    Focusable, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    ParentElement as _, Pixels, Point, Render, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Toggled, Window, anchored, deferred, div, point, prelude::FluentBuilder as _, px,
};
use mellow_client_core::shared_logic::search_text::fold_for_search;

use super::{
    ActivationHandler, FocusNextControl, FocusPreviousControl, ShownStatus, StatusIndicator,
    easing, icon, inset_focus_ring, shadow, with_type,
};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_ACTION_BAR_BUTTON_SIZE, COMPONENT_ACTION_BAR_ICON_SIZE,
    COMPONENT_BUTTON_DISABLED_OPACITY, COMPONENT_CONTEXT_MENU_BORDER_WIDTH,
    COMPONENT_CONTEXT_MENU_ITEM_HEIGHT, COMPONENT_CONTEXT_MENU_ITEM_ICON_SIZE,
    COMPONENT_CONTEXT_MENU_ITEM_PADDING_INLINE, COMPONENT_CONTEXT_MENU_ITEM_RADIUS,
    COMPONENT_CONTEXT_MENU_MAX_WIDTH, COMPONENT_CONTEXT_MENU_MIN_WIDTH,
    COMPONENT_CONTEXT_MENU_PADDING, COMPONENT_CONTEXT_MENU_RADIUS,
    COMPONENT_CONTEXT_MENU_SEPARATOR_HEIGHT, COMPONENT_CONTEXT_MENU_SEPARATOR_MARGIN_BLOCK,
    COMPONENT_CONTEXT_MENU_SEPARATOR_MARGIN_INLINE, COMPONENT_CONTEXT_MENU_VIEWPORT_MARGIN,
    COMPONENT_REACTION_BORDER_WIDTH, COMPONENT_REACTION_RADIUS, COMPONENT_STATUS_SIZE_MENU,
    MOTION_DURATION_FAST, MOTION_EASING_ENTER, SPACE_2XS, SPACE_XS, TYPE_MENU_ITEM, ThemeTokens,
};
use crate::theme::{color, length, theme};

// ---- Keyboard (menu-keyboard.ts) ----

/// The item a key moves to, or `None` for keys a menu does not handle. From nowhere, Up lands on
/// the last item.
#[must_use]
pub fn next_menu_item_index(key: &str, current: Option<usize>, item_count: usize) -> Option<usize> {
    if item_count == 0 {
        return None;
    }
    match key {
        "down" => Some(current.map_or(0, |index| (index + 1) % item_count)),
        "up" => Some(current.map_or(item_count - 1, |index| {
            (index + item_count - 1) % item_count
        })),
        "home" => Some(0),
        "end" => Some(item_count - 1),
        _ => None,
    }
}

/// Type-ahead: the next item after the current one whose label starts with the typed character,
/// wrapping; typing it again cycles through the items that share it. `None` when none does or the
/// key is not one visible character.
#[must_use]
pub fn typeahead_menu_item_index(
    typed: &str,
    current: Option<usize>,
    labels: &[SharedString],
) -> Option<usize> {
    let mut characters = typed.chars();
    let (Some(character), None) = (characters.next(), characters.next()) else {
        return None;
    };
    if character.is_whitespace() || character.is_control() || labels.is_empty() {
        return None;
    }
    let folded = fold_for_search(typed);
    let start = current.map_or(0, |index| index + 1);
    (0..labels.len())
        .map(|step| (start + step) % labels.len())
        .find(|&index| fold_for_search(labels[index].trim()).starts_with(&folded))
}

// ---- Placement (menu-placement.ts, popover-placement.ts) ----

/// An element's edges in the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MenuAnchor {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

/// Which of the element's edges the menu lines up with; `Stretch` makes it at least as wide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAlignment {
    Start,
    End,
    Stretch,
}

/// What opened the menu: a pointer at a point (right-click), or an element (a button, or the
/// focused item for Shift+F10 and the Menu key).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MenuOrigin {
    Point {
        x: f32,
        y: f32,
    },
    Anchor {
        bounds: MenuAnchor,
        alignment: MenuAlignment,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MenuPlacement {
    pub top: f32,
    pub left: f32,
    /// A stretched menu's least width (its element's).
    pub minimum_width: Option<f32>,
    /// Opened above its origin: it lifts in rather than drops in.
    pub is_above: bool,
}

/// Below the element when the window has room, else above it, never past the window's top.
#[must_use]
pub fn menu_top_next_to_anchor(
    anchor_top: f32,
    anchor_bottom: f32,
    menu_height: f32,
    viewport_height: f32,
    gap: f32,
) -> f32 {
    let top_when_below = anchor_bottom + gap;
    if top_when_below + menu_height <= viewport_height {
        top_when_below
    } else {
        (anchor_top - gap - menu_height).max(0.0)
    }
}

fn clamp_into_window(start: f32, size: f32, limit: f32, margin: f32) -> f32 {
    start.min(limit - size - margin).max(margin)
}

/// The menu's top-left corner in the window (`placeMenu`). `gap` separates it from an element;
/// `margin` is the least room it keeps from the window's edges when it opens at a point.
#[must_use]
pub fn place_menu(
    origin: MenuOrigin,
    menu: (f32, f32),
    viewport: (f32, f32),
    gap: f32,
    margin: f32,
) -> MenuPlacement {
    let (menu_width, menu_height) = menu;
    let (viewport_width, viewport_height) = viewport;
    match origin {
        MenuOrigin::Point { x, y } => {
            // Down and to the trailing side of the pointer, as desktop menus open; flipped back
            // over the pointer when that side has no room, then kept inside the window.
            let fits_right = x + menu_width + margin <= viewport_width;
            let fits_below = y + menu_height + margin <= viewport_height;
            let top = clamp_into_window(
                if fits_below { y } else { y - menu_height },
                menu_height,
                viewport_height,
                margin,
            );
            MenuPlacement {
                top,
                left: clamp_into_window(
                    if fits_right { x } else { x - menu_width },
                    menu_width,
                    viewport_width,
                    margin,
                ),
                minimum_width: None,
                is_above: top < y,
            }
        }
        MenuOrigin::Anchor { bounds, alignment } => {
            let top = menu_top_next_to_anchor(
                bounds.top,
                bounds.bottom,
                menu_height,
                viewport_height,
                gap,
            );
            let is_above = top < bounds.top;
            match alignment {
                MenuAlignment::Stretch => {
                    let minimum_width = bounds.right - bounds.left;
                    MenuPlacement {
                        top,
                        left: clamp_into_window(
                            bounds.left,
                            menu_width.max(minimum_width),
                            viewport_width,
                            0.0,
                        ),
                        minimum_width: Some(minimum_width),
                        is_above,
                    }
                }
                MenuAlignment::End => MenuPlacement {
                    top,
                    left: (bounds.right - menu_width).max(0.0),
                    minimum_width: None,
                    is_above,
                },
                MenuAlignment::Start => MenuPlacement {
                    top,
                    left: clamp_into_window(bounds.left, menu_width, viewport_width, 0.0),
                    minimum_width: None,
                    is_above,
                },
            }
        }
    }
}

/// A picker popover beside the button that opened it (`popoverPositionBesideAnchor`): its right
/// edge on the button's, below when there is room, else above, inside the window.
#[must_use]
pub fn popover_position_beside_anchor(
    anchor: MenuAnchor,
    panel: (f32, f32),
    viewport: (f32, f32),
    gap: f32,
) -> (f32, f32) {
    let top = menu_top_next_to_anchor(anchor.top, anchor.bottom, panel.1, viewport.1, gap);
    let left = (anchor.right - panel.0).max(0.0).min(viewport.0 - panel.0);
    (top, left.max(0.0))
}

// ---- The menu ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuItemTone {
    Normal,
    /// Discord's highlighted "Invite people".
    Accent,
    /// Comes last, after a separator.
    Danger,
}

/// A choice among several in a menu (the user card's statuses): the status shape leads, a check
/// marks the current one, and screen readers hear a radio item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MenuChoice {
    pub status: ShownStatus,
    pub is_checked: bool,
}

#[derive(Clone)]
pub struct MenuItem {
    pub label: SharedString,
    pub icon: IconName,
    pub choice: Option<MenuChoice>,
    /// An on/off item (a quick reaction already given): checked shows the check mark.
    pub is_checked: Option<bool>,
    /// Said after the label by screen readers ("You won't get notifications").
    pub description: Option<SharedString>,
    pub tone: MenuItemTone,
    /// The action's own shortcut, for screen readers ("Control+K").
    pub key_shortcuts: Option<SharedString>,
    pub is_disabled: bool,
    /// Runs after the menu closed and gave focus back.
    pub on_select: ActivationHandler,
}

impl MenuItem {
    #[must_use]
    pub fn new(
        label: impl Into<SharedString>,
        icon: IconName,
        on_select: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            label: label.into(),
            icon,
            choice: None,
            is_checked: None,
            description: None,
            tone: MenuItemTone::Normal,
            key_shortcuts: None,
            is_disabled: false,
            on_select: Arc::new(on_select),
        }
    }

    /// One of a set of choices, the current one checked.
    #[must_use]
    pub fn choice(
        label: impl Into<SharedString>,
        status: ShownStatus,
        is_checked: bool,
        on_select: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            choice: Some(MenuChoice { status, is_checked }),
            ..Self::new(label, IconName::Check, on_select)
        }
    }

    #[must_use]
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// `Some` makes it an on/off item, heard as checked or not.
    #[must_use]
    pub fn checked(mut self, is_checked: Option<bool>) -> Self {
        self.is_checked = is_checked;
        self
    }

    #[must_use]
    pub fn tone(mut self, tone: MenuItemTone) -> Self {
        self.tone = tone;
        self
    }

    #[must_use]
    pub fn key_shortcuts(mut self, key_shortcuts: impl Into<SharedString>) -> Self {
        self.key_shortcuts = Some(key_shortcuts.into());
        self
    }

    #[must_use]
    pub fn disabled(mut self, is_disabled: bool) -> Self {
        self.is_disabled = is_disabled;
        self
    }
}

#[derive(Clone)]
pub enum MenuEntry {
    Item(MenuItem),
    Separator,
}

/// A key of the row above a menu's items (a message's quick reactions, DESIGN.md §5.4): a glyph
/// drawn like a reaction chip, on or off, heard as a checkable item.
#[derive(Clone)]
pub struct MenuKey {
    /// The emoji drawn on the key.
    pub glyph: SharedString,
    /// What screen readers hear ("React with 👍").
    pub label: SharedString,
    /// Already given: drawn as the reader's own reaction, and a press takes it back.
    pub is_checked: bool,
    /// Runs after the menu closed and gave focus back.
    pub on_select: ActivationHandler,
}

/// The row of keys above a menu's items, and the name screen readers hear for it ("Add
/// reaction").
#[derive(Debug, Clone, Default)]
pub struct MenuKeys {
    pub label: SharedString,
    pub keys: Vec<MenuKey>,
}

/// The row of keys, with the room the row takes around its keys.
fn quick_keys_height(keys: &[MenuKey]) -> f32 {
    if keys.is_empty() {
        0.0
    } else {
        COMPONENT_ACTION_BAR_BUTTON_SIZE + 2.0 * SPACE_2XS
    }
}

/// The menu's height from its tokens: every row has a fixed height, so it is known before layout.
fn estimated_menu_height(keys: &[MenuKey], entries: &[MenuEntry]) -> f32 {
    let rows: f32 = entries
        .iter()
        .map(|entry| match entry {
            MenuEntry::Item(_) => COMPONENT_CONTEXT_MENU_ITEM_HEIGHT,
            MenuEntry::Separator => {
                COMPONENT_CONTEXT_MENU_SEPARATOR_HEIGHT
                    + 2.0 * COMPONENT_CONTEXT_MENU_SEPARATOR_MARGIN_BLOCK
            }
        })
        .sum();
    quick_keys_height(keys)
        + rows
        + 2.0 * (COMPONENT_CONTEXT_MENU_PADDING + COMPONENT_CONTEXT_MENU_BORDER_WIDTH)
}

/// An open menu. The host keeps the entity while it is open, renders it anywhere (it draws itself
/// above everything, at its place) and drops it on `DismissEvent`.
pub struct ContextMenu {
    label: SharedString,
    /// The row of keys above the items; empty for most menus.
    keys: Vec<MenuKey>,
    keys_label: SharedString,
    entries: Vec<MenuEntry>,
    focus_handle: FocusHandle,
    opener: Option<FocusHandle>,
    placement: MenuPlacement,
    /// The item keyboard focus is on, drawn with the ring.
    highlighted_index: Option<usize>,
    hovered_index: Option<usize>,
    /// The key keyboard focus is on; when set, no item is highlighted.
    highlighted_key: Option<usize>,
    hovered_key: Option<usize>,
}

impl EventEmitter<DismissEvent> for ContextMenu {}

impl Focusable for ContextMenu {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ContextMenu {
    /// Opens at `origin` and takes focus. A menu opened by a pointer at a point focuses itself
    /// with no item chosen, as desktop menus do; from a button or the keyboard, the first item
    /// takes focus. `opener` gets focus back when the menu closes.
    pub fn open(
        label: impl Into<SharedString>,
        entries: Vec<MenuEntry>,
        origin: MenuOrigin,
        opener: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::open_with_quick_keys(
            label,
            MenuKeys::default(),
            entries,
            origin,
            opener,
            window,
            cx,
        )
    }

    /// As [`Self::open`], with a row of `keys` above the items (a message's quick reactions).
    /// Left and Right move between the keys, Down goes to the first item and Up from the first
    /// item comes back to the keys.
    pub fn open_with_quick_keys(
        label: impl Into<SharedString>,
        keys: MenuKeys,
        entries: Vec<MenuEntry>,
        origin: MenuOrigin,
        opener: Option<FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let viewport = window.viewport_size();
        let rem_size = window.rem_size();
        let in_window = |pixels: f32| f32::from(length(pixels).to_pixels(rem_size));
        let placement = place_menu(
            origin,
            (
                in_window(COMPONENT_CONTEXT_MENU_MIN_WIDTH),
                in_window(estimated_menu_height(&keys.keys, &entries)),
            ),
            (f32::from(viewport.width), f32::from(viewport.height)),
            0.0,
            in_window(COMPONENT_CONTEXT_MENU_VIEWPORT_MARGIN),
        );
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        let highlighted_index = match origin {
            MenuOrigin::Point { .. } => None,
            MenuOrigin::Anchor { .. } => Some(0),
        };
        Self {
            label: label.into(),
            keys: keys.keys,
            keys_label: keys.label,
            entries,
            focus_handle,
            opener,
            placement,
            highlighted_index,
            hovered_index: None,
            highlighted_key: None,
            hovered_key: None,
        }
    }

    fn items(&self) -> impl Iterator<Item = &MenuItem> {
        self.entries.iter().filter_map(|entry| match entry {
            MenuEntry::Item(item) => Some(item),
            MenuEntry::Separator => None,
        })
    }

    /// The item keyboard focus is on.
    #[must_use]
    pub fn highlighted_index(&self) -> Option<usize> {
        self.highlighted_index
    }

    /// The key of the row above the items that keyboard focus is on.
    #[must_use]
    pub fn highlighted_key(&self) -> Option<usize> {
        self.highlighted_key
    }

    /// Closes, giving focus back to the opener first.
    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(opener) = self.opener.as_ref() {
            window.focus(opener, cx);
        }
        cx.emit(DismissEvent);
    }

    fn choose(&mut self, item_index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(item) = self.items().nth(item_index).cloned() else {
            return;
        };
        if item.is_disabled {
            return;
        }
        self.dismiss(window, cx);
        (item.on_select)(window, cx);
    }

    fn choose_key(&mut self, key_index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.keys.get(key_index).cloned() else {
            return;
        };
        self.dismiss(window, cx);
        (key.on_select)(window, cx);
    }

    /// Keys while focus is on the row of keys: `true` when handled.
    fn on_key_in_quick_keys(
        &mut self,
        key: &str,
        key_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key_count = self.keys.len();
        match key {
            "left" => self.highlighted_key = Some((key_index + key_count - 1) % key_count),
            "right" => self.highlighted_key = Some((key_index + 1) % key_count),
            "home" => self.highlighted_key = Some(0),
            "end" => self.highlighted_key = Some(key_count - 1),
            "down" if self.items().next().is_some() => {
                self.highlighted_key = None;
                self.highlighted_index = Some(0);
            }
            "enter" | "space" => {
                self.choose_key(key_index, window, cx);
                return true;
            }
            _ => return false,
        }
        cx.notify();
        true
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let item_count = self.items().count();
        if keystroke.modifiers.modified() && !keystroke.modifiers.shift {
            return;
        }
        if keystroke.key == "escape" {
            cx.stop_propagation();
            self.dismiss(window, cx);
            return;
        }
        if let Some(key_index) = self.highlighted_key
            && self.on_key_in_quick_keys(&keystroke.key, key_index, window, cx)
        {
            cx.stop_propagation();
            return;
        }
        // Up from the first item, or Left or Right before any item, reaches the row of keys.
        let is_up_from_first_item = keystroke.key == "up" && self.highlighted_index == Some(0);
        let is_sideways_from_nowhere =
            matches!(keystroke.key.as_str(), "left" | "right") && self.highlighted_index.is_none();
        if !self.keys.is_empty() && (is_up_from_first_item || is_sideways_from_nowhere) {
            cx.stop_propagation();
            self.highlighted_index = None;
            self.highlighted_key = Some(0);
            cx.notify();
            return;
        }
        match keystroke.key.as_str() {
            "enter" | "space" => {
                cx.stop_propagation();
                if let Some(index) = self.highlighted_index {
                    self.choose(index, window, cx);
                }
                return;
            }
            _ => {}
        }
        let next = next_menu_item_index(&keystroke.key, self.highlighted_index, item_count)
            .or_else(|| {
                let labels: Vec<SharedString> =
                    self.items().map(|item| item.label.clone()).collect();
                keystroke.key_char.as_deref().and_then(|typed| {
                    typeahead_menu_item_index(typed, self.highlighted_index, &labels)
                })
            });
        if let Some(next) = next {
            cx.stop_propagation();
            // A letter typed on the row of keys moves to its item too.
            self.highlighted_key = None;
            self.highlighted_index = Some(next);
            cx.notify();
        }
    }
}

/// Text and fill of an item at rest, and under the pointer or keyboard focus.
fn item_colours(tone: MenuItemTone, is_active: bool, tokens: &ThemeTokens) -> (Hsla, Option<Hsla>) {
    match (tone, is_active) {
        (MenuItemTone::Danger, true) => (
            color(tokens.component_context_menu_danger_text_focus),
            Some(color(tokens.component_context_menu_danger_background_focus)),
        ),
        (MenuItemTone::Danger, false) => (color(tokens.component_context_menu_danger_text), None),
        (_, true) => (
            color(tokens.component_context_menu_item_text_focus),
            Some(color(tokens.component_context_menu_item_background_focus)),
        ),
        (MenuItemTone::Accent, false) => (color(tokens.component_context_menu_accent_text), None),
        (MenuItemTone::Normal, false) => (color(tokens.component_context_menu_item_text), None),
    }
}

impl ContextMenu {
    /// The row of keys above the items, drawn as the message's reaction chips (the web phone
    /// sheet's `MessageMenuQuickReactions.svelte`, at the desktop's 32 px).
    fn render_quick_keys(&self, window: &Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let tokens = theme(cx);
        let is_keyboard_focus = window.last_input_was_keyboard();
        let keys = self.keys.iter().enumerate().map(|(index, key)| {
            let is_highlighted = self.highlighted_key == Some(index);
            let is_lit = is_highlighted || self.hovered_key == Some(index);
            let (background, border) = if key.is_checked {
                (
                    tokens.component_reaction_background_me,
                    tokens.component_reaction_border_me,
                )
            } else if is_lit {
                (
                    tokens.component_reaction_background_hover,
                    tokens.component_reaction_border_hover,
                )
            } else {
                (
                    tokens.component_reaction_background,
                    tokens.component_reaction_border,
                )
            };
            div()
                .id(ElementId::named_usize("menu-key", index))
                .role(Role::MenuItemCheckBox)
                .aria_label(key.label.clone())
                .aria_toggled(if key.is_checked {
                    Toggled::True
                } else {
                    Toggled::False
                })
                .when(
                    is_highlighted,
                    gpui::StatefulInteractiveElement::aria_active_descendant,
                )
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(length(COMPONENT_ACTION_BAR_BUTTON_SIZE))
                .rounded(length(COMPONENT_REACTION_RADIUS))
                .border(length(COMPONENT_REACTION_BORDER_WIDTH))
                .border_color(color(border))
                .bg(color(background))
                .text_size(length(COMPONENT_ACTION_BAR_ICON_SIZE))
                .line_height(length(COMPONENT_ACTION_BAR_ICON_SIZE))
                .cursor_pointer()
                .on_hover(cx.listener(move |menu, is_hovered: &bool, _, cx| {
                    menu.hovered_key = if *is_hovered {
                        Some(index)
                    } else {
                        menu.hovered_key.filter(|hovered| *hovered != index)
                    };
                    cx.notify();
                }))
                .on_click(cx.listener(move |menu, _, window, cx| {
                    menu.choose_key(index, window, cx);
                }))
                .child(key.glyph.clone())
                .when(is_highlighted && is_keyboard_focus, |key| {
                    key.child(inset_focus_ring(COMPONENT_REACTION_RADIUS, cx))
                })
        });
        div()
            .id("menu-keys")
            .role(Role::Group)
            .aria_label(self.keys_label.clone())
            .flex()
            .gap(length(SPACE_XS))
            .px(length(COMPONENT_CONTEXT_MENU_ITEM_PADDING_INLINE))
            .py(length(SPACE_2XS))
            .children(keys)
            .into_any_element()
    }
}

impl Render for ContextMenu {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let is_keyboard_focus = window.last_input_was_keyboard();
        let mut item_index = 0;
        let rows: Vec<gpui::AnyElement> = self
            .entries
            .iter()
            .map(|entry| match entry {
                MenuEntry::Separator => div()
                    .flex_none()
                    .mx(length(COMPONENT_CONTEXT_MENU_SEPARATOR_MARGIN_INLINE))
                    .my(length(COMPONENT_CONTEXT_MENU_SEPARATOR_MARGIN_BLOCK))
                    .h(length(COMPONENT_CONTEXT_MENU_SEPARATOR_HEIGHT))
                    .bg(color(tokens.component_context_menu_separator))
                    .into_any_element(),
                MenuEntry::Item(item) => {
                    let index = item_index;
                    item_index += 1;
                    let is_highlighted = self.highlighted_index == Some(index);
                    let is_active = is_highlighted || self.hovered_index == Some(index);
                    let (text, fill) = item_colours(item.tone, is_active, tokens);
                    let choice = item.choice;
                    let is_checked = item.is_checked;
                    with_type(div(), TYPE_MENU_ITEM)
                        .id(ElementId::named_usize("menu-item", index))
                        .role(if choice.is_some() {
                            Role::MenuItemRadio
                        } else if is_checked.is_some() {
                            Role::MenuItemCheckBox
                        } else {
                            Role::MenuItem
                        })
                        .aria_label(item.label.clone())
                        .when_some(item.description.clone(), |row, description| {
                            row.aria_description(description)
                        })
                        .when_some(choice, |row, choice| {
                            row.aria_toggled(if choice.is_checked {
                                Toggled::True
                            } else {
                                Toggled::False
                            })
                        })
                        .when_some(is_checked, |row, is_checked| {
                            row.aria_toggled(if is_checked {
                                Toggled::True
                            } else {
                                Toggled::False
                            })
                        })
                        .when_some(item.key_shortcuts.clone(), |row, shortcuts| {
                            row.aria_keyshortcuts(shortcuts)
                        })
                        .when(
                            is_highlighted,
                            gpui::StatefulInteractiveElement::aria_active_descendant,
                        )
                        .relative()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap(length(SPACE_XS))
                        .min_h(length(COMPONENT_CONTEXT_MENU_ITEM_HEIGHT))
                        .px(length(COMPONENT_CONTEXT_MENU_ITEM_PADDING_INLINE))
                        .py(length(SPACE_2XS / 2.0))
                        .rounded(length(COMPONENT_CONTEXT_MENU_ITEM_RADIUS))
                        .text_color(text)
                        .when_some(fill, gpui::Styled::bg)
                        .when(item.is_disabled, |row| {
                            row.opacity(COMPONENT_BUTTON_DISABLED_OPACITY)
                        })
                        .when(!item.is_disabled, gpui::Styled::cursor_pointer)
                        .on_hover(cx.listener(move |menu, is_hovered: &bool, _, cx| {
                            menu.hovered_index = if *is_hovered {
                                Some(index)
                            } else {
                                menu.hovered_index.filter(|hovered| *hovered != index)
                            };
                            cx.notify();
                        }))
                        .on_click(cx.listener(move |menu, _, window, cx| {
                            menu.choose(index, window, cx);
                        }))
                        .when_some(choice, |row, choice| {
                            row.child(
                                div()
                                    .flex_none()
                                    .size(length(COMPONENT_CONTEXT_MENU_ITEM_ICON_SIZE))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(StatusIndicator::new(
                                        choice.status,
                                        COMPONENT_STATUS_SIZE_MENU,
                                    )),
                            )
                        })
                        .child(div().flex_1().min_w_0().child(item.label.clone()))
                        // The check says which choice is current without relying on colour.
                        .when(choice.is_none_or(|choice| choice.is_checked), |row| {
                            let shown = if is_checked == Some(true) {
                                IconName::Check
                            } else {
                                item.icon
                            };
                            row.child(icon(shown, COMPONENT_CONTEXT_MENU_ITEM_ICON_SIZE, text))
                        })
                        .when(is_highlighted && is_keyboard_focus, |row| {
                            row.child(inset_focus_ring(COMPONENT_CONTEXT_MENU_ITEM_RADIUS, cx))
                        })
                        .into_any_element()
                }
            })
            .collect();

        let quick_keys = (!self.keys.is_empty()).then(|| self.render_quick_keys(window, cx));

        let is_above = self.placement.is_above;
        let drop_distance = SPACE_2XS;
        let card = div()
            .id("context-menu")
            .role(Role::Menu)
            .aria_label(self.label.clone())
            .track_focus(&self.focus_handle)
            .key_context("ContextMenu")
            .on_key_down(cx.listener(Self::on_key_down))
            // Tab is bound to these actions, and bindings run before key-down listeners.
            .on_action(cx.listener(|menu, _: &FocusNextControl, window, cx| {
                menu.dismiss(window, cx);
            }))
            .on_action(cx.listener(|menu, _: &FocusPreviousControl, window, cx| {
                menu.dismiss(window, cx);
            }))
            .on_mouse_down_out(cx.listener(|menu, _, window, cx| menu.dismiss(window, cx)))
            // A right-click inside the menu must not reach what lies under it.
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .flex()
            .flex_col()
            .min_w(length(
                self.placement
                    .minimum_width
                    .map_or(COMPONENT_CONTEXT_MENU_MIN_WIDTH, |width| {
                        width.max(COMPONENT_CONTEXT_MENU_MIN_WIDTH)
                    }),
            ))
            .max_w(length(COMPONENT_CONTEXT_MENU_MAX_WIDTH))
            .p(length(COMPONENT_CONTEXT_MENU_PADDING))
            .rounded(length(COMPONENT_CONTEXT_MENU_RADIUS))
            .border(length(COMPONENT_CONTEXT_MENU_BORDER_WIDTH))
            .border_color(color(tokens.component_context_menu_border))
            .bg(color(tokens.component_context_menu_background))
            .shadow(shadow(tokens.component_context_menu_shadow))
            .children(quick_keys)
            .children(rows)
            .with_animation(
                "context-menu-enter",
                Animation::new(Duration::from_millis(MOTION_DURATION_FAST.0))
                    .with_easing(|progress| easing(MOTION_EASING_ENTER).sample(progress)),
                move |card, progress| {
                    // Drops in below its origin, lifts in above it.
                    let travel = drop_distance * (1.0 - progress);
                    card.mt(length(if is_above { travel } else { -travel }))
                        .opacity(progress)
                },
            );

        let margin = px(COMPONENT_CONTEXT_MENU_VIEWPORT_MARGIN);
        let position: Point<Pixels> = point(px(self.placement.left), px(self.placement.top));
        deferred(
            anchored()
                .position(position)
                .snap_to_window_with_margin(margin)
                .child(card),
        )
        .with_priority(1)
    }
}

debug_without_callbacks!(MenuItem, label);
debug_without_callbacks!(MenuKey, glyph);
debug_without_callbacks!(MenuEntry);
debug_without_callbacks!(ContextMenu, label);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_wrap_and_home_end_jump() {
        assert_eq!(next_menu_item_index("down", Some(2), 3), Some(0));
        assert_eq!(next_menu_item_index("up", Some(0), 3), Some(2));
        assert_eq!(next_menu_item_index("up", None, 3), Some(2));
        assert_eq!(next_menu_item_index("down", None, 3), Some(0));
        assert_eq!(next_menu_item_index("home", Some(2), 3), Some(0));
        assert_eq!(next_menu_item_index("end", Some(0), 3), Some(2));
        assert_eq!(next_menu_item_index("left", Some(0), 3), None);
        assert_eq!(next_menu_item_index("down", None, 0), None);
    }

    #[test]
    fn typeahead_ignores_accents_and_cycles_through_shared_letters() {
        let labels: Vec<SharedString> = ["Copier", "Épingler", "Éditer", "Supprimer"]
            .into_iter()
            .map(SharedString::from)
            .collect();
        assert_eq!(typeahead_menu_item_index("e", None, &labels), Some(1));
        assert_eq!(typeahead_menu_item_index("e", Some(1), &labels), Some(2));
        assert_eq!(typeahead_menu_item_index("E", Some(2), &labels), Some(1));
        assert_eq!(typeahead_menu_item_index("s", Some(0), &labels), Some(3));
        assert_eq!(typeahead_menu_item_index("x", None, &labels), None);
        assert_eq!(typeahead_menu_item_index(" ", None, &labels), None);
        assert_eq!(typeahead_menu_item_index("ab", None, &labels), None);
    }

    #[test]
    fn a_pointer_menu_opens_down_and_right_and_flips_at_the_window_edges() {
        let menu = (200.0, 100.0);
        let viewport = (800.0, 600.0);
        let below = place_menu(
            MenuOrigin::Point { x: 100.0, y: 100.0 },
            menu,
            viewport,
            0.0,
            4.0,
        );
        assert_eq!(
            (below.left, below.top, below.is_above),
            (100.0, 100.0, false)
        );
        let corner = place_menu(
            MenuOrigin::Point { x: 780.0, y: 580.0 },
            menu,
            viewport,
            0.0,
            4.0,
        );
        assert_eq!(
            (corner.left, corner.top, corner.is_above),
            (580.0, 480.0, true)
        );
    }

    #[test]
    fn an_anchored_menu_goes_below_or_above_and_stretches_to_its_opener() {
        let button = MenuAnchor {
            top: 500.0,
            bottom: 540.0,
            left: 10.0,
            right: 250.0,
        };
        let above = place_menu(
            MenuOrigin::Anchor {
                bounds: button,
                alignment: MenuAlignment::Stretch,
            },
            (188.0, 100.0),
            (800.0, 600.0),
            8.0,
            4.0,
        );
        assert_eq!(above.top, 392.0);
        assert!(above.is_above);
        assert_eq!(above.minimum_width, Some(240.0));
        let end = place_menu(
            MenuOrigin::Anchor {
                bounds: MenuAnchor {
                    top: 10.0,
                    bottom: 50.0,
                    left: 100.0,
                    right: 300.0,
                },
                alignment: MenuAlignment::End,
            },
            (188.0, 100.0),
            (800.0, 600.0),
            0.0,
            4.0,
        );
        assert_eq!((end.top, end.left, end.is_above), (50.0, 112.0, false));
    }

    #[test]
    fn a_popover_lines_up_with_the_buttons_right_edge_inside_the_window() {
        let button = MenuAnchor {
            top: 100.0,
            bottom: 140.0,
            left: 50.0,
            right: 80.0,
        };
        assert_eq!(
            popover_position_beside_anchor(button, (300.0, 200.0), (800.0, 600.0), 8.0),
            (148.0, 0.0)
        );
    }

    #[test]
    fn the_estimated_height_counts_rows_separators_and_padding() {
        let noop = MenuItem::new("A", IconName::Copy, |_, _| {});
        let entries = vec![
            MenuEntry::Item(noop.clone()),
            MenuEntry::Separator,
            MenuEntry::Item(noop),
        ];
        assert!((estimated_menu_height(&[], &entries) - (32.0 * 2.0 + 9.0 + 14.0)).abs() < 1e-4);
        let key = MenuKey {
            glyph: "👍".into(),
            label: "React with 👍".into(),
            is_checked: false,
            on_select: Arc::new(|_, _| {}),
        };
        assert!(
            (estimated_menu_height(&[key], &entries) - (40.0 + 32.0 * 2.0 + 9.0 + 14.0)).abs()
                < 1e-4,
            "the row of keys: a 32 px key and `space.2xs` above and below"
        );
    }
}
