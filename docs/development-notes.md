# Development notes

Practical notes for the next iterations: how work is organised, and the traps already hit. The
project state (what is done, blocked, next, known gaps) lives in [ROADMAP.md](../ROADMAP.md#status-2026-10-19);
decisions live in [docs/adr/](adr/). Update this file when a new trap or habit appears.

## Where things stand (2026-10-19)
- `main` holds only the first PR (#1). Everything else is an open stack of pull requests, each based on the previous branch:
  #2 → #3 → #4 → #5 → #6 → #7 → #8 → #9 → #10 → #11 (`feat/client-core`) → #12 (`feat/built-in-tls`).
- Merge them in that order. After merging one into `main`, retarget the next PR's base to `main` before merging it.
- New work branches from the tip of the stack (today `feat/built-in-tls`) and opens a PR stacked on the latest one, until the stack is merged.
- CI never ran: GitHub Actions is blocked by the account's billing. Everything is verified on macOS only.

## Next candidates
1. M1 release candidate: run a real instance on a VPS (`init --domain … --use-staging-certificates`, then production), connect the desktop over https, then tag.
2. Desktop session persistence in the OS secret store (Keychain, Secret Service, Credential Manager).
3. End-to-end ACME tests against Pebble (local test certificate authority).
4. Known gaps listed in the ROADMAP status, server writer task and nonce deduplication first.

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
- **Clock in tests**: the gateway validates sessions against the real clock; fixtures that log in through the gateway must use real time, not the fixed `NOW`.
- **Integration tests of the client core** run a real server through `initialize_instance` + `serve_instance` on a probed free port (`crates/mellow-client-core/tests/`).
- **Desktop modes**: without environment variables the app shows the sign-in screen. `MELLOW_DEMO_WORKSPACE=1` (also implied by the scroll benchmark and `MELLOW_SPIKE_SNAPSHOT_DIRECTORY`) runs the fake-data shell. Snapshots are written by the app itself, because screen capture is not permitted.
- **Desktop threading**: state lives on the UI thread in `Rc<RefCell<LiveShell>>`. Network futures run on the tokio runtime and come back through `slint::spawn_local`. Never hold a `RefCell` borrow across a call that borrows again.
- **Message markdown**: escape `\` before `<`, or `\<font>` re-enables HTML styling (fixed, with a regression test).
- **TLS listener**: hyper ignores its header timeout unless a timer is set. Do not enable HTTP/2 keep-alive pings, since their answers keep idle connections alive. `axum-server`'s `http1_only()` breaks WebSocket upgrades.
- **ACME**: tests point at an unreachable directory URL (`https://127.0.0.1:9/directory`), so they never touch Let's Encrypt.
- **Logs**: never log serde or ACME error text verbatim. Serde quotes the offending value (possibly message content), and the certificate authority's messages can quote the contact email.
- **Licences**: `webpki-roots` and `webpki-root-certs` need the CDLA-Permissive-2.0 exception in `deny.toml`.
- **Clippy pedantic**: `too_many_lines` (100) shows up on large `match` reducers and connection loops; extract a helper rather than allowing the lint.
- **Editing files**: rustfmt reformats after each edit. Re-read a file before an exact-match edit if a formatter ran since.
