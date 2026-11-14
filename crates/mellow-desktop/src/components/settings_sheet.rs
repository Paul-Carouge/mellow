//! `SettingsSheet.svelte` and `SettingsNavItem.svelte` (DESIGN.md §5.8, §6.43): the server, channel
//! and user settings over the whole window. Wide, the section navigation sits on the ground and the
//! page on a sheet beside it, with the close button and its "Esc" label; narrow, one shows after
//! the other: choosing a section opens its page with a back button.
//!
//! The host decides what may close or leave a page (unsaved changes keep the reader in it): Esc,
//! the close button, Back and the nav items only ask, through the handlers. Tab stays inside;
//! Up and Down (Home, End) move between the nav items, Enter and Space choose one.

use std::sync::Arc;

use gpui::{
    AnyElement, App, ElementId, FocusHandle, InteractiveElement as _, IntoElement, KeyDownEvent,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _,
};

use super::{
    ActivationHandler, FocusNextControl, FocusPreviousControl, IconButton, child_id,
    colour_transition, focus_next_inside, focus_previous_inside, inset_focus_ring,
    is_focus_visible, next_menu_item_index, remembered_hover, with_type,
};
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_SETTINGS_CLOSE_BORDER_WIDTH, COMPONENT_SETTINGS_CLOSE_BUTTON_SIZE,
    COMPONENT_SETTINGS_CLOSE_GAP, COMPONENT_SETTINGS_CLOSE_ICON_SIZE,
    COMPONENT_SETTINGS_CLOSE_LABEL_GAP, COMPONENT_SETTINGS_CONTENT_MAX_WIDTH,
    COMPONENT_SETTINGS_CONTENT_PADDING_BOTTOM, COMPONENT_SETTINGS_CONTENT_PADDING_INLINE,
    COMPONENT_SETTINGS_CONTENT_PADDING_INLINE_NARROW, COMPONENT_SETTINGS_CONTENT_PADDING_TOP,
    COMPONENT_SETTINGS_NAV_ITEM_GAP, COMPONENT_SETTINGS_NAV_ITEM_HEIGHT,
    COMPONENT_SETTINGS_NAV_ITEM_PADDING_INLINE, COMPONENT_SETTINGS_NAV_ITEM_RADIUS,
    COMPONENT_SETTINGS_NAV_PADDING_LEADING, COMPONENT_SETTINGS_NAV_PADDING_TOP,
    COMPONENT_SETTINGS_NAV_PADDING_TRAILING, COMPONENT_SETTINGS_NAV_SECTION_PADDING_BLOCK,
    COMPONENT_SETTINGS_NAV_SECTION_PADDING_INLINE, COMPONENT_SETTINGS_NAV_SEPARATOR_MARGIN_BLOCK,
    COMPONENT_SETTINGS_NAV_SEPARATOR_MARGIN_INLINE, COMPONENT_SETTINGS_NAV_WIDTH,
    COMPONENT_SETTINGS_PAGE_TITLE_GAP, COMPONENT_SHEET_RADIUS, SIZE_HIT_WEB_MIN, SIZE_SHEET_INSET,
    SPACE_LG, SPACE_XS, TYPE_CATEGORY, TYPE_CHANNEL_NAME, TYPE_HEADING_LG, TYPE_HEADING_MD,
};
use crate::theme::{color, length, theme};
use crate::window_chrome::TITLE_BAR_BAND_HEIGHT;

/// What a nav item reports when chosen: its index among the items.
type ChooseHandler = Arc<dyn Fn(usize, &mut Window, &mut App) + 'static>;

/// A line of the section navigation.
#[derive(Debug)]
pub enum SettingsNavEntry {
    /// A group's title: the server's name, "Moderation".
    Heading(SharedString),
    Item(SettingsNavItem),
    Separator,
}

/// A section, or the danger item ("Delete server", filled red under the pointer).
#[derive(Debug)]
pub struct SettingsNavItem {
    pub label: SharedString,
    /// The host's, so it can focus the item (when the sheet opens, after Back).
    pub focus_handle: FocusHandle,
    pub is_selected: bool,
    pub is_danger: bool,
}

/// The sheet's words, all from the host's interface strings.
#[derive(Debug, Clone)]
pub struct SettingsSheetLabels {
    /// The sheet's accessible name: "Server settings, Rust Lyon".
    pub label: SharedString,
    /// The navigation's title, shown above it when narrow.
    pub title: SharedString,
    pub back: SharedString,
    pub close: SharedString,
    pub escape_key: SharedString,
}

#[derive(IntoElement)]
pub struct SettingsSheet {
    id: ElementId,
    labels: SettingsSheetLabels,
    is_wide: bool,
    /// Narrow: the page shows instead of the navigation.
    is_page_shown_when_narrow: bool,
    entries: Vec<SettingsNavEntry>,
    /// The shown section's title, or `None` while there is none.
    page_title: Option<SharedString>,
    page: Option<AnyElement>,
    /// Holds focus inside the sheet; Tab and Shift+Tab go round it.
    container_focus: FocusHandle,
    /// The page's section, focused by the host when it replaces the navigation.
    page_focus: FocusHandle,
    on_choose: ChooseHandler,
    on_request_close: ActivationHandler,
    /// The close button alone, when it must not ask as Esc does; `on_request_close` otherwise.
    on_close_button: Option<ActivationHandler>,
    on_back: ActivationHandler,
}

impl SettingsSheet {
    #[must_use]
    pub fn new(
        id: impl Into<ElementId>,
        labels: SettingsSheetLabels,
        container_focus: FocusHandle,
        page_focus: FocusHandle,
    ) -> Self {
        Self {
            id: id.into(),
            labels,
            is_wide: true,
            is_page_shown_when_narrow: false,
            entries: Vec::new(),
            page_title: None,
            page: None,
            container_focus,
            page_focus,
            on_choose: Arc::new(|_, _, _| {}),
            on_request_close: Arc::new(|_, _| {}),
            on_close_button: None,
            on_back: Arc::new(|_, _| {}),
        }
    }

    /// Wide from the navigator breakpoint, as the web's `isWide`.
    #[must_use]
    pub fn wide(mut self, is_wide: bool, is_page_shown_when_narrow: bool) -> Self {
        self.is_wide = is_wide;
        self.is_page_shown_when_narrow = is_page_shown_when_narrow;
        self
    }

    #[must_use]
    pub fn entries(mut self, entries: Vec<SettingsNavEntry>) -> Self {
        self.entries = entries;
        self
    }

    #[must_use]
    pub fn page(mut self, title: impl Into<SharedString>, page: AnyElement) -> Self {
        self.page_title = Some(title.into());
        self.page = Some(page);
        self
    }

    /// Called with the chosen item's index among the items (headings and separators left out).
    #[must_use]
    pub fn on_choose(mut self, handler: impl Fn(usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_choose = Arc::new(handler);
        self
    }

    /// Esc and the close button.
    #[must_use]
    pub fn on_request_close(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_request_close = Arc::new(handler);
        self
    }

    /// The close button only, when it closes outright while Esc first closes what is open inside
    /// (the user settings' forms, as the web's close button calls `onclose`).
    #[must_use]
    pub fn on_close_button(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_close_button = Some(Arc::new(handler));
        self
    }

    /// The back button of a page shown alone when narrow.
    #[must_use]
    pub fn on_back(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_back = Arc::new(handler);
        self
    }
}

fn close_button(
    id: &ElementId,
    labels: &SettingsSheetLabels,
    is_wide: bool,
    on_request_close: ActivationHandler,
    cx: &App,
) -> impl IntoElement {
    let tokens = theme(cx);
    let ring = div()
        .absolute()
        .size(length(COMPONENT_SETTINGS_CLOSE_BUTTON_SIZE))
        .top(length(
            (SIZE_HIT_WEB_MIN - COMPONENT_SETTINGS_CLOSE_BUTTON_SIZE) / 2.0,
        ))
        .left(length(
            (SIZE_HIT_WEB_MIN - COMPONENT_SETTINGS_CLOSE_BUTTON_SIZE) / 2.0,
        ))
        .rounded_full()
        .border(length(COMPONENT_SETTINGS_CLOSE_BORDER_WIDTH))
        .border_color(color(tokens.component_settings_close_border));
    div()
        .flex()
        .flex_none()
        .flex_col()
        .items_center()
        .gap(length(COMPONENT_SETTINGS_CLOSE_LABEL_GAP))
        .child(
            div()
                .debug_selector(|| "settings-close-button".to_owned())
                .relative()
                .size(length(SIZE_HIT_WEB_MIN))
                .child(ring)
                .child(
                    IconButton::new(child_id(id, "close"), IconName::X, labels.close.clone())
                        .sizes(SIZE_HIT_WEB_MIN, COMPONENT_SETTINGS_CLOSE_ICON_SIZE)
                        .key_shortcuts("Escape")
                        .on_click(move |window, cx| on_request_close(window, cx)),
                ),
        )
        .when(is_wide, |column| {
            column.child(
                with_type(div(), TYPE_CATEGORY)
                    .text_color(color(tokens.component_settings_close_label_text))
                    .child(labels.escape_key.clone()),
            )
        })
}

fn nav_item(
    item_id: ElementId,
    item: SettingsNavItem,
    index: usize,
    on_choose: ChooseHandler,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let tokens = theme(cx);
    let hover = remembered_hover(&item_id, window, cx);
    let is_hovered = *hover.read(cx);
    let (text, background) = if item.is_danger {
        if is_hovered {
            (
                tokens.component_settings_nav_danger_text_hover,
                Some(tokens.component_settings_nav_danger_background_hover),
            )
        } else {
            (tokens.component_settings_nav_danger_text, None)
        }
    } else if item.is_selected {
        (
            tokens.component_settings_nav_item_text_selected,
            Some(tokens.component_settings_nav_item_background_selected),
        )
    } else if is_hovered {
        (
            tokens.component_settings_nav_item_text_hover,
            Some(tokens.component_settings_nav_item_background_hover),
        )
    } else {
        (tokens.component_settings_nav_item_text, None)
    };
    let text = colour_transition(child_id(&item_id, "text"), color(text), window, cx);
    let is_ring_shown = is_focus_visible(&item.focus_handle, window);
    with_type(div(), TYPE_CHANNEL_NAME)
        .id(item_id)
        .role(Role::Button)
        .aria_label(item.label.clone())
        .when(item.is_selected, |row| row.aria_selected(true))
        .track_focus(&item.focus_handle.tab_stop(true))
        .relative()
        .flex()
        .items_center()
        .w_full()
        // The pointer size (DESIGN.md §7.4); the web grows it to 44 px only under a coarse pointer.
        .h(length(COMPONENT_SETTINGS_NAV_ITEM_HEIGHT))
        .px(length(COMPONENT_SETTINGS_NAV_ITEM_PADDING_INLINE))
        .rounded(length(COMPONENT_SETTINGS_NAV_ITEM_RADIUS))
        .when_some(background, |row, background| row.bg(color(background)))
        .text_color(text)
        .cursor_pointer()
        .on_hover(move |is_hovered, _, cx| {
            hover.update(cx, |remembered, cx| {
                *remembered = *is_hovered;
                cx.notify();
            });
        })
        .on_click(move |_, window, cx| on_choose(index, window, cx))
        .child(div().min_w_0().truncate().child(item.label))
        .when(is_ring_shown, |row| {
            row.child(inset_focus_ring(COMPONENT_SETTINGS_NAV_ITEM_RADIUS, cx))
        })
}

impl RenderOnce for SettingsSheet {
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let is_wide = self.is_wide;
        let is_navigation_shown = is_wide || !self.is_page_shown_when_narrow;
        let is_page_shown = is_wide || self.is_page_shown_when_narrow;
        let on_close_button = self
            .on_close_button
            .clone()
            .unwrap_or_else(|| self.on_request_close.clone());
        let narrow_inline = length(COMPONENT_SETTINGS_CONTENT_PADDING_INLINE_NARROW);

        let item_focus: Vec<FocusHandle> = self
            .entries
            .iter()
            .filter_map(|entry| match entry {
                SettingsNavEntry::Item(item) => Some(item.focus_handle.clone()),
                _ => None,
            })
            .collect();
        let mut item_index = 0;
        let mut rows: Vec<AnyElement> = Vec::with_capacity(self.entries.len());
        for entry in self.entries {
            match entry {
                SettingsNavEntry::Heading(heading) => rows.push(
                    with_type(div(), TYPE_CATEGORY)
                        .id(ElementId::NamedChild(
                            Arc::new(self.id.clone()),
                            SharedString::from(format!("heading-{}", rows.len())),
                        ))
                        .role(Role::Heading)
                        .truncate()
                        .px(length(COMPONENT_SETTINGS_NAV_SECTION_PADDING_INLINE))
                        .py(length(COMPONENT_SETTINGS_NAV_SECTION_PADDING_BLOCK))
                        .text_color(color(tokens.component_settings_nav_section_text))
                        .child(heading)
                        .into_any_element(),
                ),
                SettingsNavEntry::Separator => rows.push(
                    div()
                        .mx(length(COMPONENT_SETTINGS_NAV_SEPARATOR_MARGIN_INLINE))
                        .my(length(COMPONENT_SETTINGS_NAV_SEPARATOR_MARGIN_BLOCK))
                        .h(gpui::px(1.0))
                        .bg(color(tokens.component_settings_nav_separator))
                        .into_any_element(),
                ),
                SettingsNavEntry::Item(item) => {
                    let item_id = ElementId::NamedChild(
                        Arc::new(self.id.clone()),
                        SharedString::from(format!("nav-{item_index}")),
                    );
                    rows.push(
                        nav_item(
                            item_id,
                            item,
                            item_index,
                            self.on_choose.clone(),
                            window,
                            cx,
                        )
                        .into_any_element(),
                    );
                    item_index += 1;
                }
            }
        }

        let navigation = is_navigation_shown.then(|| {
            let close = on_close_button.clone();
            div()
                .id(child_id(&self.id, "navigation"))
                .flex()
                .overflow_y_scroll()
                .bg(color(tokens.component_settings_sidebar_background))
                .when(is_wide, |column| {
                    column.flex_1().flex_basis(gpui::px(0.0)).justify_end()
                })
                .when(!is_wide, |column| column.flex_1().flex_col())
                .when(!is_wide, |column| {
                    column.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(length(SPACE_XS))
                            .px(narrow_inline)
                            .pt(length(SPACE_XS))
                            .child(
                                with_type(div(), TYPE_HEADING_MD)
                                    .id(child_id(&self.id, "title"))
                                    .role(Role::Heading)
                                    .min_w_0()
                                    .truncate()
                                    .text_color(color(tokens.component_settings_page_title_text))
                                    .child(self.labels.title.clone()),
                            )
                            .child(close_button(&self.id, &self.labels, false, close, cx)),
                    )
                })
                .child(
                    div()
                        .id(child_id(&self.id, "nav"))
                        .role(Role::Navigation)
                        .aria_label(self.labels.title.clone())
                        .flex()
                        .flex_col()
                        .gap(length(COMPONENT_SETTINGS_NAV_ITEM_GAP))
                        .when(is_wide, |nav| {
                            nav.w(length(COMPONENT_SETTINGS_NAV_WIDTH))
                                .pt(length(COMPONENT_SETTINGS_NAV_PADDING_TOP))
                                .pr(length(COMPONENT_SETTINGS_NAV_PADDING_TRAILING))
                                .pl(length(COMPONENT_SETTINGS_NAV_PADDING_LEADING))
                        })
                        .when(!is_wide, |nav| nav.px(narrow_inline).py(length(SPACE_XS)))
                        .on_key_down(move |event: &KeyDownEvent, window, cx| {
                            if event.keystroke.modifiers.modified() {
                                return;
                            }
                            let current = item_focus
                                .iter()
                                .position(|handle| handle.is_focused(window));
                            let Some(next) = next_menu_item_index(
                                &event.keystroke.key,
                                current,
                                item_focus.len(),
                            ) else {
                                return;
                            };
                            cx.stop_propagation();
                            if let Some(handle) = item_focus.get(next) {
                                window.focus(handle, cx);
                            }
                        })
                        .children(rows),
                )
        });

        let page = is_page_shown.then(|| {
            let back = self.on_back.clone();
            let close = on_close_button.clone();
            div()
                .id(child_id(&self.id, "page-scroll"))
                .relative()
                .flex()
                .flex_col()
                .min_w_0()
                .overflow_y_scroll()
                .bg(color(tokens.component_settings_content_background))
                .when(is_wide, |column| {
                    // The web's `flex-[2_1_0]` beside the navigation's `flex-[1_1_0]`.
                    let mut column = column.flex_basis(gpui::px(0.0));
                    column.style().flex_grow = Some(2.0);
                    column
                        .rounded(length(COMPONENT_SHEET_RADIUS))
                        .pl(length(COMPONENT_SETTINGS_CONTENT_PADDING_INLINE))
                        .pr(length(SPACE_LG))
                })
                .when(!is_wide, gpui::Styled::flex_1)
                .when(!is_wide, |column| {
                    column.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(length(SPACE_XS))
                            .px(narrow_inline)
                            .pt(length(SPACE_XS))
                            .child(
                                IconButton::new(
                                    child_id(&self.id, "back"),
                                    IconName::CaretLeft,
                                    self.labels.back.clone(),
                                )
                                .sizes(SIZE_HIT_WEB_MIN, COMPONENT_SETTINGS_CLOSE_ICON_SIZE)
                                .on_click(move |window, cx| back(window, cx)),
                            )
                            .child(close_button(
                                &self.id,
                                &self.labels,
                                false,
                                close.clone(),
                                cx,
                            )),
                    )
                })
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .gap(length(COMPONENT_SETTINGS_CLOSE_GAP))
                        .when(is_wide, |row| {
                            row.pt(length(COMPONENT_SETTINGS_CONTENT_PADDING_TOP))
                        })
                        .when(!is_wide, |row| row.px(narrow_inline).pt(length(SPACE_XS)))
                        .when_some(self.page_title, |row, title| {
                            row.child(
                                div()
                                    .id(child_id(&self.id, "page"))
                                    .role(Role::Group)
                                    .aria_label(title.clone())
                                    .track_focus(&self.page_focus)
                                    .flex()
                                    .flex_col()
                                    .flex_1()
                                    .min_w_0()
                                    .max_w(length(COMPONENT_SETTINGS_CONTENT_MAX_WIDTH))
                                    .child(
                                        with_type(div(), TYPE_HEADING_LG)
                                            .id(child_id(&self.id, "page-title"))
                                            .role(Role::Heading)
                                            .text_color(color(
                                                tokens.component_settings_page_title_text,
                                            ))
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .mt(length(COMPONENT_SETTINGS_PAGE_TITLE_GAP))
                                            .flex()
                                            .flex_col()
                                            .flex_1()
                                            .pb(length(COMPONENT_SETTINGS_CONTENT_PADDING_BOTTOM))
                                            .children(self.page),
                                    ),
                            )
                        })
                        .when(is_wide, |row| {
                            row.child(close_button(&self.id, &self.labels, true, close, cx))
                        }),
                )
        });

        let close_on_escape = self.on_request_close;
        let window_corners = crate::window_chrome::client_frame_inner_corner_radii(window);
        let container_focus = self.container_focus.clone();
        let previous_focus = self.container_focus.clone();
        div()
            .id(self.id)
            .role(Role::Dialog)
            .aria_label(self.labels.label)
            .track_focus(&self.container_focus)
            .tab_group()
            .key_context("SettingsSheet")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            // Its fill reaches the window's bottom corners: it follows a floating window's frame.
            .rounded_bl(length(window_corners.bottom_left))
            .rounded_br(length(window_corners.bottom_right))
            .bg(color(if is_wide {
                tokens.component_settings_sidebar_background
            } else {
                tokens.component_settings_content_background
            }))
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" && !event.keystroke.modifiers.modified() {
                    cx.stop_propagation();
                    close_on_escape(window, cx);
                }
            })
            .on_action(move |_: &FocusNextControl, window, cx| {
                focus_next_inside(&container_focus, window, cx);
            })
            .on_action(move |_: &FocusPreviousControl, window, cx| {
                focus_previous_inside(&previous_focus, window, cx);
            })
            .pt(length(if is_wide {
                SIZE_SHEET_INSET.max(TITLE_BAR_BAND_HEIGHT)
            } else {
                TITLE_BAR_BAND_HEIGHT
            }))
            // The web's `sheet-inset-padding`: the card 8 px from every window edge, the
            // navigation column included, so both columns split the same width as on the web.
            .when(is_wide, |sheet| {
                sheet
                    .pb(length(SIZE_SHEET_INSET))
                    .pl(length(SIZE_SHEET_INSET))
                    .pr(length(SIZE_SHEET_INSET))
                    .gap(length(SIZE_SHEET_INSET))
            })
            .children(navigation)
            .children(page)
    }
}

impl std::fmt::Debug for SettingsSheet {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SettingsSheet")
            .field("id", &self.id)
            .field("is_wide", &self.is_wide)
            .finish_non_exhaustive()
    }
}
