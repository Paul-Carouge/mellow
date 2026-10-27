-- Group conversations (ADR-0026): a conversation with an owner. Its participants are in
-- direct_message_participants like any conversation's, and its name in channels.name.
CREATE TABLE group_conversations (
    channel_id INTEGER PRIMARY KEY REFERENCES channels (id) ON DELETE CASCADE,
    owner_id INTEGER NOT NULL REFERENCES users (id)
) STRICT;
