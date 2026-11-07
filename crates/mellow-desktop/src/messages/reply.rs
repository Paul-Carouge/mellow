//! The quoted line above a reply (`reply-references.ts`, DESIGN.md §5.4): the original's author
//! and text, with the spine running from the reply's avatar up to it.

use std::collections::HashMap;
use std::hash::BuildHasher;

use gpui::{
    App, ImageSource, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _,
    Window, div,
};
use mellow_client_core::shared_logic::format::display_name;
use mellow_protocol::{Message, PublicUser, Role, Snowflake};

use super::body::MessageBodyCache;
use crate::components::{Avatar, AvatarPerson, icon, with_type};
use crate::i18n::Strings;
use crate::theme::icons::IconName;
use crate::theme::tokens::{
    COMPONENT_MESSAGE_AVATAR_INSET, COMPONENT_MESSAGE_AVATAR_SIZE, COMPONENT_MESSAGE_GUTTER,
    COMPONENT_MESSAGE_REPLY_AVATAR_SIZE, COMPONENT_MESSAGE_REPLY_HEIGHT,
    COMPONENT_MESSAGE_REPLY_SPINE_RADIUS, COMPONENT_MESSAGE_REPLY_SPINE_WIDTH, FONT_WEIGHT_MEDIUM,
    SPACE_2XS, SPACE_XS, TYPE_BODY,
};
use crate::theme::{color, length, theme};

/// The original's author and text, or no author and a sentence when it is not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplyReference {
    pub author: Option<PublicUser>,
    pub text: SharedString,
}

/// Finds the reply's original among the loaded messages; `None` when `message` is not a reply.
/// Someone the reader blocked is never quoted (ADR-0025).
pub fn reply_reference<S: BuildHasher, R: BuildHasher>(
    message: &Message,
    messages_by_id: &HashMap<Snowflake, &Message, S>,
    is_blocked: impl Fn(Snowflake) -> bool,
    roles: &HashMap<Snowflake, Role, R>,
    cache: &mut MessageBodyCache,
    strings: Strings,
) -> Option<ReplyReference> {
    let original_id = message.reply_to_id?;
    let Some(original) = messages_by_id.get(&original_id) else {
        return Some(ReplyReference {
            author: None,
            text: strings.messages().original_unavailable().into(),
        });
    };
    if is_blocked(original.author.id) {
        return Some(ReplyReference {
            author: None,
            text: strings.messages().blocked_reply().into(),
        });
    }
    Some(ReplyReference {
        author: Some(original.author.clone()),
        text: cache.summary(original, roles, strings),
    })
}

/// The quoted line; `author_picture` is the original author's avatar, when they have one.
#[derive(IntoElement)]
pub struct ReplyLine {
    reference: ReplyReference,
    author_picture: Option<ImageSource>,
}

impl ReplyLine {
    #[must_use]
    pub fn new(reference: ReplyReference, author_picture: Option<ImageSource>) -> Self {
        Self {
            reference,
            author_picture,
        }
    }
}

impl RenderOnce for ReplyLine {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = theme(cx);
        // From the centre of the author's avatar in the gutter, up and over to the original.
        let spine_left = COMPONENT_MESSAGE_AVATAR_INSET + COMPONENT_MESSAGE_AVATAR_SIZE / 2.0
            - COMPONENT_MESSAGE_GUTTER
            - COMPONENT_MESSAGE_REPLY_SPINE_WIDTH / 2.0;
        let spine_width = COMPONENT_MESSAGE_GUTTER
            - COMPONENT_MESSAGE_AVATAR_INSET
            - COMPONENT_MESSAGE_AVATAR_SIZE / 2.0
            - SPACE_XS
            + COMPONENT_MESSAGE_REPLY_SPINE_WIDTH;
        let spine_height = COMPONENT_MESSAGE_REPLY_HEIGHT / 2.0
            + SPACE_2XS
            + COMPONENT_MESSAGE_REPLY_SPINE_WIDTH / 2.0;
        let spine = div()
            .absolute()
            .left(length(spine_left))
            .top(length(COMPONENT_MESSAGE_REPLY_HEIGHT / 2.0))
            .w(length(spine_width))
            .h(length(spine_height))
            .border_t(length(COMPONENT_MESSAGE_REPLY_SPINE_WIDTH))
            .border_l(length(COMPONENT_MESSAGE_REPLY_SPINE_WIDTH))
            .border_color(color(tokens.component_message_reply_spine_color))
            .rounded_tl(length(COMPONENT_MESSAGE_REPLY_SPINE_RADIUS));
        let lead = match &self.reference.author {
            Some(author) => div()
                .flex()
                .items_center()
                .gap(length(SPACE_2XS))
                .flex_none()
                .child(Avatar::new(
                    AvatarPerson::from_user(author, self.author_picture),
                    COMPONENT_MESSAGE_REPLY_AVATAR_SIZE,
                ))
                .child(
                    div()
                        .font_weight(gpui::FontWeight(f32::from(FONT_WEIGHT_MEDIUM)))
                        .text_color(color(tokens.color_text_heading))
                        .child(display_name(author).to_owned()),
                ),
            None => div().flex_none().child(icon(
                IconName::ArrowBendUpLeft,
                COMPONENT_MESSAGE_REPLY_AVATAR_SIZE,
                color(tokens.color_text_secondary),
            )),
        };
        with_type(div(), TYPE_BODY)
            .relative()
            .flex()
            .items_center()
            .gap(length(SPACE_2XS))
            .min_w_0()
            .h(length(COMPONENT_MESSAGE_REPLY_HEIGHT))
            .text_color(color(tokens.color_text_secondary))
            .child(spine)
            .child(lead)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(one_line(&self.reference.text)),
            )
    }
}

/// The quote on one line: its line breaks read as spaces, as the web's truncated line shows them.
fn one_line(text: &str) -> SharedString {
    text.split_whitespace().collect::<Vec<_>>().join(" ").into()
}

impl std::fmt::Debug for ReplyLine {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReplyLine")
            .field("reference", &self.reference)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Language;

    fn message(id: u64, author: u64, reply_to: Option<u64>, content: &str) -> Message {
        serde_json::from_value(serde_json::json!({
            "id": id.to_string(), "channel_id": "1",
            "author": {"id": author.to_string(), "username": "ana", "display_name": "Ana"},
            "content": content,
            "reply_to_id": reply_to.map(|id| id.to_string()),
            "edited_at_unix_milliseconds": null
        }))
        .expect("a valid message")
    }

    #[test]
    fn a_quote_reads_on_one_line() {
        assert_eq!(
            one_line("first line\n  second\tthird"),
            "first line second third"
        );
    }

    #[test]
    fn a_reply_quotes_its_original_or_says_why_it_cannot() {
        let strings = Strings::new(Language::English);
        let original = message(10, 3, None, "first");
        let reply = message(11, 4, Some(10), "answer");
        let orphan = message(12, 4, Some(99), "lost");
        let by_id = HashMap::from([(original.id, &original)]);
        let roles: HashMap<Snowflake, Role> = HashMap::new();
        let mut cache = MessageBodyCache::new();
        let quoted = |message: &Message, blocked: Snowflake, cache: &mut MessageBodyCache| {
            reply_reference(message, &by_id, |id| id == blocked, &roles, cache, strings)
        };
        let nobody = Snowflake::from_raw(0);

        assert_eq!(quoted(&original, nobody, &mut cache), None);
        let reference = quoted(&reply, nobody, &mut cache).expect("a reply");
        assert_eq!(
            reference.author.map(|author| author.id),
            Some(original.author.id)
        );
        assert_eq!(reference.text, "first");
        assert_eq!(
            quoted(&orphan, nobody, &mut cache),
            Some(ReplyReference {
                author: None,
                text: strings.messages().original_unavailable().into(),
            })
        );
        assert_eq!(
            quoted(&reply, original.author.id, &mut cache),
            Some(ReplyReference {
                author: None,
                text: strings.messages().blocked_reply().into(),
            })
        );
    }
}
