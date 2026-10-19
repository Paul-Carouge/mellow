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

## Alternatives considered
- **E2EE everywhere (MLS):** breaks moderation and search, very complex for large groups.
- **E2EE for DMs only:** valuable, but a project of its own; revisit after the MVP.
- **Rely on full-disk encryption only:** does not protect backups copied elsewhere, and depends on the host setup we do not control.
