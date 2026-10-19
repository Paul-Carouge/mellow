# Mellow — Roadmap

Each milestone ends with something usable end to end. Scope changes are made here first.

## M0 — Foundations
- Workspace scaffold, CI (fmt, clippy, tests, cargo deny), lint configuration.
- Design tokens file and generators (Slint done via `cargo xtask tokens`; Compose theme and admin CSS when those clients start).
- Discord-like desktop shell with fake data: server rail, channel sidebar, chat, member list, user panel (ADR-0012, done; icons, Noto Sans bundling, drawer and member overlay below breakpoints still open).
- Spikes: sqlx + SQLCipher on all targets (ADR-0006, done on macOS, CI covers Linux and Windows), Slint message list with rich text and selection (ADR-0010, done: no blocker, see results), UniFFI async + callbacks on Android (ADR-0011).

## M1 — Text chat, single instance, desktop
- `mellow-server init / serve`, config, SQLite storage, migrations, built-in TLS (ACME).
- Accounts (register, login, sessions, logout per device), registration modes.
- Guilds, categories, text channels, invites.
- Messages: send, edit, delete, replies, history pagination, read states.
- Gateway: Identify, Ready, Resume, message and channel events, presence, typing.
- Desktop client: connect, login, server rail, channel list, message list, composer.

## M2 — Community features
- Roles, permissions, channel overwrites, kick, ban, audit log.
- Direct messages (1:1 and groups).
- Attachments and avatars (encrypted storage, size limits).
- Multi-instance support in the client.
- Admin web UI (settings, users, storage, backups).

## M3 — Android
- `mellow-ffi`, Android app with adaptive layouts.
- Push notifications: UnifiedPush, FCM through the push relay.

## M4 — Self-hosting polish and first release
- `mellow-server update` with signed releases and rollback, `backup`, service install.
- Packaging for desktop (AppImage/deb/rpm, dmg, MSI) and Android release.
- Load test at target scale (10 000 MAU, 1 000 concurrent), security audit, documentation site.
- Release v0.1.0.

## Later (not committed)
- Voice and video (SFU in the server binary).
- Message search (SQLite FTS5).
- End-to-end encrypted DMs.
- S3-compatible attachment storage, OIDC login.
