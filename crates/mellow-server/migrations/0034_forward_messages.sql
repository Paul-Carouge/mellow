-- Forwarded messages (ADR-0039): a forward is a copy of another message, made when it was
-- forwarded. These name the message written first; NULL on every other message. No foreign key:
-- the forward stays when the original is deleted.
ALTER TABLE messages ADD COLUMN forwarded_from_channel_id INTEGER;
ALTER TABLE messages ADD COLUMN forwarded_from_message_id INTEGER;

-- A forward's attachments are new rows on the original's sealed files. They take no space on
-- disk, so the upload quota leaves them out: its index now holds uploaded rows only.
ALTER TABLE attachments ADD COLUMN is_forwarded_copy INTEGER NOT NULL DEFAULT 0
    CHECK (is_forwarded_copy IN (0, 1));

DROP INDEX attachments_by_uploader_id;

CREATE INDEX attachments_uploaded_by_uploader_id ON attachments (uploader_id, size_bytes)
    WHERE is_forwarded_copy = 0;

-- Replacing a file by a smaller one moves every row holding it, and a preview reaches every row
-- holding its file: both look rows up by the file they hold.
CREATE INDEX attachments_by_file_id ON attachments (COALESCE(file_id, id));

-- Exactly one row per file counts in a quota. When the counted row goes (its message, channel or
-- guild deleted, cascades included) while copies keep the file, the oldest copy takes it over, in
-- the original's compression state: the file is not free while it stays on disk, and a picture the
-- background compression never tried is queued again from its copy.
CREATE TRIGGER attachments_hand_the_quota_to_a_copy
AFTER DELETE ON attachments WHEN old.is_forwarded_copy = 0
BEGIN
    UPDATE attachments SET is_forwarded_copy = 0,
        optimization_attempted_at_unix_milliseconds =
            old.optimization_attempted_at_unix_milliseconds,
        compression_version = old.compression_version
    WHERE id = (SELECT id FROM attachments
        WHERE COALESCE(file_id, id) = COALESCE(old.file_id, old.id) ORDER BY id LIMIT 1);
END;
