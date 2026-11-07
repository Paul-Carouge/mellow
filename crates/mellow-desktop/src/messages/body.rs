//! What a row derives from its message's text, worked out once per version of the message (the
//! web's `$derived` values in `MessageItem.svelte` and the quote cache of `reply-references.ts`).

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::rc::Rc;

use gpui::SharedString;
use mellow_client_core::shared_logic::custom_emoji::is_jumbo_emoji_message;
use mellow_client_core::shared_logic::markdown::{
    BlockNode, MarkdownSources, markdown_mentions_user, parse_message_markdown,
};
use mellow_client_core::shared_logic::role_mentions::message_as_plain_text;
use mellow_protocol::{Message, Role, Snowflake};

use crate::i18n::Strings;

/// A few pages of history in each of a few channels: past it the cache starts over rather than
/// tracking which rows were used last, which costs one parse per visible row.
const MAXIMUM_CACHED_ENTRIES: usize = 2048;

/// A message's text, parsed.
#[derive(Debug, PartialEq, Eq)]
pub struct MessageBody {
    pub blocks: Vec<BlockNode>,
    /// Only emoji: they show large (`isJumboEmojiMessage`).
    pub is_jumbo: bool,
    /// The text names the reader (`markdownMentionsUser`), for messages from before the server
    /// found mentions; the list adds the server's `mention_ids` and roles.
    pub names_reader: bool,
}

/// One version of a message's text: an edit or `@everyone` kept or dropped parses it again.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum MessageBodyKey {
    Sent {
        id: Snowflake,
        edited_at_unix_milliseconds: Option<u64>,
        is_everyone_mentioned: bool,
    },
    /// The outbox's text never changes under its nonce.
    Pending { nonce: String },
}

/// The list's memory of parsed bodies and quoted summaries.
#[derive(Debug, Default)]
pub struct MessageBodyCache {
    sources_generation: u64,
    bodies: HashMap<MessageBodyKey, Rc<MessageBody>>,
    summaries: HashMap<(Snowflake, Option<u64>), SharedString>,
}

impl MessageBodyCache {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What text is read against changed (people, custom emoji, roles, language, the reader):
    /// the list bumps its generation and every body is parsed again.
    pub fn use_sources_generation(&mut self, generation: u64) {
        if generation != self.sources_generation {
            self.sources_generation = generation;
            self.bodies.clear();
            self.summaries.clear();
        }
    }

    /// The parsed body of `content`, parsed now only if this version was not.
    pub fn body(
        &mut self,
        key: &MessageBodyKey,
        content: &str,
        sources: MarkdownSources<'_>,
        reader_id: Option<Snowflake>,
    ) -> Rc<MessageBody> {
        if let Some(body) = self.bodies.get(key) {
            return Rc::clone(body);
        }
        if self.bodies.len() >= MAXIMUM_CACHED_ENTRIES {
            self.bodies.clear();
        }
        let blocks = parse_message_markdown(content, sources);
        let body = Rc::new(MessageBody {
            is_jumbo: is_jumbo_emoji_message(&blocks),
            names_reader: reader_id.is_some_and(|id| markdown_mentions_user(&blocks, id)),
            blocks,
        });
        self.bodies.insert(key.clone(), Rc::clone(&body));
        body
    }

    /// `message_summary` of `original`, worked out once per version of it.
    pub fn summary<S: BuildHasher>(
        &mut self,
        original: &Message,
        roles: &HashMap<Snowflake, Role, S>,
        strings: Strings,
    ) -> SharedString {
        let key = (original.id, original.edited_at_unix_milliseconds);
        if let Some(summary) = self.summaries.get(&key) {
            return summary.clone();
        }
        if self.summaries.len() >= MAXIMUM_CACHED_ENTRIES {
            self.summaries.clear();
        }
        let summary = SharedString::from(message_summary(original, roles, strings));
        self.summaries.insert(key, summary.clone());
        summary
    }
}

/// The text, roles named, then "2 attachments" when files came with it (DESIGN.md §7.3); a poll's
/// question for a poll (ADR-0043), a checklist's title or a progress bar's label (ADR-0047).
#[must_use]
pub fn message_summary<S: BuildHasher>(
    message: &Message,
    roles: &HashMap<Snowflake, Role, S>,
    strings: Strings,
) -> String {
    if let Some(poll) = &message.poll {
        return strings.polls().summary(&poll.question);
    }
    if let Some(checklist) = &message.checklist {
        return strings.checklists().summary(&checklist.title);
    }
    if let Some(progress_bar) = &message.progress_bar {
        return strings.progress_bars().summary(&progress_bar.label);
    }
    let text = message_as_plain_text(
        &message.content,
        roles,
        strings.messages().deleted_role_name(),
    );
    let attachments = match message.attachments.len() {
        0 => String::new(),
        count => strings
            .messages()
            .attachment_count(i64::try_from(count).unwrap_or(i64::MAX)),
    };
    [text, attachments]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use mellow_client_core::shared_logic::markdown::RoleMentionContext;
    use mellow_client_core::shared_logic::mentions::MentionablePeople;

    use super::*;
    use crate::i18n::Language;

    fn sources_and_parse(
        cache: &mut MessageBodyCache,
        key: &MessageBodyKey,
        text: &str,
    ) -> Rc<MessageBody> {
        let people = MentionablePeople::new();
        let emojis = HashMap::new();
        let roles = HashMap::new();
        cache.body(
            key,
            text,
            MarkdownSources {
                people: &people,
                custom_emojis: &emojis,
                role_mentions: RoleMentionContext {
                    roles: &roles,
                    is_everyone_mentioned: false,
                },
            },
            None,
        )
    }

    #[test]
    fn a_version_is_parsed_once_and_an_edit_parses_again() {
        let mut cache = MessageBodyCache::new();
        let key = MessageBodyKey::Sent {
            id: Snowflake::from_raw(1),
            edited_at_unix_milliseconds: None,
            is_everyone_mentioned: false,
        };
        let first = sources_and_parse(&mut cache, &key, "**hi**");
        let again = sources_and_parse(&mut cache, &key, "ignored: same version");
        assert!(Rc::ptr_eq(&first, &again));

        let edited = MessageBodyKey::Sent {
            id: Snowflake::from_raw(1),
            edited_at_unix_milliseconds: Some(5),
            is_everyone_mentioned: false,
        };
        let after_edit = sources_and_parse(&mut cache, &edited, "plain");
        assert_eq!(
            after_edit.blocks,
            vec![BlockNode::Paragraph(vec![
                mellow_client_core::shared_logic::markdown::InlineNode::Text("plain".into())
            ])]
        );

        cache.use_sources_generation(1);
        let after_new_sources = sources_and_parse(&mut cache, &key, "**hi**");
        assert!(!Rc::ptr_eq(&first, &after_new_sources));
    }

    #[test]
    fn a_summary_names_roles_and_counts_files() {
        let strings = Strings::new(Language::English);
        let mut message: Message = serde_json::from_value(serde_json::json!({
            "id": "10", "channel_id": "2",
            "author": {"id": "3", "username": "ana", "display_name": "Ana"},
            "content": "see <@&99> <:party:500>", "reply_to_id": null,
            "edited_at_unix_milliseconds": null
        }))
        .expect("a valid message");
        assert_eq!(
            message_summary(&message, &HashMap::new(), strings),
            format!("see @{} :party:", strings.messages().deleted_role_name())
        );
        message.content = String::new();
        message.attachments = vec![
            serde_json::from_value(serde_json::json!({
                "id": "4", "file_name": "a.png", "content_type": "image/png", "size_bytes": 1
            }))
            .expect("a valid attachment"),
        ];
        assert_eq!(
            message_summary(&message, &HashMap::new(), strings),
            strings.messages().attachment_count(1)
        );
    }
}
