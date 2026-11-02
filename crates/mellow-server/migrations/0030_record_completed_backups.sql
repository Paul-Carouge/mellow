-- When `mellow-server backup` finished a backup of this database: `admin recompress-images
-- --confirm` needs a recent one, since recompression keeps no original.
CREATE TABLE completed_backups (
    completed_at_unix_milliseconds INTEGER NOT NULL
) STRICT;
