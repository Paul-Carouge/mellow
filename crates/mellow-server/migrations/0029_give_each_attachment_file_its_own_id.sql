-- An attachment's file gets its own id (ADR-0033): a compressed file is written beside the one it
-- replaces, the row is switched to it in one transaction and the old file is deleted after the
-- commit, so a file never changes in place. NULL: the file is stored under the attachment's id, as
-- every file written before this migration.
ALTER TABLE attachments ADD COLUMN file_id INTEGER;
