// Highlighted words in search results (ADR-0029): the instance sends spans of `Message.content`
// in UTF-16 units and never markup, so the client cuts the parsed message's text with them.

import type { HighlightRange } from "../lib/protocol";
import type { BlockNode, InlineNode } from "./markdown";

export interface TextSegment {
  text: string;
  isMatch: boolean;
}

function isHighSurrogate(code: number): boolean {
  return code >= 0xd800 && code <= 0xdbff;
}

function isLowSurrogate(code: number): boolean {
  return code >= 0xdc00 && code <= 0xdfff;
}

/**
 * The ranges, clamped to `content`, sorted, merged when they touch, and widened so none cuts an
 * emoji or any other character written as two UTF-16 units in half.
 */
export function normalizedHighlightRanges(content: string, ranges: readonly HighlightRange[]): HighlightRange[] {
  const widened = ranges
    .map(({ start, end }) => {
      let from = Math.max(0, Math.min(start, content.length));
      let to = Math.max(0, Math.min(end, content.length));
      if (from > 0 && isLowSurrogate(content.charCodeAt(from)) && isHighSurrogate(content.charCodeAt(from - 1))) {
        from -= 1;
      }
      if (to > 0 && to < content.length && isHighSurrogate(content.charCodeAt(to - 1)) && isLowSurrogate(content.charCodeAt(to))) {
        to += 1;
      }
      return { start: from, end: to };
    })
    .filter((range) => range.start < range.end)
    .sort((left, right) => left.start - right.start);
  const merged: HighlightRange[] = [];
  for (const range of widened) {
    const last = merged.at(-1);
    if (last && range.start <= last.end) {
      last.end = Math.max(last.end, range.end);
    } else {
      merged.push({ ...range });
    }
  }
  return merged;
}

/** `text`, found at `offset` in the content, cut into matched and unmatched runs. */
export function splitByHighlights(text: string, offset: number, ranges: readonly HighlightRange[]): TextSegment[] {
  const segments: TextSegment[] = [];
  let position = 0;
  for (const range of ranges) {
    const start = Math.max(range.start - offset, 0);
    const end = Math.min(range.end - offset, text.length);
    if (end <= start) {
      continue;
    }
    if (start > position) {
      segments.push({ text: text.slice(position, start), isMatch: false });
    }
    segments.push({ text: text.slice(start, end), isMatch: true });
    position = end;
  }
  if (position < text.length || segments.length === 0) {
    segments.push({ text: text.slice(position), isMatch: false });
  }
  return segments;
}

/**
 * The parsed message with its matched words as `match` nodes. Each text is found in `content`
 * from where the previous one ended, so a text the parser changed (an escape it removed) simply
 * stays unmarked rather than marking the wrong place.
 */
export function withSearchHighlights(blocks: readonly BlockNode[], content: string, ranges: readonly HighlightRange[]): BlockNode[] {
  const normalized = normalizedHighlightRanges(content, ranges);
  if (normalized.length === 0) {
    return [...blocks];
  }
  let cursor = 0;

  /** Where `text` sits from the cursor on, moving the cursor past it; -1 when not found. */
  function locate(text: string): number {
    const found = text === "" ? -1 : content.indexOf(text, cursor);
    if (found !== -1) {
      cursor = found + text.length;
    }
    return found;
  }

  function markInlines(nodes: readonly InlineNode[]): InlineNode[] {
    return nodes.flatMap((node): InlineNode[] => {
      switch (node.kind) {
        case "text":
          // Line by line: a quote's text has lost the "> " starting each line of the content.
          return node.text.split("\n").flatMap((line, index): InlineNode[] => {
            const lineBreak: InlineNode[] = index > 0 ? [{ kind: "text", text: "\n" }] : [];
            const offset = locate(line);
            if (offset === -1) {
              return line === "" ? lineBreak : [...lineBreak, { kind: "text", text: line }];
            }
            return [
              ...lineBreak,
              ...splitByHighlights(line, offset, normalized).map((segment): InlineNode =>
                segment.isMatch ? { kind: "match", text: segment.text } : { kind: "text", text: segment.text },
              ),
            ];
          });
        // shortcut: a mention chip stays whole and unlit even when its name matched; mark the chip if people miss it.
        case "mention":
        case "code":
          locate(node.text);
          return [node];
        case "customEmoji":
        case "match":
          return [node];
        case "strong":
        case "emphasis":
        case "underline":
        case "strikethrough":
        case "spoiler":
        case "link":
          return [{ ...node, children: markInlines(node.children) }];
      }
    });
  }

  function markBlocks(nodes: readonly BlockNode[]): BlockNode[] {
    return nodes.map((block): BlockNode => {
      switch (block.kind) {
        case "paragraph":
        case "heading":
          return { ...block, children: markInlines(block.children) };
        case "quote":
          return { ...block, children: markBlocks(block.children) };
        case "list":
          return { ...block, items: block.items.map(markInlines) };
        case "codeBlock":
          // shortcut: matches inside code blocks are not marked; split the block's text if people miss it.
          locate(block.text);
          return block;
      }
    });
  }

  return markBlocks(blocks);
}
