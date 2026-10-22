# CLAUDE.md — Mellow

Project-specific instructions. Global rules (`~/.claude/CLAUDE.md`, `~/.claude/rules/`) still apply; this file only adds what is specific to Mellow.

## What Mellow is
A self-hostable, open-source (AGPL-3.0) Discord alternative. Rust everywhere except the Android UI (Kotlin + Compose Material 3 Expressive). The defining product promise is **trivial self-hosting**: one binary, one data directory, no Docker, one-command updates. Any change that makes installing, updating or backing up an instance harder needs an ADR.

Read before working:
- [ARCHITECTURE.md](ARCHITECTURE.md) — components, layers, data model, protocol, security.
- [DESIGN.md](DESIGN.md) — visual identity, design tokens, screens, accessibility. The UI follows the Dusk direction (ADR-0022, superseding ADR-0012): Discord's concepts and shortcuts in Mellow's own layout (the Navigator), colour, type, shape and motion; never Discord's blurple, logo or fonts. Reference mockup: `design/mockups/dusk-direction.html`.
- [docs/adr/](docs/adr/) — why things are the way they are. Do not contradict an accepted ADR silently: propose a new ADR that supersedes it.
- [ROADMAP.md](ROADMAP.md) — milestones and current scope. **Current focus: the web client only** (owner's decision, 2026-10-22): new work targets `web/` and the server it needs; desktop, Android and other platforms wait until the web is finished, but must keep compiling.
- [docs/development-notes.md](docs/development-notes.md) — where the PR stack stands, the workflow, and traps already hit. Update it at the end of each iteration.

## Repository layout
| Path | Content |
|---|---|
| `crates/mellow-protocol` | Wire types only (DTOs, gateway events, ids, permission bitflags). No I/O, no workspace dependencies. |
| `crates/mellow-server` | Server binary. Modules `http`, `gateway`, `admin` → `domain` → `storage`. Never the reverse. |
| `crates/mellow-client-core` | All client logic (connections, state, cache, outbox). No UI code. |
| `crates/mellow-ffi` | UniFFI export of client-core for Kotlin. Thin: no logic of its own. |
| `crates/mellow-desktop` | Slint desktop app. `.slint` = presentation only. |
| `android/` | Kotlin app: Compose UI → ViewModel → adapter over `mellow-ffi`. No protocol logic in Kotlin. |
| `design/tokens.json` | Single source of truth for design tokens. Never hard-code colors or sizes in Slint, Compose or admin CSS. |

## Commands
```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo deny check
cargo xtask tokens          # regenerate ui/tokens.slint after editing design/tokens.json (--check in CI)
cargo run -p mellow-desktop
```
Crates are added to the workspace when their milestone starts (see ROADMAP.md); today `mellow-protocol`, `mellow-server` (`init`, `serve`, `admin create-user`: REST API and WebSocket gateway), `mellow-client-core` (REST, gateway, state), `mellow-desktop` (Discord-like shell signed in to an instance; `MELLOW_DEMO_WORKSPACE=1` for fake data) and `xtask` (design token generator) exist. Update this section when run commands appear.
CI (`.github/workflows/ci.yml`) runs the same checks, with tests on Linux only while the web client is the focus (ADR-0023).

## Rules specific to this project
- **Protocol changes** start in `mellow-protocol` and are made in the same commit on server and clients. API is versioned (`/api/v1`, `/gateway?v=1`); never break a released version.
- **Permissions** are decided only by the pure function in `mellow-server::domain` (ADR-0007). No ad-hoc permission check in handlers or the gateway.
- **Unknown permission bits** from clients are rejected at the server boundary (`Permissions::from_bits`); only clients keep unknown bits.
- **Database key**: never log `SqliteConnectOptions` (its `Debug` prints the key) and keep `disable_statement_logging()` on them; a test guards it.
- **SQLCipher shutdown**: always `pool.close().await` before the process exits, in code and in tests (ADR-0006 spike results).
- **SQLite**: all writes go through the single writer task; keep transactions short; no I/O or fan-out inside a transaction. Queries use `sqlx` compile-time checked macros.
- **Migrations** are forward-only, embedded, and tested on realistic data. Never edit a released migration.
- **Logs** never contain message content, tokens, passwords, emails or attachment names.
- **IDs** are 64-bit Snowflake-style, serialized as strings in JSON.
- **Client core API** must stay UniFFI-friendly: owned types, no generics or lifetimes, flat error enums.
- **UI thread**: nothing blocking on the Slint event loop or the Android main thread.
- **Dependencies**: prefer what is already in the workspace; any new crate needs a reason in the PR description and must pass `cargo deny`.

## Agents
Use the agent team as defined in the global rules. For Mellow in particular:
- `security-auditor` for anything touching auth, sessions, permissions, uploads, the gateway, the updater, push or crypto.
- `ux-ui-designer` for any screen or component, always against DESIGN.md and the tokens.
- `performance-optimizer` for the gateway fan-out, SQLite write path, message list rendering and app startup.
