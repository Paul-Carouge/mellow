# 0003. Update the server in place from signed releases

- Status: Accepted, amended by [ADR-0016](0016-sign-releases-in-a-protected-ci-environment.md) (the signing key lives in a protected CI environment)
- Date: 2026-10-19

## Context
Self-hosters must be able to update without Docker, package managers or manual file juggling. An update mechanism is also a high-value attack target.

## Decision
`mellow-server update` downloads the release for the current platform, verifies its Ed25519 signature (minisign format) against the public key compiled into the running binary, replaces the executable atomically (write to a temporary file, then rename) and keeps the previous binary as `mellow-server.previous` for rollback. Database migrations run on the next start, after an automatic `VACUUM INTO` snapshot. The admin UI shows when a newer version exists; it never updates without an explicit admin action.

## Consequences
- One command updates an instance; one command (`mellow-server update --rollback`) restores the previous binary and the pre-migration snapshot.
- The release signing key is a critical secret: kept offline, used only by the release process, with a documented rotation procedure (new public key shipped in a release signed by the old key).
- Migrations must be forward-only and tested on a copy of real-shaped data.

## Alternatives considered
- **Docker images only:** contradicts the main product goal.
- **Distribution packages (deb/rpm/Homebrew):** good later as additional channels, not as the only path.
- **Unsigned downloads over HTTPS:** a compromised release host would compromise every instance.
