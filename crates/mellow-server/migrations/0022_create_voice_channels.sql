-- Voice channels (ADR-0030). `kind` has no CHECK (migration 0002): 'voice' is validated by the
-- application like the other kinds. A voice channel may cap how many people connect at once;
-- NULL means only the hard cap of 25 applies.
ALTER TABLE channels ADD COLUMN user_limit INTEGER CHECK (user_limit BETWEEN 1 AND 25);

-- CONNECT (1 << 18) and SPEAK (1 << 19) are new: grant both to every existing @everyone role (the
-- role whose id is its guild's), as new guilds do, or only owners and administrators could use
-- voice channels.
UPDATE roles SET permissions = permissions | 786432 WHERE id = guild_id;
