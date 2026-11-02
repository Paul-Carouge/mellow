-- A smaller copy of a picture attachment, shown in the message list in its place (ADR-0033): its
-- own sealed file, made by the background compression. NULL when the picture already fits the
-- preview size, when no smaller copy passes, and for uploads made before this migration: the
-- preview route then serves the attachment itself.
ALTER TABLE attachments ADD COLUMN preview_file_id INTEGER;
ALTER TABLE attachments ADD COLUMN preview_content_type TEXT;
ALTER TABLE attachments ADD COLUMN preview_size_bytes INTEGER;
