# 0064. Offer app updates in a toast, and cover the launch with a splash, on the desktop

- Status: Accepted
- Date: 2026-11-15
- Amends: [ADR-0057](0057-desktop-self-update.md) (where an update is offered, and which copies ask the release host)
- Builds on: [ADR-0018](0018-roll-back-the-executable-only-and-guard-the-install-location.md) (install-location guard), [ADR-0060](0060-windows-release-package.md) (Windows installer), [ADR-0022](0022-give-mellow-its-own-visual-language-on-a-familiar-layout.md) (Dusk, Melli)

## Context
ADR-0057 downloads, verifies and stages a new version in the background and offers it in two quiet places: Settings, About, and a small button beside the user card's gear. The owner found it too easy to miss and asked for a toast: "Mellow X.Y.Z is available", "Update and restart", a close button, with the download's progress, the installation, an error with "Try again", and the copies that cannot update themselves. A dismissed toast must not come back at every check for the same version.

At launch, the main window showed an empty ground while the session was read from the secret store, then the shell with its "Connecting" bar and empty lists until the instance's first `Ready`. The owner asked for an animated splash that covers that wait without lengthening it.

## Decision
- **The toast offers what ADR-0057 already prepares.** It appears as soon as a newer version is known (downloading in the background or ready), in the main window's bottom trailing corner, above the composer and the user card. "Update and restart" asks `SelfUpdate` to restart once the update is ready: if it is ready, it is swapped in and the app restarts at once; if it is still downloading, the toast shows the download's progress (bytes counted by `mellow-update` as they arrive, `DownloadProgress`), then "Checking the signature and unpacking…", then "Installing…", then restarts. Nothing in the verification path changes: same `download_verified_asset` (the progress is an optional counter beside it), same signature check before anything is written, same staging, swap, rollback and install-location guard, same restart (GPUI's restart on Linux and macOS, the verified installer on Windows).
- **Never during a call**, as before: "Update and restart" and "Try again" are disabled with "Leave the call to restart and update." A download asked for from the toast that finishes during a call does not restart; the toast offers the restart again.
- **Only the click restarts**: the request lasts from "Update and restart" until the restart, the toast's closing, or the next check, which clears it first. A scheduled check never restarts the app, and its failures stay silent.
- **Errors after a click stay in the toast**, with the problem's sentence (the ones Settings, About already uses) and "Try again", which checks and downloads again. A background failure nobody asked about stays silent, as in ADR-0017.
- **Dismissal is per version and per launch**: closing the toast hides it for that version until the app starts again; a newer version (a newer release replacing the ready one) shows it again. Nothing is written to disk.
- **Copies that cannot update themselves** now ask the release host for the latest version (the same anonymous request as a normal check, no package downloaded) when the reason is one the person can act on: not writable or not where `install.sh` puts it ("Open the download page", the latest release on GitHub), without a supported FFmpeg ("Check again" after installing it), opened from macOS's App Translocation (the sentence alone: moving the app and opening it from Applications is done outside this copy). While it checks again, the toast stays. A release build compiled from source counts as a copy put somewhere by hand ("not where `install.sh` puts it"): it learns of new versions and is sent to the download page. A development (debug) build outside the install location, a copy from a package manager, one run as administrator or on an unsupported system stays offline and silent, as ADR-0057 decided: the package manager or the developer owns its updates. A debug build copied to the install location itself is treated like an installed copy and updates through the same signed path.
- **Previews never update**: the launch preview and the component gallery open before the self-update is installed, and the preview's toasts have buttons that do nothing.
- **The splash** is a layer over the main window from its first frame until the session is signed out (sign-in card), or the first `Ready` arrives, or the gateway starts reconnecting (the shell's connection bar takes over), or 8 seconds pass (the shell shows with "Connecting"). It has no minimum time: it fades out over `motion.duration.base` (at once under reduced motion) as soon as the screen under it is ready, and shows only once per launch.

## Consequences
- An update is seen the day it is ready, and installing it is one click; the quiet places of ADR-0057 stay for people who dismiss the toast.
- Copies installed by hand in an unwritable or unexpected place, release builds compiled from source included, make one more anonymous request a day; they learn a new version exists instead of never hearing of it. Development builds make none.
- The splash adds no wait: the same work runs under it and the window's first frame comes as early as before (measured in the PR). Melli's animation costs frames only while the splash shows.
- The toast holds focus only when someone moves to it (Tab, or a click); it never takes focus when it appears, so typing in the composer is never interrupted.

## Alternatives considered
- **Download only after the click** (no background download): the click would then wait for the whole download; ADR-0057's install at quit would need the download too. Rejected: the background download is what makes the click fast.
- **A notice bar across the top (6.37)**: it pushes the whole window down and reads as a warning; an update is good news for one person, not a connection problem.
- **Remember the dismissed version on disk**: a person who dismissed an update would never be reminded of it while it stays the newest; the next launch is the reminder the owner asked for.
- **A splash window before the main window**: a second window to open, place and close, and on Wayland no control of where it shows; a layer inside the main window costs nothing extra and fades into the real screen.
