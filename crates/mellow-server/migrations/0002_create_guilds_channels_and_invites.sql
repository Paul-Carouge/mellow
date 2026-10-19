-- Guilds ("servers" in the UI), their @everyone role, members, channels and invites (ARCHITECTURE.md §4.5).

CREATE TABLE guilds (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    owner_id INTEGER NOT NULL REFERENCES users (id),
    created_at_unix_milliseconds INTEGER NOT NULL
) STRICT;

-- The @everyone role has the guild's id, as on Discord. Named roles arrive with M2.
CREATE TABLE roles (
    id INTEGER PRIMARY KEY,
    guild_id INTEGER NOT NULL REFERENCES guilds (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    permissions INTEGER NOT NULL,
    position INTEGER NOT NULL
) STRICT;

CREATE INDEX roles_by_guild_id ON roles (guild_id);

CREATE TABLE guild_members (
    guild_id INTEGER NOT NULL REFERENCES guilds (id) ON DELETE CASCADE,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    joined_at_unix_milliseconds INTEGER NOT NULL,
    PRIMARY KEY (guild_id, user_id)
) STRICT, WITHOUT ROWID;

CREATE INDEX guild_members_by_user_id ON guild_members (user_id);

-- guild_id is NULL for direct-message channels (M2), and `kind` has no CHECK: SQLite cannot
-- alter either without rebuilding the table, which would cascade into messages. The
-- application validates kinds on read and write.
CREATE TABLE channels (
    id INTEGER PRIMARY KEY,
    guild_id INTEGER REFERENCES guilds (id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    name TEXT NOT NULL,
    topic TEXT NOT NULL DEFAULT '',
    -- Deleting a category keeps its channels, uncategorised, as on Discord.
    parent_id INTEGER REFERENCES channels (id) ON DELETE SET NULL,
    position INTEGER NOT NULL CHECK (position >= 0)
) STRICT;

CREATE INDEX channels_by_guild_id ON channels (guild_id, position);
-- ON DELETE SET NULL looks children up by parent.
CREATE INDEX channels_by_parent_id ON channels (parent_id);

CREATE TABLE invites (
    code TEXT PRIMARY KEY,
    guild_id INTEGER NOT NULL REFERENCES guilds (id) ON DELETE CASCADE,
    creator_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    max_uses INTEGER CHECK (max_uses IS NULL OR max_uses > 0),
    uses INTEGER NOT NULL DEFAULT 0 CHECK (uses >= 0),
    expires_at_unix_milliseconds INTEGER,
    created_at_unix_milliseconds INTEGER NOT NULL
) STRICT;

CREATE INDEX invites_by_guild_id ON invites (guild_id);
