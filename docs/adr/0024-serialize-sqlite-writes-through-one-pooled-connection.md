# 0024. Serialize SQLite writes through one pooled connection

- Status: Accepted
- Date: 2026-10-23
- Amends: [ADR-0002](0002-single-binary-server-with-embedded-sqlite.md)

## Context
ADR-0002 and ARCHITECTURE §4.3 say that writes go through a dedicated writer task. That task never existed: the server used one sqlx pool of 10 connections for everything, so any connection could write, and concurrent writers polled for SQLite's write lock through `busy_timeout` (5 s). Only message sends were ordered, by a per-process lock. On 2026-10-23 the owner chose to keep SQLite + SQLCipher and fix this first.

## Decision
- `storage::Database` holds two sqlx pools on the same file: a **writer** capped at one connection, and **readers** opened with `PRAGMA query_only = ON`.
- The writer pool's fair (FIFO) acquisition is the write queue. A write waits at most 5 s for it (`acquire_timeout`, the same bound as the former `busy_timeout`), then fails.
- Each storage function picks the pool it needs; domain, HTTP and gateway code only pass `&Database` along. A write sent to a reader fails, so the test suite catches a function filed under the wrong pool.
- No dedicated actor task: it would need every write to be rewritten as a message to that task for the same ordering that one pooled connection already gives.

## Consequences
- No more lock polling between the server's own connections; writes run one after another in arrival order.
- A transaction holds the only writer until it ends: it must never await another write (it would wait for itself until the timeout), nor do slow work such as I/O, hashing or fan-out.
- One long write (deleting a big server, say) delays every other write for its duration; with `acquire_timeout` they fail after 5 s, as they did with `busy_timeout` before.
- Writes are not batched into shared transactions (group commit). Revisit only if measurements show the commit cost dominates.
- Another process writing the same file (`mellow-server admin`) still meets the server through SQLite's lock and `busy_timeout`.

## Alternatives considered
- **A dedicated writer task fed by a channel:** what ADR-0002 described. Same ordering, but every write function becomes a closure sent to the task, with boxed futures and an extra hop per write; it also makes group commit possible, which nothing measured needs yet.
- **Keep one pool and use `BEGIN IMMEDIATE` everywhere:** most transactions already do; it still leaves several connections polling for the lock and nothing stops a read path from writing.
