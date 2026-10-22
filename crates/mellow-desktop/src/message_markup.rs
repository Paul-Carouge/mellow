use slint::StyledText;

use crate::mentions::Mention;

/// Renders a message body written in Mellow's markdown subset.
///
/// Raw HTML is escaped first: Slint's markdown accepts `<font color>` and `<u>`, which would let
/// a sender restyle text to impersonate system messages. Backslashes are escaped too: otherwise a
/// sender's `\<` would become an escaped backslash followed by a live tag. Constructs Slint cannot
/// render (code blocks, block quotes, headings) make parsing fail, and the body is then shown as
/// plain text.
pub fn styled_text_from_message_body(body: &str) -> StyledText {
    StyledText::from_markdown(&escape_html(body))
        .unwrap_or_else(|_| StyledText::from_plain_text(body))
}

/// Escapes what Slint's markdown would otherwise read as HTML (see above).
fn escape_html(text: &str) -> String {
    text.replace('\\', "\\\\").replace('<', "\\<")
}

/// The body with each mention coloured as a chip's text (`color` is `#RRGGBB`, from a token).
/// shortcut: `StyledText` has no background colour, so the chip's fill cannot be drawn on desktop.
pub fn styled_text_with_mentions(body: &str, mentions: &[Mention], color: &str) -> StyledText {
    if mentions.is_empty() || color.is_empty() {
        return styled_text_from_message_body(body);
    }
    let mut markup = String::with_capacity(body.len());
    let mut text_start = 0;
    for mention in mentions {
        let (Some(before), Some(name)) = (
            body.get(text_start..mention.range.start),
            body.get(mention.range.clone()),
        ) else {
            return styled_text_from_message_body(body);
        };
        markup.push_str(&escape_html(before));
        markup.push_str("<font color=\"");
        markup.push_str(color);
        markup.push_str("\">");
        markup.push_str(&escape_html(name));
        markup.push_str("</font>");
        text_start = mention.range.end;
    }
    markup.push_str(&escape_html(body.get(text_start..).unwrap_or_default()));
    StyledText::from_markdown(&markup).unwrap_or_else(|_| StyledText::from_plain_text(body))
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
    fn mentions_are_coloured_and_the_rest_stays_escaped() {
        let body = "@ana <u>notice</u>";
        let mention = Mention {
            range: 0..4,
            user_id: mellow_protocol::Snowflake::from_raw(1),
        };
        let coloured = styled_text_with_mentions(body, std::slice::from_ref(&mention), "#FFE4D1");
        assert_ne!(coloured, styled_text_from_message_body(body));
        let only_the_rest = styled_text_with_mentions(body, &[], "#FFE4D1");
        assert_eq!(only_the_rest, StyledText::from_plain_text(body));
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
