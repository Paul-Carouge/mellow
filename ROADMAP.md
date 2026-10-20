# Mellow — Roadmap

Each milestone ends with something usable end to end. Scope changes are made here first.

## Status (2026-10-20)

### Done
Merged into `main` on 2026-10-20: #1 to #29, from the workspace and SQLCipher storage through M1 (server, gateway, client core, desktop on real data, built-in TLS with ACME, outbox, history pagination, message-list states) to the first M2 steps (#24 guild roles, #25 role reordering, #27 channel permission overwrites and `VIEW_CHANNEL` filtering, #28 kick and ban, #29 audit log). Every server PR was reviewed by `code-reviewer` and `security-auditor`, with findings fixed. No release is tagged yet.

### Blocked
- **CI does not run**: GitHub Actions refuses jobs because of the account's billing (Settings → Billing & plans). Until fixed, everything is verified on macOS only; SQLCipher on Linux and Windows is unproven, so ADR-0006 stays *Proposed*.

### Next, in order
1. **M1 release candidate**: end-to-end run on a real VPS (domain, Let's Encrypt staging then production, desktop client over https), then tag. 2026-10-20: the user's instance (behind Caddy) was redeployed from `main` after #28 (migrations 4 to 7 applied, pre-migration backup taken) and the end-to-end client check passed entirely (sign-in, gateway, send, nonce retry, edit, delete, refusals, logout). Left before tagging: version bump and tag. The ACME path is covered by the Pebble test only (user's decision: the instance stays behind Caddy).
2. **M2, continued**: the desktop server-settings screens (roles, overwrites, members, bans, audit log), then direct messages. Done on branches, not merged: client-core REST methods for every M2 endpoint; channel permissions in the desktop UI.

### Known gaps and debts
- Server: single SQLite writer task (ARCHITECTURE §4.3) not implemented; only message sends are serialized (per-process lock, so a second process such as `admin` writing messages would not be ordered); nonces are deduplicated forever per channel and author, not for a time window, and a deleted message frees its nonce (an outbox retry after the sender deleted it recreates it); the database pool keeps sqlx's 30 s acquire timeout and history reads have no rate limit, so a read flood also queues sends behind the send lock; `messages.author_id` has no `ON DELETE` rule or index, to settle with account deletion; queries are runtime SQL, not compile-time checked (`sqlx` offline data to set up); member list capped at 1 000 instead of paginated; rate limits in memory, per process, and the shared table fails closed when saturated; Windows refused until the master key file gets a protected DACL.
- TLS: the ACME path is covered by one end-to-end test against Pebble (ignored by default, run in CI on Linux; renewal and the staging/production switch are not covered); certificate and account keys are protected by file permissions only, not encrypted with the master key; no HSTS header yet (no web pages served until the admin UI); connection caps and timeouts apply to the built-in TLS listener only, not to the plain HTTP one behind a proxy; slow request bodies are bounded by the 75 s idle limit per byte, not by a total deadline.
- Gateway: no per-IP cap on concurrent connections (only an opening rate and a global cap); `GUILD_CREATE` after joining does not list who is already online; events of a restricted channel cost one member query each (not measured; ADR-0015); the 10 s identify timeout and the global connection cap are not covered by tests.
- Client core: the outbox lives in memory only (unsent messages are lost when the app quits); failed messages are resent automatically after each reconnect, so one the sender deleted from another device before its lost answer is recreated (a deleted message frees its nonce); order holds only inside a resend batch (a message typed meanwhile, or one that failed after the batch started, can land before older ones); a batch stopped by a failure waits for the next reconnect or a manual retry (no timer); a refused message whose text cannot go back to the composer (not empty, or another channel shown) stays failed and is refused again at each reconnect until discarded; a message confirmed by the gateway before its REST answer shows twice for that moment (the gateway event carries no nonce), and stays twice if that answer then fails; "Not sent." after a lost answer can be wrong, and its Delete only hides the message locally while the server kept it (to word as "couldn't confirm" once failures are told apart in the UI); a send the instance refuses (invalid text, missing permission, deleted reply target) goes back to the composer with the reason; no message cache on disk; older history loads when the list nears its top, with known limits: the list keeps its distance to the bottom after a page loads until the next scroll (a message arriving meanwhile shifts the view by its height), a channel left open keeps every page it loaded plus new messages (trimmed back to 500 when left), a page whose answer no longer joins the cache (reconnect, trim, oldest message deleted) is dropped and asked again at the next scroll or list change, and every incoming message rebuilds all rows of a deeply scrolled channel (not measured); unread markers only know cached messages, so after launch no channel shows unread until a message arrives (needs a `last_message_id` per channel in the protocol); a member join or leave racing the first member-list fetch is lost until the next `Ready`; `RestClient` sign-in methods take `&mut self`, which UniFFI cannot export, so the token must move behind interior mutability before `mellow-ffi` (M3).
- Desktop: message-list states (DESIGN.md §5.4) show skeletons, the channel-start hero and "Try again" errors, with known limits: no "Edit channel" in the hero, no offline or no-access state, the error gives no reason, the "Jump to present" bar answers `Esc` in the composer but not `End` (the list takes no keyboard focus yet), a full reconnect (`Ready`) jumps back to the latest message although the reader was in older history, a mouse wheel without scroll phases (non-Apple mice) was never tried against the bottom pinning, a failed latest page reads "Couldn't load older messages." when a new message arrived before the failure (Try again still asks the right page), a very short older page arriving while the list top is on screen is not measured against the ListView height oscillation; one stored session at a time (multi-instance in M2); secret store access never tested on Linux and Windows; the session generation guard (late results of an ended session) and the sign-out flow have no automated test (needs a Slint testing backend driving `spawn_local`); the stored session is removed unconditionally, so two Mellow processes of one OS user can wipe each other's entry (delete only the entry holding the ended token, with multi-instance in M2); a sign-out that failed to remove the stored entry and to reach the instance resumes at next launch instead of revoking the token there; typing indicators are not wired in the UI yet; edit, delete and reply work, with known limits: "(edited)" sits on its own line, Edit and Delete live in the hover bar until the More menu exists, an editor recreated after scrolling away takes the focus back, Arrow-Up does not scroll to the message it edits, and screen readers can still reach the content behind the delete dialog; mentions are not parsed; direct messages (home) are empty; block-level message rendering (code blocks, quotes); drawer and member-list overlay below breakpoints; hover action bar and row navigation from the keyboard; screen-reader pass (VoiceOver/NVDA/Orca) never done; last avatar clipped by ~3 px; Mellow logo not designed; trade-dress review before any public release.
- M0 leftovers: UniFFI async/callback spike for Android; Compose and admin CSS token generators.
- Roles: every new role starts lowest (position 1) until reordered; a role holding a permission a manager lacks is out of their reach whatever its position; deleting or taking away a role is not checked against a concurrent permission change of that role, and a reordering in flight still goes through, within the manager's former rank, if their role or `MANAGE_ROLES` is taken away meanwhile (nuisance only, no privilege gained; the order itself is guarded).
- Overwrites: categories do not pass their overwrites to their channels (each channel has its own, no Discord-style sync); a member's overwrite stays after they leave, as on Discord; the desktop hides what channel permissions forbid (locked composer, Reply and Edit in read-only channels, Delete only with `MANAGE_MESSAGES`), but in a read-only channel keyboard focus has nowhere to go after "Jump to present" or "Try again" (it is dropped).
- Moderation: kicks and bans keep the member's messages (no "delete recent messages" option); the ban list is capped at 1 000, not paginated; a kick or ban racing a role change of its target is decided on the ranks read before the write. A ban answers 404 for an unknown user id, so a guild owner can tell whether an id belongs to an account (ids are not guessable at the rate limit); bans per guild have no quota yet. The audit log keeps entries forever (no retention period yet), and a repeated ban of an already banned user records a new entry; invite creation is not audited.

## M0 — Foundations
- Workspace scaffold, CI (fmt, clippy, tests, cargo deny), lint configuration.
- Design tokens file and generators (Slint done via `cargo xtask tokens`; Compose theme and admin CSS when those clients start).
- Discord-like desktop shell with fake data: server rail, channel sidebar, chat, member list, user panel (ADR-0012, done with bundled Figtree and Phosphor icons; drawer and member overlay below breakpoints still open).
- Spikes: sqlx + SQLCipher on all targets (ADR-0006, done on macOS, CI covers Linux and Windows), Slint message list with rich text and selection (ADR-0010, done: no blocker, see results), UniFFI async + callbacks on Android (ADR-0011).

## M1 — Text chat, single instance, desktop
- `mellow-server init / serve`, config, SQLite storage, migrations (done in M1.1: encrypted database, master key, pre-migration backups, health route, graceful shutdown); built-in TLS done in M1.6 (ACME TLS-ALPN-01, ADR-0013).
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
