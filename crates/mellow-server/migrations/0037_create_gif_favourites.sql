-- GIFs a member starred (ADR-0042): Klipy's slug only, never its media, which the picker loads
-- back from Klipy. At most 200 per member, enforced by the domain.
CREATE TABLE gif_favourites (
    user_id INTEGER NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    slug TEXT NOT NULL CHECK (length(slug) BETWEEN 1 AND 64),
    added_at_unix_milliseconds INTEGER NOT NULL,
    PRIMARY KEY (user_id, slug)
) STRICT, WITHOUT ROWID;
