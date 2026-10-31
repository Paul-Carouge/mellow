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
- iOS clients. (A web client, served by the server binary, was added by [ADR-0019](docs/adr/0019-web-client-served-by-the-server-binary.md).)

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
| `voice` | Voice channels beside `gateway` ([ADR-0030](docs/adr/0030-voice-channels-with-an-sfu-in-the-server-binary.md)): participants, signalling, and the media thread (`voice::media`) relaying Opus over one UDP port | contain SQL or decide permissions |
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
| Voice (WebRTC SFU) | `str0m`, sans-IO, `aws-lc-rs` crypto ([ADR-0030](docs/adr/0030-voice-channels-with-an-sfu-in-the-server-binary.md)) |
| CLI | `clap` |
| Errors | `thiserror` in libraries and domain, `anyhow` only in `main` and CLI glue |

Exact versions are pinned in the workspace `Cargo.toml` and `Cargo.lock` is committed.

### 4.3 Storage

- **SQLite in WAL mode**, one file `mellow.db` in the data directory ([ADR-0002](docs/adr/0002-single-binary-server-with-embedded-sqlite.md)).
- One writer connection (SQLite allows a single writer); a pool of read-only connections (`query_only`). `storage::Database` holds both and each storage function picks one, so the layers above never do. The writer is a sqlx pool capped at one connection: its FIFO acquisition is the write queue, so no dedicated task is needed. A transaction holds the writer until it ends, so it must never await another write. Writes are short transactions; message fan-out happens after commit. Message sends are also serialized, by a per-process lock held from id generation to `MESSAGE_CREATE` publication, so messages commit and reach gateways in id order and an `after` cursor never skips one.
- Migrations are embedded in the binary and run at startup. Before applying a migration, the server writes a consistent snapshot with `VACUUM INTO backups/pre-migration-<version>.db`.
- Avatars and banners share that store: each version has its own id (and a still first frame when animated), served immutable at `/api/v1/profile-images/{id}` ([ADR-0021](docs/adr/0021-compress-images-losslessly-after-upload.md)). Guild and channel icons too, at `/api/v1/icons/{id}`, but only to members of their guild (and, for a channel's icon, to those who can view the channel): uploaded with an optional square crop (centred by default), stored small (guilds 256 px, may be animated; channels 96 px, always still). Custom emoji pictures are guild images too, served the same way ([ADR-0028](docs/adr/0028-custom-emoji.md)). Pictures are compressed losslessly: avatars and banners while uploading, attachments in the background after it.
- Attachments are stored encrypted on disk under `attachments/<id prefix>/<id>`. Metadata (owner, name, mime type, size, plaintext SHA-256 for deduplication) lives in SQLite. Size limits are configurable.
- Message search uses an external-content FTS5 table, `messages_fts` (migration 0021, [ADR-0029](docs/adr/0029-message-search.md)), inside the encrypted database: tokenizer `unicode61 remove_diacritics 2`, 2- and 3-character prefix indexes, `secure-delete` on, kept in step by triggers on `messages` within the writing transaction; the same migration indexes `messages (author_id, id)` for `from:`. Every connection keeps a 32 MiB page cache, since SQLCipher decrypts each page read from disk.

### 4.4 Identifiers

All entities use 64-bit, time-ordered identifiers (Snowflake-style: 42 bits of milliseconds since the Mellow epoch, 10 bits of sequence, 12 bits reserved), serialized as strings in JSON to avoid precision loss in clients. Time ordering gives cheap pagination (`before`/`after` an id) and sortable message history ([ADR-0004](docs/adr/0004-protocol-rest-and-websocket-gateway.md)).

### 4.5 Data model (MVP)

```
users            (id, username, display_name, email, password_hash, avatar_id, avatar_is_animated, banner_id, banner_is_animated, created_at, disabled_at)
profile_images   (id, user_id, still_id, size_bytes, created_at)
guild_images     (id, guild_id, channel_id?, still_id, size_bytes, created_at)   -- guild and channel icons, emoji pictures
guild_emojis     (id = its guild_images id, guild_id, name)                       -- custom emoji (ADR-0028)
sessions         (id, user_id, token_hash, device_name, created_at, last_seen_at, expires_at)
guilds           (id, name, icon_id, icon_is_animated, owner_id, created_at)
guild_members    (guild_id, user_id, nickname, joined_at)
roles            (id, guild_id, name, color, position, permissions)
member_roles     (guild_id, user_id, role_id)
channels         (id, guild_id NULL for DMs, kind, name, topic, parent_id, position, icon_id)
channel_overwrites (channel_id, target_kind, target_id, allow, deny)
direct_message_participants (channel_id, user_id)
direct_message_pairs (lower_user_id, higher_user_id, channel_id)
messages         (id, channel_id, author_id, content, reply_to_id, edited_at, pinned_at, deleted_at)
messages_fts     (rowid = messages.id, content)                                   -- FTS5 index, external content (ADR-0029)
message_reactions (message_id, user_id, emoji)
attachments      (id, message_id, uploader_id, sha256, file_name, mime_type, size_bytes)
read_states      (user_id, channel_id, last_read_message_id)
invites          (code, guild_id, creator_id, max_uses, uses, expires_at)
bans             (guild_id, user_id, reason, banned_by, created_at)
push_endpoints   (id, session_id, kind, endpoint, created_at)
audit_log        (id, guild_id, actor_id, action, target_id, context_id, channel_id, reason, changes)
```

A DM is a channel with `guild_id = NULL`, of kind `direct_message`, with its participants in `direct_message_participants`; `direct_message_pairs` keeps one conversation per pair of users, even when both open it at once. This keeps one message path (`/channels/{id}/messages`) for guild channels and DMs. `POST /users/@me/channels { recipient_id }` opens or returns the one-to-one conversation, only with someone the caller may reach: a shared guild or a friendship, and no block either way ([ADR-0025](docs/adr/0025-friends-and-blocking.md); anyone else answers `NOT_FOUND`, like an unknown user; 10 per minute and 200 per day per user). Participants view and read a conversation; they write (send, edit, typing) only while they may still reach everyone else in it, so a member kicked or banned from the only shared guild, or blocked, cannot keep messaging; no one manages messages there (`domain::permissions::direct_message_permissions`). A non-participant gets `NOT_FOUND`, and guild channel routes never match a conversation.

A group conversation ([ADR-0026](docs/adr/0026-group-conversations.md)) is a conversation with a row in `group_conversations` (its owner) and a name in `channels.name`: up to 10 people, created with and joined by friends of whoever adds them, left by anyone, emptied of others by its owner only, deleted with its last participant. Every participant writes whatever their relationships; `/api/v1/group-conversations` creates, renames and changes participants, and `DIRECT_MESSAGE_CHANNEL_UPDATE` / `DIRECT_MESSAGE_CHANNEL_DELETE` keep each participant's view current.

Relationships ([ADR-0025](docs/adr/0025-friends-and-blocking.md)) live in `relationships` (one row per side: `friend`, `incoming_request`, `outgoing_request`, `blocked`; a block has only the blocker's row). `/users/@me/relationships` sends requests by username (a missing account, oneself and a block answer the same `NOT_FOUND`), accepts, refuses, cancels, removes, blocks and unblocks; `RELATIONSHIP_ADD` and `RELATIONSHIP_REMOVE` reach only the people concerned. The event hub keeps each online user's friend ids, so presence and renames reach friends without a shared guild; relationship writes and the hub updates that follow them are serialized by one lock in `domain::relationships`.

Messages are deleted for real (no tombstone); `reply_to_id` has no foreign key, so a reply keeps pointing at a deleted original and clients show "Original message was deleted". Read states only move forward. History pages are returned oldest first.

Reactions (migration 0012): one row per message, user and Unicode emoji, deleted with their message. `PUT`/`DELETE /channels/{id}/messages/{message_id}/reactions/{emoji}/@me` add or take back the caller's reaction (the emoji percent-encoded; both idempotent, `204`); `GET …/reactions/{emoji}?after=&limit=` lists who reacted as `PublicUser`s by ascending id (25 by default, at most 100). Moderators remove other people's reactions with `DELETE …/reactions/{emoji}/{user_id}` (one user's), `DELETE …/reactions/{emoji}` (one emoji's) and `DELETE …/reactions` (all), each audited as `message_reaction_remove` (target: the message; context: the channel) when something went. An emoji is exactly one Unicode emoji of at most 64 bytes: a keycap (`1️⃣`), a flag (one pair of regional indicators), a subdivision flag (🏴 and tags), or emoji joined by ZWJ with at most a skin tone and one variation selector each; enclosed letters that are not emoji, text, spaces and stray joiners are `VALIDATION_FAILED`. It is stored without its variation selectors, so `❤️` and `❤` are one reaction, and events and answers carry that key. A message holds at most 20 distinct emoji (`LIMIT_REACHED`). Who may do what is decided by `domain::permissions::may_react`: in a guild channel, seeing the message (`VIEW_CHANNEL`, `READ_MESSAGE_HISTORY`) is enough to list and join an existing reaction, a new emoji also needs `ADD_REACTIONS`, taking one's own back needs only `VIEW_CHANNEL`, and removing other people's needs `MANAGE_MESSAGES`; in a conversation every reaction added needs write access (`direct_message_permissions` grants `ADD_REACTIONS` with it) and no one removes another's. Without `READ_MESSAGE_HISTORY`, reaction requests answer `NOT_FOUND` before the message is looked up, and taking one's own reaction back never looks it up, so neither tells an existing message from a missing one. The new-emoji right and the cap are checked in the insert's transaction. Migration 0012 grants `ADD_REACTIONS` to every existing `@everyone` role and adds it to the deny of every overwrite that denies `SEND_MESSAGES`, so read-only channels stay closed to new emoji. `Message.reactions` (`emoji`, `count`, `me`, in the order first added) is filled in REST answers (history, send, edit, pins) for the requester, and is always empty in gateway events, since `me` differs per viewer. Reaction writes (adding and removing, moderators' included): 20 per 10 s and 1 200 per hour per user, and 60 per 10 s per channel.

Custom emoji (migration 0020, [ADR-0028](docs/adr/0028-custom-emoji.md)): `GET /guilds/{id}/emojis` lists a server's emoji to its members; `POST /guilds/{id}/emojis?name=…` (the picture as the body, at most 256 KB, centred square kept, 128 px lossless WebP, animated GIF/WebP kept animated with a still frame), `PATCH /guilds/{id}/emojis/{emoji_id}` `{ "name" }` and `DELETE /guilds/{id}/emojis/{emoji_id}` need `MANAGE_EMOJIS` (bit 17, guild-scoped) and count as guild writes, except uploads, charged before their body is read to the uploader's picture budget (shared with avatars and icons); a processed emoji over 512 KB (still frame included) is refused. An emoji's id is its picture's `guild_images` id, so `GET /icons/{id}` (and `/still`) serves it to members only. A server holds at most 50 still and 50 animated emoji (`LIMIT_REACHED`); names are 2 to 32 ASCII letters, digits and `_`, unique per server whatever the case. `Guild.emojis` carries the list, `GUILD_EMOJIS_UPDATE { guild_id, emojis }` sends every member the whole new list, and each change is audited (`emoji_create`, `emoji_update`, `emoji_delete`, target the emoji, with a `name` change). Messages write them `<:name:id>` / `<a:name:id>`, as plain text the server does not check. Reactions key them `:id`: starting a new one needs the emoji to exist in the channel's own server (`domain::permissions::may_react_with_custom_emoji`, so never in conversations), while joining, listing and removing accept any well-formed key, so reactions survive renames as one reaction and stay after a deletion.

Pins (migration 0012): `messages.pinned_at` is set while pinned. `GET /channels/{id}/pins` lists them, most recently pinned first (`VIEW_CHANNEL` and `READ_MESSAGE_HISTORY`); `PUT`/`DELETE /channels/{id}/pins/{message_id}` pin and unpin (idempotent, `204`, counted as message writes), with `MANAGE_MESSAGES` in a guild channel or write access in a conversation (`domain::permissions::may_manage_pins`); a channel holds at most 50 pins (`LIMIT_REACHED`). Each change in a guild records `message_pin` or `message_unpin` in the audit log (target: the message; context: the channel).

Search (migration 0021, [ADR-0029](docs/adr/0029-message-search.md)): `POST /guilds/{id}/messages/search` searches the server's text channels where the requester holds `VIEW_CHANNEL` and `READ_MESSAGE_HISTORY` (`Guilds::channels_with_readable_history`; a non-member gets `NOT_FOUND`), `POST /users/@me/messages/search` their conversations and groups (`direct_message_permissions`). JSON body: `q` (words, each also as a prefix when its last token has 2 characters or more, at most 4 prefixes; `"phrase"`, `-excluded`, `word*`, `from:`, `mentions:`, `in:`, `has:file|image|link|pinned`, `before:`/`after:`/`during:YYYY-MM-DD`; at most 256 characters and 16 parts), `sort=relevance|newest`, `offset`, `limit` (≤ 25, `offset + limit` ≤ 1 000), `utc_offset_minutes`. `domain::search_query` parses it and quotes every word, so user text is never FTS5 syntax. The answer holds the page's `Message`s with highlight ranges (UTF-16 code units of `content`), a total counted up to 1 000 (the newest matches), and the 10 largest channel and author facets among them; only readable channels are ever queried, so hidden ones never show, even in counts. Blocked authors are included and folded by clients. 30 searches per 30 s per user, one at a time each (`RATE_LIMITED` otherwise), and at most 2 of the 4 readers for all searches together. Deleting a channel or a guild deletes its messages 1 000 per transaction first, since each deletion also costs FTS5 `secure-delete` work.

### 4.6 Permissions

Permissions are a 64-bit bitfield (`mellow-protocol::Permissions`, via `bitflags`). The effective permission of a member in a channel is computed in this order ([ADR-0007](docs/adr/0007-permission-model.md)):

1. Guild owner → all permissions.
2. Union of `@everyone` and all of the member's roles.
3. `ADMINISTRATOR` → all permissions.
4. Channel overwrites: `@everyone` deny/allow, then role denies/allows combined, then member deny/allow.

The computation is a pure function in `mellow-protocol` (`compute_guild_permissions`), unit-tested exhaustively and shared with clients ([ADR-0014](docs/adr/0014-share-the-permission-computation-with-clients.md)). Only `domain::permissions` decides with it on the server; clients use it to adapt their interface, never to grant anything. Role hierarchy: a member with `MANAGE_ROLES` manages `@everyone` and the roles strictly below their highest one that hold only permissions they hold themselves, and grants or removes only permissions they hold; the owner manages every role. A new role starts at position 1, just above `@everyone`, and `PUT /guilds/{id}/roles/order` reorders the named roles: the owner freely, anyone else moving only roles they manage, so their own role and everything above it stay in place. The permission rule also keeps an `ADMINISTRATOR` role that sits low out of a moderator's reach. Creating a server (`POST /guilds`) follows the instance's `guild_creation` setting: only instance administrators (`--administrator` accounts, unrelated to a server's owner or to its `ADMINISTRATOR` permission) by default, or everyone; `Ready.may_create_guilds` tells clients ([ADR-0027](docs/adr/0027-instance-administrators-decide-who-creates-servers.md)). Channels: `PUT /guilds/{id}/channels/order` moves channels and categories (`MANAGE_CHANNELS`, and in each channel moved to another category); the member lists every channel they can view, and channels hidden from them keep their places, so the request reveals nothing about them; online members get one fresh `GUILD_CREATE` each (a move renumbers nearly every channel), and each category change is audited on its channel. Channel and category names are kept as typed (1 to 100 characters, no hidden characters): unlike Discord, text channels are not lowercased. `POST /guilds` takes optional `initial_categories` (up to 10 categories and 50 channels), so clients offer starter templates in their own language. Channel overwrites (`PUT`/`DELETE /channels/{id}/permissions/{target_id}`) need `MANAGE_ROLES` in the channel; only bits the member holds there may change, and a role target must be `@everyone` or below the member's highest role. Moderation: `DELETE /guilds/{id}/members/{user_id}` (kick, `KICK_MEMBERS`) and `PUT`/`DELETE /guilds/{id}/bans/{user_id}` (`BAN_MEMBERS`) act only on members ranked strictly below the moderator; a banned user cannot join with an invite. Every moderation and administration action (channels, overwrites, roles, member roles, kicks, bans, a moderator deleting someone else's message, pins) writes an audit log entry in the same transaction, only when something changed, with the changed fields' old and new values. Server settings: `PATCH /guilds/{id}` renames a guild (`MANAGE_GUILD`, `GUILD_UPDATE` to every member, audited as `guild_update`); `DELETE /guilds/{id}` is the owner's alone and removes everything the guild holds by cascade (attachment files go with the orphan sweep), then sends `GUILD_DELETE` to every member; `GET /guilds/{id}/invites` lists the usable invites with their creator and uses and `DELETE /invites/{code}` revokes one (`MANAGE_GUILD`; an unknown code and one of a guild the user is not in both answer `NOT_FOUND`); creating and revoking invites are audited (`invite_create`, `invite_delete`) with only the code's first four characters, since readers of the log need not be allowed to invite. It is readable with `VIEW_AUDIT_LOG` at `GET /guilds/{id}/audit-log`; entries about a channel the reader cannot view are left out (owner and administrators see them all, deleted channels included). A channel the member cannot view answers `NOT_FOUND` and is left out of snapshots and read states; the gateway filters its events the same way.

### 4.7 Authentication and sessions

- Accounts are local to an instance ([ADR-0005](docs/adr/0005-per-instance-accounts.md)). Passwords are hashed with Argon2id.
- Login returns an opaque, random 256-bit session token. Only its SHA-256 hash is stored. Tokens are sent as `Authorization: Bearer` on REST and in the gateway `Identify` message.
- Sessions last 90 days from sign-in, whatever their use. `last_seen_at` is recorded at most once an hour per session (a write per request would queue behind message sends), a failed record never refuses the request. Sessions expired for more than a day are deleted at startup and then every hour (the day keeps a clock that jumped forward from wiping every session).
- Clients store tokens in the OS keychain (desktop: `keyring`; Android: Keystore-backed encrypted storage).
- Registration can be open, invite-only or closed, set by the instance admin.
- Rate limiting on authentication endpoints and on message writes. Messages (M1.3b): 10 writes (send, edit, delete) per 10 s and 600 per hour per user, 30 messages per 10 s per channel, 30 read-state updates per 10 s per user; content capped at 4 000 characters and 8 000 bytes; bidi overrides, tags and invisible-only messages refused; `secure_delete` overwrites deleted content on disk. Authentication (M1.2): 20 attempts per minute per client address (IPv6 grouped by /64), 10 logins per 15 minutes per account and address, 100 per 15 minutes per account; attempts are counted before the password check. Account settings: a current-password check (password change, username change) is limited to 10 per 15 minutes per session and 50 per user, counting toward the user's budget only the checks the session's budget allowed, so a stolen session cannot use up the owner's budget; username and display-name changes to 10 per minute per session and 30 per user, counted the same way, so neither a stolen session nor extra sessions multiply the fan-out. A password change replaces the hash and deletes the user's other sessions only if the hash is still the one verified, and a new session is stored only if the password it was verified against is still current. Concurrent Argon2 work is capped to the number of CPU cores.
- Quotas (M1.3a): 500 channels per guild, 100 owned guilds per user, 1 000 active invites per guild, member lists capped at 1 000 until pagination; guild writes limited to 30 per minute per user and invite redemptions to 30 per minute per address and 10 per 10 minutes per user. Invite codes read like `AE8C-78A9-K2QF`: 12 symbols of Crockford's base 32 (60 bits, as strong as the older 10-character codes; two groups were guessable from an IPv6 /48 and the audit log keeps the first group), and a typed code ignores case, spaces and dashes. Quotas are checked in the same SQL statement or `BEGIN IMMEDIATE` transaction as the insert.
- Accepted trade-offs: a sign-in flood from about 170 addresses (cheap with IPv6 prefixes, grouped by /64) can fill the unauthenticated rate-limiter table for 15 minutes, which then refuses new sign-ins; signed-in users' limits and gateway reconnections use separate tables and keep working. Open registration reveals whether a username or email is taken (usernames are public anyway; emails are rate-limited); a username and its email count as two accounts for rate limiting.

### 4.8 Real-time gateway

- Clients open one WebSocket per instance at `/gateway`, then send `Identify { token }` and receive `Ready` (user, guilds, channels, read states, conversations with at least one message).
- Events are JSON messages `{ "op": ..., "t": "MESSAGE_CREATE", "s": <sequence>, "d": { ... } }`.
- Heartbeats every 30 s detect dead connections. On reconnect, a client sends `Resume { session_id, last_sequence }`. The server keeps a short per-session replay buffer; if the buffer no longer covers the gap, the client resynchronizes through REST.
- Fan-out is in-process (`events::EventHub`): each event is serialized once and queued to the sessions of its guild or user. A session that falls behind is cut off and told to identify again instead of blocking others. Events about one channel go only to the members who can view it ([ADR-0015](docs/adr/0015-channel-visibility-in-snapshots-and-gateway-fan-out.md)): to the whole guild when `@everyone` views it and no overwrite denies `VIEW_CHANNEL`, otherwise to the viewers computed per event from a per-guild cache of members and roles that a trigger-maintained `permission_version` keeps exact, still serialized once. A permission change that can hide or reveal channels sends each affected online member a fresh `GUILD_CREATE`.
- A reaction added or taken back is `MESSAGE_REACTION_ADD` or `MESSAGE_REACTION_REMOVE` `{ guild_id?, channel_id, message_id, user_id, emoji }`; a moderator removing one emoji's or all reactions sends `MESSAGE_REACTION_REMOVE_EMOJI` `{ guild_id?, channel_id, message_id, emoji }` or `MESSAGE_REACTION_REMOVE_ALL` `{ guild_id?, channel_id, message_id }`. Each is sent only when something changed. A pin or unpin is a `MESSAGE_UPDATE` carrying the message with its new `pinned`, so clients that already apply edits need nothing more; since events never carry reactions, clients keep the reactions they hold when applying any `MESSAGE_UPDATE`.
- Conversation events (messages, reactions, typing) go to their participants only. The recipient of a new conversation learns of it with `DIRECT_MESSAGE_CHANNEL_CREATE`, sent to every participant right before the first `MESSAGE_CREATE`; typing in a conversation without messages reaches no one. In conversations, `MESSAGE_DELETE`, `MESSAGE_REACTION_ADD`, `MESSAGE_REACTION_REMOVE` and `TYPING_START` carry no `guild_id`.
- A renamed guild reaches every member as `GUILD_UPDATE` (`{ id, name }`); clients that do not know it skip it.
- Presence (online, idle, do not disturb, offline) and typing indicators live in memory, except the status each user picks and their custom status: stored on `users` (migration 0017) and set with `PUT /api/v1/users/@me/presence` (`{ status: online | idle | do_not_disturb | invisible, custom_status? }`, up to 128 characters). `Ready.presences` lists what others see of each online member, `PRESENCE_UPDATE` carries `{ user_id, status, custom_status? }`; an invisible user looks offline, without custom status, in both.
- Implementation (M1.4): a session outlives its WebSocket for 60 s with a replay buffer (1 024 dispatches, 2 MB, sharing the hub's serialized events), so `resume` replays what was missed; resuming with another user's token is refused. Logout cuts off the token's sessions at once; heartbeats (30 s, 45 s deadline) also revalidate the token, at most every 15 s. Limits: 4 KB client frames before identify, 32 KB after (a voice offer), identify within 10 s, 10 sessions per user (checked atomically), 10 gateway connections per 10 s per address, 10 000 connections in total, 10 s write timeout and 256 KB write buffer per socket. Presence comes from live sessions and the stored pick, which the hub loads at identify; each pick bumps `users.presence_version` and the hub drops anything older than what it applied, so a slow identify cannot undo a newer pick. A presence not loaded yet is shown to no one. "Offline" is published under the same lock that removes the last session, and not at all for an invisible user.
- Voice channels ([ADR-0030](docs/adr/0030-voice-channels-with-an-sfu-in-the-server-binary.md)): a channel of kind `voice` holds no messages and may carry `user_limit` (1 to 25). A client sends `voice_join { channel_id, offer_id, sdp_offer, self_mute, self_deaf }` (an offer with one `sendonly`/`sendrecv` audio m-line, then 1 to 16 `recvonly` ones, at most 28 KiB, about twice a real Chromium offer; `offer_id` is a client-chosen `u32` echoed in the answer or refusal, so a late answer to an older offer is ignored), `voice_leave` and `voice_state { self_mute, self_deaf }`. The requesting session alone gets `VOICE_SESSION_DESCRIPTION { channel_id, offer_id, sdp_answer }` or `VOICE_JOIN_REFUSED { channel_id, offer_id, reason }` (`missing_permissions`, `channel_full`, `instance_full`, `invalid_offer`, `voice_unavailable`, `rate_limited`), and `VOICE_DISCONNECTED { channel_id, reason }` when the server ends its voice (`moved_elsewhere`, `permissions_changed`, `channel_deleted`, `removed_from_guild`, `media_timeout`). The channel's viewers (ADR-0015) get `VOICE_STATE_UPDATE { voice_state }` (join, move, mute, suppression), `VOICE_STATE_DELETE { guild_id, channel_id, user_id }` and `SPEAKING { guild_id, channel_id, user_id, speaking }` edges; `Ready.voice_states` lists who is in the voice channels the user can view, `Ready.voice_enabled` whether voice works here. A move is one `VOICE_STATE_UPDATE` (viewers of the old channel who cannot view the new one, in any guild, get a `VOICE_STATE_DELETE`); joining the same channel again (an ICE restart) announces only a changed mute, deafen or suppression, plus `SPEAKING false` if it cut a sentence. After every access change (a member joining the guild included), everyone still connected there is announced again to their channel's viewers, so new viewers learn who is in it. A join reads access once to refuse early, then again under the participants lock, and is dropped if its gateway session ended meanwhile. One voice connection per user, owned by the gateway session that joined: it ends when that session ends for good, not during a resume. `voice_join` is limited to 10 per minute per user, `voice_state` to 20 per 10 s.
- Voice media: one OS thread outside Tokio owns the UDP socket (`[voice] udp_port`, default 50000, on `0.0.0.0`) and every participant's `str0m` session, ICE-lite with one host candidate: `[voice] public_address`, else the gateway's `Host` (port removed, checked, resolved to IPv4 and cached 60 s for at most 8 names, never logged; a loopback result becomes the machine's network address only on a development instance: plain HTTP on loopback without proxy). Opus only, DTX and in-band FEC asked in the answer. Each speaker's packets go as they are to a free receive slot of each other participant; with 16 slots taken, a new speaker takes the slot of the one silent the longest (over 300 ms). Nothing is forwarded from people without `SPEAK` (`suppressed`) or self-muted, nor to the self-deafened. Datagrams over 1 500 bytes and over 200 packets/s per participant are dropped. Speaking comes from the RFC 6464 audio level (louder than -50 dBov starts it, 300 ms without stops it). A connection that is not up within 10 s or silent for 30 s ends with `media_timeout`. Each participant keeps at most 4 remembered source addresses, recorded only for packets within the rate limit. Each step of the loop runs under `catch_unwind`: a panic in one participant's session drops that participant alone; the async side runs each event and re-check as its own task. Limits: 25 per channel or its `user_limit`, `[voice] max_participants` (default 50) per instance. Permission, role, overwrite, membership and channel changes note the guild in the event hub; the voice service then checks its participants there again through `domain` (`Guilds::voice_access`), disconnecting or toggling `suppressed`. The thread never touches SQLite. The WebRTC crates' logs (`str0m`, `str0m_proto`, `str0m_aws_lc_rs`, `is`, `dimpl`, `sctp_proto`) are capped at `error` by a per-layer filter `RUST_LOG` cannot lift: they print addresses at debug level and warn once per malformed packet.
- A message's `nonce` comes back only in the sender's REST response; the `MESSAGE_CREATE` event, including to the sender's own sessions, never carries it. Nonces are stored, unique per channel and author: a send repeating one returns the message created the first time, without a new event.

### 4.9 Admin web UI

Served by the same binary under `/admin`, reserved to instance administrators. Server-rendered HTML (askama) with htmx for partial updates, assets embedded at compile time, no JavaScript build step ([ADR-0009](docs/adr/0009-admin-ui-server-rendered-htmx.md)). It covers instance settings, registration mode, users (disable, reset password), storage usage, backups and the update status. Protected by session auth, CSRF tokens and strict `Content-Security-Policy`.

### 4.10 Operations

```
mellow-server init      # create mellow.toml and the data directory, first admin account
mellow-server serve     # run the server
mellow-server update    # download, verify and install the latest release (restart the service after)
mellow-server backup    # consistent snapshot of the database + attachments
mellow-server admin ... # CLI administration (create user, reset password, …)
```

- **Configuration:** one `mellow.toml` file, every key overridable by `MELLOW_*` environment variables. Validation fails fast at startup with a precise message. The optional `[voice]` table (`enabled`, `udp_port`, `public_address`, `max_participants`) is absent by default: voice is on, on UDP 50000.
- **TLS:** with a public domain (`[tls]` in `mellow.toml`), the server terminates TLS itself with rustls and obtains and renews certificates through ACME TLS-ALPN-01 on port 443, so no other port is needed ([ADR-0013](docs/adr/0013-built-in-tls-with-acme-tls-alpn.md)). Certificate and account keys live in a private directory outside the data directory. Running behind an existing reverse proxy is supported instead (`behind_tls_reverse_proxy`), trusting `X-Forwarded-For` from loopback and from the networks listed in `trusted_proxy_networks` (a proxy in a container network).
- **Updates:** releases are signed with an Ed25519 key (minisign format). `update` refuses any binary whose signature does not verify against the public key built into the current binary, checks the trusted comment names the expected asset, refuses an install location another account could change, replaces the executable atomically on Unix and keeps the previous one for `--rollback`, which restores the executable only ([ADR-0003](docs/adr/0003-self-update-with-signed-releases.md), [ADR-0016](docs/adr/0016-sign-releases-in-a-protected-ci-environment.md), [ADR-0018](docs/adr/0018-roll-back-the-executable-only-and-guard-the-install-location.md)). The shared crate `mellow-update` does this for the server and the desktop app.
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

- **Voice/video:** an SFU module (`str0m`) inside the same binary, one UDP port, signalling over the gateway (ADR-0030; ROADMAP item 16).
- **Search:** SQLite FTS5 on message content.
- **Object storage:** an S3-compatible attachment backend behind the existing storage trait.
- **Larger instances:** if a single SQLite writer becomes the bottleneck, measure first; Postgres support would be a new ADR.
