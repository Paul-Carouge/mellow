//! `MessageCodeBlock.svelte` (DESIGN.md §5.4, v0.48): a fenced code block in a message body, with
//! "Copy code" at its trailing top and, past 20 lines, a fold that shows the first 12 with
//! "Show all (42 lines)". Long lines scroll inside the block, never widen the message.

use gpui::{
    App, ClipboardItem, Context, ElementId, Hsla, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, linear_color_stop, linear_gradient, prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::code_blocks::{
    FOLDED_CODE_BLOCK_VISIBLE_LINES, code_block_line_count, first_code_block_lines,
    is_code_block_foldable,
};

use crate::components::{
    Tooltip, child_id, colour_transition, focus_ring, icon, inset_focus_ring, is_focus_visible,
    remembered_focus_handle, remembered_hover, with_type,
};
use crate::i18n;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_MESSAGE_CODE_COPIED_LABEL_DURATION, COMPONENT_MESSAGE_CODE_COPY_BUTTON_INSET,
    COMPONENT_MESSAGE_CODE_COPY_BUTTON_SIZE, COMPONENT_MESSAGE_CODE_COPY_ICON_SIZE,
    COMPONENT_MESSAGE_CODE_FADE_HEIGHT, COMPONENT_MESSAGE_CODE_FOLD_HEIGHT, FONT_WEIGHT_SEMIBOLD,
    RADIUS_SM, RADIUS_XS, SPACE_2XS, SPACE_3XS, SPACE_XS, TYPE_BODY_SM, TYPE_CODE,
};
use crate::theme::{color, delay, length, theme};

/// Whether the block is open and whether "Copied" shows, kept while the block is drawn.
#[derive(Debug, Default)]
struct CodeBlockState {
    is_open: bool,
    is_copied: bool,
    /// Bumped by each copy, so an older copy's timer leaves a newer "Copied" alone.
    copies: u64,
}

impl CodeBlockState {
    fn copy(&mut self, text: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(text.to_owned()));
        self.is_copied = true;
        self.copies += 1;
        let copy = self.copies;
        cx.spawn(async move |state, cx| {
            cx.background_executor()
                .timer(delay(COMPONENT_MESSAGE_CODE_COPIED_LABEL_DURATION))
                .await;
            // The block may be gone: nothing to reset then.
            let _ = state.update(cx, |state, cx| {
                if state.copies == copy {
                    state.is_copied = false;
                    cx.notify();
                }
            });
        })
        .detach();
        cx.notify();
    }
}

#[derive(IntoElement)]
pub struct MessageCodeBlock {
    id: ElementId,
    text: SharedString,
}

impl MessageCodeBlock {
    #[must_use]
    pub fn new(id: impl Into<ElementId>, text: SharedString) -> Self {
        Self {
            id: id.into(),
            text,
        }
    }
}

impl RenderOnce for MessageCodeBlock {
    #[allow(clippy::too_many_lines)] // The well, its copy key and its fold, as the web's markup.
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        let strings = i18n::strings(cx).messages();
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, _| {
            CodeBlockState::default()
        });
        let (is_open, is_copied) = {
            let state = state.read(cx);
            (state.is_open, state.is_copied)
        };
        let line_count = code_block_line_count(&self.text);
        let is_foldable = is_code_block_foldable(line_count);
        let is_folded = is_foldable && !is_open;
        // Only the shown lines are laid out: a 400-line paste costs nothing until it is opened.
        let shown_text: SharedString = if is_folded {
            first_code_block_lines(&self.text, FOLDED_CODE_BLOCK_VISIBLE_LINES)
                .to_owned()
                .into()
        } else {
            self.text.clone()
        };

        let block_hover = remembered_hover(&self.id, window, cx);
        let copy_id = child_id(&self.id, "copy");
        let copy_focus = remembered_focus_handle(&copy_id, window, cx);
        let copy_hover = remembered_hover(&copy_id, window, cx);
        let is_copy_focused = is_focus_visible(&copy_focus, window);
        let is_copy_shown = *block_hover.read(cx) || is_copy_focused || is_copied;
        let control_text = colour_transition(
            child_id(&copy_id, "text"),
            color(if *copy_hover.read(cx) {
                tokens.component_message_code_control_text_hover
            } else {
                tokens.component_message_code_control_text
            }),
            window,
            cx,
        );
        let copy_label = if is_copied {
            strings.code_copied()
        } else {
            strings.copy_code()
        };
        let copy_button = div()
            .id(copy_id.clone())
            .role(Role::Button)
            .aria_label(copy_label)
            .track_focus(&copy_focus)
            .absolute()
            .top(length(COMPONENT_MESSAGE_CODE_COPY_BUTTON_INSET))
            .right(length(COMPONENT_MESSAGE_CODE_COPY_BUTTON_INSET))
            .flex()
            .items_center()
            .justify_center()
            .gap(length(SPACE_3XS))
            .min_w(length(COMPONENT_MESSAGE_CODE_COPY_BUTTON_SIZE))
            .h(length(COMPONENT_MESSAGE_CODE_COPY_BUTTON_SIZE))
            .px(length(SPACE_3XS))
            .rounded(length(RADIUS_XS))
            .border_1()
            .border_color(color(tokens.component_message_code_copy_button_border))
            .bg(color(tokens.component_message_code_copy_button_background))
            .text_color(control_text)
            .cursor_pointer()
            // Hidden until the block is hovered or the key focused; kept in the Tab order.
            .when(!is_copy_shown, |button| button.opacity(0.0))
            .tooltip({
                let label = SharedString::from(strings.copy_code());
                move |_, cx| Tooltip::view(label.clone(), cx)
            })
            .tooltip_show_delay(Tooltip::show_delay())
            .on_hover(move |is_hovered, _, cx| {
                copy_hover.update(cx, |remembered, cx| {
                    *remembered = *is_hovered;
                    cx.notify();
                });
            })
            .on_click({
                let state = state.clone();
                let text = self.text.clone();
                move |_, _, cx| state.update(cx, |state, cx| state.copy(&text, cx))
            })
            .child(icon(
                if is_copied {
                    IconName::Check
                } else {
                    IconName::Copy
                },
                COMPONENT_MESSAGE_CODE_COPY_ICON_SIZE,
                control_text,
            ))
            .when(is_copied, |button| {
                button.child(
                    with_type(div(), TYPE_BODY_SM)
                        .font_weight(gpui::FontWeight(f32::from(FONT_WEIGHT_SEMIBOLD)))
                        .child(strings.code_copied()),
                )
            })
            .when(is_copy_focused, |button| {
                button.child(focus_ring(RADIUS_XS, cx))
            });

        let well = color(tokens.component_message_code_background);
        let fade = div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(length(COMPONENT_MESSAGE_CODE_FADE_HEIGHT))
            .bg(linear_gradient(
                180.0,
                linear_color_stop(Hsla { a: 0.0, ..well }, 0.0),
                linear_color_stop(well, 1.0),
            ));
        let code = div()
            .id(child_id(&self.id, "scroll"))
            .relative()
            .flex()
            .overflow_x_scroll()
            .p(length(SPACE_XS))
            // Keeps the copy key off the end of a line that fits.
            .pr(length(
                COMPONENT_MESSAGE_CODE_COPY_BUTTON_SIZE
                    + 2.0 * COMPONENT_MESSAGE_CODE_COPY_BUTTON_INSET,
            ))
            .child(
                // As wide as its longest line, so the block scrolls to its end.
                with_type(div(), TYPE_CODE)
                    .flex_none()
                    .whitespace_nowrap()
                    .text_color(color(tokens.color_text_normal))
                    .child(shown_text),
            )
            .when(is_folded, |code| code.child(fade));

        let fold = is_foldable.then(|| {
            let fold_id = child_id(&self.id, "fold");
            let fold_focus = remembered_focus_handle(&fold_id, window, cx);
            let fold_hover = remembered_hover(&fold_id, window, cx);
            let is_fold_hovered = *fold_hover.read(cx);
            let label: SharedString = if is_open {
                strings.fold_code().into()
            } else {
                strings
                    .show_all_code(i64::try_from(line_count).unwrap_or(i64::MAX))
                    .into()
            };
            let text = color(if is_fold_hovered {
                tokens.component_message_code_control_text_hover
            } else {
                tokens.component_message_code_control_text
            });
            let background = colour_transition(
                child_id(&fold_id, "background"),
                if is_fold_hovered {
                    color(tokens.component_message_code_control_background_hover)
                } else {
                    Hsla::transparent_black()
                },
                window,
                cx,
            );
            with_type(div(), TYPE_BODY_SM)
                .id(fold_id)
                .role(Role::Button)
                .aria_label(label.clone())
                .aria_expanded(is_open)
                .track_focus(&fold_focus)
                .relative()
                .flex()
                .items_center()
                .gap(length(SPACE_2XS))
                .w_full()
                .h(length(COMPONENT_MESSAGE_CODE_FOLD_HEIGHT))
                .px(length(SPACE_XS))
                .border_t_1()
                .border_color(color(tokens.component_message_code_border))
                .rounded_b(length(RADIUS_SM))
                .bg(background)
                .text_color(text)
                .font_weight(gpui::FontWeight(f32::from(FONT_WEIGHT_SEMIBOLD)))
                .cursor_pointer()
                .on_hover(move |is_hovered, _, cx| {
                    fold_hover.update(cx, |remembered, cx| {
                        *remembered = *is_hovered;
                        cx.notify();
                    });
                })
                .on_click({
                    let state = state.clone();
                    move |_, _, cx| {
                        state.update(cx, |state, cx| {
                            state.is_open = !state.is_open;
                            cx.notify();
                        });
                    }
                })
                .child(icon(
                    if is_open {
                        IconName::CaretUp
                    } else {
                        IconName::CaretDown
                    },
                    COMPONENT_MESSAGE_CODE_COPY_ICON_SIZE,
                    text,
                ))
                .child(label)
                .when(is_focus_visible(&fold_focus, window), |button| {
                    button.child(inset_focus_ring(RADIUS_SM, cx))
                })
        });

        div()
            .id(self.id)
            .relative()
            .my(length(SPACE_2XS))
            .flex()
            .flex_col()
            .min_w_0()
            .border_1()
            .border_color(color(tokens.component_message_code_border))
            .rounded(length(RADIUS_SM))
            .bg(well)
            .on_hover(move |is_hovered, _, cx| {
                block_hover.update(cx, |remembered, cx| {
                    *remembered = *is_hovered;
                    cx.notify();
                });
            })
            .child(code)
            .children(fold)
            .child(copy_button)
    }
}

impl std::fmt::Debug for MessageCodeBlock {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MessageCodeBlock")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
