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
- One writer connection behind a dedicated task (SQLite allows a single writer); a pool of reader connections. Writes are short transactions; message fan-out happens after commit.
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

### 4.6 Permissions

Permissions are a 64-bit bitfield (`mellow-protocol::Permissions`, via `bitflags`). The effective permission of a member in a channel is computed in this order ([ADR-0007](docs/adr/0007-permission-model.md)):

1. Guild owner → all permissions.
2. Union of `@everyone` and all of the member's roles.
3. `ADMINISTRATOR` → all permissions.
4. Channel overwrites: `@everyone` deny/allow, then role denies/allows combined, then member deny/allow.

The computation is a pure function in `domain`, unit-tested exhaustively, and is the only place permissions are decided. The gateway uses it to filter which events a session receives.

### 4.7 Authentication and sessions

- Accounts are local to an instance ([ADR-0005](docs/adr/0005-per-instance-accounts.md)). Passwords are hashed with Argon2id.
- Login returns an opaque, random 256-bit session token. Only its SHA-256 hash is stored. Tokens are sent as `Authorization: Bearer` on REST and in the gateway `Identify` message.
- Clients store tokens in the OS keychain (desktop: `keyring`; Android: Keystore-backed encrypted storage).
- Registration can be open, invite-only or closed, set by the instance admin.
- Rate limiting on authentication endpoints and on message sending (per user and per channel). Authentication (M1.2): 20 attempts per minute per client address (IPv6 grouped by /64), 10 logins per 15 minutes per account and address, 100 per 15 minutes per account; attempts are counted before the password check. Concurrent Argon2 work is capped to the number of CPU cores.
- Accepted trade-offs: open registration reveals whether a username or email is taken (usernames are public anyway; emails are rate-limited); a username and its email count as two accounts for rate limiting.

### 4.8 Real-time gateway

- Clients open one WebSocket per instance at `/gateway`, then send `Identify { token }` and receive `Ready` (user, guilds, channels, read states, DM list).
- Events are JSON messages `{ "op": ..., "t": "MESSAGE_CREATE", "s": <sequence>, "d": { ... } }`.
- Heartbeats every 30 s detect dead connections. On reconnect, a client sends `Resume { session_id, last_sequence }`. The server keeps a short per-session replay buffer; if the buffer no longer covers the gap, the client resynchronizes through REST.
- Fan-out is in-process: each guild has a `tokio::sync::broadcast` channel; sessions subscribe to the guilds they belong to and filter events by permission. Slow consumers are disconnected and resume instead of blocking others.
- Presence (online, idle, do not disturb, offline) and typing indicators live in memory only.

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
- **TLS:** with a public domain, the server obtains and renews certificates itself through ACME. Running behind an existing reverse proxy is supported by disabling TLS and trusting `X-Forwarded-*` from a configured address only.
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
- **Outbox:** messages sent while offline or during reconnect are queued with a client-generated nonce, shown as pending, and reconciled with the server event carrying the same nonce.
- **API surface:** async functions and an event stream, designed to be exported by UniFFI without leaking Rust-specific types (no lifetimes, no generics in the exported API).

### 6.1 Desktop (`mellow-desktop`)

- Slint UI ([ADR-0010](docs/adr/0010-desktop-ui-with-slint.md)). `.slint` files hold only presentation; Rust code adapts client-core snapshots to Slint models.
- The Slint event loop owns the UI thread; the client core runs on a separate tokio runtime. Communication goes through `slint::invoke_from_event_loop` and channels. Nothing blocking runs on the UI thread.
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
