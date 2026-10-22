-- Deleting a channel (or a whole guild) cascades into read states and attachments by channel id.
-- Neither table could look rows up by channel: read states are keyed by user first, attachments by
-- id, so each cascade scanned the whole table once per deleted channel.

CREATE INDEX read_states_by_channel_id ON read_states (channel_id);

CREATE INDEX attachments_by_channel_id ON attachments (channel_id);
