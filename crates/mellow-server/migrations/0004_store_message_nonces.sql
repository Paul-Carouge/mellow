-- An outbox retry after a lost response resends the same nonce: the server answers with the
-- message it already created instead of a duplicate (ARCHITECTURE.md §4.5).
ALTER TABLE messages ADD COLUMN nonce TEXT;

CREATE UNIQUE INDEX messages_by_channel_author_nonce ON messages (channel_id, author_id, nonce)
    WHERE nonce IS NOT NULL;
