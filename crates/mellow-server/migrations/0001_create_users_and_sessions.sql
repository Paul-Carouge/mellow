-- Accounts are local to the instance (ADR-0005). Ids are Snowflakes (ADR-0004); times are Unix milliseconds.
CREATE TABLE users (
    id INTEGER PRIMARY KEY,
    username TEXT NOT NULL UNIQUE COLLATE NOCASE,
    display_name TEXT NOT NULL,
    email TEXT NOT NULL UNIQUE COLLATE NOCASE,
    password_hash TEXT NOT NULL,
    is_instance_administrator INTEGER NOT NULL DEFAULT 0 CHECK (is_instance_administrator IN (0, 1)),
    created_at_unix_milliseconds INTEGER NOT NULL,
    disabled_at_unix_milliseconds INTEGER
) STRICT;

-- Only the SHA-256 of a session token is stored, so a database leak cannot be replayed (ARCHITECTURE.md §4.7).
CREATE TABLE sessions (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_sha256 BLOB NOT NULL UNIQUE CHECK (length(token_sha256) = 32),
    device_name TEXT NOT NULL,
    created_at_unix_milliseconds INTEGER NOT NULL,
    last_seen_at_unix_milliseconds INTEGER NOT NULL,
    expires_at_unix_milliseconds INTEGER NOT NULL
) STRICT;

CREATE INDEX sessions_by_user_id ON sessions (user_id);
