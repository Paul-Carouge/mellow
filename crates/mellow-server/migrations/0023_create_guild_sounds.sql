-- Soundboard sounds (ADR-0032). A sound's samples are a sealed file (ADR-0020) whose id is the
-- sound's; they are a canonical PCM WAV checked by the server, so only their length is kept here.
-- `emoji` is a reaction key (ADR-0028), NULL for none.
CREATE TABLE guild_sounds (
    id INTEGER PRIMARY KEY,
    guild_id INTEGER NOT NULL REFERENCES guilds (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    emoji TEXT,
    duration_milliseconds INTEGER NOT NULL CHECK (duration_milliseconds BETWEEN 1 AND 5200),
    size_bytes INTEGER NOT NULL CHECK (size_bytes > 0),
    created_at_unix_milliseconds INTEGER NOT NULL
) STRICT;

CREATE INDEX guild_sounds_by_guild_id ON guild_sounds (guild_id);

-- A member's join sound in that guild; cleared when the sound is deleted.
ALTER TABLE guild_members ADD COLUMN join_sound_id INTEGER REFERENCES guild_sounds (id) ON DELETE SET NULL;

-- USE_SOUNDBOARD (1 << 20) is new: grant it to every existing @everyone role (the role whose id
-- is its guild's), as new guilds do.
UPDATE roles SET permissions = permissions | 1048576 WHERE id = guild_id;
