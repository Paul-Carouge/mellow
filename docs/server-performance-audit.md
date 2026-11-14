# Server performance audit — `mellow-server` (2026-11-13)

Phase 1 (§1 to §8): measure and diagnose only, no code changed. Phase 2 (§9, 2026-11-14): lots 1 to
5 and 8 applied, with the numbers after.
Every number below was observed on a release build; nothing here is estimated unless it says so.

## 1. Setup and method

- **Binary**: `cargo build --release -p mellow-server --features web-client` (Cargo's default
  release profile: the workspace has no `[profile.release]`, so `lto = false`,
  `codegen-units = 16`). Two variants were built through environment variables only, to measure
  without touching the repository:
  - `CARGO_PROFILE_RELEASE_LTO=fat CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1` ("LTO");
  - `LIBSQLITE3_FLAGS="-DSQLITE_DEFAULT_MEMSTATUS=0"` ("MEMSTATUS=0").
- **Instance**: an isolated one (`init` as in the README, `listen_address = 127.0.0.1:18090`,
  `registration_mode = "open"`, `guild_creation = "everyone"`, voice on UDP 50090), plain HTTP on
  loopback, data on tmpfs. Stopped with SIGINT/SIGTERM every time.
- **Data**: 500 accounts in one guild of 50 text channels; 34 475 messages (about 690 per channel,
  4 to 18 words each); user 1 also owns 27 more guilds of 50 channels (28 guilds, 1 400 visible
  channels). Database: 22 MB (5 414 pages of 4 KiB).
- **Load**: a Rust load generator written for the audit (outside the repository, see §8): open-loop
  sends with a fixed schedule (so latencies include queueing), closed-loop reads with N workers,
  N WebSocket clients that time each `MESSAGE_CREATE` from the moment its POST left. Each client
  and user binds its own `127.x.y.z` source address, so per-address limits behave as with real
  users. Rate limits were respected, except where a run says it measured them.
- **Server-side**: CPU and memory from `/proc/<pid>/stat`, `/proc/<pid>/status` and per-thread
  `/proc/<pid>/task/*/stat`; `EXPLAIN QUERY PLAN` through a SQLCipher connection with the derived
  key; a sampling profiler made of gdb (the server started as gdb's child, since Yama
  `ptrace_scope = 1` forbids attaching) printing every thread's stack on SIGUSR2, 120 to 150
  samples per scenario; `malloc_trim(0)` called from gdb on SIGUSR1 to tell retained memory from
  live memory. `perf`, `heaptrack`, `oha` and `wrk` are not installed.

**Caveat on noise**: other agents were compiling on the same 8-core machine (load average 2 to 17,
swap full). Throughput comparisons were made back to back on the same data, during a quiet window
(load average 2 to 4), and the baseline was run twice: the two baseline runs differ by up to 6 %.
Only differences well above that are reported as gains. Latencies taken under load average > 10
are marked "(loaded)".

## 2. Baseline numbers

### HTTP routes (quiet window, two baseline runs, closed loop, 16 workers unless stated)

| Route | Response | Throughput | p50 | p99 | Server CPU |
|---|---|---|---|---|---|
| `GET /channels/{id}/messages?limit=50` | 16.2 KB | 3 288 – 3 501 /s | 4.5 – 4.8 ms | 5.8 – 6.2 ms | 3.9 cores |
| same, one worker (loaded) | | 1 114 /s | 0.56 ms | 4.4 ms | |
| same with `before=` cursor (loaded) | 16.3 KB | 1 691 /s | 9.3 ms | 18 ms | |
| `GET /guilds/{id}` (50 channels) | 9.3 KB | 5 916 – 6 257 /s | 2.5 – 2.7 ms | 3.2 – 3.5 ms | 4.1 cores |
| `GET /guilds/{id}/members` (500 members) | 71.5 KB | 1 084 – 1 147 /s | 13.7 – 14.5 ms | 18.6 – 19.6 ms | 4.3 cores |
| `GET /users/@me/read-states` | 2 B | 5 348 – 5 698 /s | 2.7 – 2.9 ms | 3.5 – 3.7 ms (p99.9 52 – 55 ms) | 4.2 cores |
| `PUT /channels/{id}/read-state` (loaded, rate limit reached) | 204 | 2 500 /s accepted | 1.1 ms | 2.6 ms | |
| `GET /users/@me/guilds` (loaded) | 59 B | 15 079 /s | 0.68 ms | 4.2 ms | |
| `POST /guilds/{id}/messages/search` `release` (about 7 000 matches), 4 workers | 11.5 KB | 143 – 161 /s | 24 – 28 ms | 28 – 32 ms | 2.2 cores |
| `GET /health` (one `SELECT 1`, loaded) | 34 B | 103 225 /s | 0.13 ms | 0.65 ms | 1.6 cores |
| `POST /channels/{id}/attachments`, 640 × 480 PNG of 485 KB | 123 B | — | 2.7 ms | 40 ms (n = 10) | then 1.8 s of CPU per picture in the background worker |

### Sending and gateway fan-out (500 WebSocket clients online in the 50-channel guild)

| Load | POST p50 / p99 | Frames out | Delivery to the last client p50 / p99 | Server CPU |
|---|---|---|---|---|
| 100 msg/s, no client online | 0.73 – 0.86 ms / 5.5 – 9.2 ms | — | — | 7 % of a core |
| 125 msg/s for 4 min, no client (loaded) | 0.83 ms / 7.8 ms | — | — | 9 % |
| 20 msg/s | 3.4 ms / 5.9 ms | 10 000 /s | 4.4 ms / 7.1 ms | 20 % |
| 100 msg/s | 1.5 ms / 2.7 ms | 49 900 /s | 2.1 ms / 3.4 ms | 43 % |
| 145 msg/s (the per-channel limits' ceiling over 50 channels) | 1.8 ms / 3.7 ms | 72 400 /s | 2.7 ms / 4.4 ms | 88 % |

About 9 to 12 µs of CPU per delivered frame. The fan-out is not a bottleneck at this scale.

### Identify and `READY`

| Case | Time from `identify` to `READY` | `READY` size |
|---|---|---|
| 1 guild, 50 channels, nobody else online | 1.2 – 16 ms (median of 5: 4.6 ms) | 16.4 KB |
| same, 500 members online | — | 52 KB (+70 B per online member: presences and `online_user_ids`) |
| 28 guilds, 1 400 channels | 11 – 34 ms (8 runs) | 261 755 B |
| 30 guilds, 1 500 channels | **fails**: `invalid_session` every time | about 270 KB |
| 500 clients identifying at once | all ready in 1.1 – 2.5 s; p50 83 – 300 ms, p99 175 – 650 ms | |

### Memory and startup

| State | RSS |
|---|---|
| Started, idle | 22 – 25 MB |
| 500 live gateway sessions, idle | +86 MB, about 170 KB per session |
| At rest after gateway load, every session expired (no socket left) | 163 – 174 MB; `malloc_trim(0)` brings it to 41 MB |
| 500 logins (Argon2id, 16 at a time): peak / 15 s later | 381 – 589 MB / 247 – 524 MB; after `malloc_trim(0)`: 222 MB |
| same with `MALLOC_MMAP_THRESHOLD_=131072` | 193 MB / **22.6 MB** |

- Time to the first healthy `/api/v1/health`: 5 – 19 ms with the 22 MB database (once 102 ms,
  the first start after an unclean data change), 17 ms on an empty database.
- Binary: 52.7 MB (LTO: 40.3 MB).

### Query plans

`EXPLAIN QUERY PLAN` of the hot queries (history page, read states, members and their roles,
membership, conversation participants, unread mention counts) shows index searches only: no full
scan and no missing index. The history query runs in 0.5 ms in SQL. The remaining
`USE TEMP B-TREE` steps sort at most a page (50 rows) or the 1 000 listed members. The schema
already has a query-plan test for the unread-mention query.

### Response sizes (no compression today)

API responses carry `content-length` with no `content-encoding`. Measured gzip ratios:

| Body | Plain | gzip level 1 | gzip level 6 |
|---|---|---|---|
| History page (50 messages) | 16 201 B | 3 213 B | 2 356 B (÷ 6.9) |
| Members (500) | 71 501 B | 10 081 B | 6 413 B (÷ 11) |
| Guild (50 channels) | 9 331 B | 1 158 B | 850 B (÷ 11) |

The gateway has no compression either (axum's WebSocket, tungstenite 0.29, has no
permessage-deflate), so a 262 KB `READY` is 262 KB on the wire.

## 3. Findings, by expected gain

### F1. `READY` above 256 KiB can never be sent: the user is locked out (correctness)

- `crates/mellow-server/src/gateway/mod.rs:51` sets `MAXIMUM_WRITE_BUFFER_BYTES = 256 * 1024`,
  applied at `:151` (`max_write_buffer_size`). tungstenite refuses to queue a message that would
  exceed it (`WriteBufferFull`, tungstenite 0.29 `protocol/mod.rs:279`), even when the buffer is
  empty. `send_frame` for `READY` (`gateway/mod.rs:400`) then fails and the client gets
  `invalid_session`; nothing is logged. A client that reconnects gets the same answer forever.
- Measured: 261 755 B goes through; one more guild of 50 channels (about 270 KB) fails, every time.
  With 500 members online, user 1's `READY` grew past the limit again, through presences alone.
- Thresholds: about 180 B per visible channel and 70 B per online member of any shared guild.
  So about 1 450 visible channels, or about 3 500 members online in one guild: past that, **every**
  member of that guild is refused at identify, whatever else they are in.
- Any single dispatch above 256 KiB (a large `GUILD_CREATE`) hits the same limit.

### F2. SQLite's global memory-statistics mutex serialises every read (about ×2 throughput)

- Profile of the history route under load: 45 % of the active thread samples are in
  `pthread_mutex_lock` (34.8 %) and `pthread_mutex_unlock` (10.2 %), called from `sqlite3Malloc`,
  `sqlite3_free`, `sqlite3_value_dup` (sqlx's `ExecuteIter` copies every column of every row) and
  the drop of `SqliteRow`. With `SQLITE_DEFAULT_MEMSTATUS=1` (SQLite's default, kept by
  `libsqlite3-sys`'s bundled build), each allocation and free takes the one process-wide
  `mem0.mutex`, shared by the writer, the 4 readers and the Tokio workers dropping rows.
- A/B, same data, back to back, quiet machine:

| Route | Baseline | MEMSTATUS=0 | Change |
|---|---|---|---|
| History page | 3 501 /s, p50 4.51 ms | 7 031 /s, p50 2.22 ms | ×2.0 |
| Guild | 6 257 /s, p50 2.54 ms | 13 916 /s, p50 1.13 ms | ×2.2 |
| Members | 1 147 /s, p50 13.7 ms | 2 228 /s, p50 7.1 ms | ×1.9 |
| Read states | 5 698 /s, p50 2.68 ms | 12 212 /s, p50 1.24 ms | ×2.1 |
| Search (CPU-bound in FTS5) | 161 /s | 163 /s | none |
| 500 identifies at once, p50 | 83 ms | 59 ms | −29 % |

  Server CPU was the same (about 4 cores), so the CPU spent per request halved.
- Where: `crates/mellow-server/src/storage/database.rs:144` opens the pools; the define has to
  reach the bundled SQLCipher build. `PRAGMA cipher_memory_security` is already 0 (SQLCipher
  4.10.0), so SQLCipher's own allocator wrapper is not the cause.

### F3. Freed memory stays with glibc: 120 MB after gateway load, 220 – 500 MB after logins

- After gateway load with every session expired, RSS stays at 163 – 174 MB; `malloc_trim(0)`
  takes it to 41 MB. That is free memory glibc keeps in its per-thread arenas, not a leak.
  `MALLOC_ARENA_MAX=2` barely helps (174 → 156 MB) and costs about 8 % of read throughput.
- Logins: each Argon2id hash allocates 19 MiB (`crates/mellow-server/src/domain/password.rs:33`,
  one slot per core). glibc's dynamic mmap threshold rises to 19 MiB after the first free, so later
  hashes are carved out of thread arenas and kept: 500 logins left 247 – 524 MB resident.
  With the threshold fixed (`MALLOC_MMAP_THRESHOLD_=131072`), the peak is 193 MB (8 slots × 19 MiB
  plus the base) and RSS goes back to 22.6 MB. The README's "1.5 GB at peak, 2 GB recommended"
  does not account for this retention.

### F4. Each WebSocket keeps a 128 KiB read buffer fully resident (about 120 KB per connection)

- `crates/mellow-server/src/gateway/mod.rs:148-152` sets the frame and write limits but not
  `read_buffer_size`, so tungstenite's default of 128 KiB applies. Its `read_in`
  (tungstenite 0.29 `protocol/frame/mod.rs:236`) zero-fills the whole capacity before every read,
  which makes all 128 KiB resident for every connection.
- Measured: 500 live sessions cost 86 MB (about 170 KB each). At the 10 000-connection cap that is
  about 1.7 GB, most of it in these buffers. Client frames are at most 4 KB before identify and
  32 KB after.

### F5. JSON responses are sent uncompressed (÷ 7 to ÷ 11 on the wire)

- The middleware stack (`crates/mellow-server/src/http/mod.rs:531-532`) has no compression; only
  the embedded web client's static files are precompressed (ADR-0019). On a phone link the history
  page (16 KB → 2.4 KB), the member list (72 KB → 6.4 KB) and `READY` (up to 262 KB) dominate
  loading time, not server time.

### F6. Sequential round trips per request (latency floor, CPU per request)

- A history page makes about 11 sequential reader queries, each a separate pool acquisition, a
  thread hop to the sqlx worker and a separate read snapshot: session (`domain/accounts.rs:311`),
  conversation participants and the channel and membership (`domain/messages.rs:513`,
  `domain/guilds/channels.rs:106-110`), the page (`storage/messages.rs:441`), then attachments,
  profile images, reactions, polls, trackers and link previews (`domain/messages.rs:874-897`),
  even when the page holds no poll, tracker or link. All are batched (no N + 1).
- `READY` reads each guild separately: `gateway/mod.rs:418` calls `get_guild`
  (`domain/guilds.rs:454-466`: membership, channels, roles, expressions, join sound), plus
  authentication twice, presence, friends and the guild list twice
  (`gateway/mod.rs:293-350`). For 28 guilds that is about 150 sequential reads (11 – 34 ms).
- Not a problem for a small community once F2 is fixed (history p50 2.2 ms at 7 000 pages/s); it
  matters for reconnect storms after a restart or an update (500 identifies: p50 59 – 83 ms, p99
  128 – 175 ms on a quiet machine).

### F7. Release profile: no measurable speed gain from LTO

- Fat LTO with one codegen unit: history 3 257 /s against 3 288 – 3 501 /s, guild 5 979 against
  5 916 – 6 257: within noise. The binary is 23 % smaller (52.7 → 40.3 MB), which helps
  `mellow-server update` downloads; the build took 3 min 30 s against 2 min 40 s (both under load).

### Inconclusive: sends during a read flood

At load average 2, 100 sends/s alongside a 16-worker history flood once gave send p50 26 ms and
p99 109 ms, and history fell from 3 500 to 748 pages/s at 48 % CPU. Re-run under heavier machine
load, the same scenario gave send p50 4 – 8 ms, and the gdb samples showed the SQLite threads idle
70 % of the time with no thread sleeping in SQLite's busy handler or WAL retry loop, so no
server-side lock could be identified. Two plausible contributors, to be checked on a quiet
machine after F2: the global send lock (`domain/messages.rs:646`) is held across reader queries
(nonce lookup, reply check), so sends queue behind a saturated reader pool; and every commit makes
each reader discard its 32 MiB page cache at its next transaction (WAL change detection), so pages
are decrypted again.

## 4. Plan: independent lots, highest gain first

Each lot can be merged alone. "Proof" is what the PR must show (CLAUDE.md "Measure": numbers
before and after).

### Lot 1 — `READY` and large dispatches above 256 KiB (P0, correctness)
- **Files**: `crates/mellow-server/src/gateway/mod.rs` (`MAXIMUM_WRITE_BUFFER_BYTES` at :51, the
  upgrade at :148-152, `send_frame` at :167), `ARCHITECTURE.md` §4.8 (the "256 KB write buffer"
  sentence).
- **Change**: let one frame be as large as the largest legitimate `READY` while keeping the backlog
  bounded. Smallest option: raise the per-socket limit to a documented bound (for example 2 MiB,
  sized from the channel and presence caps), keeping the 512-event session queue and the 10 s
  write timeout as the real backpressure. Log a warning when a frame is refused, so this never
  fails silently again. The lasting fix (lazy presences, or a compressed `READY`) changes the
  protocol: see §6.
- **Proof**: a gateway test where a user's guilds make `READY` larger than 256 KiB (count bytes,
  no timing) receives `ready`, not `invalid_session`; the same test with 3 600 online presences in
  one guild. Measurement: identify of the 30-guild user succeeds; memory of 500 sessions unchanged.

### Lot 2 — `SQLITE_DEFAULT_MEMSTATUS=0` (×2 read throughput, about half the CPU per read)
- **Files**: `.cargo/config.toml` (`[env] LIBSQLITE3_FLAGS = "-DSQLITE_DEFAULT_MEMSTATUS=0"`),
  `.github/workflows/*.yml` and `deploy/docker/Dockerfile` only if they build outside the workspace
  root (they must see the setting), ADR-0024 or ADR-0006 amendment.
- **Change**: compile the bundled SQLCipher without memory statistics. No Rust change; no
  `unsafe` (`sqlite3_config(SQLITE_CONFIG_MEMSTATUS, 0)` would need FFI before the first
  connection). Nothing in the server reads `sqlite3_memory_used` or relies on a soft heap limit.
- **Proof**: a unit test that `PRAGMA compile_options` on an opened database reports
  `DEFAULT_MEMSTATUS=0` (so a build that loses the flag fails CI); A/B as in F2 (history, guild,
  members, read states at 16 workers), expected about ×2.
- **Then**: re-measure `READER_CONNECTION_COUNT` (`storage/database.rs:20`, 4, set by ADR-0024
  under the old contention) with 4, 6 and 8 readers and a concurrent writer.

### Lot 3 — Give freed memory back to the system (−120 MB at rest after load, −220 to −500 MB after logins)
- **Files**: the README's systemd section and `deploy/docker/Dockerfile` (`ENV`), first; then,
  depending on §6, `crates/mellow-server/src/main.rs`.
- **Change, step 1 (no code)**: `Environment=MALLOC_MMAP_THRESHOLD_=131072` in the documented unit
  and `ENV MALLOC_MMAP_THRESHOLD_=131072` in the image, so Argon2's 19 MiB blocks are unmapped when
  freed. Measured: 500 logins leave 22.6 MB instead of 247 MB. Step 2 (owner's choice, §6): a
  periodic trim or another allocator for the gateway's small allocations.
- **Proof**: the login burst and the gateway-load-then-expiry scenarios (§2), RSS at rest before
  and after; read throughput unchanged within noise.

### Lot 4 — Smaller WebSocket read buffer (about −120 KB per connection, expected)
- **Files**: `crates/mellow-server/src/gateway/mod.rs:148-152`.
- **Change**: `.read_buffer_size(8 * 1024)` on the `WebSocketUpgrade` (above the 4 KB pre-identify
  limit; larger frames up to 32 KB still arrive, over several reads).
- **Proof**: existing gateway tests for 32 KB voice offers and the 4 KB pre-identify limit still
  pass; measurement: RSS of 500 held sessions minus idle RSS, expected from about 170 KB to about
  50 KB per session (this gain is inferred from tungstenite's code, not measured yet).

### Lot 5 — Compress JSON API responses (÷ 7 to ÷ 11 bytes on the wire)
- **Files**: `crates/mellow-server/src/http/mod.rs` (one layer beside `mark_api_responses_private`),
  `crates/mellow-server/Cargo.toml` if a crate is added.
- **Change**: gzip (or Brotli) for `/api/` responses above about 1 KiB when the client accepts it,
  with `Vary: Accept-Encoding`. Exclude the routes that return a session token or a TOTP secret
  (`/api/v1/auth/*`, 2FA setup): BREACH needs a secret and attacker-chosen text in the same
  compressed body, and message content is attacker-chosen. File routes already serve stored,
  compressed media and stay as they are. `flate2` is already in the dependency tree; tower-http's
  `CompressionLayer` would add `async-compression` (needs `cargo deny` and a PR reason).
- **Proof**: an HTTP test: history with `Accept-Encoding: gzip` comes back gzip-encoded and decodes
  to the same JSON, login never does; bytes per route before and after; CPU per request (gzip
  level 1 is enough: 16 KB → 3.2 KB).

### Lot 6 — Fewer round trips for `READY` (reconnect storms)
- **Files**: `crates/mellow-server/src/gateway/mod.rs:293-520`, `crates/mellow-server/src/domain/guilds.rs:454`,
  the storage functions it calls.
- **Change**: read channels, roles, expressions and join sounds for all of the user's guilds in one
  query each (`json_each` of the guild ids, as `last_message_ids` already does), and the guild list
  once instead of twice where the re-read is not needed for the race it closes.
- **Proof**: a test counting storage calls per identify (constant in the number of guilds, not
  linear); measurement: identify of the 28-guild user and the 500-client storm, p50 and p99.

### Lot 7 — Fewer round trips per history page (only if Lot 2 is not enough)
- **Files**: `crates/mellow-server/src/domain/messages.rs:847-899`, `storage/messages.rs:441`.
- **Change**: skip the poll, tracker and link-preview reads when the page has none (flags from the
  page query), and run the page's reads in one read transaction on one reader connection (one
  acquisition, one snapshot).
- **Proof**: a test counting queries for a page without polls, trackers or links; A/B of the
  history route.

### Lot 8 — Release profile (binary size)
- **Files**: `Cargo.toml` (`[profile.release]`), CI timing.
- **Change**: `lto = "fat"`, `codegen-units = 1` (or `lto = "thin"` to keep build times), for a
  23 % smaller download; no speed gain was measured.
- **Proof**: binary size and build time in CI; the A/B of §3 F7.

## 5. Left alone, and why

- **Gateway fan-out** (`events.rs`, `gateway/session.rs`): serialised once, no lock held while
  serialising, bounded queues; 72 000 frames/s at 88 % of one core with p99 delivery 4.4 ms.
- **Indexes**: every hot query is an index search (§2); no migration needed.
- **Send path**: p50 under 2 ms at the rate limits' ceiling; the global send lock is by design
  (ordering, ARCHITECTURE §4.3). Revisit only if the inconclusive mixed-load case reproduces.
- **Search**: CPU-bound in FTS5, capped counting, a concurrency limit and its own 1-million-message
  measurement already exist (ADR-0029); 24 – 28 ms here.
- **Picture compression**: 1.8 s of CPU per 640 × 480 picture is the visually lossless pipeline
  (ADR-0033), in one background worker; upload latency itself is 2.7 ms.
- **Startup**: 5 – 19 ms.
- **Allocations**: not counted (no heaptrack); the profile shows where they hurt (F2).

## 6. Decisions for the owner

1. **Large `READY`**: raise the per-socket write limit (simple, more memory per slow socket in the
   worst case) or change the protocol (presences sent lazily per guild as Discord does, or a
   compressed `READY` the client asks for at identify, in both clients). Recommendation: raise the
   limit now (Lot 1), plan lazy presences before any community nears 3 000 members online.
2. **Allocator**: after Lot 3 step 1, either a periodic `malloc_trim(0)` (glibc only, needs one
   `unsafe` FFI call and an `allow(unsafe_code)`) or a global allocator crate (mimalloc or
   jemalloc: a new dependency, also faster under thread contention). Recommendation: measure
   mimalloc on the same scenarios before choosing; it also removes the glibc arena lock seen in the
   profile (`__lll_lock_wait_private`).
3. **Gateway compression**: only through a protocol change (tungstenite has no permessage-deflate);
   worth it for phones once `READY` is large.
4. **Keep the load generator**: it lives in the auditor's temporary directory today. Committing it
   (for example `dev/loadgen` or an `xtask bench-server`) would let each performance PR show the
   same numbers; it uses only crates already in the workspace.

## 7. What the numbers mean for a small community

For 50 to 500 members, the server has a wide margin today: 500 online members receiving 100
messages per second cost under half a core, and a history page takes under 5 ms. The risks are
not throughput: they are the hard `READY` cliff (F1), memory that does not come back after a burst
(F3, F4) on the 2 GB hosts the README recommends, and bytes on mobile links (F5).

## 8. Reproducing

The load generator and scripts used here were in
`/tmp/claude-1000/-home-paul-Documents-Dev-Rust/5f0ae007-83c7-4168-bb9c-f454814fd1d7/scratchpad/server-perf/`
(temporary): `loadgen/src/main.rs` (subcommands `seed`, `send`, `bench --kind history|history-before|guild|members|read-states|ack|search|upload|login`,
`ready`, `hold`, `sql`), `ab.sh` (one block of measures per binary), `mem.sh` (load, wait for
session expiry, RSS at rest), `mixed.sh`, `gdbprof.py` and `agg.py` (sampling profiler). Main
commands:

```bash
loadgen seed --users 500 --heavy-guilds 29 --out state.json
loadgen send --rate 125 --duration 240 --senders 250            # fills the history
loadgen bench --kind history --concurrency 16 --duration 8 --pid <server pid>
loadgen send --ws 500 --rate 100 --duration 15 --senders 300 --first-sender 2 --pid <pid>
loadgen ready --user 1 --repeat 8                               # heavy READY
loadgen ready --clients 500 --concurrency 500                   # identify storm
loadgen bench --kind login --concurrency 16 --max 500 --pid <pid>
```

## 9. After the fixes (2026-11-14)

Same instance, data and load generator as §2 (the database had grown to 31 MB with the messages of
the earlier runs), release builds without `web-client`. Every A/B ran back to back; measures run
`nice -n 15` while other projects compiled are marked "(loaded)". "Before" is `main` at
`34dc2be` (plus Lot 1, which does not touch these paths); "after" has lots 1, 2, 4 and 5.

### Lot 1 — `READY` above 256 KiB

- Test `gateway::tests::a_ready_larger_than_the_former_256_kib_write_buffer_still_arrives`: six
  servers of 50 channels with 1 024-character topics give a `READY` of about 360 KB; with the
  former limit the client got `invalid_session` (the test fails), now it gets `ready`.
- Limit: 8 MiB per server frame, the desktop client's own limit. Estimate of the largest
  realistic `READY`: online members are bounded by the 10 000 connections (70 B each, 0.7 MB) and a
  heavy account in 100 guilds of 50 channels sees 5 000 channels (180 B each, 0.9 MB), so about
  2 MB with relationships and read states; 8 MiB leaves four times that. A larger frame is not sent
  and logs `gateway frame over the size limit was not sent` with its size, nothing else.
- Memory: each send waits for its flush, so a socket buffers one frame at a time; but tungstenite
  never shrinks its output buffer, so every socket keeps its `READY`'s size until it closes (part of
  the 92 KB per session below). With presences in `READY`, that is quadratic in a large guild's
  online members (3 500 online: about 250 KB each, 875 MB in all). Hence a process-wide budget
  (security review): beyond 256 KiB per socket, the capacity a socket keeps counts against
  512 MiB; a socket that would exceed what is left is closed with 1013 (`server_busy`) and its
  client retries with backoff (test
  `large_frames_share_a_memory_budget_that_closing_sockets_give_back`). A light `READY` with one
  `GUILD_CREATE` per server, or lazy presences, remains the lasting fix (roadmap).

### Lot 2 — `SQLITE_DEFAULT_MEMSTATUS=0`

| Route (16 workers, 6 s) | Before | After | Change |
|---|---|---|---|
| History page | 3 368 – 3 380 /s, p50 4.60 ms | 6 723 – 6 808 /s, p50 2.28 – 2.30 ms | ×2.0 |
| Guild | 6 057 – 6 096 /s, p50 2.60 ms | 13 376 – 13 538 /s, p50 1.16 ms | ×2.2 |
| Members (500) | 1 151 – 1 187 /s, p50 13.2 – 13.6 ms | 2 367 – 2 369 /s, p50 6.6 ms | ×2.0 |
| Read states | 5 396 – 5 411 /s, p50 2.8 ms | 11 210 – 11 478 /s, p50 1.3 ms | ×2.1 |
| 500 identifies at once, p50 / p99 | 83 ms / 172 ms (a second run, loaded: 145 / 345 ms) | 51 – 64 ms / 128 – 137 ms | −25 to −40 % |

Server CPU stayed at about 4 cores, so the CPU per request halved. A third "after" run during a
compile elsewhere (loaded) gave 5 378 history pages/s. The define reaches the release workflow
(it builds from the workspace root, so `.cargo/config.toml` applies) and the Docker image (it
installs that release binary); libsqlite3-sys appends `LIBSQLITE3_FLAGS` to its own defines, so
SQLCipher's (`SQLITE_HAS_CODEC`, `SQLITE_TEMP_STORE=2`, the extra init) are kept.
`strings` on the binaries: `DEFAULT_MEMSTATUS=0` absent before, present after.

### Lot 3 — memory given back (`MALLOC_MMAP_THRESHOLD_`, `MALLOC_TRIM_THRESHOLD_`) and allocators

Scenario: two storms of 500 identifies, 100 messages/s to 500 sockets for 10 s, a history flood,
then 150 s for every session to expire ("rest"), then 500 sign-ins at 16 at a time (Argon2id) and
RSS 15 s later. Lots 1, 2, 4 and 5 in every run; alternative allocators through `LD_PRELOAD`, so
they also replace SQLite's allocator (a Rust global allocator would not).

| Allocator | Idle | Gateway peak | Gateway rest | Sign-ins peak | 15 s after sign-ins |
|---|---|---|---|---|---|
| glibc (default) | 22 MB | 130 MB | 98 MB | 389 MB | 352 MB |
| glibc, `MALLOC_MMAP_THRESHOLD_=131072` | 23 MB | 128 MB | 110 MB | 262 MB | **109 MB** (the gateway rest) |
| mimalloc 2.x | 55 MB | 189 MB | 145 MB | 310 MB | 295 MB |
| mimalloc 3.1.5 | 45 MB | 185 MB | 149 MB | 331 MB | 110 MB |
| jemalloc 5.4 | 81 MB | 195 MB | 40 MB | 254 MB | 59 MB |

- Setting `MALLOC_MMAP_THRESHOLD_` alone also fixes glibc's trim threshold at the same value, so
  at 128 KiB the deflate state of each gzip answer (about 368 KiB) is mapped and unmapped on every
  request (review: ×2.3 on compression alone). The image and the README therefore set
  `MALLOC_MMAP_THRESHOLD_=1048576` (Argon2's 19 MiB blocks are still mapped) and
  `MALLOC_TRIM_THRESHOLD_=2097152`. Fresh server, 500 sign-ins, then history pages with
  `Accept-Encoding: gzip` (`LOADGEN_ACCEPT_GZIP=1`, 2 868 B per answer instead of 16 KB), two
  interleaved runs each, loaded (load average 7 to 10):

  | Setting | Sign-ins peak | 15 s after | History with gzip | History plain |
  |---|---|---|---|---|
  | glibc default | 335 – 547 MB | 221 – 433 MB | 3 874 – 3 950 /s | 2 280 – 4 013 /s |
  | `MMAP=131072` only | 206 – 208 MB | 54 – 57 MB | 4 484 – 4 720 /s | 5 037 – 5 134 /s |
  | `MMAP=1048576`, `TRIM=2097152` | 200 – 209 MB | 47 – 58 MB | 3 910 – 4 619 /s | 4 176 – 5 041 /s |

  End to end, the per-answer mapping does not show above the noise of this loaded machine; the
  memory given back is the same with the new values.
- The 128 KiB threshold gave every Argon2 block back (−243 MB after the sign-ins) at no cost: two runs
  each, interleaved (loaded, load average 5 to 7), history 6 126 – 6 597 pages/s with it against
  4 516 – 5 852 without, guild 11 449 – 13 535 against 7 187 – 11 744, members 2 219 – 2 254
  against 2 017 – 2 162. It is set in the Docker image
  and in the README's systemd directives.
- mimalloc is worse on memory here (more at idle and at rest) and was not faster (loaded: history
  4 217 /s against 6 680 /s for glibc in the same window). Not proposed.
- jemalloc gives the gateway's small allocations back (40 MB at rest) but starts at 81 MB and was
  slower in the same loaded window (3 359 history pages/s). Not proposed either.
- What stays with glibc after gateway load (about 90 MB after 1 000 identifies) is free memory in
  its arenas. Safest next step, if it matters on 2 GB hosts: a `malloc_trim(0)` once the last of a
  burst of sessions has ended (glibc only, one `unsafe` call, no dependency); §2 measured it bringing
  163 MB down to 41 MB.

### Lot 4 — 8 KiB WebSocket read buffer

500 sessions held open (`loadgen hold`), RSS minus idle RSS, two runs each:

| | Before | After |
|---|---|---|
| Per session | 213 – 214 KB | 91 – 92 KB |
| 500 sessions | 107 MB | 46 MB |
| RSS after 500 identifies (ab runs) | 132 – 136 MB | 77 MB |

The gateway tests of the 32 KB voice offer and the 4 KB pre-identify limit still pass. At the
10 000-connection cap that is about 1.2 GB less.

### Lot 5 — gzip for the large JSON answers

| Answer | Plain | gzip (level 1) | CPU to compress (flate2, one core) |
|---|---|---|---|
| History page (50 messages) | 16 013 B | 2 919 B (÷ 5.5) | 39 µs |
| Guild (50 channels) | 9 331 B | 981 B (÷ 9.5) | 29 µs |
| Members (500) | 71 501 B | 8 179 B (÷ 8.7) | 53 µs |
| Read states | 3 041 B | 573 B | |
| My guilds (28) | 1 343 B | 323 B | |
| `/users/@me` (not listed: email, friend code) | 184 B | 184 B, no `Content-Encoding` | |

About 590 µs of server CPU go into a history page (4 cores at 6 800 pages/s), so gzip adds about
6 %. Answers over 256 KiB are compressed on the blocking pool. Only the methods and routes listed
in `http::compression::COMPRESSED_ROUTES` are compressed, never those that
return a session token, two-factor secret or recovery code, invite code, key, email or friend code
(tests list them and check the invite list, over 1 KiB, stays plain). The desktop client's reqwest
does not ask for gzip yet, so only browsers gain.

### Lot 8 — `dist` profile (fat LTO, one codegen unit) for the published server

| | `release` | `dist` |
|---|---|---|
| Binary (without `web-client`) | 47 760 672 B | 35 516 456 B (−26 %) |
| History pages/s, two runs each (loaded) | 4 516 – 5 852 | 6 283 – 6 928 |
| Guild /s (loaded) | 7 187 – 11 744 | 12 131 – 12 913 |
| Members /s (loaded) | 2 017 – 2 162 | 2 159 – 2 317 |
| 500 identifies p50 / p99 (loaded) | 52 – 69 ms / 97 – 155 ms | 44 – 59 ms / 108 – 117 ms |

No slowdown (if anything a few percent faster, within the noise of a loaded machine). Only the
release workflow's server job builds `dist`; `cargo build --release`, CI and the desktop are
unchanged. A cold `dist` build of the server took 7 min 55 s locally with one job under `nice`; the
CI cost was not measured, but that job (8 min on x86-64 in the v0.49 release run) runs beside
desktop jobs of 16 to 25 minutes, so the release workflow's duration does not change unless it
more than doubles.

### Not done

- Lots 6 and 7 (fewer round trips for `READY` and history pages): after Lot 2, a history page takes
  2.3 ms at 16 workers and 500 identifies finish in about 1.1 s; left for a later pass.
- Presences sent lazily instead of in `READY` (protocol change, both clients): the lasting fix for
  `READY` size and its per-socket memory in large guilds.
- gzip for the desktop client: its reqwest is built without the `gzip` feature, so it does not ask.
