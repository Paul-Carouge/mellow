-- Files shared in messages (ADR-0020). The file itself is sealed on disk under
-- attachments/<two hex digits>/<id>; this row holds what the API answers. An upload not linked to
-- a message within an hour is deleted with its file.
CREATE TABLE attachments (
    id INTEGER PRIMARY KEY,
    channel_id INTEGER NOT NULL REFERENCES channels (id) ON DELETE CASCADE,
    uploader_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    message_id INTEGER REFERENCES messages (id) ON DELETE CASCADE,
    file_name TEXT NOT NULL,
    content_type TEXT NOT NULL,
    size_bytes INTEGER NOT NULL CHECK (size_bytes >= 0),
    created_at_unix_milliseconds INTEGER NOT NULL
) STRICT;

CREATE INDEX attachments_by_message_id ON attachments (message_id);
CREATE INDEX attachments_unlinked_by_creation ON attachments (created_at_unix_milliseconds)
    WHERE message_id IS NULL;
