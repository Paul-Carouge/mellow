// Discord-style Markdown in message text (DESIGN.md §5.4). Text is parsed into a typed tree that
// `MessageMarkdown.svelte` turns into elements, so a message never reaches the page as HTML.
//
// Every construct is found by scanning forward: a closing marker is searched for once per kind and
// the answer is reused by later openers, so hostile text (thousands of `*`) stays linear in its
// length times the nesting cap, with no backtracking regular expression.

import type { PublicUser, Snowflake } from "../lib/protocol";
import { type MentionablePeople, splitMentions } from "./mentions";

export type InlineNode =
  | { kind: "text"; text: string }
  | { kind: "mention"; text: string; user: PublicUser }
  | { kind: "code"; text: string }
  | { kind: "strong" | "emphasis" | "underline" | "strikethrough"; children: InlineNode[] }
  /** `id` is unique within one message, so the renderer can remember which spoilers were revealed. */
  | { kind: "spoiler"; id: number; children: InlineNode[] }
  /** `isMasked` for `[text](url)`, whose destination is not the visible text. */
  | { kind: "link"; url: string; isMasked: boolean; children: InlineNode[] };

export type BlockNode =
  | { kind: "paragraph"; children: InlineNode[] }
  | { kind: "heading"; level: 1 | 2 | 3; children: InlineNode[] }
  | { kind: "quote"; children: BlockNode[] }
  | { kind: "list"; items: InlineNode[][] }
  | { kind: "codeBlock"; language: string | null; text: string };

/** Deeper formatting is shown as plain text; Discord stops nesting well before people do. */
const MAXIMUM_INLINE_NESTING_DEPTH = 6;
const FENCE = "```";
const QUOTE_LINE_PREFIX = "> ";
const QUOTE_REST_PREFIX = ">>> ";
const HEADING_LINE = /^(#{1,3}) (.*\S.*)$/u;
const LIST_ITEM_LINE = /^[-*] (.*\S.*)$/u;
const FENCE_LANGUAGE = /([a-z0-9_+\-.#]+)\n/iuy;
const AUTOLINK = /https?:\/\/[^\s<>"]+/iuy;
const HTTP_URL_PREFIX = /^https?:\/\//iu;
/** Punctuation that usually ends the sentence around a link rather than the link itself. */
const LINK_TRAILING_PUNCTUATION = ".,;:!?'\")]}";
/** As on Discord: a backslash escapes any symbol, never a letter, digit or space. */
const ESCAPABLE_CHARACTER = /[^\p{L}\p{N}\s]/u;
const WORD_CHARACTER = /[\p{L}\p{N}_]/u;
const WHITESPACE = /\s/u;
/**
 * Masked link text that names an address ("paypal.com", "https://…") could point somewhere else:
 * such text stays literal. Each name run is tried once from its start, so the check stays linear.
 */
const LINK_TEXT_LOOKING_LIKE_AN_ADDRESS = /:\/\/|(?<![\p{L}\p{N}-])[\p{L}\p{N}-]+\.\p{L}{2,}/u;
/** Masked link text must show something: blank, zero-width and filler characters do not count. */
const VISIBLE_CHARACTER = /[^\s\p{Cf}\p{Z}\u2800\u3164\u115F\u1160\uFFA0]/u;

interface ParseContext {
  people: MentionablePeople;
  nextSpoilerId: number;
}

/** Parses a message body into blocks; `people` are those an `@name` can point at. */
export function parseMessageMarkdown(content: string, people: MentionablePeople): BlockNode[] {
  return parseBlocks(content, { people, nextSpoilerId: 0 }, false);
}

/** Whether the parsed message mentions `userId` outside code. */
export function markdownMentionsUser(blocks: readonly BlockNode[], userId: Snowflake): boolean {
  const inlinesMention = (nodes: readonly InlineNode[]) => inlinesContainMention(nodes, userId);
  return blocks.some((block) => {
    switch (block.kind) {
      case "paragraph":
      case "heading":
        return inlinesMention(block.children);
      case "quote":
        return markdownMentionsUser(block.children, userId);
      case "list":
        return block.items.some(inlinesMention);
      case "codeBlock":
        return false;
    }
  });
}

/** Whether `nodes` mention `userId`, or anyone when it is null. */
function inlinesContainMention(nodes: readonly InlineNode[], userId: Snowflake | null): boolean {
  return nodes.some((node) =>
    node.kind === "mention"
      ? userId === null || node.user.id === userId
      : "children" in node && inlinesContainMention(node.children, userId),
  );
}

/** The http(s) URL `candidate` names, normalised; null for any other scheme, a user name or password, or no URL. */
export function safeHttpUrl(candidate: string): string | null {
  if (!HTTP_URL_PREFIX.test(candidate)) {
    return null;
  }
  try {
    const url = new URL(candidate);
    // "https://paypal.com@evil.example" reads as paypal.com but leads to evil.example.
    const hasUserInfo = url.username !== "" || url.password !== "";
    return (url.protocol === "http:" || url.protocol === "https:") && !hasUserInfo ? url.href : null;
  } catch {
    // Not a URL after all: it stays plain text.
    return null;
  }
}

function findEndOfLine(source: string, from: number): number {
  const newline = source.indexOf("\n", from);
  return newline === -1 ? source.length : newline;
}

function parseBlocks(source: string, context: ParseContext, isInsideQuote: boolean): BlockNode[] {
  const blocks: BlockNode[] = [];
  let paragraphStart = 0;
  let position = 0;
  // Index of the next "```" at or after `position`, or -1 once none is left; searched lazily.
  let nextFenceIndex = source.indexOf(FENCE);
  let canAFenceStillClose = true;
  // End of the current line, kept between fence candidates on one line so it is searched once.
  let lineEndIndex = -1;

  const flushParagraph = (end: number) => {
    const text = source.slice(paragraphStart, end).replace(/\n$/u, "");
    if (text !== "") {
      blocks.push({ kind: "paragraph", children: parseInline(text, context, 0, true) });
    }
  };

  while (position < source.length) {
    // A quote or list may have swallowed the fence found earlier.
    if (nextFenceIndex !== -1 && nextFenceIndex < position) {
      nextFenceIndex = source.indexOf(FENCE, position);
    }
    if (position === 0 || source[position - 1] === "\n") {
      const lineEnd = findEndOfLine(source, position);
      const line = source.slice(position, lineEnd);

      if (!isInsideQuote && line.startsWith(QUOTE_REST_PREFIX) && source.slice(position + QUOTE_REST_PREFIX.length).trim() !== "") {
        flushParagraph(position);
        blocks.push({
          kind: "quote",
          children: parseBlocks(source.slice(position + QUOTE_REST_PREFIX.length), context, true),
        });
        return blocks;
      }

      if (!isInsideQuote && line.startsWith(QUOTE_LINE_PREFIX)) {
        flushParagraph(position);
        const quotedLines: string[] = [];
        let next = position;
        while (next < source.length && source.startsWith(QUOTE_LINE_PREFIX, next)) {
          const quotedLineEnd = findEndOfLine(source, next);
          quotedLines.push(source.slice(next + QUOTE_LINE_PREFIX.length, quotedLineEnd));
          next = quotedLineEnd + 1;
        }
        blocks.push({ kind: "quote", children: parseBlocks(quotedLines.join("\n"), context, true) });
        position = paragraphStart = next;
        continue;
      }

      const heading = HEADING_LINE.exec(line);
      if (heading?.[1] && heading[2] !== undefined) {
        flushParagraph(position);
        blocks.push({
          kind: "heading",
          level: heading[1].length as 1 | 2 | 3,
          children: parseInline(heading[2], context, 0, true),
        });
        position = paragraphStart = lineEnd + 1;
        continue;
      }

      if (LIST_ITEM_LINE.test(line)) {
        flushParagraph(position);
        const items: InlineNode[][] = [];
        let next = position;
        while (next < source.length) {
          const itemLineEnd = findEndOfLine(source, next);
          const item = LIST_ITEM_LINE.exec(source.slice(next, itemLineEnd));
          if (item?.[1] === undefined) {
            break;
          }
          items.push(parseInline(item[1], context, 0, true));
          next = itemLineEnd + 1;
        }
        blocks.push({ kind: "list", items });
        position = paragraphStart = next;
        continue;
      }
    }

    if (canAFenceStillClose && position === nextFenceIndex) {
      const fence = parseFence(source, position);
      if (fence === "unclosed") {
        canAFenceStillClose = false;
      } else if (fence) {
        flushParagraph(position);
        blocks.push(fence.block);
        position = paragraphStart = source[fence.end] === "\n" ? fence.end + 1 : fence.end;
        nextFenceIndex = source.indexOf(FENCE, position);
        continue;
      }
      nextFenceIndex = source.indexOf(FENCE, position + 1);
    }

    if (lineEndIndex < position) {
      lineEndIndex = findEndOfLine(source, position);
    }
    const nextLineStart = lineEndIndex + 1;
    position =
      canAFenceStillClose && nextFenceIndex !== -1 && nextFenceIndex < nextLineStart ? nextFenceIndex : nextLineStart;
  }
  flushParagraph(source.length);
  return blocks;
}

/** A fenced code block starting at `start`; "unclosed" when no "```" follows, so none can close later. */
function parseFence(source: string, start: number): { block: BlockNode; end: number } | "unclosed" | null {
  let contentStart = start + FENCE.length;
  let language: string | null = null;
  FENCE_LANGUAGE.lastIndex = contentStart;
  const languageMatch = FENCE_LANGUAGE.exec(source);
  if (languageMatch?.[1]) {
    language = languageMatch[1];
    contentStart += languageMatch[0].length;
  }
  const close = source.indexOf(FENCE, contentStart);
  if (close === -1) {
    return "unclosed";
  }
  // Index loops rather than /\n+$/, which retries at every newline of a long blank run.
  let textStart = contentStart;
  let textEnd = close;
  while (textStart < textEnd && source[textStart] === "\n") {
    textStart += 1;
  }
  while (textEnd > textStart && source[textEnd - 1] === "\n") {
    textEnd -= 1;
  }
  const text = source.slice(textStart, textEnd);
  if (text.trim() === "") {
    return null;
  }
  return { block: { kind: "codeBlock", language, text }, end: close + FENCE.length };
}

/** Inline code spans by start index, found as Markdown reads them: a run of backticks up to the next run of the same length. */
function findCodeSpans(text: string): Map<number, { end: number; content: string }> {
  const spans = new Map<number, { end: number; content: string }>();
  const unclosedRunLengths = new Set<number>();
  const countBacktickRunLength = (from: number) => {
    let end = from;
    while (text[end] === "`") {
      end += 1;
    }
    return end - from;
  };
  let index = 0;
  while (index < text.length) {
    if (text[index] === "\\") {
      index += 2;
      continue;
    }
    if (text[index] !== "`") {
      index += 1;
      continue;
    }
    const opening = countBacktickRunLength(index);
    let search = index + opening;
    let isCodeSpanClosed = false;
    while (!unclosedRunLengths.has(opening)) {
      const candidate = text.indexOf("`", search);
      if (candidate === -1) {
        break;
      }
      const closing = countBacktickRunLength(candidate);
      if (closing === opening) {
        const content = text.slice(index + opening, candidate);
        spans.set(index, { end: candidate + closing, content: content.trim() || content });
        index = candidate + closing;
        isCodeSpanClosed = true;
        break;
      }
      search = candidate + closing;
    }
    if (!isCodeSpanClosed) {
      unclosedRunLengths.add(opening);
      index += opening;
    }
  }
  return spans;
}

type DelimitedKind = "spoiler" | "strong" | "underline" | "strikethrough" | "emphasis";

interface DelimiterRule {
  kind: DelimitedKind;
  marker: string;
  isOpenerAt(text: string, position: number): boolean;
  /** Must not depend on where the opener is, so a search's answer can be reused by later openers. */
  isCloserAt(text: string, position: number): boolean;
}

const isWhitespace = (character: string | undefined) => character !== undefined && WHITESPACE.test(character);
const isWordCharacter = (character: string | undefined) => character !== undefined && WORD_CHARACTER.test(character);

/** Tried in this order at each position, as Discord does. */
const DELIMITER_RULES: readonly DelimiterRule[] = [
  {
    kind: "spoiler",
    marker: "||",
    isOpenerAt: (text, position) => text.startsWith("||", position),
    isCloserAt: (text, position) => text.startsWith("||", position),
  },
  {
    kind: "strong",
    marker: "**",
    isOpenerAt: (text, position) => text.startsWith("**", position),
    isCloserAt: (text, position) => text.startsWith("**", position) && text[position + 2] !== "*",
  },
  {
    kind: "underline",
    marker: "__",
    isOpenerAt: (text, position) => text.startsWith("__", position),
    isCloserAt: (text, position) => text.startsWith("__", position) && text[position + 2] !== "_",
  },
  {
    kind: "strikethrough",
    marker: "~~",
    isOpenerAt: (text, position) => text.startsWith("~~", position),
    isCloserAt: (text, position) => text.startsWith("~~", position),
  },
  {
    kind: "emphasis",
    marker: "*",
    // "* " is a list or a lone star, and "**" is bold.
    isOpenerAt: (text, position) =>
      text[position] === "*" && text[position + 1] !== "*" && text[position + 1] !== undefined && !isWhitespace(text[position + 1]),
    isCloserAt: (text, position) =>
      text[position] === "*" && text[position - 1] !== "*" && text[position + 1] !== "*" && !isWhitespace(text[position - 1]),
  },
  {
    kind: "emphasis",
    marker: "_",
    // Underscores inside words (snake_case_names) stay literal.
    isOpenerAt: (text, position) =>
      text[position] === "_" &&
      text[position + 1] !== "_" &&
      text[position + 1] !== undefined &&
      !isWhitespace(text[position + 1]) &&
      !isWordCharacter(text[position - 1]),
    isCloserAt: (text, position) =>
      text[position] === "_" &&
      text[position - 1] !== "_" &&
      text[position + 1] !== "_" &&
      !isWhitespace(text[position - 1]) &&
      !isWordCharacter(text[position + 1]),
  },
];

function isEscaped(text: string, position: number): boolean {
  let backslashes = 0;
  for (let index = position - 1; index >= 0 && text[index] === "\\"; index -= 1) {
    backslashes += 1;
  }
  return backslashes % 2 === 1;
}

/**
 * The first position at or after `from` where `isCloser` holds, outside code and escapes; -1 if none.
 * Openers are met in increasing order, so the last answer stays right until `from` passes it.
 */
function createCachedCloserSearch(text: string, insideCode: Uint8Array, isCloser: (position: number) => boolean) {
  let answer = -2;
  return (from: number): number => {
    if (answer === -1 || answer >= from) {
      return answer;
    }
    answer = -1;
    for (let position = from; position < text.length; position += 1) {
      if (!insideCode[position] && isCloser(position) && !isEscaped(text, position)) {
        answer = position;
        break;
      }
    }
    return answer;
  };
}

function parseInline(text: string, context: ParseContext, depth: number, areLinksAllowed: boolean): InlineNode[] {
  const nodes: InlineNode[] = [];
  let textStart = 0;
  const flushText = (end: number) => {
    if (end > textStart) {
      nodes.push(...splitMentions(text.slice(textStart, end), context.people));
    }
  };
  if (depth >= MAXIMUM_INLINE_NESTING_DEPTH) {
    flushText(text.length);
    return nodes;
  }

  const codeSpans = findCodeSpans(text);
  const insideCode = new Uint8Array(text.length);
  for (const [start, span] of codeSpans) {
    insideCode.fill(1, start, span.end);
  }
  const findClosers = DELIMITER_RULES.map((rule) => createCachedCloserSearch(text, insideCode, (position) => rule.isCloserAt(text, position)));
  const findClosingBracket = createCachedCloserSearch(text, insideCode, (position) => text[position] === "]");
  let closingParentheses: Map<number, number> | null = null;

  let position = 0;
  while (position < text.length) {
    const character = text[position];
    const codeSpan = codeSpans.get(position);
    if (codeSpan) {
      flushText(position);
      nodes.push({ kind: "code", text: codeSpan.content });
      position = textStart = codeSpan.end;
      continue;
    }

    if (character === "\\" && ESCAPABLE_CHARACTER.test(text[position + 1] ?? "")) {
      flushText(position);
      // Its own node, so an escaped "\@name" is never read as a mention.
      nodes.push({ kind: "text", text: text[position + 1] ?? "" });
      position = textStart = position + 2;
      continue;
    }

    if (areLinksAllowed && (character === "h" || character === "H")) {
      const link = autolinkAt(text, position);
      if (link) {
        flushText(position);
        nodes.push({ kind: "link", url: link.url, isMasked: false, children: [{ kind: "text", text: link.text }] });
        position = textStart = position + link.text.length;
        continue;
      }
    }

    if (areLinksAllowed && character === "[") {
      const closingBracket = findClosingBracket(position + 1);
      if (closingBracket > position + 1 && text[closingBracket + 1] === "(") {
        closingParentheses ??= findBalancedClosingParentheses(text);
        const closingParenthesis = closingParentheses.get(closingBracket + 1) ?? -1;
        const target = closingParenthesis === -1 ? "" : text.slice(closingBracket + 2, closingParenthesis);
        const url = safeHttpUrl(target.replace(/^<(.*)>$/u, "$1"));
        const label = text.slice(position + 1, closingBracket);
        if (url && VISIBLE_CHARACTER.test(label) && !LINK_TEXT_LOOKING_LIKE_AN_ADDRESS.test(label)) {
          const spoilerIdBeforeLabel = context.nextSpoilerId;
          const children = parseInline(label, context, depth + 1, false);
          if (!inlinesContainMention(children, null)) {
            flushText(position);
            nodes.push({ kind: "link", url, isMasked: true, children });
            position = textStart = closingParenthesis + 1;
            continue;
          }
          context.nextSpoilerId = spoilerIdBeforeLabel;
        }
      }
    }

    let hasMatchedDelimiter = false;
    for (const [ruleIndex, rule] of DELIMITER_RULES.entries()) {
      // A spoiler inside a link would put its reveal button inside the link.
      if (!rule.isOpenerAt(text, position) || (rule.kind === "spoiler" && !areLinksAllowed)) {
        continue;
      }
      const contentStart = position + rule.marker.length;
      const closer = findClosers[ruleIndex]?.(contentStart) ?? -1;
      if (closer <= contentStart) {
        continue;
      }
      flushText(position);
      const children = parseInline(text.slice(contentStart, closer), context, depth + 1, areLinksAllowed);
      nodes.push(rule.kind === "spoiler" ? { kind: "spoiler", id: context.nextSpoilerId++, children } : { kind: rule.kind, children });
      position = textStart = closer + rule.marker.length;
      hasMatchedDelimiter = true;
      break;
    }
    if (!hasMatchedDelimiter) {
      position += 1;
    }
  }
  flushText(text.length);
  return nodes;
}

/**
 * For each "(", the ")" that balances it within the same run of non-space characters, so a link
 * target such as "https://en.wikipedia.org/wiki/Mellow_(album)" keeps its own parentheses.
 */
function findBalancedClosingParentheses(text: string): Map<number, number> {
  const closingByOpening = new Map<number, number>();
  const openings: number[] = [];
  for (let index = 0; index < text.length; index += 1) {
    const character = text[index];
    if (character === "(") {
      openings.push(index);
    } else if (character === ")") {
      const opening = openings.pop();
      if (opening !== undefined) {
        closingByOpening.set(opening, index);
      }
    } else if (isWhitespace(character)) {
      openings.length = 0;
    }
  }
  return closingByOpening;
}

function autolinkAt(text: string, position: number): { text: string; url: string } | null {
  AUTOLINK.lastIndex = position;
  const match = AUTOLINK.exec(text);
  if (!match) {
    return null;
  }
  let end = match[0].length;
  let unbalancedClosingParentheses = 0;
  for (const character of match[0]) {
    unbalancedClosingParentheses += character === ")" ? 1 : character === "(" ? -1 : 0;
  }
  while (end > 0 && LINK_TRAILING_PUNCTUATION.includes(match[0][end - 1] ?? "")) {
    // As on GitHub: a closing parenthesis ends the link only when the link did not open it.
    if (match[0][end - 1] === ")") {
      if (unbalancedClosingParentheses <= 0) {
        break;
      }
      unbalancedClosingParentheses -= 1;
    }
    end -= 1;
  }
  const candidate = match[0].slice(0, end);
  const url = safeHttpUrl(candidate);
  return url ? { text: candidate, url } : null;
}
