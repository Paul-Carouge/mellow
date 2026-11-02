-- A picture's size as it shows (EXIF orientation applied), recorded at upload so clients can
-- hold its place before it loads. NULL for anything that is not a picture and for uploads made
-- before this migration: reading every sealed file back would make the update slow, and clients
-- keep their fallback for those.
ALTER TABLE attachments ADD COLUMN width INTEGER;
ALTER TABLE attachments ADD COLUMN height INTEGER;
