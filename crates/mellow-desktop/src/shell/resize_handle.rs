//! A focusable window splitter in the gap beside a column (`ColumnResizeHandle.svelte`,
//! DESIGN.md §4.5): drag it, or use the arrows, Home and End; a double-click puts the default
//! width back and Esc cancels a drag. It covers the gap and the leading edge of the card after it,
//! never the trailing edge before it, where the scrollbars are.

use gpui::{
    Context, CursorStyle, ElementId, InteractiveElement as _, IntoElement, KeyDownEvent,
    MouseButton, MouseDownEvent, ParentElement as _, Role, StatefulInteractiveElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _, relative,
};

use super::{ColumnDrag, DuskShell, ResizableColumn, logical_x};
use crate::column_widths::{ColumnBounds, clamp_column_width, width_after_key};
use crate::components::{child_id, colour_transition, remembered_focus_handle, remembered_hover};
use crate::i18n::strings;
use crate::theme::tokens::{
    COMPONENT_COLUMN_RESIZE_HIT_WIDTH, COMPONENT_COLUMN_RESIZE_KEYBOARD_STEP,
    COMPONENT_COLUMN_RESIZE_LINE_INSET, COMPONENT_COLUMN_RESIZE_LINE_WIDTH, SIZE_SHEET_INSET,
};
use crate::theme::{color, length, theme};

impl DuskShell {
    /// `width` is the column's width laid out now; `bounds.maximum` what the window allows now.
    #[allow(clippy::too_many_lines)] // One element tree, in the order it is drawn.
    pub(super) fn render_resize_handle(
        &mut self,
        column: ResizableColumn,
        width: f32,
        bounds: ColumnBounds,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tokens = theme(cx);
        let strings = strings(cx);
        let id = ElementId::from(match column {
            ResizableColumn::Channels => "resize-channels",
            ResizableColumn::Members => "resize-members",
            ResizableColumn::Search => "resize-search-results",
        });
        let label = match column {
            ResizableColumn::Channels => strings.shell().resize_channels(),
            ResizableColumn::Members => strings.shell().resize_members(),
            ResizableColumn::Search => strings.shell().resize_search_results(),
        };
        let focus_handle = remembered_focus_handle(&id, window, cx);
        let hover = remembered_hover(&id, window, cx);
        let is_dragging = self.column_drag.is_some_and(|drag| drag.column == column);
        let line_target = if is_dragging {
            Some(tokens.component_column_resize_line_active)
        } else if focus_handle.is_focused(window) && window.last_input_was_keyboard() {
            Some(tokens.component_column_resize_line_focus)
        } else if *hover.read(cx) {
            Some(tokens.component_column_resize_line_hover)
        } else {
            None
        };
        let resting = color(tokens.component_column_resize_line_hover);
        let line = colour_transition(
            child_id(&id, "line"),
            line_target.map_or(gpui::Hsla { a: 0.0, ..resting }, color),
            window,
            cx,
        );
        // Rounded down: the reported width is a whole number of pixels, as on the web.
        #[allow(clippy::cast_possible_truncation)]
        let whole_pixels = width.round() as i64;

        div()
            .id(id)
            .role(Role::Splitter)
            .aria_label(label)
            .aria_orientation(gpui::Orientation::Vertical)
            .aria_numeric_value(f64::from(width))
            .aria_min_numeric_value(f64::from(bounds.minimum))
            .aria_max_numeric_value(f64::from(bounds.maximum))
            .aria_value(strings.shell().column_width(whole_pixels))
            .track_focus(&focus_handle)
            .absolute()
            .top_0()
            .bottom_0()
            .map(|handle| match column {
                // From the start of the gap onwards: the gap, then the leading edge of the next card.
                ResizableColumn::Channels => handle.left(relative(1.0)),
                ResizableColumn::Members | ResizableColumn::Search => {
                    handle.left(-length(SIZE_SHEET_INSET))
                }
            })
            .w(length(COMPONENT_COLUMN_RESIZE_HIT_WIDTH))
            .flex()
            .cursor(CursorStyle::ResizeLeftRight)
            .on_hover(move |is_hovered, _, cx| {
                hover.update(cx, |remembered, cx| {
                    *remembered = *is_hovered;
                    cx.notify();
                });
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |shell, event: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    if event.click_count >= 2 {
                        let reset =
                            clamp_column_width(column.bounds().default, bounds, bounds.maximum);
                        shell.column_drag = None;
                        DuskShell::remember_column_width(column, reset, cx);
                        cx.notify();
                        return;
                    }
                    shell.column_drag = Some(ColumnDrag {
                        column,
                        start_x: logical_x(event.position.x, window),
                        start_width: width,
                        width,
                    });
                    cx.notify();
                }),
            )
            .on_key_down(cx.listener(move |_, event: &KeyDownEvent, _, cx| {
                let modifiers = event.keystroke.modifiers;
                if modifiers.control || modifiers.platform || modifiers.alt {
                    return;
                }
                if let Some(next) = width_after_key(
                    &event.keystroke.key,
                    width,
                    bounds,
                    column.grows_rightwards(),
                    COMPONENT_COLUMN_RESIZE_KEYBOARD_STEP,
                ) {
                    cx.stop_propagation();
                    DuskShell::remember_column_width(column, next, cx);
                    cx.notify();
                }
            }))
            // The line stays centred on the gap.
            .child(
                div()
                    .my(length(COMPONENT_COLUMN_RESIZE_LINE_INSET))
                    .ml(length(
                        (SIZE_SHEET_INSET - COMPONENT_COLUMN_RESIZE_LINE_WIDTH) / 2.0,
                    ))
                    .w(length(COMPONENT_COLUMN_RESIZE_LINE_WIDTH))
                    .rounded_full()
                    .bg(line),
            )
            .when(is_dragging, |handle| {
                handle.cursor(CursorStyle::ResizeLeftRight)
            })
    }
}
