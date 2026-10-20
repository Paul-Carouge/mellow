-- Guild bans: a banned user is removed from the guild and cannot join it again through an
-- invite until unbanned. `banned_by` keeps the moderator for moderators to see; it is cleared if
-- that account is deleted.
CREATE TABLE bans (
    guild_id INTEGER NOT NULL REFERENCES guilds (id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    reason TEXT NOT NULL DEFAULT '',
    banned_by INTEGER REFERENCES users (id) ON DELETE SET NULL,
    created_at_unix_milliseconds INTEGER NOT NULL,
    PRIMARY KEY (guild_id, user_id)
) STRICT, WITHOUT ROWID;
