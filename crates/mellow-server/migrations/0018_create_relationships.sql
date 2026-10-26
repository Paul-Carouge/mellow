-- Friends, friend requests and blocks (ADR-0025). A friendship or a pending request has a row on
-- each side, written in one transaction; a block has only the blocker's row.
CREATE TABLE relationships (
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    other_user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind TEXT NOT NULL
        CHECK (kind IN ('friend', 'incoming_request', 'outgoing_request', 'blocked')),
    since_unix_milliseconds INTEGER NOT NULL,
    PRIMARY KEY (user_id, other_user_id),
    CHECK (user_id != other_user_id)
) STRICT, WITHOUT ROWID;

-- The other side of a pair, and the ON DELETE CASCADE from users.
CREATE INDEX relationships_by_other_user_id ON relationships (other_user_id);
