-- Guild and channel icons (ADR-0021). Each version is a sealed file (ADR-0020) with its own id;
-- an animated guild icon also has a still first frame. A channel's icon row names its channel,
-- so reading it can check who may view that channel; both go with their guild or channel.
CREATE TABLE guild_images (
    id INTEGER PRIMARY KEY,
    guild_id INTEGER NOT NULL REFERENCES guilds (id) ON DELETE CASCADE,
    channel_id INTEGER REFERENCES channels (id) ON DELETE CASCADE,
    still_id INTEGER,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    created_at_unix_milliseconds INTEGER NOT NULL
) STRICT;

CREATE INDEX guild_images_by_guild_id ON guild_images (guild_id);
CREATE INDEX guild_images_by_channel_id ON guild_images (channel_id);

ALTER TABLE guilds ADD COLUMN icon_id INTEGER;
ALTER TABLE guilds ADD COLUMN icon_is_animated INTEGER NOT NULL DEFAULT 0;
ALTER TABLE channels ADD COLUMN icon_id INTEGER;
