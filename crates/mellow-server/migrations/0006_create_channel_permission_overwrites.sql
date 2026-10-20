-- Channel permission overwrites (ADR-0007 step 4). `target_id` is a role id (the guild's id for
-- @everyone) or a user id, so it has no foreign key: deleting a role deletes its overwrites in the
-- same transaction, and a member's overwrite stays when they leave, as on Discord.
CREATE TABLE channel_permission_overwrites (
    channel_id INTEGER NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    target_id INTEGER NOT NULL,
    target_kind TEXT NOT NULL CHECK (target_kind IN ('role', 'member')),
    allow INTEGER NOT NULL,
    deny INTEGER NOT NULL CHECK (allow & deny = 0),
    PRIMARY KEY (channel_id, target_id)
) STRICT, WITHOUT ROWID;

-- Deleting a role looks its overwrites up by target.
CREATE INDEX channel_permission_overwrites_by_target_id ON channel_permission_overwrites (target_id);
