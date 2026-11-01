-- The upload quota sums an uploader's attachment sizes on every upload; with the size in the
-- index it is answered from the index alone instead of scanning every attachment.
CREATE INDEX attachments_by_uploader_id ON attachments (uploader_id, size_bytes);
