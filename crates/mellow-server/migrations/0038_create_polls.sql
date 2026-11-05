-- Polls in messages (ADR-0043). A poll belongs to its message and goes with it.
CREATE TABLE polls (
    message_id INTEGER PRIMARY KEY REFERENCES messages (id) ON DELETE CASCADE,
    question TEXT NOT NULL,
    allows_multiple_answers INTEGER NOT NULL CHECK (allows_multiple_answers IN (0, 1)),
    expires_at_unix_milliseconds INTEGER NOT NULL,
    -- Set when the author ended it early.
    ended_at_unix_milliseconds INTEGER
) STRICT;

CREATE TABLE poll_answers (
    message_id INTEGER NOT NULL REFERENCES polls (message_id) ON DELETE CASCADE,
    answer_id INTEGER NOT NULL CHECK (answer_id BETWEEN 1 AND 10),
    text TEXT NOT NULL,
    PRIMARY KEY (message_id, answer_id)
) STRICT, WITHOUT ROWID;

CREATE TABLE poll_votes (
    message_id INTEGER NOT NULL,
    answer_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, user_id, answer_id),
    FOREIGN KEY (message_id, answer_id) REFERENCES poll_answers (message_id, answer_id)
        ON DELETE CASCADE
) STRICT, WITHOUT ROWID;

-- Counts per answer, voters of an answer, and the cascade from poll_answers.
CREATE INDEX poll_votes_by_answer ON poll_votes (message_id, answer_id, user_id);
-- The cascade from users.
CREATE INDEX poll_votes_by_user_id ON poll_votes (user_id);

-- SEND_POLLS (1 << 22, ADR-0043) is new: grant it to every existing @everyone role (the role whose
-- id is its guild's), as new guilds do.
UPDATE roles SET permissions = permissions | 4194304 WHERE id = guild_id;
