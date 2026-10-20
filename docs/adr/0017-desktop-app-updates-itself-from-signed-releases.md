# 0017. The desktop app updates itself from signed releases

- Status: Accepted
- Date: 2026-10-20

## Context
Users should get new versions of the desktop app as soon as they are released, from the app's settings, without a package manager. Releases are built and signed in CI (ADR-0016); the server already installs them with `mellow-server update` (ADR-0003). Desktop packaging (dmg, MSI, AppImage, deb/rpm) is planned for M4 and does not exist yet: the app runs from the plain executable published in each release.

## Decision
- The desktop app reuses the `mellow-update` crate shared with the server: latest release from the GitHub API, this platform's `mellow-desktop` asset, minisign signature checked against the key built into the app with the asset's exact name as trusted comment, install by renames next to the running executable, which is kept as `<executable>.previous`.
- It checks 10 seconds after launch and every 6 hours. A newer version shows a badge on the user panel's settings button; User settings → About shows the state and one action. It never downloads or installs without the user's click ("Update Mellow"), then offers "Restart Mellow", which starts the new executable (its path is resolved at launch, since Linux reports a replaced running executable as "(deleted)") and quits; a failed restart says so. Once installed, checks stop until the restart.
- Background check failures stay silent; failures the user asked for show a fixed message chosen from the error kind, never the error's text. A folder the app cannot write to, or a release without this platform's asset, is reported once with a link to the releases page, and checks stop until the next launch rather than offering the same failing update every 6 hours. Only plain `X.Y.Z` tags count as releases, so the version shown before any verification never carries text chosen by the release host.
- Development builds never update themselves.

## Consequences
- One code path verifies every Mellow update, server or desktop.
- Replacing the executable inside a macOS `.app` bundle or a future package breaks code signatures and package-manager ownership: when packaging lands (M4), each package format decides whether it keeps this updater or defers to its own (Sparkle-style feeds, MSI upgrades, distribution repositories). macOS asks again for the Keychain entry after each update while builds are not signed by Apple (known since M1).
- The app contacts `api.github.com` periodically (User-Agent `mellow-update`, without the version), which reveals to GitHub that a Mellow client runs at that address and roughly how long it stays open; acceptable for now, to make optional in settings before a public release.
- On Windows the install location is not checked (no installer yet): an account that can modify the folder holding the executable can replace it anyway, with or without the updater.
- No automatic roll-back: a broken release is fixed by a newer one; `<executable>.previous` allows a manual one.

## Alternatives considered
- **Self-update crates (`self_update`, `cargo-dist` updaters)**: more dependencies and their own trust model, while the server needs the same verification anyway.
- **Only notify, link to the releases page**: simpler, but the user asked to update from the settings.
- **Silent automatic install**: surprises users mid-conversation and hides a security-relevant action.
