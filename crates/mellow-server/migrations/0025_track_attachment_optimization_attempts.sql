-- When the background compression tried an attachment (ADR-0021), whatever the outcome. A PNG
-- or GIF still NULL was never tried: its job was dropped by a full queue or a restart, and the
-- worker queues it again once idle. Rows from before this migration are NULL, so uploads whose
-- job was lost earlier are caught up too.
ALTER TABLE attachments ADD COLUMN optimization_attempted_at_unix_milliseconds INTEGER;

CREATE INDEX attachments_awaiting_optimization ON attachments (id)
    WHERE optimization_attempted_at_unix_milliseconds IS NULL
        AND content_type IN ('image/png', 'image/gif');
