// Long code blocks in messages fold to their first lines (DESIGN.md §5.4, v0.48).

/**
 * A block up to this many lines shows whole: at the code type's 18 px line it stays under 380 px,
 * half of a laptop's message list, so it never pushes the conversation out of view.
 */
export const MAXIMUM_CODE_BLOCK_LINES_SHOWN_WHOLE = 20;

/**
 * A folded block shows this many lines: enough to recognise the snippet (a function's signature
 * and first statements), and a fold always hides at least 9 lines, never a lone one.
 */
export const FOLDED_CODE_BLOCK_VISIBLE_LINES = 12;

/** Lines of a code block's text, as `\n` separates them (the parser trims leading and trailing newlines). */
export function codeBlockLineCount(text: string): number {
  let lines = 1;
  for (let index = text.indexOf("\n"); index !== -1; index = text.indexOf("\n", index + 1)) {
    lines += 1;
  }
  return lines;
}

/** Whether a block of `lineCount` lines folds rather than showing whole. */
export function isCodeBlockFoldable(lineCount: number): boolean {
  return lineCount > MAXIMUM_CODE_BLOCK_LINES_SHOWN_WHOLE;
}

/** The first `lineCount` lines of `text`, without the newline that ends the last of them. */
export function firstCodeBlockLines(text: string, lineCount: number): string {
  let end = -1;
  for (let line = 0; line < lineCount; line += 1) {
    end = text.indexOf("\n", end + 1);
    if (end === -1) {
      return text;
    }
  }
  return text.slice(0, end);
}
