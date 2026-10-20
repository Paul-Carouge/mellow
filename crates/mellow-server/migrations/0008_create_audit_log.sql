-- Guild audit log: what moderators and administrators did. An entry is written in the same
-- transaction as the action it records, so no committed action goes unrecorded. The id is a
-- Snowflake and encodes the time; entries are listed newest first.
CREATE TABLE audit_log (
    id INTEGER PRIMARY KEY,
    guild_id INTEGER NOT NULL REFERENCES guilds (id) ON DELETE CASCADE,
    actor_id INTEGER REFERENCES users (id) ON DELETE SET NULL,
    action TEXT NOT NULL,
    target_id INTEGER,
    context_id INTEGER,
    reason TEXT NOT NULL DEFAULT ''
) STRICT;

CREATE INDEX audit_log_by_guild_id ON audit_log (guild_id, id);
