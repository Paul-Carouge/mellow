-- STREAM (1 << 21, "Video", ADR-0035) is new: grant it to every existing @everyone role (the role
-- whose id is its guild's), as new guilds do.
UPDATE roles SET permissions = permissions | 2097152 WHERE id = guild_id;
