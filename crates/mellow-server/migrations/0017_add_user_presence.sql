-- The status each user picks for themselves and their custom status (ROADMAP "Next" item 6), kept
-- across sessions and devices. Others see 'invisible' as offline.
ALTER TABLE users ADD COLUMN presence_status TEXT NOT NULL DEFAULT 'online'
    CHECK (presence_status IN ('online', 'idle', 'do_not_disturb', 'invisible'));
ALTER TABLE users ADD COLUMN custom_status TEXT;
-- Bumped by each change, so the live presence never goes back to an older one read concurrently.
ALTER TABLE users ADD COLUMN presence_version INTEGER NOT NULL DEFAULT 0;
