# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Security
- File transfers are bounded end to end: a download keeps its memory slot until its last byte is sent, uploads and downloads have separate slots, an account uploads one file at a time within a deadline, and avatar reads have their own slots. A slow or abusive client can no longer exhaust the server's memory or block everyone's transfers.
- The storage quota is checked when the upload ends, so parallel uploads cannot exceed it; slow background compression no longer takes the slot avatars and banners need.

## [0.4.0] - 2026-10-21
Profiles and files: avatars, banners, attachments, and Melli.

### Added
- Melli, Mellow's mascot, lives in the web client's Home button: a little cloud that floats, worries when the connection drops and hops for unread messages; it stays still with reduced motion. It is also the web favicon.
- Avatars and profile banners for everyone, animated ones included (GIF or WebP): Profile in the web client's user settings, shown on messages, members, conversations and the member popout. Animated pictures play on hover or focus, never with reduced motion. They are fitted to their display size and stored as lossless WebP (ADR-0021).
- Pictures sent as attachments take less disk space without losing a pixel: PNG is recompressed, animated GIF becomes animated WebP, in the background after the upload is answered.
- Attachments: share files and pictures in channels and conversations from the web client (attach button, drag and drop, paste); pictures show inline, other files as download cards; the desktop app lists them. Files are encrypted on disk, 25 MB each and 2 GiB per account by default (`maximum_attachment_bytes`, `maximum_attachment_storage_bytes_per_user`).
- My account in user settings, on desktop and on the web: change the display name, the username (with the current password) and the password (other devices are signed out). The web client gets user settings with About and Sign out.

### Security
- Photos sent as attachments lose their metadata (EXIF, GPS position, camera serial, embedded extra pictures) before anyone can download them; the orientation is kept. A JPEG whose header is too damaged to remove them is refused.
- Picture processing checks dimensions before decoding, holds at most two pictures at once and bounds memory, so a forged picture cannot exhaust the server.

## [0.3.0] - 2026-10-21
The web client, direct messages, and joining servers from the apps.

### Added
- Web client (Svelte): the instance serves it at `/`, so people can join from a browser or a phone without installing anything. Sign-in keeps the session in an HttpOnly cookie; servers, channels, messages, members and direct messages work as in the desktop app.
- Direct messages: one-to-one conversations with someone you share a server with. In the desktop app, click a member then Message; conversations are listed under Home, most recent first, with unread markers. Writing stops once you no longer share a server.
- Desktop app: create a server or join one with an invite code from the rail's `+`; the server menu gains Invite people (an invite code to copy), Create channel and Leave server.

### Security
- The web session is a `__Host-` cookie (HttpOnly, Secure, SameSite=Strict) accepted only from the instance's own origin; the gateway refuses other origins; API answers are not cacheable; built-in TLS sends HSTS. Behind a reverse proxy, keep the `Host` header (README).

## [0.2.0] - 2026-10-21
Second milestone step: server settings and account API.

### Added
- Desktop app: server settings, opened from the server menu, with a Roles section: create, rename, colour presets, permission switches with an unsaved-changes bar, move up or down, delete with confirmation; roles above the member's rank are locked.
- Accounts: change the display name, the username (with the current password) and the password (`PATCH /api/v1/users/@me`, `POST /api/v1/users/@me/password`); a password change signs out the other devices; renames reach other members live (`USER_UPDATE`).

## [0.1.1] - 2026-10-21
First update, to check the update paths end to end.

### Added
- One-line installer for the desktop app on Linux and macOS (`install.sh`): it checks the release signature when `minisign` is installed, the SHA-256 digest otherwise.

## [0.1.0] - 2026-10-20
First release (milestone M1, plus the first M2 steps).

### Added
- Server (`mellow-server`): `init`, `serve` and `admin create-user`; accounts and sessions, guilds, categories and text channels, invites, messages with replies, edits and deletes, read states, typing, presence; REST API `/api/v1` and WebSocket gateway `/gateway?v=1`; SQLCipher database encrypted at rest with pre-migration backups; built-in TLS with ACME, or running behind a reverse proxy.
- Moderation (M2): roles and their order, channel permission overwrites with hidden channels, kick and ban, audit log.
- Desktop app (`mellow-desktop`): Discord-like shell on a real instance, sign-in with the session in the OS secret store, live messages with an outbox, history pagination, message-list states, read-only channels.
- Signed releases for Linux (x86_64, aarch64), macOS (Apple Silicon) and Windows (x86_64), built in CI ([ADR-0016](docs/adr/0016-sign-releases-in-a-protected-ci-environment.md)).
- Updates: `mellow-server update` (`--check`, `--rollback`), a Docker Compose kit with `mellow-update`, and in the desktop app, User settings → About shows new versions and installs them on request ([ADR-0017](docs/adr/0017-desktop-app-updates-itself-from-signed-releases.md), [ADR-0018](docs/adr/0018-roll-back-the-executable-only-and-guard-the-install-location.md)).
