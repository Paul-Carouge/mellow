# 0036. Web client only; native clients come back later, possibly with GPUI

- Status: Accepted
- Date: 2026-11-02
- Supersedes: [ADR-0010](0010-desktop-ui-with-slint.md), [ADR-0023](0023-release-only-the-linux-server-during-the-web-phase.md)
- Sets aside until a native client starts: [ADR-0008](0008-android-push-notifications.md), [ADR-0011](0011-shared-client-core-with-uniffi.md), [ADR-0017](0017-desktop-app-updates-itself-from-signed-releases.md), the desktop parts of [ADR-0018](0018-roll-back-the-executable-only-and-guard-the-install-location.md)

## Context
Since 2026-10-22 all new work targets the web client (ADR-0023). The Slint desktop app still had to compile and pass CI, and every protocol change touched it, but it had received nothing new since `v0.7.0` and had not been released since `v0.8.0`. The Android app (ADR-0008, ADR-0011) was never started. On 2026-11-02 the owner asked to remove everything related to the desktop and mobile apps until the web client is ready, and said he is thinking of building the next native client in Rust with GPUI rather than Slint.

## Decision
- The Slint desktop app (`crates/mellow-desktop`), its installer (`install.sh`), its app icons (`release/icons`), the Slint token generator in `xtask` and the desktop system packages in CI are removed. Nothing for Android existed in the repository.
- `mellow-protocol`, `mellow-client-core` (client logic without UI: REST, gateway, state, outbox) and `mellow-update` (now for the server only) stay. A native client, whatever its toolkit, starts from `mellow-client-core`.
- The Phosphor icons and the Figtree fonts that the web client reads move to `design/icons` and `design/fonts`. `design/tokens.json` stays the single source of design tokens; `cargo xtask tokens` writes the web stylesheet, the favicon and the web app icons.
- Releases carry `mellow-server` for Linux x86_64 and aarch64 only, and CI tests on Linux only, as ADR-0023 decided.
- The next native client gets its own ADR when the owner starts it. GPUI (Zed's Rust UI framework) is the candidate the owner named; that ADR compares it with the alternatives, measures it, and decides how it shares tokens and `mellow-client-core`.

## Consequences
- Desktop apps installed up to `v0.8.0` keep working against newer servers while API v1 stays compatible, and never receive an update. A new desktop install is no longer offered.
- Protocol changes touch the server, `mellow-client-core` (which still compiles and is tested) and the web client only.
- DESIGN.md's Slint and Compose notes describe a future client, not code in the repository; they are kept as design intent and revised by the native client's ADR.
- The desktop's code stays in the git history (last commit before removal: the parent of the commit that removes `crates/mellow-desktop`), for reference when a native client starts.
