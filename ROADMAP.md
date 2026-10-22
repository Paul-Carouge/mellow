# Mellow — Roadmap

Each milestone ends with something usable end to end. Scope changes are made here first.

## Status (2026-10-22)

**Current focus: the web client only** (owner's decision, 2026-10-22). Until the web client is finished, all new features, UI work and fixes target `web/` and the server and protocol it needs. The desktop app (Slint), Android and other platforms wait; they come back once the web is done, and then catch up on what the web gained. Meanwhile the desktop only has to keep compiling and passing CI (shared tokens, client-core); no new desktop work unless the owner asks. Reason: building every feature twice halved the pace.

### Done
Merged into `main`: #1 to #60. Released up to `v0.7.0` (2026-10-22), which runs on the owner's instance (server and embedded web client) and as signed desktop builds for Linux (x86_64, aarch64), macOS (Apple Silicon) and Windows.
- M1: server, gateway, client core, desktop on real data, built-in TLS with ACME, outbox, history pagination, message-list states (`v0.1.0`).
- M2 so far: roles and their order, channel permission overwrites and `VIEW_CHANNEL` filtering, kick and ban, audit log (API, client core, desktop Roles settings); one-to-one direct messages; create or join a server with an invite code, invite people, create a channel, leave a server; account settings (username, password); attachments with picture previews; avatars and banners (animated, lossless, ADR-0021); Melli, the mascot and app icon.
- M2.5, web client (ADR-0019): served by the server binary at `/` since `v0.3.0`, with cookie sessions, servers, channels, messages (replies, edit, delete, mentions, unread divider, typing), members, direct messages, attachments, avatars, My account and Profile settings, the quick switcher, phone layout.
- Pulled forward from M4: `mellow-server update`, the Docker Compose kit with `mellow-update`, in-app desktop updates, a one-line desktop installer (ADR-0016 to ADR-0018).
- Visual direction Dusk (ADR-0022): tokens and dark app icon (`v0.6.0`), the Navigator layout and the mockup's content features on web and desktop (`v0.7.0`).

### Blocked
- Nothing blocked. CI runs on Linux, macOS and Windows.

### Next, in order (web client and the server it needs)
The goal is Discord's feature set on the web client. In order:
1. **French interface** (owner's request): done on the web, waiting for a native speaker's review before release. The whole web UI is in natural French (tutoiement) next to English, picked from the browser language (typed TS catalogues, ADR-0022).
2. **Markdown in messages**: done on the web: bold, italic, underline, strikethrough, inline code, code blocks, quotes, spoilers, headings, lists and links, as Discord renders them (subtext, numbered lists and "Copy code" not yet).
3. **Reactions and pinned messages**: emoji reactions on messages (add, remove, who reacted) and pinned messages per channel, with their gateway events. Server and protocol done (#64, migration 0012, `ADD_REACTIONS`); web UI on `feat/web-reactions-and-pins` (chips, quick reactions, emoji picker, who reacted, moderator removal, pin confirmations, pins panel).
4. **Server settings on the web**: done on `feat/web-server-settings` (overview with rename and delete, roles, invites, members, bans, moderation log, with the new rename, delete and invite APIs); channel settings (name, topic, permissions) are the next step of this item.
5. **Unread and mentions from the server**: `last_message_id` per channel and mention counts in `Ready`, so channels opened in no session still show unread state and badges.
6. **Presence**: idle, do not disturb, invisible, and a custom status.
7. **Group direct messages** and a "New message" picker.
8. **Categories and channel management**: create categories, reorder channels, channel topics.
9. **Message search** (SQLite FTS5), then threads, custom emoji, link previews, notifications (browser), voice and video (own ADR).
The desktop, Android and the admin web UI wait until the web is finished, then catch up.

### Known gaps and debts
- Server: single SQLite writer task (ARCHITECTURE §4.3) not implemented; only message sends are serialized (per-process lock, so a second process such as `admin` writing messages would not be ordered); nonces are deduplicated forever per channel and author, not for a time window, and a deleted message frees its nonce (an outbox retry after the sender deleted it recreates it); the database pool keeps sqlx's 30 s acquire timeout and history reads have no rate limit, so a read flood also queues sends behind the send lock; `messages.author_id` has no `ON DELETE` rule or index, to settle with account deletion; queries are runtime SQL, not compile-time checked (`sqlx` offline data to set up); member list capped at 1 000 instead of paginated; rate limits in memory, per process, and the shared table fails closed when saturated; Windows refused until the master key file gets a protected DACL.
- TLS: the ACME path is covered by one end-to-end test against Pebble (ignored by default, run in CI on Linux; renewal and the staging/production switch are not covered); certificate and account keys are protected by file permissions only, not encrypted with the master key; no HSTS header yet (no web pages served until the admin UI); connection caps and timeouts apply to the built-in TLS listener only, not to the plain HTTP one behind a proxy; slow request bodies are bounded by the 75 s idle limit per byte, not by a total deadline.
- Gateway: no per-IP cap on concurrent connections (only an opening rate and a global cap); `GUILD_CREATE` after joining does not list who is already online; events of a restricted channel cost one member query each (not measured; ADR-0015); the 10 s identify timeout and the global connection cap are not covered by tests.
- Client core: the outbox lives in memory only (unsent messages are lost when the app quits); failed messages are resent automatically after each reconnect, so one the sender deleted from another device before its lost answer is recreated (a deleted message frees its nonce); order holds only inside a resend batch (a message typed meanwhile, or one that failed after the batch started, can land before older ones); a batch stopped by a failure waits for the next reconnect or a manual retry (no timer); a refused message whose text cannot go back to the composer (not empty, or another channel shown) stays failed and is refused again at each reconnect until discarded; a message confirmed by the gateway before its REST answer shows twice for that moment (the gateway event carries no nonce), and stays twice if that answer then fails; "Not sent." after a lost answer can be wrong, and its Delete only hides the message locally while the server kept it (to word as "couldn't confirm" once failures are told apart in the UI); a send the instance refuses (invalid text, missing permission, deleted reply target) goes back to the composer with the reason; no message cache on disk; older history loads when the list nears its top, with known limits: the list keeps its distance to the bottom after a page loads until the next scroll (a message arriving meanwhile shifts the view by its height), a channel left open keeps every page it loaded plus new messages (trimmed back to 500 when left), a page whose answer no longer joins the cache (reconnect, trim, oldest message deleted) is dropped and asked again at the next scroll or list change, and every incoming message rebuilds all rows of a deeply scrolled channel (not measured); unread markers only know cached messages, so after launch no channel shows unread until a message arrives (needs a `last_message_id` per channel in the protocol); a member join or leave racing the first member-list fetch is lost until the next `Ready`; `RestClient` sign-in methods take `&mut self`, which UniFFI cannot export, so the token must move behind interior mutability before `mellow-ffi` (M3).
- Desktop: message-list states (DESIGN.md §5.4) show skeletons, the channel-start hero and "Try again" errors, with known limits: no "Edit channel" in the hero, no offline or no-access state, the error gives no reason, the "Jump to present" bar answers `Esc` in the composer but not `End` (the list takes no keyboard focus yet), a full reconnect (`Ready`) jumps back to the latest message although the reader was in older history, a mouse wheel without scroll phases (non-Apple mice) was never tried against the bottom pinning, a failed latest page reads "Couldn't load older messages." when a new message arrived before the failure (Try again still asks the right page), a very short older page arriving while the list top is on screen is not measured against the ListView height oscillation; one stored session at a time (multi-instance in M2); secret store access never tested on Linux and Windows; the session generation guard (late results of an ended session) and the sign-out flow have no automated test (needs a Slint testing backend driving `spawn_local`); the stored session is removed unconditionally, so two Mellow processes of one OS user can wipe each other's entry (delete only the entry holding the ended token, with multi-instance in M2); a sign-out that failed to remove the stored entry and to reach the instance resumes at next launch instead of revoking the token there; typing indicators are not wired in the UI yet; edit, delete and reply work, with known limits: "(edited)" sits on its own line, Edit and Delete live in the hover bar until the More menu exists, an editor recreated after scrolling away takes the focus back, Arrow-Up does not scroll to the message it edits, and screen readers can still reach the content behind the delete dialog; mentions are not parsed; direct messages: one-to-one only (no groups, no "New message" picker: conversations start from a member's profile popout), the profile popout and keyboard focus on member and conversation rows are lost when the list is rebuilt (presence changes), and home with no conversation still shows an inert composer; block-level message rendering (code blocks, quotes); drawer and member-list overlay below breakpoints; hover action bar and row navigation from the keyboard; screen-reader pass (VoiceOver/NVDA/Orca) never done; last avatar clipped by ~3 px; Mellow logo not designed; trade-dress review before any public release; server settings (Roles only so far) have no Members tab on a role, no drag-and-drop reordering, no colour readability preview and no custom colour (presets only), and their animations ignore reduced motion until that setting exists.
- M0 leftovers: UniFFI async/callback spike for Android; Compose and admin CSS token generators.
- Roles: every new role starts lowest (position 1) until reordered; a role holding a permission a manager lacks is out of their reach whatever its position; deleting or taking away a role is not checked against a concurrent permission change of that role, and a reordering in flight still goes through, within the manager's former rank, if their role or `MANAGE_ROLES` is taken away meanwhile (nuisance only, no privilege gained; the order itself is guarded).
- Overwrites: categories do not pass their overwrites to their channels (each channel has its own, no Discord-style sync); a member's overwrite stays after they leave, as on Discord; the desktop hides what channel permissions forbid (locked composer, Reply and Edit in read-only channels, Delete only with `MANAGE_MESSAGES`), but in a read-only channel keyboard focus has nowhere to go after "Jump to present" or "Try again" (it is dropped).
- Server dialogs: every "Invite people" creates a new invite that never expires and has no use limit, and the desktop cannot list or revoke invites yet; only text channels at the top level can be created (no categories, no topic).
- Moderation: kicks and bans keep the member's messages (no "delete recent messages" option); the ban list is capped at 1 000, not paginated; a kick or ban racing a role change of its target is decided on the ranks read before the write. A ban answers 404 for an unknown user id, so a guild owner can tell whether an id belongs to an account (ids are not guessable at the rate limit); bans per guild have no quota yet. The audit log keeps entries forever (no retention period yet), and a repeated ban of an already banned user records a new entry; every invite created by any member is recorded, which can crowd the log of a busy server.
- Web server settings: no channel settings yet; no Members tab on a role, no drag-and-drop reordering, presets only for colours, no preview on the other theme; invites made there never expire and have no use limit; deleting a server is one cascade in one transaction (fine for community sizes, not measured on a server with a very long history).

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

## M2.5 — Web client (ADR-0019, accepted)
- Svelte SPA embedded in the server binary, served at `/`; tokens from `design/tokens.json`; TypeScript types generated from `mellow-protocol`.
- Browser session handling (cookie flow), Content-Security-Policy, HSTS.
- Same screens as the desktop: sign-in, servers, channels, messages, settings.

## M3 — Android
- `mellow-ffi`, Android app with adaptive layouts.
- Push notifications: UnifiedPush, FCM through the push relay.

## M4 — Self-hosting polish and first release
- `mellow-server update` with signed releases and rollback: written early (2026-10-20), with the desktop updater and a Docker Compose kit; `backup` and service install remain.
- Packaging for desktop (AppImage/deb/rpm, dmg, MSI) and Android release; until then `install.sh` installs the desktop app per user on Linux and macOS.
- Load test at target scale (10 000 MAU, 1 000 concurrent), security audit, documentation site.
- Announced public release (`v0.1.0` exists since 2026-10-20 for testing), after the trade-dress review.

## Later (not committed)
- Voice and video (SFU in the server binary).
- Message search (SQLite FTS5).
- End-to-end encrypted DMs.
- S3-compatible attachment storage, OIDC login.
