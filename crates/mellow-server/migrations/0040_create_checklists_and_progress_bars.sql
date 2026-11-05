-- Checklists and progress bars in messages (ADR-0047). Each belongs to its message and goes with
-- it. `revision` grows with every change, so clients keep the newest copy they saw.
CREATE TABLE checklists (
    message_id INTEGER PRIMARY KEY REFERENCES messages (id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision >= 0),
    -- Item ids are never reused, so a late tick cannot land on an item added after a removal.
    next_item_id INTEGER NOT NULL CHECK (next_item_id >= 1)
) STRICT;

CREATE TABLE checklist_items (
    message_id INTEGER NOT NULL REFERENCES checklists (message_id) ON DELETE CASCADE,
    item_id INTEGER NOT NULL CHECK (item_id >= 1),
    text TEXT NOT NULL,
    -- Who ticked it and when, both or neither.
    checked_by_user_id INTEGER REFERENCES users (id),
    checked_at_unix_milliseconds INTEGER,
    CHECK ((checked_by_user_id IS NULL) = (checked_at_unix_milliseconds IS NULL)),
    PRIMARY KEY (message_id, item_id)
) STRICT, WITHOUT ROWID;

CREATE TABLE progress_bars (
    message_id INTEGER PRIMARY KEY REFERENCES messages (id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    value INTEGER NOT NULL CHECK (value >= 0 AND value <= maximum),
    maximum INTEGER NOT NULL CHECK (maximum >= 1),
    revision INTEGER NOT NULL CHECK (revision >= 0)
) STRICT;

-- SEND_CHECKLISTS (1 << 23, ADR-0047) is new: grant it to every existing @everyone role (the role
-- whose id is its guild's), as new guilds do.
UPDATE roles SET permissions = permissions | 8388608 WHERE id = guild_id;
