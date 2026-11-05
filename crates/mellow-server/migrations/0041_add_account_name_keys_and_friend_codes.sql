-- One name per account and a permanent friend code (ADR-0048).
-- `name_key` is the name folded for comparison (NFKC, case folded, accents removed): SQLite cannot
-- fold Unicode, so the server fills it, and the friend codes from the OS random generator, for
-- every account left without them, right after the migrations run (`storage::account_names`).
-- Both stay NULL only until then; unique indexes allow many NULLs.
ALTER TABLE users ADD COLUMN name_key TEXT;
ALTER TABLE users ADD COLUMN friend_code INTEGER CHECK (friend_code >= 0 AND friend_code < 1099511627776);

CREATE UNIQUE INDEX users_by_name_key ON users (name_key);
CREATE UNIQUE INDEX users_by_friend_code ON users (friend_code);
