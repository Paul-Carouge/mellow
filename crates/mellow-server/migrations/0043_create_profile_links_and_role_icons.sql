-- Richer profiles and role icons (ADR-0050).

-- A user's "about me" text; users without a row have none.
CREATE TABLE user_profiles (
    user_id INTEGER PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    about_me TEXT NOT NULL
) STRICT;

-- Up to five links per user, in the user's order. `host` is the address's host, lowercased, which
-- names the site icon the link shows (`profile_link_icons`).
CREATE TABLE user_profile_links (
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position BETWEEN 0 AND 4),
    url TEXT NOT NULL,
    label TEXT,
    host TEXT NOT NULL,
    PRIMARY KEY (user_id, position)
) STRICT, WITHOUT ROWID;

-- The icons left without a link are purged with the profile saves that drop them.
CREATE INDEX user_profile_links_by_host ON user_profile_links (host);

-- Each site's icon, fetched by the server once per host and fetched again after
-- `refresh_after_unix_milliseconds`. `file_id` is a sealed file (ADR-0034) with a random id, so
-- it tells nobody when a site was first linked; NULL when the site has no usable icon.
CREATE TABLE profile_link_icons (
    host TEXT PRIMARY KEY,
    file_id INTEGER UNIQUE,
    size_bytes INTEGER CHECK (size_bytes >= 0),
    refresh_after_unix_milliseconds INTEGER NOT NULL,
    CHECK ((file_id IS NULL) = (size_bytes IS NULL))
) STRICT, WITHOUT ROWID;

-- A role's icon: a picture stored as a guild image (migration 0014, so it is served to members,
-- swept and dropped with its guild like an icon), or a Unicode emoji; never both.
ALTER TABLE roles ADD COLUMN icon_id INTEGER;
ALTER TABLE roles ADD COLUMN icon_is_animated INTEGER NOT NULL DEFAULT 0
    CHECK (icon_is_animated IN (0, 1));
ALTER TABLE roles ADD COLUMN unicode_emoji TEXT
    CHECK (unicode_emoji IS NULL OR icon_id IS NULL);
