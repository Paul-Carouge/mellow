//! Whether a message mentions its reader (ADR-0040, the web's `reader-mentions.ts`): the one rule
//! behind the message tint, notifications and live unread mention badges.

use mellow_protocol::{Message, Snowflake};

/// Who reads, and where: `guild_id` is `None` and `role_ids` empty in a direct conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MentionReader<'roles> {
    pub user_id: Snowflake,
    pub guild_id: Option<Snowflake>,
    pub role_ids: &'roles [Snowflake],
}

/// The server decided who a message mentions: `mention_ids` names people, `mention_role_ids` the
/// roles the author could mention and the guild's id for `@everyone`. The reader is mentioned by
/// name, by a role they hold now, or by `@everyone` in their guild; never by their own message,
/// even one naming a role they hold.
#[must_use]
pub fn does_message_mention_reader(message: &Message, reader: &MentionReader<'_>) -> bool {
    if message.author.id == reader.user_id {
        return false;
    }
    if message.mention_ids.contains(&reader.user_id) {
        return true;
    }
    let role_ids = &message.mention_role_ids;
    reader.guild_id.is_some_and(|guild_id| {
        role_ids.contains(&guild_id)
            || reader
                .role_ids
                .iter()
                .any(|role_id| role_ids.contains(role_id))
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::message;
    use super::*;

    const GUILD: Snowflake = Snowflake::from_raw(100);
    const MODERATORS: Snowflake = Snowflake::from_raw(201);
    const ARTISTS: Snowflake = Snowflake::from_raw(202);
    const READER: Snowflake = Snowflake::from_raw(7);
    const AUTHOR: Snowflake = Snowflake::from_raw(8);

    fn reader() -> MentionReader<'static> {
        MentionReader {
            user_id: READER,
            guild_id: Some(GUILD),
            role_ids: &[MODERATORS],
        }
    }

    fn mentioning(people: &[u64], roles: &[Snowflake]) -> Message {
        let mut mentioning = message(1, AUTHOR);
        mentioning.mention_ids = people.iter().copied().map(Snowflake::from_raw).collect();
        mentioning.mention_role_ids = roles.to_vec();
        mentioning
    }

    #[test]
    fn is_true_when_the_reader_is_named() {
        assert!(does_message_mention_reader(
            &mentioning(&[7], &[]),
            &reader()
        ));
        assert!(!does_message_mention_reader(
            &mentioning(&[9], &[]),
            &reader()
        ));
    }

    #[test]
    fn is_true_for_a_role_the_reader_holds_not_for_another_one() {
        let both = mentioning(&[], &[ARTISTS, MODERATORS]);
        assert!(does_message_mention_reader(&both, &reader()));
        let artists = mentioning(&[], &[ARTISTS]);
        assert!(!does_message_mention_reader(&artists, &reader()));
    }

    #[test]
    fn is_true_for_everyone_written_as_the_guilds_id_even_without_any_role() {
        let without_roles = MentionReader {
            role_ids: &[],
            ..reader()
        };
        let everyone = mentioning(&[], &[GUILD]);
        assert!(does_message_mention_reader(&everyone, &without_roles));
    }

    #[test]
    fn is_false_for_the_readers_own_message_even_naming_a_role_they_hold() {
        let mut own = mentioning(&[7], &[MODERATORS, GUILD]);
        own.author.id = READER;
        assert!(!does_message_mention_reader(&own, &reader()));
    }

    #[test]
    fn is_false_without_mentions_and_for_roles_in_a_conversation() {
        assert!(!does_message_mention_reader(
            &mentioning(&[], &[]),
            &reader()
        ));
        let in_a_conversation = MentionReader {
            user_id: READER,
            guild_id: None,
            role_ids: &[],
        };
        let everyone = mentioning(&[], &[GUILD]);
        assert!(!does_message_mention_reader(&everyone, &in_a_conversation));
    }
}
