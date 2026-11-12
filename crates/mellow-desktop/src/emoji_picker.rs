//! `EmojiPicker.svelte` and `recent-emoji.svelte.ts` (DESIGN.md §6.52): search first, then a bar
//! of categories, then the recently used, the open server's own emoji (§6.58) and the built-in
//! set by category. One Tab stop each for the search, the bar and the emoji; arrows move within
//! them, Page Up and Page Down jump between categories, Enter picks, Esc closes. A light-dismiss
//! popover beside the button that opened it, kept inside the window.
//!
//! The rows are a virtual list, so opening it draws only what shows. The host owns the entity
//! while it is open, renders it anywhere (it draws itself above everything) and decides what a
//! pick does and where focus goes; the picker never moves focus out of itself.
//!
//! shortcut: section headings scroll with their rows instead of sticking to the top, and the
//! phone sheet form is left out (the desktop has no coarse pointer layout); revisit if the
//! picker gets a GIF tab beside it.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, Bounds, Context, ElementId,
    Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement as _, IntoElement, KeyBinding,
    KeyDownEvent, ListAlignment, ListOffset, ListState, ObjectFit, ParentElement as _, Pixels,
    Render, Role, SharedString, StatefulInteractiveElement as _, Styled as _, StyledImage as _,
    Subscription, Window, actions, anchored, deferred, div, img, point,
    prelude::FluentBuilder as _, px, relative,
};
use gpui_component::input::{Input, InputEvent, InputState};
use mellow_client_core::DownloadableFile;
use mellow_client_core::shared_logic::emoji_catalogue::EmojiCategory;
use mellow_client_core::shared_logic::emoji_picker::{
    EmojiGridKeyPress, EmojiGridMove, PickerEmoji, PickerSection, PickerSectionKind,
    next_emoji_cell, picker_sections,
};
use mellow_protocol::{CustomEmoji, Snowflake};

use crate::components::{
    FocusNextControl, FocusPreviousControl, LiveRegion as _, MenuAnchor, Tooltip, easing,
    focus_next_inside, focus_previous_inside, icon, inset_focus_ring,
    popover_position_beside_anchor, shadow, with_type,
};
use crate::i18n::{Strings, strings};
use crate::messages::{picture_animation, picture_for_box};
use crate::preferences::{change_preferences, preferences};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_CONTEXT_MENU_ITEM_ICON_SIZE, COMPONENT_CONTEXT_MENU_VIEWPORT_MARGIN,
    COMPONENT_EMOJI_PICKER_BORDER_WIDTH, COMPONENT_EMOJI_PICKER_CATEGORY_ICON_SIZE,
    COMPONENT_EMOJI_PICKER_CATEGORY_RADIUS, COMPONENT_EMOJI_PICKER_CATEGORY_SIZE,
    COMPONENT_EMOJI_PICKER_CELL_RADIUS, COMPONENT_EMOJI_PICKER_CELL_SIZE,
    COMPONENT_EMOJI_PICKER_EMOJI_SIZE, COMPONENT_EMOJI_PICKER_GAP, COMPONENT_EMOJI_PICKER_HEIGHT,
    COMPONENT_EMOJI_PICKER_PADDING, COMPONENT_EMOJI_PICKER_RADIUS,
    COMPONENT_EMOJI_PICKER_SEARCH_HEIGHT, COMPONENT_EMOJI_PICKER_WIDTH, MOTION_DURATION_FAST,
    MOTION_EASING_ENTER, RADIUS_SM, SPACE_2XS, SPACE_XS, TYPE_BODY, TYPE_CATEGORY,
};
use crate::theme::{color, length, theme};

/// Popovers draw at menus' priority; modal dialogs' scrims draw at 2.
const POPOVER_DRAW_PRIORITY: usize = 1;
const OVER_DIALOGS_DRAW_PRIORITY: usize = 3;

/// Cells in a row: as many as the panel's inside holds (the web's `auto-fill` grid).
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Positive, far below `usize::MAX`.
pub const EMOJI_PICKER_COLUMNS: usize = ((COMPONENT_EMOJI_PICKER_WIDTH
    - 2.0 * (COMPONENT_EMOJI_PICKER_PADDING + COMPONENT_EMOJI_PICKER_BORDER_WIDTH))
    / COMPONENT_EMOJI_PICKER_CELL_SIZE) as usize;
/// Each cell's share of a row.
#[allow(clippy::cast_precision_loss)] // A handful of columns.
const CELL_WIDTH_SHARE: f32 = 1.0 / EMOJI_PICKER_COLUMNS as f32;
/// Rows drawn beyond the visible part, so scrolling does not pop in.
const LIST_OVERDRAW: f32 = COMPONENT_EMOJI_PICKER_HEIGHT;
const KEY_CONTEXT: &str = "EmojiPicker";

actions!(
    mellow_emoji_picker,
    [
        /// Down in the search field: into the emoji.
        DownInEmojiSearch,
        /// Esc anywhere in the picker: it closes.
        CloseEmojiPicker
    ]
);

/// Binds the picker's keys over the search field's own; call once at start, after
/// `gpui_component::init`, so these win over the field's bindings for the same keys.
pub fn bind_emoji_picker_keys(cx: &mut App) {
    let search = Some("EmojiPicker > Input");
    cx.bind_keys([
        KeyBinding::new("down", DownInEmojiSearch, search),
        KeyBinding::new("escape", CloseEmojiPicker, search),
        KeyBinding::new("tab", FocusNextControl, search),
        KeyBinding::new("shift-tab", FocusPreviousControl, search),
        KeyBinding::new("escape", CloseEmojiPicker, Some(KEY_CONTEXT)),
    ]);
}

/// The open server's own emoji, offered after the recent ones and titled with its name; the host
/// passes none in a conversation, where custom emoji are not offered (ADR-0028).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmojiPickerServer {
    pub name: SharedString,
    pub emojis: Vec<CustomEmoji>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EmojiPickerEvent {
    Picked(PickerEmoji),
    /// Esc (focus belongs back where the picker came from) or a click outside (focus stays
    /// where the click put it).
    Dismissed {
        should_return_focus: bool,
    },
}

/// One line of the virtual list: a section's heading, or a row of its cells (indices over all
/// sections).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickerLine {
    Heading(usize),
    Cells {
        section: usize,
        start: usize,
        end: usize,
    },
}

impl PickerLine {
    fn section(self) -> usize {
        match self {
            Self::Heading(section) | Self::Cells { section, .. } => section,
        }
    }
}

/// The sections laid out in lines of `columns` cells, with each section's first cell and line.
fn lay_out(
    sections: &[PickerSection],
    columns: usize,
) -> (Vec<PickerLine>, Vec<usize>, Vec<usize>) {
    let mut lines = Vec::new();
    let mut section_starts = Vec::with_capacity(sections.len());
    let mut heading_lines = Vec::with_capacity(sections.len());
    let mut start = 0;
    for (section, contents) in sections.iter().enumerate() {
        section_starts.push(start);
        heading_lines.push(lines.len());
        lines.push(PickerLine::Heading(section));
        for row in contents.emoji.chunks(columns.max(1)) {
            lines.push(PickerLine::Cells {
                section,
                start,
                end: start + row.len(),
            });
            start += row.len();
        }
    }
    (lines, section_starts, heading_lines)
}

pub struct EmojiPicker {
    search: Entity<InputState>,
    recent: Vec<String>,
    server: Option<EmojiPickerServer>,
    sections: Vec<PickerSection>,
    lines: Vec<PickerLine>,
    section_starts: Vec<usize>,
    heading_lines: Vec<usize>,
    list: ListState,
    /// The emoji's one Tab stop.
    active_index: usize,
    /// The category the list shows, lit in the bar.
    current_section: usize,
    /// The category keyboard focus is on in the bar.
    highlighted_category: usize,
    hovered_index: Option<usize>,
    focus_handle: FocusHandle,
    bar_focus: FocusHandle,
    grid_focus: FocusHandle,
    position: gpui::Point<Pixels>,
    /// Drawn above menus, or above modal dialogs for a picker inside one.
    draw_priority: usize,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<EmojiPickerEvent> for EmojiPicker {}

impl Focusable for EmojiPicker {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EmojiPicker {
    /// Opens beside `anchor` (the button that opened it: right edges lined up, below when the
    /// window has room, else above) with the search field focused. `recent` are the account's
    /// recently used emoji, latest first.
    pub fn open(
        anchor: Bounds<Pixels>,
        recent: Vec<String>,
        server: Option<EmojiPickerServer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let catalogue = strings(cx);
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(catalogue.emoji_picker().placeholder())
        });
        let subscriptions = vec![cx.subscribe_in(
            &search,
            window,
            |picker, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => picker.follow_query(cx),
                InputEvent::PressEnter { .. } => picker.pick(0, cx),
                _ => {}
            },
        )];
        search.update(cx, |search, cx| search.focus(window, cx));

        let viewport = window.viewport_size();
        let rem_size = window.rem_size();
        let in_window = |pixels: f32| f32::from(length(pixels).to_pixels(rem_size));
        let (top, left) = popover_position_beside_anchor(
            MenuAnchor {
                top: f32::from(anchor.top()),
                bottom: f32::from(anchor.bottom()),
                left: f32::from(anchor.left()),
                right: f32::from(anchor.right()),
            },
            (
                in_window(COMPONENT_EMOJI_PICKER_WIDTH),
                in_window(COMPONENT_EMOJI_PICKER_HEIGHT),
            ),
            (f32::from(viewport.width), f32::from(viewport.height)),
            in_window(COMPONENT_EMOJI_PICKER_GAP),
        );

        let list = ListState::new(0, ListAlignment::Top, px(LIST_OVERDRAW));
        let picker = cx.entity().downgrade();
        list.set_scroll_handler(move |event, window, cx| {
            let was_end_shown = event.visible_range.end;
            let _ = picker.update(cx, |_, cx| {
                // The list is borrowed while it calls this, and the event's range predates the
                // move: read the new scroll top once it is done.
                cx.defer_in(window, move |picker, window, cx| {
                    picker.follow_scroll(was_end_shown, window, cx);
                });
            });
        });
        let mut picker = Self {
            search,
            recent,
            server,
            sections: Vec::new(),
            lines: Vec::new(),
            section_starts: Vec::new(),
            heading_lines: Vec::new(),
            list,
            active_index: 0,
            current_section: 0,
            highlighted_category: 0,
            hovered_index: None,
            focus_handle: cx.focus_handle(),
            bar_focus: cx.focus_handle().tab_stop(true),
            grid_focus: cx.focus_handle().tab_stop(true),
            position: point(px(left), px(top)),
            draw_priority: POPOVER_DRAW_PRIORITY,
            _subscriptions: subscriptions,
        };
        picker.rebuild_sections("");
        picker
    }

    /// A picker opened from inside a modal dialog draws above the dialog's scrim.
    #[must_use]
    pub fn over_dialogs(mut self) -> Self {
        self.draw_priority = OVER_DIALOGS_DRAW_PRIORITY;
        self
    }

    fn rebuild_sections(&mut self, query: &str) {
        let server_emojis = self.server.as_ref().map(|server| server.emojis.as_slice());
        self.sections = picker_sections(query, &self.recent, server_emojis);
        let (lines, section_starts, heading_lines) = lay_out(&self.sections, EMOJI_PICKER_COLUMNS);
        self.list.reset(lines.len());
        self.lines = lines;
        self.section_starts = section_starts;
        self.heading_lines = heading_lines;
        self.active_index = 0;
        self.current_section = 0;
        self.highlighted_category = 0;
        self.hovered_index = None;
    }

    fn follow_query(&mut self, cx: &mut Context<Self>) {
        let query = self.search.read(cx).value().to_string();
        self.rebuild_sections(&query);
        cx.notify();
    }

    fn is_browsing(&self, cx: &App) -> bool {
        self.search.read(cx).value().trim().is_empty()
    }

    fn emoji_count(&self) -> usize {
        self.sections
            .iter()
            .map(|section| section.emoji.len())
            .sum()
    }

    fn emoji_at(&self, index: usize) -> Option<&PickerEmoji> {
        let section = self
            .section_starts
            .iter()
            .rposition(|&start| start <= index)?;
        self.sections[section]
            .emoji
            .get(index - self.section_starts[section])
    }

    fn pick(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(emoji) = self.emoji_at(index).cloned() {
            cx.emit(EmojiPickerEvent::Picked(emoji));
        }
    }

    /// The bar follows the list: the section of the first line shown, or the last one once the
    /// list is scrolled to its end (a short last section never reaches the top).
    fn follow_scroll(
        &mut self,
        visible_end_before: usize,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let is_at_end =
            self.list.is_scrolled_to_end() == Some(true) || visible_end_before >= self.lines.len();
        let line = if is_at_end {
            self.lines.len().checked_sub(1)
        } else {
            Some(self.list.logical_scroll_top().item_ix)
        };
        if let Some(section) = line
            .and_then(|line| self.lines.get(line))
            .map(|line| line.section())
        {
            self.show_current_section(section, window, cx);
        }
    }

    fn show_current_section(&mut self, section: usize, window: &Window, cx: &mut Context<Self>) {
        if section != self.current_section {
            self.current_section = section;
            // The bar's keyboard place follows too, unless the reader is moving through it.
            if !self.bar_focus.is_focused(window) {
                self.highlighted_category = section;
            }
            cx.notify();
        }
    }

    /// A category from the bar: the list scrolls to it, and the emoji's Tab stop moves to its
    /// first.
    fn show_section(&mut self, section: usize, cx: &mut Context<Self>) {
        let Some(&line) = self.heading_lines.get(section) else {
            return;
        };
        self.list.scroll_to(ListOffset {
            item_ix: line,
            offset_in_item: px(0.0),
        });
        self.current_section = section;
        self.highlighted_category = section;
        self.active_index = self.section_starts[section];
        cx.notify();
    }

    fn line_of_cell(&self, index: usize) -> Option<usize> {
        let section = self
            .section_starts
            .iter()
            .rposition(|&start| start <= index)?;
        let row = (index - self.section_starts[section]) / EMOJI_PICKER_COLUMNS;
        Some(self.heading_lines[section] + 1 + row)
    }

    fn focus_cell(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active_index = index;
        if let Some(line) = self.line_of_cell(index) {
            // Its heading too when it opens a section, so the reader sees where they are.
            let is_first_row =
                index - self.section_starts[self.lines[line].section()] < EMOJI_PICKER_COLUMNS;
            self.list
                .scroll_to_reveal_item(if is_first_row { line - 1 } else { line });
            self.list.scroll_to_reveal_item(line);
            // Revealing does not call the scroll handler: the bar follows the active cell.
            self.show_current_section(self.lines[line].section(), window, cx);
        }
        window.focus(&self.grid_focus, cx);
        cx.notify();
    }

    fn down_in_search(
        &mut self,
        _: &DownInEmojiSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.emoji_count() > 0 {
            self.focus_cell(0, window, cx);
        }
    }

    fn on_grid_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if modifiers.alt || modifiers.shift {
            return;
        }
        let is_control_held = if cfg!(target_os = "macos") {
            modifiers.platform
        } else {
            modifiers.control
        };
        if matches!(keystroke.key.as_str(), "enter" | "space") && !modifiers.modified() {
            cx.stop_propagation();
            self.pick(self.active_index, cx);
            return;
        }
        let section_lengths: Vec<usize> = self
            .sections
            .iter()
            .map(|section| section.emoji.len())
            .collect();
        let Some(next) = next_emoji_cell(
            &section_lengths,
            EMOJI_PICKER_COLUMNS,
            self.active_index,
            EmojiGridKeyPress {
                key: &keystroke.key,
                is_control_held,
            },
        ) else {
            return;
        };
        cx.stop_propagation();
        match next {
            EmojiGridMove::Search => {
                self.search
                    .update(cx, |search, cx| search.focus(window, cx));
            }
            EmojiGridMove::Cell(index) => {
                self.focus_cell(index, window, cx);
                // A new category: its heading goes to the top, as a click in the bar does.
                if matches!(keystroke.key.as_str(), "pageup" | "pagedown")
                    && let Some(section) =
                        self.section_starts.iter().position(|&start| start == index)
                {
                    self.show_section(section, cx);
                }
            }
        }
    }

    /// Arrows between categories (wrapping), Home and End; Enter or Space shows one.
    fn on_bar_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.modified() {
            return;
        }
        let last = self.sections.len().saturating_sub(1);
        let current = self.highlighted_category.min(last);
        let next = match keystroke.key.as_str() {
            "right" => Some(if current == last { 0 } else { current + 1 }),
            "left" => Some(if current == 0 { last } else { current - 1 }),
            "home" => Some(0),
            "end" => Some(last),
            "enter" | "space" => {
                cx.stop_propagation();
                self.show_section(current, cx);
                return;
            }
            _ => None,
        };
        if let Some(next) = next {
            cx.stop_propagation();
            self.highlighted_category = next;
            cx.notify();
        }
    }

    fn section_title(&self, section: &PickerSection, catalogue: Strings) -> SharedString {
        let words = catalogue.emoji_picker();
        let categories = words.categories();
        match section.kind {
            PickerSectionKind::Results => words.results().into(),
            PickerSectionKind::Recent => words.recent().into(),
            PickerSectionKind::Server => self
                .server
                .as_ref()
                .map(|server| server.name.clone())
                .unwrap_or_default(),
            PickerSectionKind::Category(category) => match category {
                EmojiCategory::People => categories.people(),
                EmojiCategory::Nature => categories.nature(),
                EmojiCategory::Food => categories.food(),
                EmojiCategory::Activities => categories.activities(),
                EmojiCategory::Travel => categories.travel(),
                EmojiCategory::Objects => categories.objects(),
                EmojiCategory::Symbols => categories.symbols(),
                EmojiCategory::Flags => categories.flags(),
            }
            .into(),
        }
    }

    fn render_line(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let tokens = theme(cx);
        match self.lines.get(index).copied() {
            Some(PickerLine::Heading(section)) => {
                let title = self.section_title(&self.sections[section], strings(cx));
                with_type(div(), TYPE_CATEGORY)
                    .id(ElementId::named_usize("emoji-heading", section))
                    .role(Role::Heading)
                    .aria_level(3)
                    .w_full()
                    .px(length(SPACE_2XS))
                    .py(length(SPACE_2XS))
                    .text_color(color(tokens.component_emoji_picker_heading_text))
                    .truncate()
                    .child(title)
                    .into_any_element()
            }
            Some(PickerLine::Cells { start, end, .. }) => div()
                .w_full()
                .flex()
                .children((start..end).map(|cell| self.render_cell(cell, window, cx)))
                .into_any_element(),
            None => div().into_any_element(),
        }
    }

    fn render_cell(&self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let tokens = theme(cx);
        let Some(emoji) = self.emoji_at(index) else {
            return div().into_any_element();
        };
        let is_grid_focused = self.grid_focus.is_focused(window);
        let is_active = index == self.active_index;
        let is_lit = self.hovered_index == Some(index) || (is_active && is_grid_focused);
        let (label, picture): (SharedString, AnyElement) = match emoji {
            PickerEmoji::Unicode(emoji) => (
                emoji.clone().into(),
                div()
                    .text_size(length(COMPONENT_EMOJI_PICKER_EMOJI_SIZE))
                    .line_height(length(COMPONENT_EMOJI_PICKER_EMOJI_SIZE))
                    .child(emoji.clone())
                    .into_any_element(),
            ),
            PickerEmoji::Custom(custom) => (
                format!(":{}:", custom.name).into(),
                custom_emoji_picture(custom.id, COMPONENT_EMOJI_PICKER_EMOJI_SIZE, window, cx),
            ),
        };
        let tooltip = matches!(emoji, PickerEmoji::Custom(_)).then(|| label.clone());
        div()
            .id(ElementId::named_usize("emoji-cell", index))
            .role(Role::Button)
            .aria_label(label)
            .when(is_active && is_grid_focused, |cell| {
                cell.aria_active_descendant()
            })
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .w(relative(CELL_WIDTH_SHARE))
            .h(length(COMPONENT_EMOJI_PICKER_CELL_SIZE))
            .rounded(length(COMPONENT_EMOJI_PICKER_CELL_RADIUS))
            .when(is_lit, |cell| {
                cell.bg(color(tokens.component_emoji_picker_cell_background_hover))
            })
            .cursor_pointer()
            .when_some(tooltip, |cell, tooltip| {
                cell.tooltip(move |_, cx| Tooltip::view(tooltip.clone(), cx))
                    .tooltip_show_delay(Tooltip::show_delay())
            })
            .on_hover(cx.listener(move |picker, is_hovered: &bool, _, cx| {
                picker.hovered_index = if *is_hovered {
                    Some(index)
                } else {
                    picker.hovered_index.filter(|hovered| *hovered != index)
                };
                cx.notify();
            }))
            .on_click(cx.listener(move |picker, _, _, cx| picker.pick(index, cx)))
            .child(picture)
            .when(
                is_active && is_grid_focused && window.last_input_was_keyboard(),
                |cell| cell.child(inset_focus_ring(COMPONENT_EMOJI_PICKER_CELL_RADIUS, cx)),
            )
            .into_any_element()
    }

    fn render_category_bar(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let is_bar_focused = self.bar_focus.is_focused(window);
        let is_ring_shown = is_bar_focused && window.last_input_was_keyboard();
        let buttons = self.sections.iter().enumerate().map(|(index, section)| {
            let title = self.section_title(section, catalogue);
            let is_current = index == self.current_section;
            let is_highlighted = is_bar_focused && index == self.highlighted_category;
            let tint = color(if is_current {
                tokens.component_emoji_picker_category_icon_current
            } else {
                tokens.component_emoji_picker_category_icon
            });
            // The server's own emoji: its first one stands for them, as its picture is all they
            // share.
            let symbol = match (section.kind, section.emoji.first()) {
                (PickerSectionKind::Server, Some(PickerEmoji::Custom(first))) => {
                    custom_emoji_picture(
                        first.id,
                        COMPONENT_EMOJI_PICKER_CATEGORY_ICON_SIZE,
                        window,
                        cx,
                    )
                }
                (kind, _) => icon(
                    category_icon(kind),
                    COMPONENT_EMOJI_PICKER_CATEGORY_ICON_SIZE,
                    tint,
                )
                .into_any_element(),
            };
            div()
                .id(ElementId::named_usize("emoji-category", index))
                .role(Role::Button)
                .aria_label(title.clone())
                .aria_selected(is_current)
                .when(
                    is_highlighted,
                    gpui::StatefulInteractiveElement::aria_active_descendant,
                )
                .relative()
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(length(COMPONENT_EMOJI_PICKER_CATEGORY_SIZE))
                .rounded(length(COMPONENT_EMOJI_PICKER_CATEGORY_RADIUS))
                .when(is_current, |button| {
                    button.bg(color(
                        tokens.component_emoji_picker_category_background_current,
                    ))
                })
                .cursor_pointer()
                .tooltip(move |_, cx| Tooltip::view(title.clone(), cx))
                .tooltip_show_delay(Tooltip::show_delay())
                .on_click(cx.listener(move |picker, _, _, cx| picker.show_section(index, cx)))
                .child(symbol)
                .when(is_highlighted && is_ring_shown, |button| {
                    button.child(inset_focus_ring(COMPONENT_EMOJI_PICKER_CATEGORY_RADIUS, cx))
                })
        });
        div()
            .id("emoji-categories")
            .role(Role::Toolbar)
            .aria_label(catalogue.emoji_picker().category_bar())
            .track_focus(&self.bar_focus)
            .on_key_down(cx.listener(Self::on_bar_key_down))
            .flex()
            .flex_none()
            .justify_between()
            .children(buttons)
            .into_any_element()
    }
}

/// `CATEGORY_ICONS`; results and the server's emoji (without a picture) take the magnifier.
fn category_icon(kind: PickerSectionKind) -> IconName {
    match kind {
        PickerSectionKind::Results | PickerSectionKind::Server => IconName::MagnifyingGlass,
        PickerSectionKind::Recent => IconName::Clock,
        PickerSectionKind::Category(category) => match category {
            EmojiCategory::People => IconName::Smiley,
            EmojiCategory::Nature => IconName::PawPrint,
            EmojiCategory::Food => IconName::BowlFood,
            EmojiCategory::Activities => IconName::SoccerBall,
            EmojiCategory::Travel => IconName::AirplaneTilt,
            EmojiCategory::Objects => IconName::Lightbulb,
            EmojiCategory::Symbols => IconName::Heart,
            EmojiCategory::Flags => IconName::Flag,
        },
    }
}

/// A server emoji's picture at `size`, still under reduced motion (`CustomEmojiImage.svelte`).
pub(crate) fn custom_emoji_picture(
    id: Snowflake,
    size: f32,
    window: &Window,
    cx: &App,
) -> AnyElement {
    img(picture_for_box(
        DownloadableFile::Icon(id),
        size,
        size,
        picture_animation(cx),
        window,
    ))
    .flex_none()
    .size(length(size))
    .object_fit(ObjectFit::Contain)
    .into_any_element()
}

impl Render for EmojiPicker {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = theme(cx);
        let catalogue = strings(cx);
        let words = catalogue.emoji_picker();
        let query = self.search.read(cx).value().trim().to_owned();
        let has_emoji = self.emoji_count() > 0;
        let is_search_focused = self.search.read(cx).focus_handle(cx).is_focused(window);
        let search_row = div()
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .gap(length(SPACE_XS))
            .h(length(COMPONENT_EMOJI_PICKER_SEARCH_HEIGHT))
            .px(length(SPACE_XS))
            .rounded(length(RADIUS_SM))
            .bg(color(tokens.component_emoji_picker_search_background))
            .child(icon(
                IconName::MagnifyingGlass,
                COMPONENT_CONTEXT_MENU_ITEM_ICON_SIZE,
                color(tokens.component_emoji_picker_search_placeholder),
            ))
            .child(
                with_type(div(), TYPE_BODY)
                    .flex_1()
                    .min_w_0()
                    .text_color(color(tokens.component_emoji_picker_search_text))
                    .child(Input::new(&self.search).appearance(false).aria_label(
                        SharedString::from(format!("{}, {}", words.search(), words.help())),
                    )),
            )
            .when(
                is_search_focused && window.last_input_was_keyboard(),
                |row| row.child(inset_focus_ring(RADIUS_SM, cx)),
            );
        let bar = (self.is_browsing(cx) && has_emoji).then(|| self.render_category_bar(window, cx));
        let picker = cx.entity();
        let body = if has_emoji {
            div()
                .id("emoji-grid")
                .role(Role::Group)
                .aria_label(words.label())
                .track_focus(&self.grid_focus)
                .on_key_down(cx.listener(Self::on_grid_key_down))
                .flex_1()
                .min_h_0()
                .child(
                    gpui::list(self.list.clone(), move |index, window, cx| {
                        picker.update(cx, |picker, cx| picker.render_line(index, window, cx))
                    })
                    .size_full(),
                )
                .into_any_element()
        } else {
            with_type(div(), TYPE_BODY)
                .id("emoji-no-results")
                .live_polite(words.no_results(&query))
                .p(length(SPACE_XS))
                .text_color(color(tokens.component_emoji_picker_empty_text))
                .child(words.no_results(&query))
                .into_any_element()
        };

        let card = div()
            .id("emoji-picker")
            .role(Role::Dialog)
            .aria_label(words.label())
            .track_focus(&self.focus_handle)
            .key_context(KEY_CONTEXT)
            .on_action(cx.listener(Self::down_in_search))
            .on_action(cx.listener(|_, _: &CloseEmojiPicker, _, cx| {
                cx.emit(EmojiPickerEvent::Dismissed {
                    should_return_focus: true,
                });
            }))
            // Tab stays inside, as in the web's popover, until Esc or a click outside.
            .on_action(cx.listener(|picker, _: &FocusNextControl, window, cx| {
                focus_next_inside(&picker.focus_handle, window, cx);
            }))
            .on_action(cx.listener(|picker, _: &FocusPreviousControl, window, cx| {
                focus_previous_inside(&picker.focus_handle, window, cx);
            }))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| {
                cx.emit(EmojiPickerEvent::Dismissed {
                    should_return_focus: false,
                });
            }))
            // A right-click inside must not reach what lies under it.
            .on_mouse_down(gpui::MouseButton::Right, |_, _, cx| cx.stop_propagation())
            // Clicks inside never reach what lies under it (a dialog's scrim closes the dialog).
            .occlude()
            .flex()
            .flex_col()
            .gap(length(COMPONENT_EMOJI_PICKER_GAP))
            .w(length(COMPONENT_EMOJI_PICKER_WIDTH))
            .h(length(COMPONENT_EMOJI_PICKER_HEIGHT))
            .p(length(COMPONENT_EMOJI_PICKER_PADDING))
            .rounded(length(COMPONENT_EMOJI_PICKER_RADIUS))
            .border(length(COMPONENT_EMOJI_PICKER_BORDER_WIDTH))
            .border_color(color(tokens.component_emoji_picker_border))
            .bg(color(tokens.component_emoji_picker_background))
            .shadow(shadow(tokens.component_emoji_picker_shadow))
            .child(search_row)
            .children(bar)
            .child(body);
        // Drops in, as the web's `mw-drop-in`; nothing moves under reduced motion.
        let card = if cx.reduce_motion() {
            card.into_any_element()
        } else {
            let drop_distance = SPACE_2XS;
            card.with_animation(
                "emoji-picker-enter",
                Animation::new(Duration::from_millis(MOTION_DURATION_FAST.0))
                    .with_easing(|progress| easing(MOTION_EASING_ENTER).sample(progress)),
                move |card, progress| {
                    card.mt(-length(drop_distance * (1.0 - progress)))
                        .opacity(progress)
                },
            )
            .into_any_element()
        };
        deferred(
            anchored()
                .position(self.position)
                .snap_to_window_with_margin(px(COMPONENT_CONTEXT_MENU_VIEWPORT_MARGIN))
                .child(card),
        )
        .with_priority(self.draw_priority)
    }
}

impl std::fmt::Debug for EmojiPicker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("EmojiPicker")
            .field("sections", &self.sections.len())
            .field("active_index", &self.active_index)
            .finish_non_exhaustive()
    }
}

// ---- Recently used emoji (recent-emoji.svelte.ts) ----

/// The emoji `user_id` picked lately, latest first; none until the preferences are read.
#[must_use]
pub fn recent_emoji(user_id: Option<Snowflake>, cx: &App) -> Vec<String> {
    user_id
        .zip(preferences(cx))
        .map(|(user_id, preferences)| preferences.recent_emoji_of(user_id).to_vec())
        .unwrap_or_default()
}

/// Called for every built-in emoji the user adds, from the picker, a quick reaction or the
/// composer's suggestions; server emoji are not remembered, as on the web.
pub fn note_emoji_used(user_id: Option<Snowflake>, emoji: &str, cx: &mut App) {
    if let Some(user_id) = user_id {
        let emoji = emoji.to_owned();
        change_preferences(cx, move |preferences| {
            preferences.remember_emoji(user_id, &emoji);
        });
    }
}

#[cfg(test)]
mod tests;
