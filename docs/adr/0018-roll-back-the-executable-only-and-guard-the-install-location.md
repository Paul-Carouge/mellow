# 0018. Roll back the executable only, and refuse unsafe install locations

- Status: Accepted
- Date: 2026-10-20
- Amends: [ADR-0003](0003-self-update-with-signed-releases.md)

## Context
ADR-0003 promised that `mellow-server update --rollback` restores "the previous binary and the pre-migration snapshot", and that the executable is replaced atomically. Implementing it (crate `mellow-update`, shared with the desktop app per ADR-0017) showed three problems. Restoring the database automatically would throw away every message written since the update, and the server cannot tell whether that is what the admin wants. A rename-aside-then-rename-in replacement leaves a moment without an executable, which a service manager restarting at that moment fails on. And `sudo mellow-server update` runs as root on files that a less privileged account may be able to change.

## Decision
- `--rollback` puts back `<executable>.previous` only. The database is never restored automatically: the server writes `backups/pre-migration-<version>.db` before migrating and logs `database migration starting` at warn level first; restoring is a documented manual step (stop the server, set `mellow.db`, `mellow.db-wal` and `mellow.db-shm` aside, copy the backup to `mellow.db`). The Docker kit's `mellow-update` never rolls back once a pre-migration backup appeared during the update.
- On Unix the replacement is atomic: the old executable is hard-linked to `.previous`, then the new file is renamed over it. On Windows, where a running executable cannot be overwritten, it is renamed aside first, and a failure to put it back is reported with where the original now is.
- Before replacing anything, the updater refuses an executable or a directory holding it that is a symbolic link, owned by someone other than the updating user or root, or writable by group or others. The new file gets the old one's mode and owner through its open descriptor, never through its path.
- Release assets are downloaded from URLs the updater builds (`github.com/<repository>/releases/download/v<version>/<asset>`), not from the URLs the release API returns.

## Consequences
- An admin who wants the old version back after a migration restores the backup by hand, knowingly, losing what was written since.
- A native install must keep the executable in a root-owned location (`/usr/local/bin`, mode 0755) and run the service as a dedicated account that cannot write there; other layouts are refused with the reason.
- A second update before a restart replaces `.previous` with a version that never ran.

## Alternatives considered
- **Automatic database restore on rollback**: silent loss of recent data.
- **Keeping every previous executable**: unbounded disk use for little benefit; one step back covers a failed update.
