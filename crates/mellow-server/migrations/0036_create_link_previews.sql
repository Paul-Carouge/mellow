-- Link previews fetched by the server (ADR-0041): one row per link fetched, shared by every
-- message linking it while it is fresh, and the previews each message shows, in order.
CREATE TABLE link_previews (
    id INTEGER PRIMARY KEY,
    -- SHA-256 of the link as fetched, to find a fresh preview without an index on long text.
    url_sha256 BLOB NOT NULL CHECK (length(url_sha256) = 32),
    url TEXT NOT NULL,
    title TEXT,
    description TEXT,
    site_name TEXT,
    theme_color INTEGER CHECK (theme_color BETWEEN 0 AND 16777215),
    -- The picture, fitted and sealed like an attachment preview (ADR-0033, ADR-0034).
    image_file_id INTEGER,
    image_content_type TEXT,
    image_size_bytes INTEGER,
    image_width INTEGER,
    image_height INTEGER,
    -- A fetch that gave nothing to show is kept too, so the link is not fetched again for a while.
    is_empty INTEGER NOT NULL CHECK (is_empty IN (0, 1)),
    fetched_at_unix_milliseconds INTEGER NOT NULL,
    expires_at_unix_milliseconds INTEGER NOT NULL,
    CHECK ((image_file_id IS NULL) = (image_content_type IS NULL)
        AND (image_file_id IS NULL) = (image_size_bytes IS NULL)
        AND (image_file_id IS NULL) = (image_width IS NULL)
        AND (image_file_id IS NULL) = (image_height IS NULL))
) STRICT;

-- The newest preview of a link; older ones stay while messages point at them.
CREATE INDEX link_previews_by_url_sha256 ON link_previews (url_sha256, fetched_at_unix_milliseconds);
-- The purge looks for expired previews.
CREATE INDEX link_previews_by_expiry ON link_previews (expires_at_unix_milliseconds);

CREATE TABLE message_link_previews (
    message_id INTEGER NOT NULL REFERENCES messages (id) ON DELETE CASCADE,
    position INTEGER NOT NULL CHECK (position BETWEEN 0 AND 2),
    link_preview_id INTEGER NOT NULL REFERENCES link_previews (id),
    PRIMARY KEY (message_id, position)
) STRICT, WITHOUT ROWID;

-- The purge keeps previews a message still shows.
CREATE INDEX message_link_previews_by_link_preview_id ON message_link_previews (link_preview_id);
