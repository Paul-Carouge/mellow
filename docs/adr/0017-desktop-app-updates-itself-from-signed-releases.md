# 0017. The desktop app updates itself from signed releases

- Status: Accepted
- Date: 2026-10-20

## Context
Users should get new versions of the desktop app as soon as they are released, from the app's settings, without a package manager. Releases are built and signed in CI (ADR-0016); the server already installs them with `mellow-server update` (ADR-0003). Desktop packaging (dmg, MSI, AppImage, deb/rpm) is planned for M4 and does not exist yet: the app runs from the plain executable published in each release.

## Decision
- The desktop app reuses the `mellow-update` crate shared with the server: latest release from the GitHub API, this platform's `mellow-desktop` asset, minisign signature checked against the key built into the app with the asset's exact name as trusted comment, install by renames next to the running executable, which is kept as `<executable>.previous`.
- It checks 10 seconds after launch and every 6 hours. A newer version shows a badge on the user panel's settings button; User settings → About shows the state and one action. It never downloads or installs without the user's click ("Update"), then offers "Restart Mellow", which starts the new executable and quits.
- Background check failures stay silent; failures the user asked for show a fixed message chosen from the error kind, never the error's text. A folder the app cannot write to, or a platform without an asset, is reported as "can't update from here" with a link to the releases page.
- Development builds never update themselves.

## Consequences
- One code path verifies every Mellow update, server or desktop.
- Replacing the executable inside a macOS `.app` bundle or a future package breaks code signatures and package-manager ownership: when packaging lands (M4), each package format decides whether it keeps this updater or defers to its own (Sparkle-style feeds, MSI upgrades, distribution repositories). macOS asks again for the Keychain entry after each update while builds are not signed by Apple (known since M1).
- The app contacts `api.github.com` periodically, which reveals to GitHub that a Mellow client runs at that address; acceptable for now, to make optional in settings later.
- No automatic roll-back: a broken release is fixed by a newer one; `<executable>.previous` allows a manual one.

## Alternatives considered
- **Self-update crates (`self_update`, `cargo-dist` updaters)**: more dependencies and their own trust model, while the server needs the same verification anyway.
- **Only notify, link to the releases page**: simpler, but the user asked to update from the settings.
- **Silent automatic install**: surprises users mid-conversation and hides a security-relevant action.
