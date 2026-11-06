//! Discord-style Markdown in message text (`web/src/components/markdown.ts`, DESIGN.md §5.4).
//! Text is parsed into a typed tree the client turns into elements.
//!
//! Every construct is found by scanning forward: a closing marker is searched for once per kind
//! and the answer is reused by later openers, so hostile text (thousands of `*`) stays linear in
//! its length times the nesting cap. Positions are byte offsets; every marker is ASCII, so a
//! position holding one is always a character boundary.

use std::collections::{BTreeMap, HashMap, HashSet};

use mellow_protocol::{CustomEmoji, Role, Snowflake};
use url::Url;

use super::mentions::{MentionSegment, MentionablePeople, split_mentions};
use super::unicode_classes::{
    character_at, character_before, character_length_at, is_format_character, is_js_whitespace,
    is_letter, is_letter_or_number, is_separator, js_trim,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineNode {
    Text(String),
    Mention {
        text: String,
        user: mellow_protocol::PublicUser,
    },
    /// `<@&id>` (ADR-0040): `None` when this client knows no such role (a deleted one).
    RoleMention(Option<Role>),
    /// `@everyone` as written, when the server kept it as a mention.
    EveryoneMention(String),
    Code(String),
    /// `<:name:id>` of an emoji this client knows, labelled with its server's name for it: the
    /// name written in the markup is anyone's choice, so it never labels a real picture.
    CustomEmoji(CustomEmoji),
    /// Words a search matched; the parser itself never makes one.
    Match(String),
    Strong(Vec<InlineNode>),
    Emphasis(Vec<InlineNode>),
    Underline(Vec<InlineNode>),
    Strikethrough(Vec<InlineNode>),
    /// `id` is unique within one message, so the client can remember which spoilers were revealed.
    Spoiler {
        id: u32,
        children: Vec<InlineNode>,
    },
    /// `is_masked` for `[text](url)`, whose destination is not the visible text.
    Link {
        url: String,
        is_masked: bool,
        children: Vec<InlineNode>,
    },
}

impl InlineNode {
    /// The nodes inside a formatting node; `None` for leaves.
    #[must_use]
    pub fn children(&self) -> Option<&[InlineNode]> {
        match self {
            Self::Strong(children)
            | Self::Emphasis(children)
            | Self::Underline(children)
            | Self::Strikethrough(children)
            | Self::Spoiler { children, .. }
            | Self::Link { children, .. } => Some(children),
            _ => None,
        }
    }
}

/// A `-`/`*` or `1.`/`1)` list; items hold one level of sublists, as Discord nests by indentation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownList {
    /// The number written on the first item of a numbered list; `None` for bullets.
    pub start: Option<u32>,
    pub items: Vec<MarkdownListItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownListItem {
    pub children: Vec<InlineNode>,
    /// Indented items under this one; a switch between bullets and numbers starts another sublist.
    pub sublists: Vec<MarkdownList>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockNode {
    Paragraph(Vec<InlineNode>),
    /// `level` is 1 to 3.
    Heading {
        level: u8,
        children: Vec<InlineNode>,
    },
    /// `-# text`: small, muted text under the rest.
    Subtext(Vec<InlineNode>),
    Quote(Vec<BlockNode>),
    List(MarkdownList),
    CodeBlock {
        language: Option<String>,
        text: String,
    },
}

/// Deeper formatting is shown as plain text; Discord stops nesting well before people do.
const MAXIMUM_INLINE_NESTING_DEPTH: usize = 6;
const FENCE: &str = "```";
const QUOTE_LINE_PREFIX: &str = "> ";
const QUOTE_REST_PREFIX: &str = ">>> ";
const SUBTEXT_PREFIX: &str = "-# ";
const MAXIMUM_HEADING_LEVEL: usize = 3;
/// List numbers stop at nine digits, as in `CommonMark`.
const MAXIMUM_LIST_NUMBER_DIGITS: usize = 9;
const CUSTOM_EMOJI_NAME_LENGTHS: std::ops::RangeInclusive<usize> = 2..=32;
const MAXIMUM_ID_DIGITS: usize = 20;
/// Punctuation that usually ends the sentence around a link rather than the link itself.
const LINK_TRAILING_PUNCTUATION: &[u8] = b".,;:!?'\")]}";

/// What role and `@everyone` mentions of one message read from (ADR-0040).
#[derive(Debug, Clone, Copy)]
pub struct RoleMentionContext<'a> {
    /// The roles a `<@&id>` may name.
    pub roles: &'a HashMap<Snowflake, Role>,
    /// The server kept `@everyone` as a mention: the message's `mention_role_ids` holds its guild.
    pub is_everyone_mentioned: bool,
}

/// What a message's text is read against.
#[derive(Debug, Clone, Copy)]
pub struct MarkdownSources<'a> {
    /// Those an `@name` can point at.
    pub people: &'a MentionablePeople,
    /// The emoji whose picture this client may load; any other reads as `:name:`.
    pub custom_emojis: &'a HashMap<Snowflake, CustomEmoji>,
    pub role_mentions: RoleMentionContext<'a>,
}

struct ParseContext<'a> {
    sources: MarkdownSources<'a>,
    next_spoiler_id: u32,
    /// Loop iterations, counted so a test can check the parser stays linear on hostile text
    /// without a clock. Every loop adds one per iteration: a loop that does not is invisible to that test.
    steps: usize,
}

/// Parses a message body into blocks.
#[must_use]
pub fn parse_message_markdown(content: &str, sources: MarkdownSources<'_>) -> Vec<BlockNode> {
    parse_message_markdown_counting_steps(content, sources).0
}

/// [`parse_message_markdown`] and the loop iterations it took.
fn parse_message_markdown_counting_steps(
    content: &str,
    sources: MarkdownSources<'_>,
) -> (Vec<BlockNode>, usize) {
    let mut state = ParseContext {
        sources,
        next_spoiler_id: 0,
        steps: 0,
    };
    let blocks = parse_blocks(content, &mut state, false);
    (blocks, state.steps)
}

/// A profile's "about me" (ADR-0050): inline Markdown only, so headings, lists, quotes and code
/// blocks stay as written. Nobody is mentioned and no custom emoji is loaded: the text is read
/// outside any server.
#[must_use]
pub fn parse_inline_markdown(content: &str) -> Vec<BlockNode> {
    let people = MentionablePeople::new();
    let custom_emojis = HashMap::new();
    let roles = HashMap::new();
    let sources = MarkdownSources {
        people: &people,
        custom_emojis: &custom_emojis,
        role_mentions: RoleMentionContext {
            roles: &roles,
            is_everyone_mentioned: false,
        },
    };
    if content.is_empty() {
        return Vec::new();
    }
    let mut state = ParseContext {
        sources,
        next_spoiler_id: 0,
        steps: 0,
    };
    vec![BlockNode::Paragraph(parse_inline(
        content, &mut state, 0, true,
    ))]
}

/// Whether the parsed message mentions `user_id` outside code.
#[must_use]
pub fn markdown_mentions_user(blocks: &[BlockNode], user_id: Snowflake) -> bool {
    blocks.iter().any(|block| match block {
        BlockNode::Paragraph(children)
        | BlockNode::Heading { children, .. }
        | BlockNode::Subtext(children) => inlines_contain_mention(children, Some(user_id)),
        BlockNode::Quote(children) => markdown_mentions_user(children, user_id),
        BlockNode::List(list) => list_mentions_user(list, user_id),
        BlockNode::CodeBlock { .. } => false,
    })
}

fn list_mentions_user(list: &MarkdownList, user_id: Snowflake) -> bool {
    list.items.iter().any(|item| {
        inlines_contain_mention(&item.children, Some(user_id))
            || item
                .sublists
                .iter()
                .any(|sublist| list_mentions_user(sublist, user_id))
    })
}

/// Whether `nodes` mention `user_id` by name, or anyone in any way when it is `None`.
fn inlines_contain_mention(nodes: &[InlineNode], user_id: Option<Snowflake>) -> bool {
    nodes.iter().any(|node| match node {
        InlineNode::Mention { user, .. } => user_id.is_none_or(|id| user.id == id),
        InlineNode::RoleMention(_) | InlineNode::EveryoneMention(_) => user_id.is_none(),
        other => other
            .children()
            .is_some_and(|children| inlines_contain_mention(children, user_id)),
    })
}

/// Whether `candidate` starts with `http://` or `https://`, in any case.
fn has_http_prefix(candidate: &str) -> bool {
    let bytes = candidate.as_bytes();
    let after_http = match bytes.get(..4) {
        Some(http) if http.eq_ignore_ascii_case(b"http") => 4,
        _ => return false,
    };
    let after_scheme = if bytes
        .get(after_http)
        .is_some_and(|byte| byte.eq_ignore_ascii_case(&b's'))
    {
        after_http + 1
    } else {
        after_http
    };
    bytes.get(after_scheme..after_scheme + 3) == Some(b"://".as_slice())
}

/// The http(s) URL `candidate` names, normalised; `None` for any other scheme, a user name or
/// password, or no URL.
#[must_use]
pub fn safe_http_url(candidate: &str) -> Option<String> {
    if !has_http_prefix(candidate) {
        return None;
    }
    let url = Url::parse(candidate).ok()?;
    // "https://paypal.com@evil.example" reads as paypal.com but leads to evil.example.
    let has_user_info =
        !url.username().is_empty() || url.password().is_some_and(|password| !password.is_empty());
    (matches!(url.scheme(), "http" | "https") && !has_user_info).then(|| url.as_str().to_owned())
}

fn slice(text: &str, start: usize, end: usize) -> &str {
    text.get(start..end).unwrap_or_default()
}

fn find_from(source: &str, needle: &str, from: usize) -> Option<usize> {
    source.get(from..)?.find(needle).map(|offset| offset + from)
}

fn find_end_of_line(source: &str, from: usize) -> usize {
    find_from(source, "\n", from).unwrap_or(source.len())
}

/// `.*\S.*` on one line: no line terminator (`.` stops at them) and something visible.
fn is_line_with_content(text: &str) -> bool {
    !text.contains(['\n', '\r', '\u{2028}', '\u{2029}'])
        && text.chars().any(|character| !is_js_whitespace(character))
}

/// `# One` to `### Three`: the level and the text.
fn read_heading(line: &str) -> Option<(u8, &str)> {
    let level = line.bytes().take_while(|byte| *byte == b'#').count();
    let text = line.get(level..)?.strip_prefix(' ')?;
    let level = u8::try_from(level)
        .ok()
        .filter(|level| (1..=MAXIMUM_HEADING_LEVEL).contains(&usize::from(*level)))?;
    is_line_with_content(text).then_some((level, text))
}

fn read_subtext(line: &str) -> Option<&str> {
    line.strip_prefix(SUBTEXT_PREFIX)
        .filter(|text| is_line_with_content(text))
}

struct ListItemLine<'a> {
    is_nested: bool,
    start: Option<u32>,
    text: &'a str,
}

/// A bullet or numbered item: two spaces or a tab before the marker nest it under the item
/// above, one space keeps it out of the list.
fn read_list_item_line(line: &str) -> Option<ListItemLine<'_>> {
    let indented = line.strip_prefix('\t').or_else(|| line.strip_prefix("  "));
    let is_nested = indented.is_some();
    let rest = indented.map_or(line, |rest| rest.trim_start_matches([' ', '\t']));
    let (start, after_marker) = if let Some(after) = rest.strip_prefix(['-', '*']) {
        (None, after)
    } else {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if !(1..=MAXIMUM_LIST_NUMBER_DIGITS).contains(&digits) {
            return None;
        }
        let after = rest.get(digits..)?.strip_prefix(['.', ')'])?;
        (Some(rest.get(..digits)?.parse().ok()?), after)
    };
    let text = after_marker.strip_prefix(' ')?;
    is_line_with_content(text).then_some(ListItemLine {
        is_nested,
        start,
        text,
    })
}

fn flush_paragraph(
    source: &str,
    start: usize,
    end: usize,
    context: &mut ParseContext<'_>,
    blocks: &mut Vec<BlockNode>,
) {
    let text = slice(source, start, end);
    let text = text.strip_suffix('\n').unwrap_or(text);
    if !text.is_empty() {
        blocks.push(BlockNode::Paragraph(parse_inline(text, context, 0, true)));
    }
}

#[allow(clippy::too_many_lines)] // One scan over lines with each block kind side by side, as the web reads them.
fn parse_blocks(
    source: &str,
    context: &mut ParseContext<'_>,
    is_inside_quote: bool,
) -> Vec<BlockNode> {
    let mut blocks = Vec::new();
    let mut paragraph_start = 0;
    let mut position = 0;
    // The next "```" at or after `position`, `None` once none is left; searched lazily.
    let mut next_fence = find_from(source, FENCE, 0);
    let mut can_a_fence_still_close = true;
    // End of the current line, kept between fence candidates on one line so it is searched once.
    let mut line_end_index: Option<usize> = None;

    while position < source.len() {
        context.steps += 1;
        // A quote or list may have swallowed the fence found earlier.
        if next_fence.is_some_and(|fence| fence < position) {
            next_fence = find_from(source, FENCE, position);
        }
        if position == 0 || source.as_bytes().get(position - 1) == Some(&b'\n') {
            let line_end = find_end_of_line(source, position);
            let line = slice(source, position, line_end);
            let rest_after_quote_prefix = source
                .get(position + QUOTE_REST_PREFIX.len()..)
                .unwrap_or_default();

            if !is_inside_quote
                && line.starts_with(QUOTE_REST_PREFIX)
                && !js_trim(rest_after_quote_prefix).is_empty()
            {
                flush_paragraph(source, paragraph_start, position, context, &mut blocks);
                let quoted = parse_blocks(rest_after_quote_prefix, context, true);
                blocks.push(BlockNode::Quote(quoted));
                return blocks;
            }

            if !is_inside_quote && line.starts_with(QUOTE_LINE_PREFIX) {
                flush_paragraph(source, paragraph_start, position, context, &mut blocks);
                let mut quoted_lines = Vec::new();
                let mut next = position;
                while next < source.len()
                    && source
                        .get(next..)
                        .is_some_and(|rest| rest.starts_with(QUOTE_LINE_PREFIX))
                {
                    context.steps += 1;
                    let quoted_line_end = find_end_of_line(source, next);
                    quoted_lines.push(slice(
                        source,
                        next + QUOTE_LINE_PREFIX.len(),
                        quoted_line_end,
                    ));
                    next = quoted_line_end + 1;
                }
                let quoted = parse_blocks(&quoted_lines.join("\n"), context, true);
                blocks.push(BlockNode::Quote(quoted));
                position = next;
                paragraph_start = next;
                continue;
            }

            if let Some((level, text)) = read_heading(line) {
                flush_paragraph(source, paragraph_start, position, context, &mut blocks);
                let children = parse_inline(text, context, 0, true);
                blocks.push(BlockNode::Heading { level, children });
                position = line_end + 1;
                paragraph_start = position;
                continue;
            }

            if let Some(text) = read_subtext(line) {
                flush_paragraph(source, paragraph_start, position, context, &mut blocks);
                let children = parse_inline(text, context, 0, true);
                blocks.push(BlockNode::Subtext(children));
                position = line_end + 1;
                paragraph_start = position;
                continue;
            }

            if let Some(first_item) = read_list_item_line(line).filter(|item| !item.is_nested) {
                flush_paragraph(source, paragraph_start, position, context, &mut blocks);
                let (list, end) = parse_list(source, position, first_item.start, context);
                blocks.push(BlockNode::List(list));
                position = end;
                paragraph_start = end;
                continue;
            }
        }

        if can_a_fence_still_close && next_fence == Some(position) {
            match parse_fence(source, position, context) {
                FenceReading::Unclosed => can_a_fence_still_close = false,
                FenceReading::Block { block, end } => {
                    flush_paragraph(source, paragraph_start, position, context, &mut blocks);
                    blocks.push(block);
                    position = if source.as_bytes().get(end) == Some(&b'\n') {
                        end + 1
                    } else {
                        end
                    };
                    paragraph_start = position;
                    next_fence = find_from(source, FENCE, position);
                    continue;
                }
                FenceReading::Blank => {}
            }
            next_fence = find_from(source, FENCE, position + 1);
        }

        let line_end = match line_end_index {
            Some(end) if end >= position => end,
            _ => find_end_of_line(source, position),
        };
        line_end_index = Some(line_end);
        let next_line_start = line_end + 1;
        position = match next_fence {
            Some(fence) if can_a_fence_still_close && fence < next_line_start => fence,
            _ => next_line_start,
        };
    }
    flush_paragraph(source, paragraph_start, source.len(), context, &mut blocks);
    blocks
}

/// The list whose first, unindented item starts at `from`, up to the first line that is not an
/// item of it; the second value is where the next block starts.
fn parse_list(
    source: &str,
    from: usize,
    start: Option<u32>,
    context: &mut ParseContext<'_>,
) -> (MarkdownList, usize) {
    let mut block = MarkdownList {
        start,
        items: Vec::new(),
    };
    let is_same_kind = |list_start: Option<u32>, line: &ListItemLine<'_>| {
        list_start.is_none() == line.start.is_none()
    };
    let mut next = from;
    while next < source.len() {
        context.steps += 1;
        let line_end = find_end_of_line(source, next);
        let Some(line) = read_list_item_line(slice(source, next, line_end)) else {
            break;
        };
        if !line.is_nested && !is_same_kind(block.start, &line) {
            break;
        }
        let children = parse_inline(line.text, context, 0, true);
        let item = MarkdownListItem {
            children,
            sublists: Vec::new(),
        };
        match block.items.last_mut() {
            Some(parent) if line.is_nested => match parent.sublists.last_mut() {
                Some(sublist) if is_same_kind(sublist.start, &line) => sublist.items.push(item),
                _ => parent.sublists.push(MarkdownList {
                    start: line.start,
                    items: vec![item],
                }),
            },
            _ => block.items.push(item),
        }
        next = line_end + 1;
    }
    (block, next)
}

enum FenceReading {
    Block {
        block: BlockNode,
        end: usize,
    },
    /// No closing fence follows, so none can close later.
    Unclosed,
    /// Only blank lines inside: the fence stays text.
    Blank,
}

fn is_fence_language_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'+' | b'-' | b'.' | b'#')
}

/// A fenced code block starting at `start`.
fn parse_fence(source: &str, start: usize, context: &mut ParseContext<'_>) -> FenceReading {
    let mut content_start = start + FENCE.len();
    let rest = source.get(content_start..).unwrap_or_default();
    let language_length = rest
        .bytes()
        .take_while(|byte| is_fence_language_byte(*byte))
        .count();
    let mut language = None;
    if language_length > 0 && rest.as_bytes().get(language_length) == Some(&b'\n') {
        language = Some(slice(rest, 0, language_length).to_owned());
        content_start += language_length + 1;
    }
    let Some(close) = find_from(source, FENCE, content_start) else {
        return FenceReading::Unclosed;
    };
    let bytes = source.as_bytes();
    let mut text_start = content_start;
    let mut text_end = close;
    while text_start < text_end && bytes.get(text_start) == Some(&b'\n') {
        context.steps += 1;
        text_start += 1;
    }
    while text_end > text_start && bytes.get(text_end - 1) == Some(&b'\n') {
        context.steps += 1;
        text_end -= 1;
    }
    let text = slice(source, text_start, text_end);
    if js_trim(text).is_empty() {
        return FenceReading::Blank;
    }
    FenceReading::Block {
        block: BlockNode::CodeBlock {
            language,
            text: text.to_owned(),
        },
        end: close + FENCE.len(),
    }
}

/// An inline code span: where it ends and what it shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodeSpan {
    pub(crate) end: usize,
    pub(crate) content: String,
}

/// Inline code spans by start, found as Markdown reads them: a run of backticks up to the next
/// run of the same length.
pub(crate) fn find_code_spans(text: &str) -> BTreeMap<usize, CodeSpan> {
    find_code_spans_counting_steps(text, &mut 0)
}

fn find_code_spans_counting_steps(text: &str, steps: &mut usize) -> BTreeMap<usize, CodeSpan> {
    let bytes = text.as_bytes();
    let mut spans = BTreeMap::new();
    let mut unclosed_run_lengths = HashSet::new();
    let count_backtick_run_length = |from: usize, steps: &mut usize| {
        let length = bytes
            .get(from..)
            .unwrap_or_default()
            .iter()
            .take_while(|byte| **byte == b'`')
            .count();
        *steps += length;
        length
    };
    let mut index = 0;
    while index < text.len() {
        *steps += 1;
        match bytes.get(index) {
            Some(b'\\') => {
                index += 1 + character_at(text, index + 1).map_or(1, char::len_utf8);
                continue;
            }
            Some(b'`') => {}
            _ => {
                index += character_length_at(text, index);
                continue;
            }
        }
        let opening = count_backtick_run_length(index, steps);
        let mut search = index + opening;
        let mut is_code_span_closed = false;
        while !unclosed_run_lengths.contains(&opening) {
            *steps += 1;
            let Some(candidate) = find_from(text, "`", search) else {
                break;
            };
            let closing = count_backtick_run_length(candidate, steps);
            if closing == opening {
                let content = slice(text, index + opening, candidate);
                let trimmed = js_trim(content);
                let content = if trimmed.is_empty() { content } else { trimmed };
                spans.insert(
                    index,
                    CodeSpan {
                        end: candidate + closing,
                        content: content.to_owned(),
                    },
                );
                index = candidate + closing;
                is_code_span_closed = true;
                break;
            }
            search = candidate + closing;
        }
        if !is_code_span_closed {
            unclosed_run_lengths.insert(opening);
            index += opening;
        }
    }
    spans
}

/// `<:name:id>` or `<a:name:id>` (ADR-0028) at `start`: its end, name and id (`None` past a u64).
fn read_custom_emoji_markup(text: &str, start: usize) -> Option<(usize, &str, Option<Snowflake>)> {
    let rest = text.get(start..)?.strip_prefix('<')?;
    let rest = rest.strip_prefix("a:").or_else(|| rest.strip_prefix(':'))?;
    let name_length = rest
        .bytes()
        .take_while(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
        .count();
    if !CUSTOM_EMOJI_NAME_LENGTHS.contains(&name_length) {
        return None;
    }
    let name = rest.get(..name_length)?;
    let (digits, after) = read_id_digits(rest.get(name_length..)?.strip_prefix(':')?)?;
    let after = after.strip_prefix('>')?;
    Some((
        text.len() - after.len(),
        name,
        digits.parse().ok().map(Snowflake::from_raw),
    ))
}

/// `<@&id>` (ADR-0040) at `start`: its end and the id (`None` past a u64).
fn read_role_mention_markup(text: &str, start: usize) -> Option<(usize, Option<Snowflake>)> {
    let rest = text.get(start..)?.strip_prefix("<@&")?;
    let (digits, after) = read_id_digits(rest)?;
    let after = after.strip_prefix('>')?;
    Some((
        text.len() - after.len(),
        digits.parse().ok().map(Snowflake::from_raw),
    ))
}

/// One to twenty ASCII digits, then the rest.
fn read_id_digits(text: &str) -> Option<(&str, &str)> {
    let length = text.bytes().take_while(u8::is_ascii_digit).count();
    (1..=MAXIMUM_ID_DIGITS)
        .contains(&length)
        .then(|| text.split_at(length))
}

/// Every custom emoji's markup as `:name:`, for plain text (previews, notifications).
#[must_use]
pub fn with_custom_emoji_as_names(text: &str) -> String {
    let mut converted = String::with_capacity(text.len());
    let mut copied_up_to = 0;
    let mut search_from = 0;
    while let Some(start) = find_from(text, "<", search_from) {
        if let Some((end, name, _)) = read_custom_emoji_markup(text, start) {
            converted.push_str(slice(text, copied_up_to, start));
            converted.push(':');
            converted.push_str(name);
            converted.push(':');
            copied_up_to = end;
            search_from = end;
        } else {
            search_from = start + 1;
        }
    }
    converted.push_str(text.get(copied_up_to..).unwrap_or_default());
    converted
}

enum MarkupSpan<'a> {
    Emoji {
        end: usize,
        name: &'a str,
        id: Option<Snowflake>,
    },
    Role {
        end: usize,
        id: Option<Snowflake>,
    },
}

impl MarkupSpan<'_> {
    fn end(&self) -> usize {
        match self {
            Self::Emoji { end, .. } | Self::Role { end, .. } => *end,
        }
    }
}

/// Custom emoji and role mention markup by start, outside code spans and escapes.
fn find_markup_spans<'t>(
    text: &'t str,
    inside_code: &[bool],
    steps: &mut usize,
) -> HashMap<usize, MarkupSpan<'t>> {
    let mut spans = HashMap::new();
    let mut search_from = 0;
    while let Some(start) = find_from(text, "<", search_from) {
        search_from = start + 1;
        *steps += 1;
        if inside_code.get(start).copied().unwrap_or(false) || is_escaped(text, start, steps) {
            continue;
        }
        if let Some((end, name, id)) = read_custom_emoji_markup(text, start) {
            spans.insert(start, MarkupSpan::Emoji { end, name, id });
        } else if let Some((end, id)) = read_role_mention_markup(text, start) {
            spans.insert(start, MarkupSpan::Role { end, id });
        }
    }
    spans
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DelimiterRule {
    Spoiler,
    Strong,
    Underline,
    Strikethrough,
    EmphasisWithStar,
    EmphasisWithUnderscore,
}

/// Tried in this order at each position, as Discord does.
const DELIMITER_RULES: [DelimiterRule; 6] = [
    DelimiterRule::Spoiler,
    DelimiterRule::Strong,
    DelimiterRule::Underline,
    DelimiterRule::Strikethrough,
    DelimiterRule::EmphasisWithStar,
    DelimiterRule::EmphasisWithUnderscore,
];

fn is_whitespace_character(character: Option<char>) -> bool {
    character.is_some_and(is_js_whitespace)
}

/// `[\p{L}\p{N}_]`.
fn is_word_character(character: Option<char>) -> bool {
    character.is_some_and(|character| is_letter_or_number(character) || character == '_')
}

impl DelimiterRule {
    fn marker(self) -> &'static str {
        match self {
            Self::Spoiler => "||",
            Self::Strong => "**",
            Self::Underline => "__",
            Self::Strikethrough => "~~",
            Self::EmphasisWithStar => "*",
            Self::EmphasisWithUnderscore => "_",
        }
    }

    fn is_opener_at(self, text: &str, position: usize) -> bool {
        let bytes = text.as_bytes();
        let byte_at = |offset: usize| bytes.get(position + offset).copied();
        let next = character_at(text, position + 1);
        match self {
            Self::Spoiler | Self::Strong | Self::Underline | Self::Strikethrough => text
                .get(position..)
                .is_some_and(|rest| rest.starts_with(self.marker())),
            // "* " is a list or a lone star, and "**" is bold.
            Self::EmphasisWithStar => {
                byte_at(0) == Some(b'*')
                    && byte_at(1) != Some(b'*')
                    && next.is_some()
                    && !is_whitespace_character(next)
            }
            // Underscores inside words (snake_case_names) stay literal.
            Self::EmphasisWithUnderscore => {
                byte_at(0) == Some(b'_')
                    && byte_at(1) != Some(b'_')
                    && next.is_some()
                    && !is_whitespace_character(next)
                    && !is_word_character(character_before(text, position))
            }
        }
    }

    /// Must not depend on where the opener is, so a search's answer can be reused by later openers.
    fn is_closer_at(self, text: &str, position: usize) -> bool {
        let bytes = text.as_bytes();
        let byte_at = |offset: usize| bytes.get(position + offset).copied();
        let marker_byte = self.marker().as_bytes().first().copied();
        if byte_at(0) != marker_byte {
            return false;
        }
        let previous = character_before(text, position);
        match self {
            Self::Spoiler | Self::Strikethrough => byte_at(1) == marker_byte,
            Self::Strong | Self::Underline => {
                byte_at(1) == marker_byte && byte_at(2) != marker_byte
            }
            Self::EmphasisWithStar => {
                previous != Some('*')
                    && byte_at(1) != Some(b'*')
                    && !is_whitespace_character(previous)
            }
            Self::EmphasisWithUnderscore => {
                previous != Some('_')
                    && byte_at(1) != Some(b'_')
                    && !is_whitespace_character(previous)
                    && !is_word_character(character_at(text, position + 1))
            }
        }
    }
}

fn is_escaped(text: &str, position: usize, steps: &mut usize) -> bool {
    let backslashes = text
        .as_bytes()
        .get(..position)
        .unwrap_or_default()
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count();
    *steps += backslashes;
    backslashes % 2 == 1
}

/// The first position at or after `from` where a closer stands, outside code and escapes.
/// Openers are met in increasing order, so the last answer stays right until `from` passes it.
#[derive(Default)]
enum CachedCloserSearch {
    #[default]
    NotSearchedYet,
    NoneLeft,
    At(usize),
}

impl CachedCloserSearch {
    fn find(
        &mut self,
        from: usize,
        text: &str,
        inside_code: &[bool],
        steps: &mut usize,
        is_closer: impl Fn(usize) -> bool,
    ) -> Option<usize> {
        match *self {
            Self::NoneLeft => return None,
            Self::At(answer) if answer >= from => return Some(answer),
            _ => {}
        }
        let mut answer = None;
        for position in from..text.len() {
            *steps += 1;
            if !inside_code.get(position).copied().unwrap_or(false)
                && is_closer(position)
                && !is_escaped(text, position, steps)
            {
                answer = Some(position);
                break;
            }
        }
        *self = answer.map_or(Self::NoneLeft, Self::At);
        answer
    }
}

/// As on Discord: a backslash escapes any symbol, never a letter, digit or space.
fn is_escapable(character: char) -> bool {
    !is_letter_or_number(character) && !is_js_whitespace(character)
}

/// Masked link text must show something: blank, zero-width and filler characters do not count.
fn has_visible_character(text: &str) -> bool {
    text.chars().any(|character| {
        !is_js_whitespace(character)
            && !is_format_character(character)
            && !is_separator(character)
            && !matches!(
                character,
                '\u{2800}' | '\u{3164}' | '\u{115F}' | '\u{1160}' | '\u{FFA0}'
            )
    })
}

/// Masked link text that names an address ("paypal.com", "https://…") could point somewhere
/// else: such text stays literal. Each name run is tried once from its start, so this stays linear.
fn looks_like_an_address(text: &str) -> bool {
    if text.contains("://") {
        return true;
    }
    let is_name_character = |character: char| is_letter_or_number(character) || character == '-';
    let characters: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < characters.len() {
        if !characters
            .get(index)
            .copied()
            .is_some_and(is_name_character)
        {
            index += 1;
            continue;
        }
        let run_end = index
            + characters
                .get(index..)
                .unwrap_or_default()
                .iter()
                .take_while(|character| is_name_character(**character))
                .count();
        let is_letter_at = |offset: usize| {
            characters
                .get(run_end + offset)
                .copied()
                .is_some_and(is_letter)
        };
        if characters.get(run_end) == Some(&'.') && is_letter_at(1) && is_letter_at(2) {
            return true;
        }
        index = run_end;
    }
    false
}

fn push_text_with_mentions(nodes: &mut Vec<InlineNode>, text: &str, context: &ParseContext<'_>) {
    let segments = split_mentions(
        text,
        context.sources.people,
        context.sources.role_mentions.is_everyone_mentioned,
    );
    nodes.extend(segments.into_iter().map(|segment| match segment {
        MentionSegment::Text(text) => InlineNode::Text(text),
        MentionSegment::Mention { text, user } => InlineNode::Mention { text, user },
        MentionSegment::EveryoneMention { text } => InlineNode::EveryoneMention(text),
    }));
}

#[allow(clippy::too_many_lines)] // One scan with its constructs side by side, as the web's parser reads.
fn parse_inline(
    text: &str,
    context: &mut ParseContext<'_>,
    depth: usize,
    are_links_allowed: bool,
) -> Vec<InlineNode> {
    let mut nodes = Vec::new();
    if depth >= MAXIMUM_INLINE_NESTING_DEPTH {
        if !text.is_empty() {
            push_text_with_mentions(&mut nodes, text, context);
        }
        return nodes;
    }

    let code_spans = find_code_spans_counting_steps(text, &mut context.steps);
    let mut inside_code = vec![false; text.len()];
    let mut mark_inside = |start: usize, end: usize| {
        if let Some(range) = inside_code.get_mut(start..end) {
            range.fill(true);
        }
    };
    for (start, span) in &code_spans {
        mark_inside(*start, span.end);
    }
    let markup_spans = find_markup_spans(text, &inside_code, &mut context.steps);
    // A marker inside an emoji's name ("<:ab_:1>") never closes formatting around it.
    for (start, span) in &markup_spans {
        if let Some(range) = inside_code.get_mut(*start..span.end()) {
            range.fill(true);
        }
    }
    let mut closer_searches: [CachedCloserSearch; 6] = Default::default();
    let mut closing_bracket_search = CachedCloserSearch::default();
    let mut closing_parentheses: Option<HashMap<usize, usize>> = None;

    let mut text_start = 0;
    let mut position = 0;
    macro_rules! flush_text {
        ($end:expr) => {
            if $end > text_start {
                push_text_with_mentions(&mut nodes, slice(text, text_start, $end), context);
            }
        };
    }
    while position < text.len() {
        context.steps += 1;
        if let Some(span) = code_spans.get(&position) {
            flush_text!(position);
            nodes.push(InlineNode::Code(span.content.clone()));
            position = span.end;
            text_start = position;
            continue;
        }

        if let Some(span) = markup_spans.get(&position) {
            flush_text!(position);
            let role_mentions = context.sources.role_mentions;
            nodes.push(match span {
                MarkupSpan::Role { id, .. } => {
                    InlineNode::RoleMention(id.and_then(|id| role_mentions.roles.get(&id)).cloned())
                }
                MarkupSpan::Emoji { name, id, .. } => {
                    match id.and_then(|id| context.sources.custom_emojis.get(&id)) {
                        Some(emoji) => InlineNode::CustomEmoji(emoji.clone()),
                        None => InlineNode::Text(format!(":{name}:")),
                    }
                }
            });
            position = span.end();
            text_start = position;
            continue;
        }

        let character = character_at(text, position);
        if character == Some('\\')
            && let Some(escaped) =
                character_at(text, position + 1).filter(|next| is_escapable(*next))
        {
            flush_text!(position);
            // Its own node, so an escaped "\@name" is never read as a mention.
            nodes.push(InlineNode::Text(escaped.to_string()));
            position += 1 + escaped.len_utf8();
            text_start = position;
            continue;
        }

        if are_links_allowed
            && matches!(character, Some('h' | 'H'))
            && let Some((link_text, url)) = autolink_at(text, position, &mut context.steps)
        {
            flush_text!(position);
            position += link_text.len();
            nodes.push(InlineNode::Link {
                url,
                is_masked: false,
                children: vec![InlineNode::Text(link_text)],
            });
            text_start = position;
            continue;
        }

        if are_links_allowed && character == Some('[') {
            let closing_bracket = closing_bracket_search.find(
                position + 1,
                text,
                &inside_code,
                &mut context.steps,
                |candidate| text.as_bytes().get(candidate) == Some(&b']'),
            );
            if let Some(closing_bracket) = closing_bracket.filter(|closing| *closing > position + 1)
                && text.as_bytes().get(closing_bracket + 1) == Some(&b'(')
            {
                let parentheses = closing_parentheses.get_or_insert_with(|| {
                    find_balanced_closing_parentheses(text, &mut context.steps)
                });
                if let Some(closing_parenthesis) = parentheses.get(&(closing_bracket + 1)).copied()
                {
                    let target = slice(text, closing_bracket + 2, closing_parenthesis);
                    let target = target
                        .strip_prefix('<')
                        .and_then(|inner| inner.strip_suffix('>'))
                        .unwrap_or(target);
                    let label = slice(text, position + 1, closing_bracket);
                    if let Some(url) = safe_http_url(target)
                        && has_visible_character(label)
                        && !looks_like_an_address(label)
                    {
                        let spoiler_id_before_label = context.next_spoiler_id;
                        let children = parse_inline(label, context, depth + 1, false);
                        if !inlines_contain_mention(&children, None) {
                            flush_text!(position);
                            nodes.push(InlineNode::Link {
                                url,
                                is_masked: true,
                                children,
                            });
                            position = closing_parenthesis + 1;
                            text_start = position;
                            continue;
                        }
                        context.next_spoiler_id = spoiler_id_before_label;
                    }
                }
            }
        }

        let mut has_matched_delimiter = false;
        for (rule, closer_search) in DELIMITER_RULES.iter().zip(closer_searches.iter_mut()) {
            // A spoiler inside a link would put its reveal button inside the link.
            if !rule.is_opener_at(text, position)
                || (*rule == DelimiterRule::Spoiler && !are_links_allowed)
            {
                continue;
            }
            let content_start = position + rule.marker().len();
            let closer = closer_search.find(
                content_start,
                text,
                &inside_code,
                &mut context.steps,
                |candidate| rule.is_closer_at(text, candidate),
            );
            let Some(closer) = closer.filter(|closer| *closer > content_start) else {
                continue;
            };
            flush_text!(position);
            let children = parse_inline(
                slice(text, content_start, closer),
                context,
                depth + 1,
                are_links_allowed,
            );
            nodes.push(match rule {
                DelimiterRule::Spoiler => {
                    let id = context.next_spoiler_id;
                    context.next_spoiler_id += 1;
                    InlineNode::Spoiler { id, children }
                }
                DelimiterRule::Strong => InlineNode::Strong(children),
                DelimiterRule::Underline => InlineNode::Underline(children),
                DelimiterRule::Strikethrough => InlineNode::Strikethrough(children),
                DelimiterRule::EmphasisWithStar | DelimiterRule::EmphasisWithUnderscore => {
                    InlineNode::Emphasis(children)
                }
            });
            position = closer + rule.marker().len();
            text_start = position;
            has_matched_delimiter = true;
            break;
        }
        if !has_matched_delimiter {
            position += character_length_at(text, position);
        }
    }
    flush_text!(text.len());
    nodes
}

/// For each "(", the ")" that balances it within the same run of non-space characters, so a
/// link target such as `https://en.wikipedia.org/wiki/Mellow_(album)` keeps its own parentheses.
fn find_balanced_closing_parentheses(text: &str, steps: &mut usize) -> HashMap<usize, usize> {
    let mut closing_by_opening = HashMap::new();
    let mut openings = Vec::new();
    for (index, character) in text.char_indices() {
        *steps += 1;
        if character == '(' {
            openings.push(index);
        } else if character == ')' {
            if let Some(opening) = openings.pop() {
                closing_by_opening.insert(opening, index);
            }
        } else if is_js_whitespace(character) {
            openings.clear();
        }
    }
    closing_by_opening
}

/// The http(s) link starting at `position`: its text and normalised URL.
fn autolink_at(text: &str, position: usize, steps: &mut usize) -> Option<(String, String)> {
    let rest = text.get(position..)?;
    if !has_http_prefix(rest) {
        return None;
    }
    let scheme_length = rest.find("://")? + "://".len();
    let body_length = rest
        .get(scheme_length..)?
        .find(|character: char| is_js_whitespace(character) || matches!(character, '<' | '>' | '"'))
        .unwrap_or(rest.len() - scheme_length);
    if body_length == 0 {
        return None;
    }
    let matched = rest.get(..scheme_length + body_length)?;
    let mut unbalanced_closing_parentheses: i64 = 0;
    for character in matched.chars() {
        *steps += 1;
        unbalanced_closing_parentheses += match character {
            ')' => 1,
            '(' => -1,
            _ => 0,
        };
    }
    let bytes = matched.as_bytes();
    let mut end = matched.len();
    while let Some(last) = end
        .checked_sub(1)
        .and_then(|index| bytes.get(index))
        .filter(|byte| LINK_TRAILING_PUNCTUATION.contains(byte))
    {
        *steps += 1;
        // As on GitHub: a closing parenthesis ends the link only when the link did not open it.
        if *last == b')' {
            if unbalanced_closing_parentheses <= 0 {
                break;
            }
            unbalanced_closing_parentheses -= 1;
        }
        end -= 1;
    }
    let candidate = matched.get(..end)?;
    safe_http_url(candidate).map(|url| (candidate.to_owned(), url))
}

#[cfg(test)]
mod tests {
    use mellow_protocol::{Permissions, PublicUser};

    use super::*;
    use crate::shared_logic::mentions::mentionable_people;

    fn paul() -> PublicUser {
        PublicUser {
            id: Snowflake::from_raw(1),
            username: "paul".into(),
            display_name: "Paul Carouge".into(),
            avatar: None,
            banner: None,
        }
    }

    /// Owns what a parse reads, so each test builds its sources in one line.
    struct Sources {
        people: MentionablePeople,
        custom_emojis: HashMap<Snowflake, CustomEmoji>,
        roles: HashMap<Snowflake, Role>,
        is_everyone_mentioned: bool,
    }

    impl Sources {
        fn with_paul() -> Self {
            Self {
                people: mentionable_people(&[paul()]),
                custom_emojis: HashMap::new(),
                roles: HashMap::new(),
                is_everyone_mentioned: false,
            }
        }

        fn borrow(&self) -> MarkdownSources<'_> {
            MarkdownSources {
                people: &self.people,
                custom_emojis: &self.custom_emojis,
                role_mentions: RoleMentionContext {
                    roles: &self.roles,
                    is_everyone_mentioned: self.is_everyone_mentioned,
                },
            }
        }

        fn parse(&self, content: &str) -> Vec<BlockNode> {
            parse_message_markdown(content, self.borrow())
        }

        /// The inline nodes of a message that is one paragraph.
        fn inlines(&self, content: &str) -> Vec<InlineNode> {
            match self.parse(content).as_slice() {
                [BlockNode::Paragraph(children)] => children.clone(),
                other => panic!("expected one paragraph, got {other:?}"),
            }
        }
    }

    fn parse(content: &str) -> Vec<BlockNode> {
        Sources::with_paul().parse(content)
    }

    fn inlines(content: &str) -> Vec<InlineNode> {
        Sources::with_paul().inlines(content)
    }

    fn text(value: &str) -> InlineNode {
        InlineNode::Text(value.to_owned())
    }

    fn paragraph(children: Vec<InlineNode>) -> BlockNode {
        BlockNode::Paragraph(children)
    }

    fn item(children: Vec<InlineNode>, sublists: Vec<MarkdownList>) -> MarkdownListItem {
        MarkdownListItem { children, sublists }
    }

    fn bullets(items: Vec<MarkdownListItem>) -> MarkdownList {
        MarkdownList { start: None, items }
    }

    fn numbered(start: u32, items: Vec<MarkdownListItem>) -> MarkdownList {
        MarkdownList {
            start: Some(start),
            items,
        }
    }

    fn link(url: &str, is_masked: bool, children: Vec<InlineNode>) -> InlineNode {
        InlineNode::Link {
            url: url.to_owned(),
            is_masked,
            children,
        }
    }

    fn is_masked_link(node: &InlineNode) -> bool {
        matches!(
            node,
            InlineNode::Link {
                is_masked: true,
                ..
            }
        )
    }

    #[test]
    fn reads_each_inline_construct() {
        assert_eq!(
            inlines("**bold**"),
            vec![InlineNode::Strong(vec![text("bold")])]
        );
        assert_eq!(
            inlines("*italic*"),
            vec![InlineNode::Emphasis(vec![text("italic")])]
        );
        assert_eq!(
            inlines("_italic_"),
            vec![InlineNode::Emphasis(vec![text("italic")])]
        );
        assert_eq!(
            inlines("__underline__"),
            vec![InlineNode::Underline(vec![text("underline")])]
        );
        assert_eq!(
            inlines("~~strike~~"),
            vec![InlineNode::Strikethrough(vec![text("strike")])]
        );
        assert_eq!(inlines("`code`"), vec![InlineNode::Code("code".into())]);
        assert_eq!(
            inlines("||secret||"),
            vec![InlineNode::Spoiler {
                id: 0,
                children: vec![text("secret")]
            }]
        );
    }

    #[test]
    fn nests_constructs() {
        assert_eq!(
            inlines("***both***"),
            vec![InlineNode::Strong(vec![InlineNode::Emphasis(vec![text(
                "both"
            )])])]
        );
        assert_eq!(
            inlines("*a **b** c*"),
            vec![InlineNode::Emphasis(vec![
                text("a "),
                InlineNode::Strong(vec![text("b")]),
                text(" c")
            ])]
        );
        assert_eq!(
            inlines("||__~~x~~__||"),
            vec![InlineNode::Spoiler {
                id: 0,
                children: vec![InlineNode::Underline(vec![InlineNode::Strikethrough(
                    vec![text("x")]
                )])]
            }]
        );
    }

    #[test]
    fn numbers_spoilers_in_reading_order() {
        assert_eq!(
            inlines("||a|| ||b||"),
            vec![
                InlineNode::Spoiler {
                    id: 0,
                    children: vec![text("a")]
                },
                text(" "),
                InlineNode::Spoiler {
                    id: 1,
                    children: vec![text("b")]
                }
            ]
        );
    }

    #[test]
    fn leaves_unclosed_and_spaced_markers_literal() {
        for literal in [
            "**bold",
            "*italic",
            "a * b * c",
            "~~strike",
            "||spoiler",
            "`code",
            "2 * 3 = 6",
            "****",
        ] {
            assert_eq!(inlines(literal), vec![text(literal)], "{literal}");
        }
        assert_eq!(inlines("snake_case_name"), vec![text("snake_case_name")]);
    }

    #[test]
    fn lets_code_beat_everything() {
        assert_eq!(
            inlines("`**not bold** ||nor hidden||`"),
            vec![InlineNode::Code("**not bold** ||nor hidden||".into())]
        );
        assert_eq!(
            inlines("**a `b** c`"),
            vec![text("**a "), InlineNode::Code("b** c".into())]
        );
        assert_eq!(inlines("``a ` b``"), vec![InlineNode::Code("a ` b".into())]);
    }

    #[test]
    fn reads_backslash_escapes_as_literal_symbols() {
        assert_eq!(
            inlines("\\*not italic\\*"),
            vec![text("*"), text("not italic"), text("*")]
        );
        assert_eq!(
            inlines("\\`not code\\`"),
            vec![text("`"), text("not code"), text("`")]
        );
        assert_eq!(inlines("\\a"), vec![text("\\a")]);
    }

    #[test]
    fn autolinks_http_and_https_leaving_sentence_punctuation_outside() {
        assert_eq!(
            inlines("See https://example.org/a?b=1. Thanks"),
            vec![
                text("See "),
                link(
                    "https://example.org/a?b=1",
                    false,
                    vec![text("https://example.org/a?b=1")]
                ),
                text(". Thanks")
            ]
        );
        assert_eq!(
            inlines("http://example.org/a_b_c"),
            vec![link(
                "http://example.org/a_b_c",
                false,
                vec![text("http://example.org/a_b_c")]
            )]
        );
    }

    #[test]
    fn reads_masked_links_with_formatted_text() {
        assert_eq!(
            inlines("[the **docs**](https://example.org/docs)"),
            vec![link(
                "https://example.org/docs",
                true,
                vec![text("the "), InlineNode::Strong(vec![text("docs")])]
            )]
        );
    }

    #[test]
    fn keeps_balanced_parentheses_inside_links() {
        let wikipedia = "https://en.wikipedia.org/wiki/Mellow_(album)";
        assert_eq!(
            inlines(&format!("[the album]({wikipedia})")),
            vec![link(wikipedia, true, vec![text("the album")])]
        );
        assert_eq!(
            inlines(&format!("see {wikipedia}.")),
            vec![
                text("see "),
                link(wikipedia, false, vec![text(wikipedia)]),
                text(".")
            ]
        );
        assert_eq!(
            inlines("(https://example.org/a)"),
            vec![
                text("("),
                link(
                    "https://example.org/a",
                    false,
                    vec![text("https://example.org/a")]
                ),
                text(")")
            ]
        );
        assert!(
            !inlines("[a](https://example.org/x_(y)")
                .iter()
                .any(is_masked_link)
        );
    }

    #[test]
    fn keeps_spoiler_bars_literal_inside_masked_link_text() {
        assert_eq!(
            inlines("[||x||](https://e.org)"),
            vec![link("https://e.org/", true, vec![text("||x||")])]
        );
    }

    #[test]
    fn keeps_masked_links_literal_when_their_text_could_mislead() {
        for content in [
            "[paypal.com](https://evil.example)",
            "[login.paypal.com/secure](https://evil.example)",
            "[https://paypal.com](https://evil.example)",
            "[see ftp://files](https://evil.example)",
            "[\u{200B}](https://evil.example)",
            "[ ](https://evil.example)",
            "[\u{2800}](https://evil.example)",
            "[@paul](https://evil.example)",
        ] {
            assert!(!inlines(content).iter().any(is_masked_link), "{content}");
        }
        assert_eq!(
            inlines("[e.g. this](https://example.org)"),
            vec![link("https://example.org/", true, vec![text("e.g. this")])]
        );
    }

    #[test]
    fn never_links_a_url_carrying_a_user_name_or_password() {
        assert_eq!(
            inlines("https://paypal.com@evil.example"),
            vec![text("https://paypal.com@evil.example")]
        );
        assert_eq!(
            inlines("[pay](https://user:secret@evil.example)"),
            vec![text("[pay](https://user:secret@evil.example)")]
        );
        assert_eq!(safe_http_url("https://paypal.com@evil.example"), None);
    }

    #[test]
    fn never_links_other_schemes() {
        for literal in [
            "[click](javascript:alert(1))",
            "[click](data:text/html,x)",
            "[click](JaVaScRiPt:alert(1))",
            "[click](//example.org)",
            "javascript:alert(1) data:text/html,x ftp://example.org",
        ] {
            let nodes = inlines(literal);
            assert!(
                !nodes
                    .iter()
                    .any(|node| matches!(node, InlineNode::Link { .. })),
                "{literal}"
            );
            let joined: String = nodes
                .iter()
                .map(|node| {
                    if let InlineNode::Text(value) = node {
                        value.as_str()
                    } else {
                        ""
                    }
                })
                .collect();
            assert_eq!(joined, literal);
        }
        assert_eq!(safe_http_url("javascript:alert(1)"), None);
        assert_eq!(safe_http_url("https:example.org"), None);
        assert_eq!(
            inlines("<img src=x onerror=\"y\">"),
            vec![text("<img src=x onerror=\"y\">")]
        );
    }

    #[test]
    fn finds_mentions_in_text_and_formatting_never_in_code() {
        let paul_id = Snowflake::from_raw(1);
        assert_eq!(
            inlines("hi **@paul**"),
            vec![
                text("hi "),
                InlineNode::Strong(vec![InlineNode::Mention {
                    text: "@paul".into(),
                    user: paul()
                }])
            ]
        );
        assert_eq!(inlines("`@paul`"), vec![InlineNode::Code("@paul".into())]);
        assert!(!markdown_mentions_user(&parse("`@paul`"), paul_id));
        assert!(!markdown_mentions_user(&parse("```\n@paul\n```"), paul_id));
        assert!(markdown_mentions_user(&parse("> - ||@paul||"), paul_id));
        assert!(markdown_mentions_user(&parse("- a\n  1. @paul"), paul_id));
        assert!(markdown_mentions_user(&parse("-# @paul"), paul_id));
        assert!(!markdown_mentions_user(&parse("@zoe"), paul_id));
        assert_eq!(inlines("\\@paul"), vec![text("@"), text("paul")]);
    }

    #[test]
    fn reads_fenced_code_blocks_with_an_optional_language_literally() {
        assert_eq!(
            parse("before\n```ts\nconst a = **1**;\n# no heading\n```\nafter"),
            vec![
                paragraph(vec![text("before")]),
                BlockNode::CodeBlock {
                    language: Some("ts".into()),
                    text: "const a = **1**;\n# no heading".into()
                },
                paragraph(vec![text("after")])
            ]
        );
        assert_eq!(
            parse("```one line```"),
            vec![BlockNode::CodeBlock {
                language: None,
                text: "one line".into()
            }]
        );
        assert_eq!(
            parse("```ts\nno end"),
            vec![paragraph(vec![text("```ts\nno end")])]
        );
    }

    #[test]
    fn reads_headings_at_line_start_only() {
        assert_eq!(
            parse("# One\n## Two\n### Three\n#### Four\n#nospace"),
            vec![
                BlockNode::Heading {
                    level: 1,
                    children: vec![text("One")]
                },
                BlockNode::Heading {
                    level: 2,
                    children: vec![text("Two")]
                },
                BlockNode::Heading {
                    level: 3,
                    children: vec![text("Three")]
                },
                paragraph(vec![text("#### Four\n#nospace")])
            ]
        );
    }

    #[test]
    fn reads_bullet_and_numbered_lists() {
        assert_eq!(
            parse("Shopping:\n- milk\n* **eggs**\nthanks"),
            vec![
                paragraph(vec![text("Shopping:")]),
                BlockNode::List(bullets(vec![
                    item(vec![text("milk")], vec![]),
                    item(vec![InlineNode::Strong(vec![text("eggs")])], vec![])
                ])),
                paragraph(vec![text("thanks")])
            ]
        );
        assert_eq!(
            parse("1. un\n2. deux"),
            vec![BlockNode::List(numbered(
                1,
                vec![
                    item(vec![text("un")], vec![]),
                    item(vec![text("deux")], vec![])
                ]
            ))]
        );
        assert_eq!(
            parse("3) trois\n3) quatre"),
            vec![BlockNode::List(numbered(
                3,
                vec![
                    item(vec![text("trois")], vec![]),
                    item(vec![text("quatre")], vec![])
                ]
            ))]
        );
        assert_eq!(
            parse("0. zéro"),
            vec![BlockNode::List(numbered(
                0,
                vec![item(vec![text("zéro")], vec![])]
            ))]
        );
    }

    #[test]
    fn starts_a_new_list_when_bullets_and_numbers_alternate() {
        assert_eq!(
            parse("- a\n1. b\n- c"),
            vec![
                BlockNode::List(bullets(vec![item(vec![text("a")], vec![])])),
                BlockNode::List(numbered(1, vec![item(vec![text("b")], vec![])])),
                BlockNode::List(bullets(vec![item(vec![text("c")], vec![])]))
            ]
        );
    }

    #[test]
    fn leaves_numbers_without_a_space_past_nine_digits_or_with_nothing_after_them_as_text() {
        assert_eq!(
            parse("1.5 kg\n1234567890. trop\n2. "),
            vec![paragraph(vec![text("1.5 kg\n1234567890. trop\n2. ")])]
        );
    }

    #[test]
    fn nests_an_item_indented_by_two_spaces_or_a_tab_under_the_one_before_once() {
        assert_eq!(
            parse("- a\n  - b\n\t1. c\n      2. d\n- e"),
            vec![BlockNode::List(bullets(vec![
                item(
                    vec![text("a")],
                    vec![
                        bullets(vec![item(vec![text("b")], vec![])]),
                        numbered(
                            1,
                            vec![item(vec![text("c")], vec![]), item(vec![text("d")], vec![])]
                        )
                    ]
                ),
                item(vec![text("e")], vec![])
            ]))]
        );
        assert_eq!(
            parse("  - orphan"),
            vec![paragraph(vec![text("  - orphan")])]
        );
        assert_eq!(
            parse("- a\n - b"),
            vec![
                BlockNode::List(bullets(vec![item(vec![text("a")], vec![])])),
                paragraph(vec![text(" - b")])
            ]
        );
    }

    #[test]
    fn reads_subtext_at_line_start_with_formatting_inside() {
        assert_eq!(
            parse("hello\n-# **petit** mot\n-#nospace\n-# "),
            vec![
                paragraph(vec![text("hello")]),
                BlockNode::Subtext(vec![InlineNode::Strong(vec![text("petit")]), text(" mot")]),
                paragraph(vec![text("-#nospace\n-# ")])
            ]
        );
    }

    #[test]
    fn reads_quotes_with_blocks_inside() {
        assert_eq!(
            parse("> 1. a\n>   - b\n> -# note"),
            vec![BlockNode::Quote(vec![
                BlockNode::List(numbered(
                    1,
                    vec![item(
                        vec![text("a")],
                        vec![bullets(vec![item(vec![text("b")], vec![])])]
                    )]
                )),
                BlockNode::Subtext(vec![text("note")])
            ])]
        );
        assert_eq!(
            parse("> quoted\n> ```\n> code\n> ```\nreply"),
            vec![
                BlockNode::Quote(vec![
                    paragraph(vec![text("quoted")]),
                    BlockNode::CodeBlock {
                        language: None,
                        text: "code".into()
                    }
                ]),
                paragraph(vec![text("reply")])
            ]
        );
        assert_eq!(
            parse("intro\n>>> all\nof\n> this"),
            vec![
                paragraph(vec![text("intro")]),
                BlockNode::Quote(vec![paragraph(vec![text("all\nof\n> this")])])
            ]
        );
    }

    #[test]
    fn keeps_line_breaks_inside_paragraphs_and_returns_nothing_for_an_empty_message() {
        assert_eq!(
            parse("one\ntwo\n\nthree"),
            vec![paragraph(vec![text("one\ntwo\n\nthree")])]
        );
        assert_eq!(parse(""), vec![]);
    }

    /// A hostile text's name and how to build it at a scale.
    type HostileText = (&'static str, Box<dyn Fn(usize) -> String>);

    /// Doubling the text may raise the work per character by at most this factor: linear
    /// parsing keeps it flat, and a quadratic one doubles it.
    const MAXIMUM_GROWTH_OF_WORK_PER_CHARACTER: f64 = 1.25;

    #[test]
    #[allow(clippy::cast_precision_loss)] // Step and character counts stay far below 2^52.
    fn parses_hostile_text_in_linear_work() {
        let cases: Vec<HostileText> = vec![
            ("stars", Box::new(|scale| "*".repeat(4_000 * scale))),
            ("underscores", Box::new(|scale| "_".repeat(4_000 * scale))),
            (
                "alternating stars",
                Box::new(|scale| "*a".repeat(2_000 * scale)),
            ),
            ("spoiler bars", Box::new(|scale| "|".repeat(4_000 * scale))),
            ("tildes", Box::new(|scale| "~".repeat(4_000 * scale))),
            (
                "backticks of growing length",
                Box::new(|scale| {
                    (0..80 * scale)
                        .map(|index| "`".repeat(index + 1))
                        .collect::<Vec<_>>()
                        .join("a")
                }),
            ),
            (
                "brackets",
                Box::new(|scale| "[".repeat(2_000 * scale) + &"](".repeat(1_000 * scale)),
            ),
            (
                "backslashes",
                Box::new(|scale| "\\".repeat(4_000 * scale) + "*"),
            ),
            ("fences", Box::new(|scale| "```".repeat(1_300 * scale))),
            ("quote lines", Box::new(|scale| "> ".repeat(2_000 * scale))),
            (
                "numbered items",
                Box::new(|scale| "1. a\n".repeat(2_000 * scale)),
            ),
            (
                "nested items",
                Box::new(|scale| "- a\n".to_owned() + &"  - b\n\t1. c\n".repeat(1_000 * scale)),
            ),
            (
                "subtext lines",
                Box::new(|scale| "-# a\n".repeat(2_000 * scale)),
            ),
            (
                "links",
                Box::new(|scale| "http://a".repeat(500 * scale) + &".".repeat(2_000 * scale)),
            ),
            (
                "unclosed stars",
                Box::new(|scale| "*a ".repeat(15_000 * scale)),
            ),
            (
                "unclosed underscores",
                Box::new(|scale| "_a ".repeat(15_000 * scale)),
            ),
            ("spoilers", Box::new(|scale| "||a ".repeat(15_000 * scale))),
            (
                "unclosed masked links",
                Box::new(|scale| "[a](".repeat(15_000 * scale)),
            ),
            (
                "blank lines inside a code block",
                Box::new(|scale| "```\na".to_owned() + &"\n".repeat(50_000 * scale) + "b\n```"),
            ),
        ];
        let sources = Sources::with_paul();
        for (name, content_at_scale) in cases {
            let work_per_character = |scale: usize| {
                let content = content_at_scale(scale);
                let (blocks, steps) =
                    parse_message_markdown_counting_steps(&content, sources.borrow());
                assert!(!blocks.is_empty(), "{name}");
                steps as f64 / content.len() as f64
            };
            let single = work_per_character(1);
            let double = work_per_character(2);
            assert!(
                double <= single * MAXIMUM_GROWTH_OF_WORK_PER_CHARACTER,
                "{name}: {single} then {double} steps per character"
            );
        }
    }

    #[test]
    fn caps_nesting_and_shows_the_rest_as_text() {
        let mut nodes = inlines("||[**__~~*_`code`_*~~__**](https://example.org)||");
        let mut depth = 0;
        while let [node] = nodes.as_slice()
            && let Some(children) = node.children()
        {
            nodes = children.to_vec();
            depth += 1;
        }
        assert_eq!(depth, 6);
        assert_eq!(nodes, vec![text("_`code`_")]);
    }

    fn emoji(id: u64, name: &str, is_animated: bool) -> CustomEmoji {
        CustomEmoji {
            id: Snowflake::from_raw(id),
            name: name.to_owned(),
            is_animated,
        }
    }

    fn with_known_emoji() -> Sources {
        let mut sources = Sources::with_paul();
        sources.custom_emojis = [emoji(500, "party", false), emoji(501, "par_rot_", true)]
            .into_iter()
            .map(|emoji| (emoji.id, emoji))
            .collect();
        sources
    }

    #[test]
    fn shows_a_known_custom_emoji_by_its_id_whatever_name_the_markup_gives_it() {
        let sources = with_known_emoji();
        assert_eq!(
            sources.inlines("hi <:fiesta:500>!"),
            vec![
                text("hi "),
                InlineNode::CustomEmoji(emoji(500, "party", false)),
                text("!")
            ]
        );
        assert_eq!(
            sources.inlines("<a:par_rot_:501>"),
            vec![InlineNode::CustomEmoji(emoji(501, "par_rot_", true))]
        );
        assert_eq!(sources.inlines("<:gone:999>"), vec![text(":gone:")]);
        assert_eq!(
            parse("<:party:500>"),
            vec![paragraph(vec![text(":party:")])]
        );
    }

    #[test]
    fn keeps_custom_emoji_literal_in_code_and_when_escaped_and_formatting_around_them_intact() {
        let sources = with_known_emoji();
        assert_eq!(
            sources.inlines("`<:party:500>`"),
            vec![InlineNode::Code("<:party:500>".into())]
        );
        assert_eq!(
            sources.inlines("\\<:party:500>"),
            vec![text("<"), text(":party:500>")]
        );
        assert_eq!(
            sources.inlines("_a <a:par_rot_:501>_"),
            vec![InlineNode::Emphasis(vec![
                text("a "),
                InlineNode::CustomEmoji(emoji(501, "par_rot_", true))
            ])]
        );
        assert_eq!(
            sources.inlines("**<:party:500>**"),
            vec![InlineNode::Strong(vec![InlineNode::CustomEmoji(emoji(
                500, "party", false
            ))])]
        );
        assert_eq!(
            sources.inlines("<:x:500> <:party:abc>"),
            vec![text("<:x:500> <:party:abc>")]
        );
    }

    fn moderators() -> Role {
        Role {
            id: Snowflake::from_raw(201),
            name: "Modérateurs".into(),
            color: 0x0034_98db,
            position: 1,
            permissions: Permissions::empty(),
            is_displayed_separately: false,
            is_mentionable: false,
            icon: None,
            unicode_emoji: None,
        }
    }

    fn with_roles(is_everyone_mentioned: bool) -> Sources {
        let mut sources = Sources::with_paul();
        sources.roles = HashMap::from([(moderators().id, moderators())]);
        sources.is_everyone_mentioned = is_everyone_mentioned;
        sources
    }

    #[test]
    fn shows_a_role_by_its_id_and_an_unknown_one_as_deleted() {
        let sources = with_roles(false);
        assert_eq!(
            sources.inlines("hi <@&201>!"),
            vec![
                text("hi "),
                InlineNode::RoleMention(Some(moderators())),
                text("!")
            ]
        );
        assert_eq!(
            sources.inlines("<@&999>"),
            vec![InlineNode::RoleMention(None)]
        );
        assert_eq!(
            sources.inlines("**<@&201>**"),
            vec![InlineNode::Strong(vec![InlineNode::RoleMention(Some(
                moderators()
            ))])]
        );
        assert_eq!(
            sources.inlines("`<@&201>`"),
            vec![InlineNode::Code("<@&201>".into())]
        );
        assert_eq!(
            sources.inlines("\\<@&201>"),
            vec![text("<"), text("@&201>")]
        );
        assert_eq!(sources.inlines("<@&abc> <@&>"), vec![text("<@&abc> <@&>")]);
        assert!(!matches!(
            sources.inlines("[<@&201>](https://example.com)").first(),
            Some(InlineNode::Link { .. })
        ));
    }

    #[test]
    fn reads_everyone_as_a_mention_only_when_the_server_kept_it() {
        let everyone = |value: &str| InlineNode::EveryoneMention(value.to_owned());
        assert_eq!(
            with_roles(true).inlines("hey @Everyone."),
            vec![text("hey "), everyone("@Everyone"), text(".")]
        );
        assert_eq!(
            with_roles(false).inlines("hey @everyone"),
            vec![text("hey @everyone")]
        );
        assert_eq!(
            with_roles(true).inlines("`@everyone`"),
            vec![InlineNode::Code("@everyone".into())]
        );
        assert_eq!(
            with_roles(true).inlines("||@everyone||"),
            vec![InlineNode::Spoiler {
                id: 0,
                children: vec![everyone("@everyone")]
            }]
        );
        let blocks = with_roles(true).parse("<@&201> @everyone");
        assert!(!markdown_mentions_user(&blocks, Snowflake::from_raw(1)));
    }

    #[test]
    fn reads_inline_markdown_only_in_an_about_me() {
        assert_eq!(
            parse_inline_markdown("# **hi** @paul"),
            vec![paragraph(vec![
                text("# "),
                InlineNode::Strong(vec![text("hi")]),
                text(" @paul")
            ])]
        );
        assert_eq!(parse_inline_markdown(""), vec![]);
    }

    #[test]
    fn writes_custom_emoji_markup_as_names() {
        assert_eq!(
            with_custom_emoji_as_names("look <:party:123> <a:par_rot_:4> <:x:1>"),
            "look :party: :par_rot_: <:x:1>"
        );
    }
}
