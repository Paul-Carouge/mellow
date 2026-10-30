-- Custom emoji (ADR-0028). An emoji's picture is a guild image (migration 0014) with the same id,
-- so it is served to members like an icon, swept like one, and goes with its guild; it is
-- animated when that image has a still frame. Names are unique in a guild whatever their case.
CREATE TABLE guild_emojis (
    id INTEGER PRIMARY KEY REFERENCES guild_images (id) ON DELETE CASCADE,
    guild_id INTEGER NOT NULL REFERENCES guilds (id) ON DELETE CASCADE,
    name TEXT NOT NULL COLLATE NOCASE
) STRICT;

CREATE UNIQUE INDEX guild_emojis_by_guild_id_and_name ON guild_emojis (guild_id, name);
