//! Highlighted words in search results (`web/src/components/search-highlights.ts`, ADR-0029): the
//! instance sends spans of `Message::content` in UTF-16 units and never markup, so the client cuts
//! the parsed message's text with them.
//!
//! The web slices UTF-16 strings; here ranges become byte ranges of the UTF-8 content first,
//! widened to whole characters, which also keeps an emoji (two UTF-16 units) from being cut.

use std::ops::Range;

use mellow_protocol::HighlightRange;

use super::markdown::{BlockNode, InlineNode, MarkdownList, MarkdownListItem};

/// A run of text, matched or not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSegment<'a> {
    pub text: &'a str,
    pub is_match: bool,
}

/// The byte offset in `content` of UTF-16 unit `unit`: the start of the character holding it,
/// or the end of that character when `rounds_up` and the unit falls inside it.
fn byte_offset_of_utf16_unit(content: &str, unit: u32, rounds_up: bool) -> usize {
    let wanted = usize::try_from(unit).unwrap_or(usize::MAX);
    let mut units = 0;
    for (byte_offset, character) in content.char_indices() {
        if units >= wanted {
            return byte_offset;
        }
        let next = units + character.len_utf16();
        if next > wanted {
            return if rounds_up {
                byte_offset + character.len_utf8()
            } else {
                byte_offset
            };
        }
        units = next;
    }
    content.len()
}

/// The ranges as byte ranges of `content`: clamped, widened to whole characters, sorted, merged
/// when they touch, empty ones dropped.
#[must_use]
pub fn normalized_highlight_ranges(content: &str, ranges: &[HighlightRange]) -> Vec<Range<usize>> {
    let mut widened: Vec<Range<usize>> = ranges
        .iter()
        .map(|range| {
            byte_offset_of_utf16_unit(content, range.start, false)
                ..byte_offset_of_utf16_unit(content, range.end, true)
        })
        .filter(|range| range.start < range.end)
        .collect();
    widened.sort_by_key(|range| range.start);
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(widened.len());
    for range in widened {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    merged
}

/// `text`, found at byte `offset` in the content, cut into matched and unmatched runs.
#[must_use]
pub fn split_by_highlights<'a>(
    text: &'a str,
    offset: usize,
    ranges: &[Range<usize>],
) -> Vec<TextSegment<'a>> {
    let mut segments = Vec::new();
    let mut position = 0;
    for range in ranges {
        let start = range.start.saturating_sub(offset).max(position);
        let end = range.end.saturating_sub(offset).min(text.len());
        if end <= start {
            continue;
        }
        // Ranges are whole characters of the content and `text` is a slice of it at `offset`, so
        // these are character boundaries; `get` keeps a mismatch from panicking anyway.
        let (Some(before), Some(matched)) = (text.get(position..start), text.get(start..end))
        else {
            continue;
        };
        if !before.is_empty() {
            segments.push(TextSegment {
                text: before,
                is_match: false,
            });
        }
        segments.push(TextSegment {
            text: matched,
            is_match: true,
        });
        position = end;
    }
    if position < text.len() || segments.is_empty() {
        segments.push(TextSegment {
            text: text.get(position..).unwrap_or_default(),
            is_match: false,
        });
    }
    segments
}

/// Finds each text of the parsed message in its content, from where the previous one ended.
struct Marker<'a> {
    content: &'a str,
    ranges: Vec<Range<usize>>,
    cursor: usize,
}

impl Marker<'_> {
    /// Where `text` sits from the cursor on, moving the cursor past it; `None` when not found.
    fn locate(&mut self, text: &str) -> Option<usize> {
        if text.is_empty() {
            return None;
        }
        let found = self.content.get(self.cursor..)?.find(text)? + self.cursor;
        self.cursor = found + text.len();
        Some(found)
    }

    fn mark_text(&mut self, text: &str, marked: &mut Vec<InlineNode>) {
        // Line by line: a quote's text has lost the "> " starting each line of the content.
        for (index, line) in text.split('\n').enumerate() {
            if index > 0 {
                marked.push(InlineNode::Text("\n".to_owned()));
            }
            match self.locate(line) {
                None if line.is_empty() => {}
                None => marked.push(InlineNode::Text(line.to_owned())),
                Some(offset) => marked.extend(
                    split_by_highlights(line, offset, &self.ranges)
                        .into_iter()
                        .map(|segment| {
                            if segment.is_match {
                                InlineNode::Match(segment.text.to_owned())
                            } else {
                                InlineNode::Text(segment.text.to_owned())
                            }
                        }),
                ),
            }
        }
    }

    fn mark_inlines(&mut self, nodes: &[InlineNode]) -> Vec<InlineNode> {
        let mut marked = Vec::with_capacity(nodes.len());
        for node in nodes {
            match node {
                InlineNode::Text(text) => self.mark_text(text, &mut marked),
                // shortcut: a mention chip stays whole and unlit even when its name matched; mark the chip if people miss it.
                InlineNode::Mention { text, .. }
                | InlineNode::EveryoneMention(text)
                | InlineNode::Code(text) => {
                    self.locate(text);
                    marked.push(node.clone());
                }
                // The content holds `<@&id>`, never the role's name: nothing to find in it.
                InlineNode::RoleMention(_) | InlineNode::CustomEmoji(_) | InlineNode::Match(_) => {
                    marked.push(node.clone());
                }
                InlineNode::Strong(children) => {
                    marked.push(InlineNode::Strong(self.mark_inlines(children)));
                }
                InlineNode::Emphasis(children) => {
                    marked.push(InlineNode::Emphasis(self.mark_inlines(children)));
                }
                InlineNode::Underline(children) => {
                    marked.push(InlineNode::Underline(self.mark_inlines(children)));
                }
                InlineNode::Strikethrough(children) => {
                    marked.push(InlineNode::Strikethrough(self.mark_inlines(children)));
                }
                InlineNode::Spoiler { id, children } => marked.push(InlineNode::Spoiler {
                    id: *id,
                    children: self.mark_inlines(children),
                }),
                InlineNode::Link {
                    url,
                    is_masked,
                    children,
                } => marked.push(InlineNode::Link {
                    url: url.clone(),
                    is_masked: *is_masked,
                    children: self.mark_inlines(children),
                }),
            }
        }
        marked
    }

    fn mark_blocks(&mut self, blocks: &[BlockNode]) -> Vec<BlockNode> {
        blocks
            .iter()
            .map(|block| match block {
                BlockNode::Paragraph(children) => BlockNode::Paragraph(self.mark_inlines(children)),
                BlockNode::Heading { level, children } => BlockNode::Heading {
                    level: *level,
                    children: self.mark_inlines(children),
                },
                BlockNode::Subtext(children) => BlockNode::Subtext(self.mark_inlines(children)),
                BlockNode::Quote(children) => BlockNode::Quote(self.mark_blocks(children)),
                BlockNode::List(list) => BlockNode::List(self.mark_list(list)),
                // shortcut: matches inside code blocks are not marked; split the block's text if people miss it.
                BlockNode::CodeBlock { text, .. } => {
                    self.locate(text);
                    block.clone()
                }
            })
            .collect()
    }

    fn mark_list(&mut self, list: &MarkdownList) -> MarkdownList {
        MarkdownList {
            start: list.start,
            items: list
                .items
                .iter()
                .map(|item| MarkdownListItem {
                    children: self.mark_inlines(&item.children),
                    sublists: item
                        .sublists
                        .iter()
                        .map(|sublist| self.mark_list(sublist))
                        .collect(),
                })
                .collect(),
        }
    }
}

/// The parsed message with its matched words as [`InlineNode::Match`]. Each text is found in
/// `content` from where the previous one ended, so a text the parser changed (an escape it
/// removed) simply stays unmarked rather than marking the wrong place.
#[must_use]
pub fn with_search_highlights(
    blocks: &[BlockNode],
    content: &str,
    ranges: &[HighlightRange],
) -> Vec<BlockNode> {
    let ranges = normalized_highlight_ranges(content, ranges);
    if ranges.is_empty() {
        return blocks.to_vec();
    }
    Marker {
        content,
        ranges,
        cursor: 0,
    }
    .mark_blocks(blocks)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use mellow_protocol::{PublicUser, Snowflake};

    use super::*;
    use crate::shared_logic::markdown::{
        MarkdownSources, RoleMentionContext, parse_message_markdown,
    };
    use crate::shared_logic::mentions::mentionable_people;

    fn range(start: u32, end: u32) -> HighlightRange {
        HighlightRange { start, end }
    }

    fn parsed(content: &str, people: &[PublicUser]) -> Vec<BlockNode> {
        let people = mentionable_people(people);
        let custom_emojis = HashMap::new();
        let roles = HashMap::new();
        parse_message_markdown(
            content,
            MarkdownSources {
                people: &people,
                custom_emojis: &custom_emojis,
                role_mentions: RoleMentionContext {
                    roles: &roles,
                    is_everyone_mentioned: false,
                },
            },
        )
    }

    fn marked(content: &str, ranges: &[HighlightRange]) -> Vec<BlockNode> {
        with_search_highlights(&parsed(content, &[]), content, ranges)
    }

    fn text(text: &str) -> InlineNode {
        InlineNode::Text(text.to_owned())
    }

    fn matched(text: &str) -> InlineNode {
        InlineNode::Match(text.to_owned())
    }

    fn count_matches(blocks: &[BlockNode], word: &str) -> usize {
        format!("{blocks:?}")
            .matches(&format!("Match({word:?})"))
            .count()
    }

    #[test]
    fn slices_by_utf16_units_and_never_cuts_an_emoji_in_half() {
        let content = "a😀b pizza";
        // 😀 is two UTF-16 units (1 and 2) and four bytes (1 to 5).
        assert_eq!(
            normalized_highlight_ranges(content, &[range(2, 3)]),
            vec![1..5]
        );
        assert_eq!(
            normalized_highlight_ranges(content, &[range(0, 2)]),
            vec![0..5]
        );
        let ranges = normalized_highlight_ranges(content, &[range(5, 10)]);
        assert_eq!(
            split_by_highlights(content, 0, &ranges),
            vec![
                TextSegment {
                    text: "a😀b ",
                    is_match: false
                },
                TextSegment {
                    text: "pizza",
                    is_match: true
                },
            ]
        );
    }

    #[test]
    fn clamps_sorts_and_merges_ranges_and_drops_empty_ones() {
        assert_eq!(
            normalized_highlight_ranges(
                "hello world",
                &[range(6, 99), range(0, 2), range(2, 5), range(4, 4)]
            ),
            vec![0..5, 6..11]
        );
    }

    #[test]
    fn marks_matched_words_inside_formatting_across_the_parsers_text_nodes() {
        let content = "the **pizza** is *ready*";
        assert_eq!(
            marked(content, &[range(6, 11), range(18, 23)]),
            vec![BlockNode::Paragraph(vec![
                text("the "),
                InlineNode::Strong(vec![matched("pizza")]),
                text(" is "),
                InlineNode::Emphasis(vec![matched("ready")]),
            ])]
        );
    }

    #[test]
    fn finds_each_text_after_the_previous_one_so_a_repeated_word_is_marked_where_it_matched() {
        let content = "pizza `pizza` pizza";
        assert_eq!(
            marked(content, &[range(14, 19)]),
            vec![BlockNode::Paragraph(vec![
                text("pizza "),
                InlineNode::Code("pizza".to_owned()),
                text(" "),
                matched("pizza"),
            ])]
        );
    }

    #[test]
    fn marks_a_word_written_after_an_emoji_at_the_right_place() {
        let content = "🍕🍕 pizza";
        assert_eq!(
            marked(content, &[range(5, 10)]),
            vec![BlockNode::Paragraph(vec![text("🍕🍕 "), matched("pizza")])]
        );
    }

    #[test]
    fn marks_words_on_every_line_of_a_quote_whose_text_lost_each_lines_prefix() {
        let content = "> one pizza\n> two pizza\nafter";
        assert_eq!(
            count_matches(&marked(content, &[range(6, 11), range(18, 23)]), "pizza"),
            2
        );
    }

    #[test]
    fn marks_words_in_numbered_and_nested_list_items_and_in_subtext() {
        let content = "1. pizza\n  - pizza\n-# pizza";
        let ranges = [range(3, 8), range(13, 18), range(22, 27)];
        assert_eq!(count_matches(&marked(content, &ranges), "pizza"), 3);
    }

    #[test]
    fn leaves_a_mention_chip_whole_and_unlit_when_its_name_matched() {
        let ada = PublicUser {
            id: Snowflake::from_raw(1),
            username: "ada".into(),
            display_name: "Ada".into(),
            avatar: None,
            banner: None,
        };
        let content = "ask @ada about pizza";
        let blocks = with_search_highlights(
            &parsed(content, std::slice::from_ref(&ada)),
            content,
            &[range(5, 8), range(15, 20)],
        );
        assert_eq!(
            blocks,
            vec![BlockNode::Paragraph(vec![
                text("ask "),
                InlineNode::Mention {
                    text: "@ada".into(),
                    user: ada,
                },
                text(" about "),
                matched("pizza"),
            ])]
        );
    }

    #[test]
    fn leaves_the_message_as_parsed_without_ranges() {
        let content = "> quoted\nplain";
        assert_eq!(marked(content, &[]), parsed(content, &[]));
    }
}
