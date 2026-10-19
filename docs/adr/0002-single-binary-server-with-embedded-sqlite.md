# 0002. Ship the server as a single binary with embedded SQLite

- Status: Accepted
- Date: 2026-10-19

## Context
The first product goal is self-hosting that is "hyper simple": no Docker requirement, no external database, simple updates. The target is up to about 10 000 monthly active users on one node. The user referenced PocketBase as the model to follow, but PocketBase is written in Go and the project is 100% Rust.

## Decision
Follow the PocketBase model inside `mellow-server`: one statically linked binary that embeds its migrations, admin UI assets and TLS stack, and stores everything in one data directory with a SQLite database (WAL mode, accessed through `sqlx`) and an attachments folder.

## Consequences
- Installation is: download one file, run `mellow-server init`, run `mellow-server serve`.
- Backups are a consistent snapshot (`VACUUM INTO`) plus the attachments folder.
- SQLite accepts one writer at a time. Writes are serialized through a dedicated writer task and kept short. This fits the target scale but caps vertical growth; we will measure write throughput in load tests before claiming numbers.
- No multi-node deployment. Accepted, see ARCHITECTURE.md non-goals.

## Alternatives considered
- **PocketBase:** Go, violates the 100% Rust constraint, and its collection model does not fit a real-time chat protocol.
- **TrailBase (Rust PocketBase-like):** a second process to run and update, and an imposed data model.
- **Postgres:** better concurrency but forces every self-hoster to install and maintain a database server.
- **SQLite plus optional Postgres:** doubles query and migration maintenance. Revisit only if measurements demand it.
