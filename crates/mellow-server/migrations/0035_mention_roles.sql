-- Role mentions (ADR-0040). A role anyone may mention (Discord's "Allow anyone to @mention this
-- role"); without it, only members holding MENTION_EVERYONE in the channel mention it. Existing
-- roles are not mentionable; `@everyone` never is (the domain refuses it: mentioning everyone
-- always needs MENTION_EVERYONE). Changing it bumps the guild's permission version through the
-- existing roles trigger, so cached snapshots carry the new value.
ALTER TABLE roles ADD COLUMN is_mentionable INTEGER NOT NULL DEFAULT 0
    CHECK (is_mentionable IN (0, 1));

-- The roles each message mentions, `@everyone` stored as the guild's id (the id of its
-- `@everyone` role). Kept per role rather than expanded to every holder, so a mention of a large
-- role writes one row and its holders are found when unread mentions are counted. No foreign key
-- on the role: a deleted role's rows match no holder any more, and the role's id is never reused.
CREATE TABLE message_role_mentions (
    role_id INTEGER NOT NULL,
    -- No foreign key, as in `message_mentions`: deleting a message removes its rows.
    channel_id INTEGER NOT NULL,
    message_id INTEGER NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    PRIMARY KEY (channel_id, role_id, message_id)
) STRICT, WITHOUT ROWID;

-- Reading a message's role mentions, and the ON DELETE CASCADE from messages.
CREATE INDEX message_role_mentions_by_message_id ON message_role_mentions (message_id);
