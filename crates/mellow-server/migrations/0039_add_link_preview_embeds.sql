-- Provider embeds (ADR-0044): what a known provider says about a link, as the protocol's
-- `LinkEmbed` JSON (an object with a `kind`). Previews stored before have none.
-- `IS`, not `=`: a NULL comparison would let an array or an object without a kind through.
ALTER TABLE link_previews ADD COLUMN embed TEXT
    CHECK (embed IS NULL OR (json_valid(embed) AND json_type(embed, '$.kind') IS 'text'));
