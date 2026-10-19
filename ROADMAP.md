# Mellow — Roadmap

Each milestone ends with something usable end to end. Scope changes are made here first.

## Status (2026-10-19)

### Done, in review
Stacked pull requests, to merge in this order: #2 workspace + SQLCipher storage → #3 Slint spike → #4 Discord-like desktop shell + token generator → #5 Figtree font and Phosphor icons → #6 server `init`/`serve` (M1.1) → #7 accounts and sessions (M1.2) → #8 guilds, channels, invites (M1.3a) → #9 messages and read states (M1.3b) → #10 WebSocket gateway (M1.4) → #11 client core and desktop on real data (M1.5). Every server PR was reviewed by `code-reviewer` and `security-auditor`, with findings fixed.

### Blocked
- **CI does not run**: GitHub Actions refuses jobs because of the account's billing (Settings → Billing & plans). Until fixed, everything is verified on macOS only; SQLCipher on Linux and Windows is unproven, so ADR-0006 stays *Proposed*.

### Next, in order
1. **Built-in TLS (ACME)**, then M1 release candidate.
2. **Desktop session persistence** in the OS secret store (Keychain, Secret Service, Credential Manager), so the app does not ask for the password on every launch.

### Known gaps and debts
- Server: single SQLite writer task (ARCHITECTURE §4.3) not implemented, so a message id can be issued before a concurrent one commits and an `after` cursor could skip it (generate ids inside the writer task); nonces are not deduplicated, so an outbox retry after a lost response creates a duplicate; `messages.author_id` has no `ON DELETE` rule or index, to settle with account deletion; queries are runtime SQL, not compile-time checked (`sqlx` offline data to set up); `sessions.last_seen` never updated and expired sessions never purged; member list capped at 1 000 instead of paginated; rate limits in memory, per process, and the shared table fails closed when saturated; Windows refused until the master key file gets a protected DACL.
- Gateway: no per-IP cap on concurrent connections (only an opening rate and a global cap); `GUILD_CREATE` after joining does not list who is already online; event delivery is not filtered by channel permissions (needed with M2 overwrites); the 10 s identify timeout and the global connection cap are not covered by tests.
- Client core: no outbox (a message whose send fails goes back to the composer, no retry row); no message cache on disk; only the latest history page is loaded (no scroll-up pagination); unread markers only know cached messages, so after launch no channel shows unread until a message arrives (needs a `last_message_id` per channel in the protocol); a member join or leave racing the first member-list fetch is lost until the next `Ready`; `RestClient` sign-in methods take `&mut self`, which UniFFI cannot export, so the token must move behind interior mutability before `mellow-ffi` (M3).
- Desktop: the session token lives in memory only and is revoked on exit, so every launch asks for the password; edit, delete, replies and typing indicators are not wired in the UI yet; mentions are not parsed; new messages always scroll to the bottom; direct messages (home) are empty; block-level message rendering (code blocks, quotes); drawer and member-list overlay below breakpoints; hover action bar and row navigation from the keyboard; screen-reader pass (VoiceOver/NVDA/Orca) never done; last avatar clipped by ~3 px; Mellow logo not designed; trade-dress review before any public release.
- M0 leftovers: UniFFI async/callback spike for Android; Compose and admin CSS token generators.
- M2 follow-ups recorded in ADR-0007: roles and channel overwrites, `VIEW_CHANNEL` filtering, bans checked on invite joins, invites of departed members.

## M0 — Foundations
- Workspace scaffold, CI (fmt, clippy, tests, cargo deny), lint configuration.
- Design tokens file and generators (Slint done via `cargo xtask tokens`; Compose theme and admin CSS when those clients start).
- Discord-like desktop shell with fake data: server rail, channel sidebar, chat, member list, user panel (ADR-0012, done with bundled Figtree and Phosphor icons; drawer and member overlay below breakpoints still open).
- Spikes: sqlx + SQLCipher on all targets (ADR-0006, done on macOS, CI covers Linux and Windows), Slint message list with rich text and selection (ADR-0010, done: no blocker, see results), UniFFI async + callbacks on Android (ADR-0011).

## M1 — Text chat, single instance, desktop
- `mellow-server init / serve`, config, SQLite storage, migrations (done in M1.1: encrypted database, master key, pre-migration backups, health route, graceful shutdown); built-in TLS (ACME) still to do.
- Accounts (register, login, sessions, logout per device), registration modes (done in M1.2: open/closed, invite-only comes with invites; `admin create-user`).
- Guilds, categories, text channels, invites (done in M1.3a, with owner/@everyone permissions; named roles and overwrites in M2).
- Messages: send, edit, delete, replies, history pagination, read states (done in M1.3b).
- Gateway: Identify, Ready, Resume, message and channel events, presence, typing (done in M1.4; presence is online/offline only, idle and do-not-disturb come with the client).
- Desktop client: connect, login, server rail, channel list, message list, composer (done in M1.5 on `mellow-client-core`: REST, gateway with resume and backoff, state reducer; `MELLOW_DEMO_WORKSPACE=1` keeps the fake-data shell).

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
