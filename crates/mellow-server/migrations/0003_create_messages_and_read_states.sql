-- Messages and per-user read positions (ARCHITECTURE.md §4.5).

CREATE TABLE messages (
    id INTEGER PRIMARY KEY,
    channel_id INTEGER NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    author_id INTEGER NOT NULL REFERENCES users (id),
    content TEXT NOT NULL,
    -- No foreign key: a reply keeps pointing at a deleted message ("Original message was deleted").
    reply_to_id INTEGER,
    edited_at_unix_milliseconds INTEGER
) STRICT;

-- History pages walk one channel by id (ids are time-ordered).
CREATE INDEX messages_by_channel_id ON messages (channel_id, id);

CREATE TABLE read_states (
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    channel_id INTEGER NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    last_read_message_id INTEGER NOT NULL,
    PRIMARY KEY (user_id, channel_id)
) STRICT, WITHOUT ROWID;
