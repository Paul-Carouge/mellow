# 0006. Use TLS in transit and server-side encryption at rest, not E2EE

- Status: Proposed (needs a technical spike on sqlx + SQLCipher)
- Date: 2026-10-19

## Context
Users want privacy, but the MVP also needs server-side moderation, multi-device history and later search. End-to-end encryption for group channels conflicts with all three. The realistic threats for a self-hosted instance are network interception, a stolen disk or VPS snapshot, and a leaked backup.

## Decision
- **In transit:** TLS everywhere through rustls, with built-in ACME certificates.
- **At rest:** the SQLite database is encrypted with SQLCipher, and attachments are encrypted file by file with XChaCha20-Poly1305. Both use keys derived from one 256-bit master key stored in a key file **outside** the data directory, readable only by the service user (mode 0600). `mellow-server init` generates it.
- The server can read message content. This is stated clearly in the documentation.

## Consequences
- A copied data directory or backup without the key file reveals nothing.
- An attacker with full access to the running host can still read data; this is not a defense against a malicious admin.
- Losing the key file means losing the instance. `init` and the admin UI must warn about this and the backup command must remind admins to back up the key separately.
- Small CPU cost on every page read and attachment access; to be measured.
- **Spike required:** confirm that `sqlx` builds against SQLCipher (`libsqlite3-sys` with `bundled-sqlcipher`) on all target platforms. If it does not, fall back to `rusqlite`, and record the change here.

## Spike results (2026-10-19)
- `sqlx` 0.9 works with SQLCipher by adding `libsqlite3-sys` (within the `>=0.30.1, <0.38` range sqlx accepts) with `bundled-sqlcipher`; Cargo feature unification switches the SQLite that sqlx bundles. `PRAGMA cipher_version` is checked at startup so the server refuses to run on plain SQLite.
- Crypto backend: CommonCrypto on Apple platforms (no extra dependency); vendored OpenSSL elsewhere (`bundled-sqlcipher-vendored-openssl`), so the Linux and Windows binaries do not depend on a system libcrypto. This is the only OpenSSL in the build; TLS stays on rustls.
- The raw-key form (`PRAGMA key = "x'…'"`) is used, which skips SQLCipher's passphrase derivation since the master key is already random.
- **Shutdown hazard found:** if a connection is still being closed on a sqlx worker thread while the process exits, SQLCipher crashes in `sqlite3FreeCodecArg` (reproduced on macOS). The server must call `SqlitePool::close().await` during graceful shutdown before returning from `main`.
- **Key leak found and fixed:** sqlx logs connection pragmas, including the key, at DEBUG and at WARN when slow. Statement logging is disabled on database connections and an integration test asserts the key never appears in logs at TRACE level.
- Opening first checks SQLCipher and the key on a single connection that is always closed, before the pool or WAL mode touches the file; only `SQLITE_NOTADB` is reported as a wrong key.
- Operations: core dumps must be disabled for the server (`LimitCORE=0` in the systemd unit), since a dump would contain the key; key file loading must zeroize its buffers (`zeroize`).
- Key schedule (M1.1): the SQLCipher key is HKDF-SHA256(master key, info `mellow/sqlcipher-database-key/v1`), never the stored secret itself. The master key file is created with mode 0600 and `create_new` (never overwritten), and the server refuses to start if it is readable by group or others. Pre-migration `VACUUM INTO` backups are encrypted with the same key (tested).
- Hardening after the M1.1 security audit: the master key file is opened once and checked on its handle (no check-then-read race), read with a size bound, and the master key, derived key and their hex forms are wiped from memory (`zeroize`); `serve` drops the master key as soon as the database key is derived. Residual: sqlx keeps the key pragma string in its connection options for reconnections while the pool lives. Data, key and backup directories are created with mode 0700. The "key outside the data directory" rule is enforced both lexically (no `..`) and on resolved paths (symbolic links, case-insensitive file systems). Windows is refused until a protected DACL can be set on the key file.
- Verified locally on macOS (aarch64). Linux and Windows are verified by the CI test matrix; this ADR becomes *Accepted* once it is green on all three.

## Alternatives considered
- **E2EE everywhere (MLS):** breaks moderation and search, very complex for large groups.
- **E2EE for DMs only:** valuable, but a project of its own; revisit after the MVP.
- **Rely on full-disk encryption only:** does not protect backups copied elsewhere, and depends on the host setup we do not control.
