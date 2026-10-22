# 0023. Release and test only the Linux server while the web client is the focus

- Status: Accepted
- Date: 2026-10-22
- Amends: [ADR-0016](0016-sign-releases-in-a-protected-ci-environment.md), [ADR-0017](0017-desktop-app-updates-itself-from-signed-releases.md)

## Context
On 2026-10-22 the owner decided to finish the web client before porting Mellow to each other platform, one at a time. The web client ships inside `mellow-server` (ADR-0019), and the owner's instance runs the Linux server. Yet every release still built the server and the desktop app for Linux (x86_64, aarch64), macOS and Windows, and every pull request ran the tests on all three systems: long runs for binaries nobody receives anything new in.

## Decision
- Releases carry `mellow-server` for `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu` only. The desktop app and the macOS and Windows targets come back, one platform at a time, when their port starts.
- CI runs the tests on Linux only. Clippy and the workspace tests still build the desktop crate there, so it keeps compiling.
- `install.sh` installs the newest release that carries the desktop app for the platform, not the latest release.

## Consequences
- Installed desktop apps (up to `v0.8.0`) find no executable for their platform in newer releases and offer no update (`Release::has_executable_for`); they keep working against newer servers as long as API v1 stays compatible.
- A new desktop install gets `v0.8.0` until desktop releases resume.
- Regressions specific to macOS or Windows (SQLCipher's crypto backend differs per system, ADR-0006) go unnoticed until their port starts; that port begins by restoring its CI job.
