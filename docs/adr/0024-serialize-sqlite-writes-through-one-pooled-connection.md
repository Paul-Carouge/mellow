# 0024. Serialize SQLite writes through one pooled connection

- Status: Accepted
- Date: 2026-10-23
- Amends: [ADR-0002](0002-single-binary-server-with-embedded-sqlite.md)

## Context
ADR-0002 and ARCHITECTURE §4.3 say that writes go through a dedicated writer task. That task never existed: the server used one sqlx pool of 10 connections for everything, so any connection could write, and concurrent writers polled for SQLite's write lock through `busy_timeout` (5 s). Only message sends were ordered, by a per-process lock. On 2026-10-23 the owner chose to keep SQLite + SQLCipher and fix this first.

## Decision
- `storage::Database` holds two sqlx pools on the same file: a **writer** capped at one connection, and **readers** opened with `PRAGMA query_only = ON`, capped at 4 connections.
- The writer pool's fair (FIFO) acquisition is the write queue. A write waits at most 5 s for it (`acquire_timeout`, the same bound as the former `busy_timeout`), then fails.
- Each storage function picks the pool it needs; domain, HTTP and gateway code only pass `&Database` along. A write sent to a reader fails, so the test suite catches a function filed under the wrong pool.
- No dedicated actor task: it would need every write to be rewritten as a message to that task for the same ordering that one pooled connection already gives.

## Measurements
An ad hoc `performance-optimizer` harness (ignored tests driving the real domain API, not committed), release build, SQLCipher, 1 000 members, Apple M3 with 8 cores, median of 5 runs, quiet machine; `main` at 0c4d4ed against this design with 4 readers:

| Case | Before | After |
|---|---|---|
| Mixed writes and reads, 16 tasks: throughput | 5 062 ops/s | 6 157 ops/s |
| Same: p50 / p99 / max latency | 1.5 / 23.6 / 152 ms | 2.3 / 3.9 / about 12 ms |
| Mixed, 32 tasks: throughput | 5 432 ops/s | 6 131 ops/s |
| Same: p50 / p99 | 3.5 / 29.9 ms | 4.6 / 9.4 ms |
| History page of 100 while 16 tasks write: reads/s, writes/s | 1 131, 2 328 | about 1 520, 3 890 |
| Single send, public channel: p50 (10 readers) | 396 µs | 363 µs |
| Sends while a 50 000-message channel is deleted: p99 (10 readers) | 100 ms | 75 ms |

The queue trades a slightly higher median under contention for a tail bounded by the queue length: no request waits behind a writer sleeping in SQLite's busy handler. With sqlx's default of 10 readers, history reads under write load were 26 % slower than before and throughput 11 to 22 % lower; 4 readers fixed both (2 and 32 were worse). The reader count was measured on one 8-core machine only.

### Note (2026-11-02): `synchronous = FULL` kept, pending a Linux measurement
The writer runs with sqlx's default `synchronous = FULL`: in WAL mode, every commit syncs the WAL. `NORMAL` syncs only at checkpoints: a process crash still loses nothing, but a power loss can drop the last commits since the previous checkpoint, without corrupting the file. The owner decided on 2026-11-01 to switch to `NORMAL` only if a Linux measurement shows a clear gain (at least 20 % on commit latency).

Measured so far on macOS only: 1 000 sequential writer commits, each inserting one 200-byte message-like row in its own transaction, release build, SQLCipher, Apple M3, 5 alternating runs per setting (ad hoc ignored test, not committed). Mean commit latency, median of the 5 runs: **118 µs with `FULL`, 91 µs with `NORMAL` (−23 %)**; p50 103 µs against 79 µs. This does not settle the question: on macOS, SQLite's `fsync` does not flush the drive cache (`PRAGMA fullfsync` is off), so it is far cheaper than an `fsync` on Linux ext4, and the gain on a real Linux disk or an SD card is expected to be larger but is unmeasured. The fsync count was not taken either (no `strace` on macOS).

No Linux machine was available for the measurement (no Docker on the development Mac; the owner's instance host is outward-facing and was not used without asking). To decide: on Linux, time the same loop under `strace -f -c -e trace=fsync,fdatasync` with each setting, on ext4 and ideally on an SD card, then set `.synchronous(SqliteSynchronous::Normal)` in `open_encrypted_database` and amend this ADR only if the gain reaches 20 %.

## Consequences
- No more lock polling between the server's own connections; writes run one after another in arrival order.
- A transaction holds the only writer until it ends: it must never await another write (it would wait for itself until the timeout), nor do slow work such as I/O, hashing or fan-out.
- One long write (deleting a big server, say) delays every other write for its duration; with `acquire_timeout` they fail after 5 s, as they did with `busy_timeout` before.
- Writes are not batched into shared transactions (group commit). Revisit only if measurements show the commit cost dominates.
- Another process writing the same file (`mellow-server admin`) still meets the server through SQLite's lock and `busy_timeout`.

## Alternatives considered
- **A dedicated writer task fed by a channel:** what ADR-0002 described. Same ordering, but every write function becomes a closure sent to the task, with boxed futures and an extra hop per write; it also makes group commit possible, which nothing measured needs yet.
- **Keep one pool and use `BEGIN IMMEDIATE` everywhere:** most transactions already do; it still leaves several connections polling for the lock and nothing stops a read path from writing.
