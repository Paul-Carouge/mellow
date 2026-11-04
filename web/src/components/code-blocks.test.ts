import { describe, expect, it } from "vitest";

import {
  codeBlockLineCount,
  FOLDED_CODE_BLOCK_VISIBLE_LINES,
  firstCodeBlockLines,
  isCodeBlockFoldable,
  MAXIMUM_CODE_BLOCK_LINES_SHOWN_WHOLE,
} from "./code-blocks";

const linesOf = (count: number) => Array.from({ length: count }, (_, index) => `line ${index + 1}`).join("\n");

describe("codeBlockLineCount", () => {
  it("counts a single line, however long", () => {
    expect(codeBlockLineCount("x".repeat(4000))).toBe(1);
  });

  it("counts every line, blank ones included", () => {
    expect(codeBlockLineCount("a\n\nb")).toBe(3);
    expect(codeBlockLineCount(linesOf(42))).toBe(42);
  });
});

describe("isCodeBlockFoldable", () => {
  it("shows a block whole up to the threshold", () => {
    expect(isCodeBlockFoldable(1)).toBe(false);
    expect(isCodeBlockFoldable(MAXIMUM_CODE_BLOCK_LINES_SHOWN_WHOLE)).toBe(false);
  });

  it("folds a block past the threshold", () => {
    expect(isCodeBlockFoldable(MAXIMUM_CODE_BLOCK_LINES_SHOWN_WHOLE + 1)).toBe(true);
  });

  it("always folds away at least 9 lines", () => {
    // The smallest folded block still hides a real run of lines, not one or two.
    expect(MAXIMUM_CODE_BLOCK_LINES_SHOWN_WHOLE + 1 - FOLDED_CODE_BLOCK_VISIBLE_LINES).toBeGreaterThanOrEqual(9);
  });
});

describe("firstCodeBlockLines", () => {
  it("keeps the first lines without a trailing newline", () => {
    expect(firstCodeBlockLines(linesOf(42), 3)).toBe("line 1\nline 2\nline 3");
  });

  it("returns the whole text when it has no more lines than asked", () => {
    expect(firstCodeBlockLines(linesOf(3), 3)).toBe(linesOf(3));
    expect(firstCodeBlockLines("one line", FOLDED_CODE_BLOCK_VISIBLE_LINES)).toBe("one line");
  });

  it("keeps blank lines as lines", () => {
    expect(firstCodeBlockLines("a\n\n\nb", 2)).toBe("a\n");
  });
});
