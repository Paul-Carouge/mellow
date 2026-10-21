-- Avatars and profile banners (ADR-0021). Each version is a sealed file (ADR-0020) with its own
-- id; an animated one also has a still first frame. Users point at their current ones, with the
-- animation flag kept beside so user payloads need no join.
CREATE TABLE profile_images (
    id INTEGER PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    still_id INTEGER,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    created_at_unix_milliseconds INTEGER NOT NULL
) STRICT;

CREATE INDEX profile_images_by_user_id ON profile_images (user_id);

ALTER TABLE users ADD COLUMN avatar_id INTEGER;
ALTER TABLE users ADD COLUMN avatar_is_animated INTEGER NOT NULL DEFAULT 0;
ALTER TABLE users ADD COLUMN banner_id INTEGER;
ALTER TABLE users ADD COLUMN banner_is_animated INTEGER NOT NULL DEFAULT 0;
