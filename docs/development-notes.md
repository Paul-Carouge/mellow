# Development notes

Practical notes for the next iterations: how work is organised, and the traps already hit. The
project state (what is done, blocked, next, known gaps) lives in [ROADMAP.md](../ROADMAP.md#status-2026-10-20);
decisions live in [docs/adr/](adr/). Update this file when a new trap or habit appears.

## Where things stand (2026-10-20)
- `main` holds only the first PR (#1). Everything else is an open stack of pull requests, each based on the previous branch:
  #2 → #3 → #4 → #5 → #6 → #7 → #8 → #9 → #10 → #11 (`feat/client-core`) → #12 (`feat/built-in-tls`) → #13 (`feat/desktop-session-persistence`) → #14 (`fix/trusted-proxy-addresses`) → #15 (`feat/desktop-message-actions`) → #16 (`fix/server-message-ordering-and-nonce-deduplication`) → #17 (`feat/client-message-outbox`) → #18 (`fix/session-last-seen-and-expired-purge`) → #19 (`feat/outbox-resend-on-reconnect`) → #20 (`feat/history-pagination-on-scroll-up`) → #21 (`feat/message-list-states`, message-list states of DESIGN.md §5.4) → #22 (`feat/message-list-jump-to-present`, "Jump to present" bar; manual check in the app pending) → #23 (`test/acme-against-pebble`, end-to-end ACME test against Pebble and its CI job) → `feat/guild-roles` (first M2 step: named roles and member roles, server, protocol and client-core state; PR not opened yet). The user checked #20 and #21 by hand in the app on 2026-10-20 (scroll to a channel's top, error and Try again).
- Local try-out instance: `mellow-server init --listen-address 127.0.0.1:18080 --registration-mode open` in the scratchpad, seeded through the REST API (rate limit: 10 sends per 10 s per user, so seed with several users).
- Merge them in that order. After merging one into `main`, retarget the next PR's base to `main` before merging it.
- New work branches from the tip of the stack (today `feat/guild-roles`) and opens a PR stacked on the latest one, until the stack is merged.
- CI never ran: GitHub Actions is blocked by the account's billing. Everything is verified on macOS only.

## Next candidates
1. M2 roles, continued: role reordering, channel overwrites with `VIEW_CHANNEL` filtering, kick/ban/audit log, then desktop server settings (through `ux-ui-designer`).
2. M1 release candidate: run a real instance (`init --domain … --use-staging-certificates`, then production), connect the desktop over https, then tag. The user already runs a test instance they own; ask before changing anything on it.
2. Desktop message list, from DESIGN.md §5.4: the offline state (needs the local cache first) and the no-access state (needs channel permissions, M2).
3. Then an on-disk outbox (needs the local cache of ARCHITECTURE §6 first).

## Workflow that worked
- One branch per milestone step (`feat/…`), atomic Conventional Commits, a `docs:` commit for README/ROADMAP/ARCHITECTURE/ADR.
- Before every PR, run `code-reviewer` and `security-auditor` in parallel on `git diff <base>...HEAD`, fix the findings in `fix(...)` commits, and have the auditor re-check any High fix.
- UI work (Slint screens, tokens) goes to `ux-ui-designer`, with an exact interface contract (property and callback names) so the Rust side can be written in parallel.
- The user approves every push, PR, merge and release.

## Checks
```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
cargo xtask tokens --check
```
A local hook refuses `git commit` when fmt or clippy fail on the touched crate: run them first. The hook checks before the command runs, so a combined "fix && commit" command is refused as a whole.

## Traps already hit
- **SQLCipher**: every pool must be closed (`pool.close().await`) before the process or test ends, or the process crashes at exit (ADR-0006). Integration tests stop the server and await it.
- **Nonces in tests**: a repeated nonce returns the first message, so fixtures sending several messages need a distinct nonce each (see `post_message` in the gateway tests).
- **Clock in tests**: the gateway validates sessions against the real clock; fixtures that log in through the gateway must use real time, not the fixed `NOW`.
- **Integration tests of the client core** run a real server through `initialize_instance` + `serve_instance` on a probed free port (`crates/mellow-client-core/tests/`).
- **Desktop modes**: without environment variables the app resumes the session stored in the OS secret store, or shows the sign-in screen; sign out from the gear menu of the user panel to get back to it. On macOS an unsigned build changes identity on every rebuild, so Keychain asks again for access to the stored entry. `MELLOW_DEMO_WORKSPACE=1` (also implied by the scroll benchmark and `MELLOW_SPIKE_SNAPSHOT_DIRECTORY`) runs the fake-data shell. Snapshots are written by the app itself, because screen capture is not permitted.
- **Desktop sessions**: results of network calls are dropped when `session_generation` changed since they started (sign out, sign in again, server rejection), so a late 401 of the previous session cannot end or forget the new one. Use `run_request` for every call, and check the generation in any new UI-thread loop. Secret store calls are serialized by a mutex in `session_store` and bounded by `SECRET_STORE_TIMEOUT` (10 s); release builds never store or resume a session on a loopback (plain http) instance. Known limit: if the secret store can neither be read at launch nor written after sign-in, an older stored session stays there, valid, until a later launch resumes it.
- **Slint repeaters**: `VecModel::set_vec` resets the model, which recreates every row instance. Any state typed into a row (the inline message editor's draft) must live above the repeater (`editing-message-draft` on `ChatPane`), and rows are addressed by index only through `displayed_message_ids`, remapped after each rebuild (`live_shell/message_actions.rs`).
- **StyledText in layouts**: a `StyledText` inside a `HorizontalLayout` loses its height-for-width, so wrapped messages overlap the next row. Keep it a direct child of the message's `VerticalLayout`; check long messages in a snapshot after any change around it.
- **Snapshots of UI states**: hover and dialogs cannot be captured directly. Temporarily set the window properties (`set_editing_message_index`, `set_deleting_message_index`, …) in `window_snapshot::schedule`, capture with `MELLOW_SPIKE_SNAPSHOT_DIRECTORY`, then revert the file.
- **Outbox rows**: pending and failed messages are extra rows after the confirmed ones; a row index at or past `displayed_message_ids.len()` maps into `displayed_pending_nonces`. They carry `is_own = false`, so no message action ever targets them.
- **Outbox resend**: after `Ready` and `Resumed`, failed messages go out one after another (`resend_failed_messages`, each step through `run_request`), so they keep their order among themselves; sending them in parallel could reorder them. Messages still `Sending` are never part of a batch.
- **Message list scrolling**: `refresh_messages` never scrolls. The Slint list stays pinned at `pinned-distance-from-latest-message` from the bottom (zero by default; set by `keep-distance-from-latest-message` before older rows are prepended); only a user scroll (`scrolled`) re-decides pinning. `scrolled` alone misses flicks that coast to the top, so the older-history check also runs on `content-y` and `content-height` changes. Opening a channel and sending call `scroll_to_latest_message_after_layout` / `invoke_scroll_to_latest_message` explicitly.
- **Row 0 height in the message list**: never pin the list at a distance (`keep-distance-from-latest-message`) while row 0 is on screen and changing height. Slint 1.18's ListView estimates its content height from the visible rows, and the estimate and the pinned position then chase each other every frame ("long callback/instantiation chain detected" warnings). The history-start block above row 0 compensates its own height changes (`keep-rows-in-place-after-history-start-change`), and an empty older page skips the pin.
- **Jump to present**: the bar follows `is-viewing-older-messages`, not `is-pinned-to-latest-message`, because `keep-distance-from-latest-message` pins the list far from the bottom while older pages are prepended. `scrolled` reads the position a wheel or flick animation starts from (Slint 1.18 `Flickable`), so a scroll coasting to the bottom is released there and pinned again by `pin-when-latest-message-reached` on `content-y`, `content-height` and `visible-height` changes, at distance zero only. Rust calls `note-new-message` for each `MessageCreate` from someone else in the open channel.
- **History failures**: a failed history request puts the channel in `failed_history_channel_ids`, which stops the automatic older-history loads on scroll. Only "Try again" (`retry_loading_history`), reopening the channel or the next `Ready` clears it.
- **History pages**: always request `limit: HISTORY_PAGE_SIZE`, since `insert_history_page` reads a shorter page as the channel's start. An older page is inserted only if the channel's oldest cached message is still its `before` cursor, so the cache never has a gap.
- **Server text in the UI**: never show `ClientError::Api`'s `message` in a trusted place; build the text from `ApiErrorCode` (see `message_action_error_text`).
- **Desktop threading**: state lives on the UI thread in `Rc<RefCell<LiveShell>>`. Network futures run on the tokio runtime and come back through `slint::spawn_local`. Never hold a `RefCell` borrow across a call that borrows again.
- **Message markdown**: escape `\` before `<`, or `\<font>` re-enables HTML styling (fixed, with a regression test).
- **TLS listener**: hyper ignores its header timeout unless a timer is set. Do not enable HTTP/2 keep-alive pings, since their answers keep idle connections alive. `axum-server`'s `http1_only()` breaks WebSocket upgrades.
- **ACME**: tests point at an unreachable directory URL (`https://127.0.0.1:9/directory`), so they never touch Let's Encrypt. The end-to-end test against Pebble is `#[ignore]`d: download a release binary from github.com/letsencrypt/pebble, `chmod +x` it (the archive drops the executable bit), then `MELLOW_PEBBLE_BINARY=/path/to/pebble cargo test -p mellow-server --lib tls::tests -- --ignored`. It sets `PEBBLE_WFE_NONCEREJECT=0` because rustls-acme does not retry a bad-nonce refusal.
- **Reverse proxy in Docker**: the proxy reaches Mellow from its container address, not loopback, so `X-Forwarded-For` is ignored unless that address is in `trusted_proxy_networks`. Trust the proxy's `/32`, never the whole Docker network (its gateway is the host and the userland proxy's source).
- **Role checks**: every role action goes through `RoleManager::can_manage_role` (rank and permission subset); `add_member_role` repeats the subset check in its SQL and `update_role_fields` writes only the bits the actor holds, so a permission granted meanwhile is neither handed out nor erased. Never add a role action that skips them.
- **Live instance check**: `crates/mellow-client-core/examples/live_instance_check.rs` (untracked, never commit it) drives a real instance; the user's instance has closed registration, so its account `claude_rc_check` was made with `admin create-user` through `docker exec` (password in the session scratchpad only).
- **Logs**: never log serde or ACME error text verbatim. Serde quotes the offending value (possibly message content), and the certificate authority's messages can quote the contact email.
- **Licences**: `webpki-roots` and `webpki-root-certs` need the CDLA-Permissive-2.0 exception in `deny.toml`.
- **Clippy pedantic**: `too_many_lines` (100) shows up on large `match` reducers and connection loops; extract a helper rather than allowing the lint.
- **Editing files**: rustfmt reformats after each edit. Re-read a file before an exact-match edit if a formatter ran since.
