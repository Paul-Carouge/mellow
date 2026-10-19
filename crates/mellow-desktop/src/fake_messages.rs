//! Deterministic fake conversation for the spike: no randomness, so measurements are repeatable.

const AUTHORS: [&str; 5] = ["Aurore", "Basile", "Chloé", "Dimitri", "Élise"];
const SAMPLE_BODIES: [&str; 8] = [
    "Morning everyone! Anyone up for a quick sync about the **release notes**?",
    "I pushed the fix, see `storage::database` and [the PR](https://example.org/pr/2).",
    "Sounds good. *Italic thoughts* and ~~struck ideas~~ welcome.",
    "@you can you check the migration before we ship?",
    "```\nfn main() {\n    println!(\"code blocks are not supported by StyledText\");\n}\n```",
    "Here is a much longer message to check wrapping behaviour. It keeps going for a while, \
     describing how the self-hosted instance should update itself with one command, keep a \
     snapshot before migrations, and never lose data when something goes wrong halfway.",
    "> quoting someone is a block quote, which StyledText cannot render either",
    "ok 👍",
];
const MESSAGES_PER_AUTHOR_TURN: usize = 3;
const MINUTES_PER_MESSAGE: usize = 2;
const MINUTES_PER_DAY: usize = 24 * 60;
const MENTION_MARKER: &str = "@you";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeMessage {
    pub author: &'static str,
    pub timestamp: String,
    pub body: &'static str,
    pub starts_author_group: bool,
    pub reply_preview: Option<&'static str>,
    pub mentions_current_user: bool,
}

pub fn fake_conversation(message_count: usize) -> Vec<FakeMessage> {
    (0..message_count)
        .map(|index| {
            let author_turn = index / MESSAGES_PER_AUTHOR_TURN;
            let body = SAMPLE_BODIES[index % SAMPLE_BODIES.len()];
            let minutes_of_day = (index * MINUTES_PER_MESSAGE) % MINUTES_PER_DAY;
            FakeMessage {
                author: AUTHORS[author_turn % AUTHORS.len()],
                timestamp: format!("{:02}:{:02}", minutes_of_day / 60, minutes_of_day % 60),
                body,
                starts_author_group: index % MESSAGES_PER_AUTHOR_TURN == 0,
                reply_preview: (index % 11 == 5).then_some(SAMPLE_BODIES[0]),
                mentions_current_user: body.contains(MENTION_MARKER),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_is_deterministic_and_grouped() {
        let conversation = fake_conversation(10);
        assert_eq!(conversation, fake_conversation(10));
        assert_eq!(conversation.len(), 10);
        assert!(conversation[0].starts_author_group);
        assert!(!conversation[1].starts_author_group);
        assert!(conversation[3].starts_author_group);
        assert_ne!(conversation[0].author, conversation[3].author);
        assert!(conversation[3].mentions_current_user);
    }
}
