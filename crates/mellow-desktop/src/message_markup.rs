use slint::StyledText;

/// Renders a message body written in Mellow's markdown subset.
///
/// Raw HTML is escaped first: Slint's markdown accepts `<font color>` and `<u>`, which would let
/// a sender restyle text to impersonate system messages. Backslashes are escaped too: otherwise a
/// sender's `\<` would become an escaped backslash followed by a live tag. Constructs Slint cannot
/// render (code blocks, block quotes, headings) make parsing fail, and the body is then shown as
/// plain text.
pub fn styled_text_from_message_body(body: &str) -> StyledText {
    let body_without_html = body.replace('\\', "\\\\").replace('<', "\\<");
    StyledText::from_markdown(&body_without_html)
        .unwrap_or_else(|_| StyledText::from_plain_text(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_formatting_is_rendered_as_markdown() {
        assert_ne!(
            styled_text_from_message_body("**bold** and `code`"),
            StyledText::from_plain_text("**bold** and `code`")
        );
    }

    #[test]
    fn unsupported_blocks_fall_back_to_the_plain_body() {
        let body = "```\nfn main() {}\n```";
        assert_eq!(
            styled_text_from_message_body(body),
            StyledText::from_plain_text(body)
        );
    }

    #[test]
    fn html_styling_tags_are_shown_literally_instead_of_applied() {
        let body = "<font color=\"red\">SYSTEM</font> <u>notice</u>";
        assert_eq!(
            styled_text_from_message_body(body),
            StyledText::from_plain_text(body)
        );
    }

    #[test]
    fn an_escaped_backslash_cannot_turn_html_tags_back_on() {
        for body in [r#"\<font color="red">SYSTEM\</font>"#, r"\\<u>notice</u>"] {
            assert_eq!(
                styled_text_from_message_body(body),
                StyledText::from_plain_text(body),
                "{body}"
            );
        }
    }
}
