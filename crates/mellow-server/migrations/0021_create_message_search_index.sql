-- Full-text index of message content for search (ADR-0029). An external-content FTS5 table: the
-- text stays once, in messages, and the index holds only its tokens, read back from messages for
-- highlights. Case and accents are ignored; two- and three-character prefixes are indexed so
-- type-ahead prefixes stay cheap.
CREATE VIRTUAL TABLE messages_fts USING fts5(
    content,
    content = 'messages',
    content_rowid = 'id',
    tokenize = 'unicode61 remove_diacritics 2',
    prefix = '2 3'
);

-- Deleted and edited text leaves the index at once instead of lingering in older index segments
-- until a merge, as secure_delete does for the rest of the file (ADR-0006).
INSERT INTO messages_fts (messages_fts, rank) VALUES ('secure-delete', 1);

-- Kept in step by the write transactions themselves, whatever path writes messages (deleting a
-- channel or a guild deletes its messages by cascade, which fires these too). Never write
-- `messages` with REPLACE (`INSERT OR REPLACE`, `REPLACE INTO`): the rows it deletes fire no
-- trigger, and their words would stay in the index.
CREATE TRIGGER messages_fts_after_insert AFTER INSERT ON messages BEGIN
    INSERT INTO messages_fts (rowid, content) VALUES (new.id, new.content);
END;

CREATE TRIGGER messages_fts_after_delete AFTER DELETE ON messages BEGIN
    INSERT INTO messages_fts (messages_fts, rowid, content) VALUES ('delete', old.id, old.content);
END;

-- Only content changes: pinning and other updates leave the index alone.
CREATE TRIGGER messages_fts_after_content_update AFTER UPDATE OF content ON messages BEGIN
    INSERT INTO messages_fts (messages_fts, rowid, content) VALUES ('delete', old.id, old.content);
    INSERT INTO messages_fts (rowid, content) VALUES (new.id, new.content);
END;

-- `from:` filters walk one author's messages, newest first, instead of a whole server's.
CREATE INDEX messages_by_author_id ON messages (author_id, id);

-- Indexes every message written before this migration.
INSERT INTO messages_fts (messages_fts) VALUES ('rebuild');
