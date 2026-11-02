-- Which pipeline stored each picture: 0 before ADR-0033 (lossless only), 1 since. Rows written
-- after this migration take the default; `admin recompress-images` sets 1 before it tries a
-- picture, so a picture that kills the process is never tried again and a stopped run resumes
-- with the next one. Custom emoji keep 0: they are never recompressed.
ALTER TABLE attachments ADD COLUMN compression_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE profile_images ADD COLUMN compression_version INTEGER NOT NULL DEFAULT 1;
ALTER TABLE guild_images ADD COLUMN compression_version INTEGER NOT NULL DEFAULT 1;

-- ADR-0033's pipeline came with migration 0027: what was stored once it had run (when an earlier
-- release shipped it) is already compressed, and compressing it again would lose quality twice.
-- Run in the same update, 0027 is only seconds old and every existing row comes before it.
CREATE TEMPORARY TABLE adr_0033_pipeline_start AS
    SELECT CAST(strftime('%s', installed_on) AS INTEGER) * 1000 AS unix_milliseconds
    FROM _sqlx_migrations WHERE version = 27;

UPDATE attachments SET compression_version = 0
    WHERE content_type IN ('image/png', 'image/jpeg', 'image/gif', 'image/webp')
        AND created_at_unix_milliseconds < (SELECT unix_milliseconds FROM adr_0033_pipeline_start);
UPDATE profile_images SET compression_version = 0
    WHERE created_at_unix_milliseconds < (SELECT unix_milliseconds FROM adr_0033_pipeline_start);
UPDATE guild_images SET compression_version = 0
    WHERE created_at_unix_milliseconds < (SELECT unix_milliseconds FROM adr_0033_pipeline_start);

DROP TABLE adr_0033_pipeline_start;

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
