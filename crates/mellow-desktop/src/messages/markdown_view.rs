//! `MessageMarkdown.svelte`: a parsed message body as elements. Every node becomes a run of text
//! or an element here; message text is never read as anything else.

use std::collections::HashSet;
use std::hash::{DefaultHasher, Hash as _, Hasher as _};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Div, ElementId, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, RenderOnce, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Window, div, prelude::FluentBuilder as _,
};
use mellow_client_core::shared_logic::markdown::{BlockNode, InlineNode, MarkdownList};

use super::body::MessageBody;
use super::code_block::MessageCodeBlock;
use super::rich_text::{EmojiRangesByText, RichText, RichTextContext, RichTextElement};
use crate::components::{child_id, with_type};
use crate::i18n;
use crate::theme::tokens::{
    COMPONENT_CUSTOM_EMOJI_INLINE_SIZE, COMPONENT_CUSTOM_EMOJI_JUMBO_SIZE,
    COMPONENT_MESSAGE_QUOTE_BAR_WIDTH, RADIUS_XS, SPACE_2XS, SPACE_MD, SPACE_SM, SPACE_XS,
    TYPE_MESSAGE, TYPE_MESSAGE_H1, TYPE_MESSAGE_H2, TYPE_MESSAGE_H3, TYPE_MESSAGE_SUBTEXT,
    ThemeTokens, TypeToken,
};
use crate::theme::{color, length, theme};

/// A figure's width in the message font, for numbered list markers (`1ch`, tabular digits).
const DIGIT_SHARE_OF_EM: f32 = 0.6;
/// Headings sit at aria levels 4 to 6, below the page's and the channel's (the web's `3 + level`).
const HEADING_LEVEL_OFFSET: usize = super::item::MESSAGE_HEADER_HEADING_LEVEL;
/// Chrome draws a list's disc a third of the font's ascent wide: 5 px at the message's 16 px.
const MARKER_DISC_SHARE_OF_EM: f32 = 0.32;
/// A hollow (nested) disc's ring.
const MARKER_CIRCLE_BORDER_WIDTH: f32 = 1.0;

/// "(edited)" at the end of the body, as Discord places it.
#[derive(Debug, Clone)]
pub struct EditedMarker {
    pub label: SharedString,
    /// "Edited Monday, October 19, 2026 at 8:30 AM".
    pub tooltip: SharedString,
}

/// A message's body. `id` is the row's, so revealed spoilers and opened code blocks are kept while
/// the row is drawn.
#[derive(IntoElement)]
pub struct MessageMarkdown {
    id: ElementId,
    body: Rc<MessageBody>,
    edited: Option<EditedMarker>,
    selection_order: u64,
}

impl MessageMarkdown {
    #[must_use]
    pub fn new(id: impl Into<ElementId>, body: Rc<MessageBody>) -> Self {
        Self {
            id: id.into(),
            body,
            edited: None,
            selection_order: 0,
        }
    }

    #[must_use]
    pub fn edited(mut self, edited: Option<EditedMarker>) -> Self {
        self.edited = edited;
        self
    }

    /// Where the body's texts start in the window's reading order; its texts follow one by one.
    #[must_use]
    pub fn selection_order(mut self, order: u64) -> Self {
        self.selection_order = order;
        self
    }
}

/// What every block of one body is drawn with.
struct BlockContext<'a> {
    id: &'a ElementId,
    tokens: &'static ThemeTokens,
    strings: i18n::Strings,
    revealed_spoilers: &'a Entity<HashSet<u32>>,
    revealed: &'a HashSet<u32>,
    is_jumbo: bool,
    emoji_ranges: &'a EmojiRangesByText,
    /// Numbers the texts and code blocks, so each keeps its own state between frames.
    next_index: usize,
    selection_order: u64,
}

impl BlockContext<'_> {
    fn next_id(&mut self, kind: &str) -> ElementId {
        self.next_index += 1;
        ElementId::NamedChild(
            Arc::new(self.id.clone()),
            format!("{kind}-{}", self.next_index).into(),
        )
    }

    fn rich_text(&self, nodes: &[InlineNode]) -> RichText {
        let (picture_side, font_size) = if self.is_jumbo {
            (
                COMPONENT_CUSTOM_EMOJI_JUMBO_SIZE,
                COMPONENT_CUSTOM_EMOJI_JUMBO_SIZE,
            )
        } else {
            (COMPONENT_CUSTOM_EMOJI_INLINE_SIZE, TYPE_MESSAGE.font_size)
        };
        RichText::from_inlines(
            nodes,
            RichTextContext {
                tokens: self.tokens,
                strings: self.strings,
                revealed_spoilers: self.revealed,
                picture_em: picture_side / font_size,
                emoji_ranges: Some(self.emoji_ranges),
            },
        )
    }

    fn text_element(&mut self, rich_text: RichText) -> RichTextElement {
        let picture_side = if self.is_jumbo {
            COMPONENT_CUSTOM_EMOJI_JUMBO_SIZE
        } else {
            COMPONENT_CUSTOM_EMOJI_INLINE_SIZE
        };
        let id = self.next_id("text");
        RichTextElement::new(
            id,
            rich_text,
            picture_side,
            Some(self.revealed_spoilers.clone()),
            self.tokens,
        )
        .selection_order(self.selection_order + self.next_index as u64)
    }
}

fn typed(token: TypeToken) -> Div {
    with_type(div(), token)
}

impl MessageMarkdown {
    fn blocks(
        blocks: &[BlockNode],
        trailing: Option<&EditedMarker>,
        context: &mut BlockContext<'_>,
    ) -> Vec<AnyElement> {
        let last_index = blocks.len().checked_sub(1);
        blocks
            .iter()
            .enumerate()
            .map(|(index, block)| {
                // "(edited)" follows the last word when the body ends in a line of text.
                let trailing = trailing.filter(|_| Some(index) == last_index);
                Self::block(block, trailing, index == 0, context)
            })
            .collect()
    }

    fn block(
        block: &BlockNode,
        trailing: Option<&EditedMarker>,
        is_first: bool,
        context: &mut BlockContext<'_>,
    ) -> AnyElement {
        let tokens = context.tokens;
        let with_trailing = |mut rich_text: RichText| {
            if let Some(edited) = trailing {
                rich_text.push_trailing(
                    &edited.label,
                    color(tokens.component_message_edited_text),
                    edited.tooltip.clone(),
                );
            }
            rich_text
        };
        match block {
            BlockNode::Paragraph(children) => {
                let rich_text = with_trailing(context.rich_text(children));
                let paragraph = div().w_full().when(context.is_jumbo, |paragraph| {
                    paragraph
                        .text_size(length(COMPONENT_CUSTOM_EMOJI_JUMBO_SIZE))
                        .line_height(length(COMPONENT_CUSTOM_EMOJI_JUMBO_SIZE))
                });
                paragraph
                    .child(context.text_element(rich_text))
                    .into_any_element()
            }
            BlockNode::Heading { level, children } => {
                let token = match level {
                    1 => TYPE_MESSAGE_H1,
                    2 => TYPE_MESSAGE_H2,
                    _ => TYPE_MESSAGE_H3,
                };
                let rich_text = context.rich_text(children);
                typed(token)
                    .id(context.next_id("heading"))
                    .role(Role::Heading)
                    .aria_level(HEADING_LEVEL_OFFSET + usize::from(*level))
                    .when(!is_first, |heading| heading.mt(length(SPACE_XS)))
                    .mb(length(SPACE_2XS))
                    .text_color(color(tokens.color_text_heading))
                    .child(context.text_element(rich_text))
                    .into_any_element()
            }
            BlockNode::Subtext(children) => {
                let rich_text = with_trailing(context.rich_text(children));
                typed(TYPE_MESSAGE_SUBTEXT)
                    .text_color(color(tokens.color_text_muted))
                    .child(context.text_element(rich_text))
                    .into_any_element()
            }
            BlockNode::Quote(children) => div()
                .flex()
                .flex_col()
                .border_l(length(COMPONENT_MESSAGE_QUOTE_BAR_WIDTH))
                .border_color(color(tokens.component_message_quote_bar_color))
                .rounded(length(RADIUS_XS))
                .pl(length(SPACE_SM))
                .children(Self::blocks(children, None, context))
                .into_any_element(),
            BlockNode::List(list) => Self::list(list, false, context),
            BlockNode::CodeBlock { text, .. } => {
                // Keyed by its text too: an edit that changes the block starts it folded again,
                // as the web's `{#key block.text}`.
                let mut hasher = DefaultHasher::new();
                text.hash(&mut hasher);
                let id = context.next_id(&format!("code-{:x}", hasher.finish()));
                MessageCodeBlock::new(id, text.clone().into()).into_any_element()
            }
        }
    }

    fn list(list: &MarkdownList, is_nested: bool, context: &mut BlockContext<'_>) -> AnyElement {
        let tokens = context.tokens;
        // Each digit past the first takes one more figure, so "10." lines up with "9.".
        let largest_number = list
            .start
            .map(|start| u64::from(start) + list.items.len().saturating_sub(1) as u64);
        let extra_digits = largest_number.map_or(0, |number| number.to_string().len() - 1);
        #[allow(clippy::cast_precision_loss)] // A handful of digits.
        let marker_width =
            SPACE_MD + extra_digits as f32 * DIGIT_SHARE_OF_EM * TYPE_MESSAGE.font_size;
        let marker_colour = color(tokens.component_message_body_text);
        let line_height = TYPE_MESSAGE.font_size * TYPE_MESSAGE.line_height;
        let disc_side = MARKER_DISC_SHARE_OF_EM * TYPE_MESSAGE.font_size;
        let items: Vec<AnyElement> = list
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                // The web's outside markers: a number ends a space before the text; a disc sits
                // centred on the first line, `space.sm` before the text, as Chrome draws it.
                let marker = match list.start {
                    Some(start) => div()
                        .size_full()
                        .flex()
                        .justify_end()
                        .pr(length(SPACE_2XS))
                        .child(SharedString::from(format!(
                            "{}.",
                            u64::from(start) + index as u64
                        )))
                        .into_any_element(),
                    None => div()
                        .absolute()
                        .right(length(SPACE_SM))
                        .top(length((line_height - disc_side) / 2.0))
                        .size(length(disc_side))
                        .rounded_full()
                        .map(|disc| {
                            if is_nested {
                                disc.border(length(MARKER_CIRCLE_BORDER_WIDTH))
                                    .border_color(marker_colour)
                            } else {
                                disc.bg(marker_colour)
                            }
                        })
                        .into_any_element(),
                };
                let rich_text = context.rich_text(&item.children);
                let text = context.text_element(rich_text);
                let sublists: Vec<AnyElement> = item
                    .sublists
                    .iter()
                    .map(|sublist| Self::list(sublist, true, context))
                    .collect();
                div()
                    .flex()
                    .child(
                        div()
                            .relative()
                            .flex_none()
                            .w(length(marker_width))
                            .h(length(line_height))
                            .child(marker),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(text)
                            .children(sublists),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .id(context.next_id("list"))
            .role(Role::List)
            .flex()
            .flex_col()
            .children(items)
            .into_any_element()
    }
}

/// A body and its emoji, found when the row first drew this version of it.
type FoundEmoji = Option<(Rc<MessageBody>, Rc<EmojiRangesByText>)>;

impl MessageMarkdown {
    /// The body's emoji, found once per version of the body the row shows, not at every frame.
    fn emoji_ranges(
        id: &ElementId,
        body: &Rc<MessageBody>,
        window: &mut Window,
        cx: &mut App,
    ) -> Rc<EmojiRangesByText> {
        let state =
            window.use_keyed_state(child_id(id, "emoji-ranges"), cx, |_, _| -> FoundEmoji {
                None
            });
        if let Some((found_for, found)) = state.read(cx).as_ref()
            && Rc::ptr_eq(found_for, body)
        {
            return Rc::clone(found);
        }
        let found = Rc::new(EmojiRangesByText::of_blocks(&body.blocks));
        state.update(cx, |state, _| {
            *state = Some((Rc::clone(body), Rc::clone(&found)));
        });
        found
    }
}

impl RenderOnce for MessageMarkdown {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let revealed_spoilers =
            window.use_keyed_state(child_id(&self.id, "revealed-spoilers"), cx, |_, _| {
                HashSet::new()
            });
        let revealed = revealed_spoilers.read(cx).clone();
        let emoji_ranges = Self::emoji_ranges(&self.id, &self.body, window, cx);
        let mut context = BlockContext {
            id: &self.id,
            tokens: theme(cx),
            strings: i18n::strings(cx),
            revealed_spoilers: &revealed_spoilers,
            revealed: &revealed,
            is_jumbo: self.body.is_jumbo,
            emoji_ranges: &emoji_ranges,
            next_index: 0,
            selection_order: self.selection_order,
        };
        // A jumbo paragraph's size would carry over to "(edited)": it goes on its own line.
        let ends_in_text = !self.body.is_jumbo
            && matches!(
                self.body.blocks.last(),
                Some(BlockNode::Paragraph(_) | BlockNode::Subtext(_))
            );
        let trailing_inline = self.edited.as_ref().filter(|_| ends_in_text);
        let mut children = Self::blocks(&self.body.blocks, trailing_inline, &mut context);
        // After a heading, a quote, a list or a code block, or an empty body: on its own line.
        if let Some(edited) = self.edited.as_ref().filter(|_| !ends_in_text) {
            let mut rich_text = RichText::default();
            rich_text.push_trailing(
                &edited.label,
                color(context.tokens.component_message_edited_text),
                edited.tooltip.clone(),
            );
            children.push(context.text_element(rich_text).into_any_element());
        }
        div().flex().flex_col().min_w_0().children(children)
    }
}

impl std::fmt::Debug for MessageMarkdown {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MessageMarkdown")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
