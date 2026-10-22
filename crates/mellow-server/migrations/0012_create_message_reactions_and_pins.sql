-- Emoji reactions and pinned messages (ARCHITECTURE.md §4.5).

-- One row per user and emoji on a message. The rowid follows insertion, so a message lists its
-- emoji in the order they were first added; the unique index also serves lookups by message.
CREATE TABLE message_reactions (
    message_id INTEGER NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    emoji TEXT NOT NULL,
    UNIQUE (message_id, emoji, user_id)
) STRICT;

-- Set while the message is pinned; a channel lists its pins newest first.
ALTER TABLE messages ADD COLUMN pinned_at_unix_milliseconds INTEGER;

CREATE INDEX messages_pinned_by_channel_id ON messages (channel_id, pinned_at_unix_milliseconds)
    WHERE pinned_at_unix_milliseconds IS NOT NULL;

-- ADD_REACTIONS (1 << 16) is new: grant it to every existing @everyone role (the role whose id is
-- its guild's), as new guilds do, or only owners and administrators could add an emoji there.
UPDATE roles SET permissions = permissions | 65536 WHERE id = guild_id;

-- An overwrite that takes SEND_MESSAGES (1 << 2) away made a read-only channel; it now takes
-- ADD_REACTIONS away too, so the grant above does not open those channels to new reactions.
-- Overwrites that allow ADD_REACTIONS keep it (none can exist yet: the check refuses both).
UPDATE channel_permission_overwrites SET deny = deny | 65536
    WHERE deny & 4 <> 0 AND allow & 65536 = 0;
