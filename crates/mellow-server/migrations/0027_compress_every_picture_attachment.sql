-- The background compression now takes JPEG and WebP attachments too (ADR-0033). Pictures stored
-- before this migration are left as they are, whatever their type: recompressing them is an
-- administrator's decision, never an update's. That includes a PNG or GIF whose job a full queue
-- or a restart had dropped.
UPDATE attachments SET optimization_attempted_at_unix_milliseconds = created_at_unix_milliseconds
    WHERE optimization_attempted_at_unix_milliseconds IS NULL
        AND content_type IN ('image/png', 'image/gif', 'image/jpeg', 'image/webp');

DROP INDEX attachments_awaiting_optimization;

CREATE INDEX attachments_awaiting_optimization ON attachments (id)
    WHERE optimization_attempted_at_unix_milliseconds IS NULL
        AND content_type IN ('image/png', 'image/jpeg', 'image/gif', 'image/webp');
