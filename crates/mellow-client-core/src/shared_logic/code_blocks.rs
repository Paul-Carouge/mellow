//! Long code blocks in messages fold to their first lines (`web/src/components/code-blocks.ts`,
//! DESIGN.md §5.4).

/// A block up to this many lines shows whole: at the code type's 18 px line it stays under
/// 380 px, half of a laptop's message list, so it never pushes the conversation out of view.
pub const MAXIMUM_CODE_BLOCK_LINES_SHOWN_WHOLE: usize = 20;

/// A folded block shows this many lines: enough to recognise the snippet, and a fold always
/// hides at least 9 lines, never a lone one.
pub const FOLDED_CODE_BLOCK_VISIBLE_LINES: usize = 12;

/// Lines of a code block's text, as `\n` separates them (the parser trims leading and trailing newlines).
#[must_use]
pub fn code_block_line_count(text: &str) -> usize {
    text.bytes().filter(|byte| *byte == b'\n').count() + 1
}

/// Whether a block of `line_count` lines folds rather than showing whole.
#[must_use]
pub fn is_code_block_foldable(line_count: usize) -> bool {
    line_count > MAXIMUM_CODE_BLOCK_LINES_SHOWN_WHOLE
}

/// The first `line_count` lines of `text`, without the newline that ends the last of them.
#[must_use]
pub fn first_code_block_lines(text: &str, line_count: usize) -> &str {
    match text.match_indices('\n').nth(line_count.saturating_sub(1)) {
        Some((end, _)) if line_count > 0 => text.get(..end).unwrap_or(text),
        Some(_) => "",
        None if line_count == 0 => "",
        None => text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines_of(count: usize) -> String {
        (1..=count)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn counts_every_line_blank_ones_included() {
        assert_eq!(code_block_line_count(&"x".repeat(4_000)), 1);
        assert_eq!(code_block_line_count("a\n\nb"), 3);
        assert_eq!(code_block_line_count(&lines_of(42)), 42);
    }

    #[test]
    fn folds_past_the_threshold_and_always_hides_at_least_nine_lines() {
        assert!(!is_code_block_foldable(1));
        assert!(!is_code_block_foldable(
            MAXIMUM_CODE_BLOCK_LINES_SHOWN_WHOLE
        ));
        assert!(is_code_block_foldable(
            MAXIMUM_CODE_BLOCK_LINES_SHOWN_WHOLE + 1
        ));
        const {
            assert!(
                MAXIMUM_CODE_BLOCK_LINES_SHOWN_WHOLE + 1 - FOLDED_CODE_BLOCK_VISIBLE_LINES >= 9
            );
        };
    }

    #[test]
    fn keeps_the_first_lines_without_a_trailing_newline() {
        assert_eq!(
            first_code_block_lines(&lines_of(42), 3),
            "line 1\nline 2\nline 3"
        );
        assert_eq!(first_code_block_lines(&lines_of(3), 3), lines_of(3));
        assert_eq!(
            first_code_block_lines("one line", FOLDED_CODE_BLOCK_VISIBLE_LINES),
            "one line"
        );
        assert_eq!(first_code_block_lines("a\n\n\nb", 2), "a\n");
    }
}
