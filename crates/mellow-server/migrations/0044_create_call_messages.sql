-- Private calls in conversations (ADR-0062): each call posts a message, whose record is here. Who
-- joined and the end are written once, when the call ends; a row without an end is a call going
-- on, or one a server stop interrupted (closed at the next start).
CREATE TABLE call_messages (
    message_id INTEGER PRIMARY KEY REFERENCES messages (id) ON DELETE CASCADE,
    ended_at_unix_milliseconds INTEGER CHECK (ended_at_unix_milliseconds >= 0)
) STRICT;

-- Unfinished calls, read at every start.
CREATE INDEX call_messages_unfinished ON call_messages (message_id)
    WHERE ended_at_unix_milliseconds IS NULL;

CREATE TABLE call_message_participants (
    message_id INTEGER NOT NULL REFERENCES call_messages (message_id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, user_id)
) STRICT, WITHOUT ROWID;

-- A user's calls, for an account deletion to come and a reader's "missed" queries.
CREATE INDEX call_message_participants_by_user_id ON call_message_participants (user_id);
