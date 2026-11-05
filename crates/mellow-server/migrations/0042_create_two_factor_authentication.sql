-- Two-factor sign-in with time-based one-time passwords (ADR-0049). The secret is stored as is:
-- the whole database is encrypted with SQLCipher under a key derived from the instance master key
-- (ADR-0006), the same key any second sealing would use.
CREATE TABLE two_factor_secrets (
    user_id INTEGER PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    secret BLOB NOT NULL CHECK (length(secret) = 20),
    -- NULL while enrolment waits for its first code; sign-in asks for a code only once it is set.
    enabled_at_unix_milliseconds INTEGER,
    -- The newest time step whose code was accepted: a code works once (RFC 6238 §5.2).
    last_used_time_step INTEGER NOT NULL DEFAULT -1,
    created_at_unix_milliseconds INTEGER NOT NULL
) STRICT;

-- Only the SHA-256 of each single-use recovery code (80 random bits each) is kept.
CREATE TABLE two_factor_recovery_codes (
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    code_sha256 BLOB NOT NULL CHECK (length(code_sha256) = 32),
    PRIMARY KEY (user_id, code_sha256)
) STRICT, WITHOUT ROWID;

-- Resets made by an instance administrator with `mellow-server admin reset-two-factor`, kept as
-- the audit trail of an action that removes an account's protection.
CREATE TABLE two_factor_resets (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    reset_at_unix_milliseconds INTEGER NOT NULL
) STRICT;

CREATE INDEX two_factor_resets_by_user_id ON two_factor_resets (user_id);
