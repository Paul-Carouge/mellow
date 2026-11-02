-- Which pipeline stored each picture: 0 before ADR-0033 (lossless only), 1 since. Rows written
-- after this migration take the default; `admin recompress-images` sets 1 before it tries a
-- picture, so a picture that kills the process is never tried again and a stopped run resumes
-- with the next one. Custom emoji keep 0: they are never recompressed.
ALTER TABLE attachments ADD COLUMN compression_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE profile_images ADD COLUMN compression_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE guild_images ADD COLUMN compression_version INTEGER NOT NULL DEFAULT 1;

UPDATE attachments SET compression_version = 0
    WHERE content_type IN ('image/png', 'image/jpeg', 'image/gif', 'image/webp');
UPDATE profile_images SET compression_version = 0;
UPDATE guild_images SET compression_version = 0;

CREATE INDEX attachments_stored_before_adr_0033 ON attachments (id)
    WHERE compression_version = 0;
CREATE INDEX profile_images_stored_before_adr_0033 ON profile_images (id)
    WHERE compression_version = 0;
CREATE INDEX guild_images_stored_before_adr_0033 ON guild_images (id)
    WHERE compression_version = 0;

-- Set by `admin recompress-images --confirm`, read by the running server's picture worker, which
-- deletes it once no picture is left to recompress.
CREATE TABLE image_recompression_requests (
    requested_at_unix_milliseconds INTEGER NOT NULL
) STRICT;
