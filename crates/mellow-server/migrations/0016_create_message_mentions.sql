-- Who each message mentions, found by the server when the message is sent or edited, so unread
-- mention counts survive across sessions and devices (ROADMAP "Next" item 5). Messages written
-- before this migration mention no one here.
CREATE TABLE message_mentions (
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- No foreign key: deleting a message (and so a channel) already removes its rows, and an
    -- unindexed one would scan this table for every channel deleted.
    channel_id INTEGER NOT NULL,
    message_id INTEGER NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, channel_id, message_id)
) STRICT, WITHOUT ROWID;

-- Reading a message's mentions, and the ON DELETE CASCADE from messages.
CREATE INDEX message_mentions_by_message_id ON message_mentions (message_id);
