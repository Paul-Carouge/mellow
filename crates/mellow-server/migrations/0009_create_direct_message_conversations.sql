-- Direct-message conversations (ARCHITECTURE.md §4.5): a channel without a guild, of kind
-- 'direct_message', whose participants are listed here. Its messages and read states use the
-- existing tables.
CREATE TABLE direct_message_participants (
    channel_id INTEGER NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    PRIMARY KEY (channel_id, user_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX direct_message_participants_by_user_id ON direct_message_participants (user_id);

-- One conversation per pair of users: opening it twice, even concurrently, answers the same one.
CREATE TABLE direct_message_pairs (
    lower_user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    higher_user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    channel_id INTEGER NOT NULL UNIQUE REFERENCES channels (id) ON DELETE CASCADE,
    PRIMARY KEY (lower_user_id, higher_user_id),
    CHECK (lower_user_id < higher_user_id)
) STRICT, WITHOUT ROWID;
