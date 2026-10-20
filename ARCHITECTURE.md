# Mellow — Architecture

Mellow is a self-hostable, open-source (AGPL-3.0) alternative to Discord, written in Rust. This document describes how the system is built and why. Structural decisions are recorded as ADRs in [`docs/adr/`](docs/adr/); this document is the map, the ADRs are the reasoning.

## 1. Goals and non-goals

### Goals
- **Trivial self-hosting.** One server binary, one data directory, no container, no external database, no reverse proxy required. Install, update and back up in one command each.
- **Familiar chat model.** Servers (guilds) with categories and text channels, real-time messages, direct messages, roles and permissions, moderation, attachments.
- **Native clients.** A desktop app in Rust (Slint) for Linux, macOS and Windows, and an Android app in Kotlin (Jetpack Compose, Material 3 Expressive). Both share one Rust client core.
- **Single-node scale.** Up to about 10 000 monthly active users and about 1 000 concurrently connected clients per instance on a small VPS (2 vCPU, 2 GB RAM).
- **Privacy by default.** TLS everywhere, sensitive data encrypted at rest, no telemetry.

### Non-goals (MVP)
- Voice and video. The architecture leaves room for them, but the MVP does not ship them.
- Federation between instances. A client can connect to several instances, but instances do not talk to each other.
- End-to-end encryption. The server can read messages, which keeps moderation, search and multi-device sync simple ([ADR-0006](docs/adr/0006-tls-and-encryption-at-rest.md)).
- Multi-node clustering or horizontal scaling.
- iOS and web clients.

## 2. System overview

```
┌────────────────────┐   ┌────────────────────┐
│  Desktop (Slint)   │   │ Android (Compose)  │
│  mellow-desktop    │   │  android/          │
└─────────┬──────────┘   └─────────┬──────────┘
          │ Rust API               │ Kotlin API (UniFFI)
          │                ┌───────┴──────────┐
          │                │   mellow-ffi     │
          │                └───────┬──────────┘
          └──────────┬─────────────┘
           ┌─────────┴──────────┐
           │ mellow-client-core │  state, sync, local cache, multi-instance
           └─────────┬──────────┘
                     │  HTTPS (REST)  +  WSS (gateway)
                     │  types from mellow-protocol
           ┌─────────┴──────────────────────────────────┐
           │ mellow-server  (single binary)             │
           │  ┌──────────┐ ┌──────────┐ ┌─────────────┐ │
           │  │ REST API │ │ Gateway  │ │ Admin (htmx)│ │
           │  └────┬─────┘ └────┬─────┘ └──────┬──────┘ │
           │       └──────┬─────┴──────────────┘        │
           │         ┌────┴─────┐                       │
           │         │  Domain  │                       │
           │         └────┬─────┘                       │
           │      ┌───────┴────────┐                    │
           │      │    Storage     │                    │
           │      └──┬──────────┬──┘                    │
           └─────────┼──────────┼───────────────────────┘
                     │          │
              mellow.db     attachments/
              (SQLite)      (files on disk)
```

Push notifications for Android leave the instance through UnifiedPush (user's distributor) or Firebase Cloud Messaging ([ADR-0008](docs/adr/0008-android-push-notifications.md)).

## 3. Repository layout

A single Cargo workspace plus a Gradle project for Android.

```
mellow/
├── Cargo.toml                  # workspace, shared lints and dependency versions
├── crates/
│   ├── mellow-protocol/        # wire types: REST DTOs, gateway events, ids, permissions bitflags
│   ├── mellow-server/          # server binary: REST, gateway, admin UI, storage, CLI
│   ├── mellow-client-core/     # client logic: connection, sync, cache, state, multi-instance
│   ├── mellow-ffi/             # UniFFI bindings of mellow-client-core for Kotlin
│   └── mellow-desktop/         # Slint desktop app
├── android/                    # Gradle project, Kotlin + Compose M3 Expressive, consumes mellow-ffi
├── xtask/                      # dev tasks (`cargo xtask tokens` generates design tokens per platform)
├── design/
│   └── tokens.json             # design tokens, single source of truth (see DESIGN.md)
├── docs/
│   └── adr/                    # architecture decision records
├── ARCHITECTURE.md
├── DESIGN.md
├── CLAUDE.md
└── README.md
```

Dependency rule between crates (arrows mean "depends on"):

```
mellow-desktop ──► mellow-client-core ──► mellow-protocol
mellow-ffi     ──► mellow-client-core
mellow-server  ──────────────────────────► mellow-protocol
```

`mellow-protocol` depends on nothing in the workspace and has no I/O. The server never depends on client crates and vice versa.

## 4. Server (`mellow-server`)

### 4.1 Internal layers

The server is one crate split into modules with a strict direction of dependency: transport → domain → storage.

| Module | Responsibility | Must not |
|---|---|---|
| `http` | REST routes (axum), request validation, auth extraction, rate limiting, mapping domain errors to HTTP | contain business rules or SQL |
| `gateway` | WebSocket sessions, heartbeat, event fan-out, presence, typing | contain SQL |
| `admin` | Admin web UI: server-rendered templates + htmx, embedded assets | duplicate domain logic |
| `domain` | Use cases (send message, create channel, assign role…), permission checks, invariants | know about HTTP, WebSocket or SQL |
| `storage` | Repositories over SQLite (sqlx), migrations, attachment files | make permission decisions |
| `commands` | `serve`, `init`, `update`, `backup`, `admin` subcommands (parsed by `main.rs`) | — |
| `config` | Load and validate `mellow.toml` and environment overrides | — |

The domain calls concrete `storage` functions (no repository traits: there is one storage backend), and domain tests run against a temporary encrypted SQLite database without the network stack.

### 4.2 Main libraries

| Need | Choice |
|---|---|
| Async runtime | `tokio` |
| HTTP + WebSocket | `axum` (with `tower` middleware) |
| Database | `sqlx` with SQLite, compile-time checked queries, embedded migrations |
| TLS | `rustls` (no OpenSSL), automatic certificates via ACME (`rustls-acme` or `instant-acme`) |
| Password hashing | `argon2` (Argon2id) |
| Serialization | `serde`, `serde_json` |
| Templates | `askama` (compile-time templates) + htmx served from embedded assets |
| Observability | `tracing`, `tracing-subscriber` (JSON logs) |
| CLI | `clap` |
| Errors | `thiserror` in libraries and domain, `anyhow` only in `main` and CLI glue |

Exact versions are pinned in the workspace `Cargo.toml` and `Cargo.lock` is committed.

### 4.3 Storage

- **SQLite in WAL mode**, one file `mellow.db` in the data directory ([ADR-0002](docs/adr/0002-single-binary-server-with-embedded-sqlite.md)).
- One writer connection behind a dedicated task (SQLite allows a single writer); a pool of reader connections. Writes are short transactions; message fan-out happens after commit. Today only message sends are serialized, by a per-process lock held from id generation to `MESSAGE_CREATE` publication, so messages commit and reach gateways in id order and an `after` cursor never skips one.
- Migrations are embedded in the binary and run at startup. Before applying a migration, the server writes a consistent snapshot with `VACUUM INTO backups/pre-migration-<version>.db`.
- Attachments are stored encrypted on disk under `attachments/<id prefix>/<id>`. Metadata (owner, name, mime type, size, plaintext SHA-256 for deduplication) lives in SQLite. Size limits are configurable.
- Full-text search uses SQLite FTS5 (post-MVP, but the schema keeps it possible).

### 4.4 Identifiers

All entities use 64-bit, time-ordered identifiers (Snowflake-style: 42 bits of milliseconds since the Mellow epoch, 10 bits of sequence, 12 bits reserved), serialized as strings in JSON to avoid precision loss in clients. Time ordering gives cheap pagination (`before`/`after` an id) and sortable message history ([ADR-0004](docs/adr/0004-protocol-rest-and-websocket-gateway.md)).

### 4.5 Data model (MVP)

```
users            (id, username, display_name, email, password_hash, avatar_id, created_at, disabled_at)
sessions         (id, user_id, token_hash, device_name, created_at, last_seen_at, expires_at)
guilds           (id, name, icon_id, owner_id, created_at)
guild_members    (guild_id, user_id, nickname, joined_at)
roles            (id, guild_id, name, color, position, permissions)
member_roles     (guild_id, user_id, role_id)
channels         (id, guild_id NULL for DMs, kind, name, topic, parent_id, position)
channel_overwrites (channel_id, target_kind, target_id, allow, deny)
dm_participants  (channel_id, user_id)
messages         (id, channel_id, author_id, content, reply_to_id, edited_at, deleted_at)
attachments      (id, message_id, uploader_id, sha256, file_name, mime_type, size_bytes)
read_states      (user_id, channel_id, last_read_message_id)
invites          (code, guild_id, creator_id, max_uses, uses, expires_at)
bans             (guild_id, user_id, reason, banned_by, created_at)
push_endpoints   (id, session_id, kind, endpoint, created_at)
audit_log        (id, guild_id, actor_id, action, target_id, changes, created_at)
```

A DM is a channel with `guild_id = NULL` and its participants in `dm_participants`. This keeps one message path for guild channels and DMs.

Messages are deleted for real (no tombstone); `reply_to_id` has no foreign key, so a reply keeps pointing at a deleted original and clients show "Original message was deleted". Read states only move forward. History pages are returned oldest first.

### 4.6 Permissions

Permissions are a 64-bit bitfield (`mellow-protocol::Permissions`, via `bitflags`). The effective permission of a member in a channel is computed in this order ([ADR-0007](docs/adr/0007-permission-model.md)):

1. Guild owner → all permissions.
2. Union of `@everyone` and all of the member's roles.
3. `ADMINISTRATOR` → all permissions.
4. Channel overwrites: `@everyone` deny/allow, then role denies/allows combined, then member deny/allow.

The computation is a pure function in `mellow-protocol` (`compute_guild_permissions`), unit-tested exhaustively and shared with clients ([ADR-0014](docs/adr/0014-share-the-permission-computation-with-clients.md)). Only `domain::permissions` decides with it on the server; clients use it to adapt their interface, never to grant anything. Role hierarchy: a member with `MANAGE_ROLES` manages `@everyone` and the roles strictly below their highest one that hold only permissions they hold themselves, and grants or removes only permissions they hold; the owner manages every role. A new role starts at position 1, just above `@everyone`, and `PUT /guilds/{id}/roles/order` reorders the named roles: the owner freely, anyone else moving only roles they manage, so their own role and everything above it stay in place. The permission rule also keeps an `ADMINISTRATOR` role that sits low out of a moderator's reach. The gateway uses the computation to filter which events a session receives (with channel overwrites).

### 4.7 Authentication and sessions

- Accounts are local to an instance ([ADR-0005](docs/adr/0005-per-instance-accounts.md)). Passwords are hashed with Argon2id.
- Login returns an opaque, random 256-bit session token. Only its SHA-256 hash is stored. Tokens are sent as `Authorization: Bearer` on REST and in the gateway `Identify` message.
- Sessions last 90 days from sign-in, whatever their use. `last_seen_at` is recorded at most once an hour per session (a write per request would queue behind message sends), a failed record never refuses the request. Sessions expired for more than a day are deleted at startup and then every hour (the day keeps a clock that jumped forward from wiping every session).
- Clients store tokens in the OS keychain (desktop: `keyring`; Android: Keystore-backed encrypted storage).
- Registration can be open, invite-only or closed, set by the instance admin.
- Rate limiting on authentication endpoints and on message writes. Messages (M1.3b): 10 writes (send, edit, delete) per 10 s and 600 per hour per user, 30 messages per 10 s per channel, 30 read-state updates per 10 s per user; content capped at 4 000 characters and 8 000 bytes; bidi overrides, tags and invisible-only messages refused; `secure_delete` overwrites deleted content on disk. Authentication (M1.2): 20 attempts per minute per client address (IPv6 grouped by /64), 10 logins per 15 minutes per account and address, 100 per 15 minutes per account; attempts are counted before the password check. Concurrent Argon2 work is capped to the number of CPU cores.
- Quotas (M1.3a): 500 channels per guild, 100 owned guilds per user, 1 000 active invites per guild, member lists capped at 1 000 until pagination; guild writes limited to 30 per minute per user and invite redemptions to 30 per minute per address. Quotas are checked in the same SQL statement or `BEGIN IMMEDIATE` transaction as the insert.
- Accepted trade-offs: open registration reveals whether a username or email is taken (usernames are public anyway; emails are rate-limited); a username and its email count as two accounts for rate limiting.

### 4.8 Real-time gateway

- Clients open one WebSocket per instance at `/gateway`, then send `Identify { token }` and receive `Ready` (user, guilds, channels, read states, DM list).
- Events are JSON messages `{ "op": ..., "t": "MESSAGE_CREATE", "s": <sequence>, "d": { ... } }`.
- Heartbeats every 30 s detect dead connections. On reconnect, a client sends `Resume { session_id, last_sequence }`. The server keeps a short per-session replay buffer; if the buffer no longer covers the gap, the client resynchronizes through REST.
- Fan-out is in-process (`events::EventHub`): each event is serialized once and queued to the sessions of its guild or user. A session that falls behind is cut off and told to identify again instead of blocking others. No per-recipient permission filter yet: in M1 the `@everyone` role always holds `VIEW_CHANNEL`; filtering by effective channel permissions must land with roles and overwrites (M2, ADR-0007).
- Presence (online, idle, do not disturb, offline) and typing indicators live in memory only.
- Implementation (M1.4): a session outlives its WebSocket for 60 s with a replay buffer (1 024 dispatches, 2 MB, sharing the hub's serialized events), so `resume` replays what was missed; resuming with another user's token is refused. Logout cuts off the token's sessions at once; heartbeats (30 s, 45 s deadline) also revalidate the token, at most every 15 s. Limits: 4 KB client frames, identify within 10 s, 10 sessions per user (checked atomically), 10 gateway connections per 10 s per address, 10 000 connections in total, 10 s write timeout and 256 KB write buffer per socket. Presence is online/offline from live sessions; "offline" is published under the same lock that removes the last session.
- A message's `nonce` comes back only in the sender's REST response; the `MESSAGE_CREATE` event, including to the sender's own sessions, never carries it. Nonces are stored, unique per channel and author: a send repeating one returns the message created the first time, without a new event.

### 4.9 Admin web UI

Served by the same binary under `/admin`, reserved to instance administrators. Server-rendered HTML (askama) with htmx for partial updates, assets embedded at compile time, no JavaScript build step ([ADR-0009](docs/adr/0009-admin-ui-server-rendered-htmx.md)). It covers instance settings, registration mode, users (disable, reset password), storage usage, backups and the update status. Protected by session auth, CSRF tokens and strict `Content-Security-Policy`.

### 4.10 Operations

```
mellow-server init      # create mellow.toml and the data directory, first admin account
mellow-server serve     # run the server
mellow-server update    # download, verify and install the latest release, then restart
mellow-server backup    # consistent snapshot of the database + attachments
mellow-server admin ... # CLI administration (create user, reset password, …)
```

- **Configuration:** one `mellow.toml` file, every key overridable by `MELLOW_*` environment variables. Validation fails fast at startup with a precise message.
- **TLS:** with a public domain (`[tls]` in `mellow.toml`), the server terminates TLS itself with rustls and obtains and renews certificates through ACME TLS-ALPN-01 on port 443, so no other port is needed ([ADR-0013](docs/adr/0013-built-in-tls-with-acme-tls-alpn.md)). Certificate and account keys live in a private directory outside the data directory. Running behind an existing reverse proxy is supported instead (`behind_tls_reverse_proxy`), trusting `X-Forwarded-For` from loopback and from the networks listed in `trusted_proxy_networks` (a proxy in a container network).
- **Updates:** releases are signed with an Ed25519 key (minisign format). `update` refuses any binary whose signature does not verify against the public key built into the current binary, replaces the executable atomically and keeps the previous one for rollback ([ADR-0003](docs/adr/0003-self-update-with-signed-releases.md)).
- **Service install:** `init` can write a systemd unit (Linux), a launchd plist (macOS) or register a Windows service.
- **Data directory:** `mellow.db`, `attachments/`, `backups/`, `mellow.toml`. The master key file lives outside it (default `/etc/mellow/master.key` on Linux). Restoring an instance needs the data directory **and** the key file, which must be backed up separately ([ADR-0006](docs/adr/0006-tls-and-encryption-at-rest.md)).

## 5. Protocol (`mellow-protocol`)

- REST over HTTPS, versioned under `/api/v1`, JSON bodies, errors as `{ "code": "CHANNEL_NOT_FOUND", "message": "..." }` with stable machine-readable codes.
- WebSocket gateway as described in 4.8, versioned with `/gateway?v=1`.
- All request, response and event types are defined once in `mellow-protocol` with `serde`, shared by the server and the client core. Kotlin types come from UniFFI, not from hand-written copies.
- Pagination by id cursors (`before`, `after`, `limit ≤ 100`).
- Every input is validated at the server boundary (lengths, formats, allowed values) even though the client validates too.

## 6. Client core (`mellow-client-core`)

The client core contains everything that is not drawing pixels, so desktop and Android behave the same.

- **Connections:** one `InstanceConnection` per configured instance (REST client on `reqwest` with rustls, gateway on `tokio-tungstenite`), with reconnection, resume and exponential backoff with jitter.
- **State:** an in-memory store of guilds, channels, members, messages, presence and read states, updated by gateway events and REST responses. UIs read immutable snapshots and subscribe to change notifications.
- **Local cache:** a per-instance SQLite database storing recent messages and metadata, so the app opens instantly and shows content offline. The cache is disposable: it can always be rebuilt from the server.
- **Outbox:** messages sent while offline or during reconnect are queued with a client-generated nonce, shown as pending, and reconciled with the REST response echoing the nonce; a retry after a lost response reuses the nonce, so the server returns the message it already created instead of a duplicate.
- **API surface:** async functions and an event stream, designed to be exported by UniFFI without leaking Rust-specific types (no lifetimes, no generics in the exported API).
- **Built so far (M1.5):** `InstanceAddress` (https only, plain http on loopback), `RestClient`, `run_gateway` (identify, heartbeat, resume, backoff with jitter; reports `Ready`, events, `Disconnected`, `SignedOut`) and `ClientState`, a pure reducer the UI owns. One instance at a time; local cache not built yet. The outbox (`ClientState::queue_message`, `generate_message_nonce`) keeps each unconfirmed send with a random 128-bit nonce, marks it failed when the request fails, and resends it with the same nonce on retry; after each gateway `Ready` or `Resumed`, the desktop sends every failed message again (`ClientState::retry_failed_messages`), one after another in send order, stopping at the first failure the instance does not refuse. It is in memory only. History loads by pages of 50 (`HISTORY_PAGE_SIZE`): the latest when a channel opens, then older ones `before` the oldest cached message as the reader scrolls up; a channel keeps at most 500 messages once the reader leaves it (`ClientState::trim_history`).

### 6.1 Desktop (`mellow-desktop`)

- Slint UI ([ADR-0010](docs/adr/0010-desktop-ui-with-slint.md)). `.slint` files hold only presentation; Rust code adapts client-core snapshots to Slint models.
- The Slint event loop owns the UI thread and the `ClientState`; network calls run on a separate tokio runtime. Their results and gateway updates come back to the UI thread as futures driven by `slint::spawn_local`, so no lock is shared with the UI. Nothing blocking runs on the UI thread.
- The session (instance address and token) is stored in the OS secret store through `keyring` (Keychain on macOS, Credential Manager on Windows with local-machine persistence so it never roams with a domain profile, Secret Service on Linux), read back off the UI thread at launch and validated like any input. It is removed when the server rejects the token or the user signs out from the user panel; signing out revokes the token on the server, and the app waits for that request before exiting. If the secret store refuses the write, the session lives in memory only and is revoked on exit. One stored session at a time until multi-instance support (M2).
- Design tokens are generated from `design/tokens.json` into a Slint `global` (see DESIGN.md).
- Packaging: AppImage/deb/rpm (Linux), signed `.dmg` (macOS), MSI (Windows).

### 6.2 Android (`android/`)

- Kotlin, Jetpack Compose, Material 3 Expressive, adaptive layouts for phones, tablets and foldables.
- Uses `mellow-ffi` (UniFFI-generated Kotlin bindings + native libraries for `arm64-v8a` and `x86_64`, built with `cargo-ndk`) ([ADR-0011](docs/adr/0011-shared-client-core-with-uniffi.md)).
- Architecture: Compose UI → ViewModel (StateFlow) → repository adapter over the FFI client core. No protocol logic in Kotlin.
- Push: UnifiedPush when a distributor is installed, FCM otherwise ([ADR-0008](docs/adr/0008-android-push-notifications.md)). Push payloads contain only identifiers; the app fetches content through the authenticated connection.

## 7. Security summary

| Area | Measure |
|---|---|
| Transport | TLS 1.2+ via rustls everywhere; HSTS on web endpoints |
| Passwords | Argon2id with per-user salt |
| Sessions | Random 256-bit tokens, stored hashed, revocable per device |
| Authorization | Single pure permission function, checked in the domain layer for every use case |
| Input | Validation at every boundary (REST, gateway, admin forms, uploads, deep links) |
| Uploads | Size limits, MIME sniffing, served with `Content-Disposition` and `X-Content-Type-Options: nosniff` from a separate path; no inline HTML/SVG rendering |
| Abuse | Rate limiting per IP, per account, per channel |
| Admin UI | CSRF tokens, strict CSP, `SameSite=Strict` cookies |
| At rest | See [ADR-0006](docs/adr/0006-tls-and-encryption-at-rest.md) |
| Supply chain | `Cargo.lock` committed, `cargo deny` in CI (RustSec advisories, licenses, sources, banned crates) on every PR and weekly, actions pinned by SHA, signed releases |
| Logs | Structured, no message content, no tokens, no emails, no IPs beyond rate-limit needs |

## 8. Quality and CI

- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check`.
- Android: `./gradlew lint test` and Compose UI tests.
- Domain logic (permissions, message rules) has exhaustive unit tests; storage has tests against in-memory SQLite; the gateway has integration tests with real WebSocket clients against a server started on an ephemeral port.
- Release builds are produced by CI for Linux (x86_64, aarch64), macOS (aarch64, x86_64) and Windows (x86_64), then signed.

## 9. Evolution path

These are explicitly out of the MVP, listed only so current choices do not block them:

- **Voice/video:** an SFU module (WebRTC in Rust, e.g. `str0m`) inside the same binary.
- **Search:** SQLite FTS5 on message content.
- **Object storage:** an S3-compatible attachment backend behind the existing storage trait.
- **Larger instances:** if a single SQLite writer becomes the bottleneck, measure first; Postgres support would be a new ADR.
