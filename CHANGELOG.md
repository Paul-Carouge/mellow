# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.9.0] - 2026-10-23
Channel settings and icons on the web.

### Added
- Web: channel settings, from the gear on a channel row or the channel header: rename the channel, change its topic and icon, and set who can see and use it, role by role or member by member (deny, inherit or allow each permission).
- Web: server and channel icons, cropped in a dialog (the centred square by default; drag, zoom, keyboard) and shown in the server rail, the channel list and the channel header; animated server icons move on hover or focus, never with reduced motion.
- Server: servers and text channels can have an icon, set by members who can manage the server or the channel and recorded in the moderation log. The picture is cropped to a square (centred, or the square chosen in the app) and stored small; a server icon may be an animated GIF or WebP.

## [0.8.0] - 2026-10-22
Reactions, pins, server settings, Markdown and French on the web.

### Added
- Web: the whole interface in French or English, following the browser's language; dates, times and sizes follow it too.
- Web: messages render Discord-style Markdown: bold, italic, underline, strikethrough, inline code, code blocks, quotes, spoilers, headings, lists and links.
- Server: emoji reactions on messages (one Unicode emoji each, up to 20 different ones per message) with a new "Add reactions" permission, given to everyone in new and existing servers except in channels where they cannot send messages; anyone who can read a message can join a reaction already there, and members who can manage messages can remove other people's reactions.
- Server: pinned messages, up to 50 per channel, pinned by members who can manage messages (anyone in a direct conversation) and recorded in the moderation log.
- Web: react to messages with emoji, from three quick reactions or an emoji picker searchable in English and French, and see who reacted; moderators can remove reactions.
- Web: pin and unpin messages after a confirmation, see a pin beside pinned messages, and open a channel's pinned messages from the header.
- Server settings on the web, from the server menu: rename the server, edit roles and their permissions and order, give roles, kick and ban members, revoke bans and invites, read the moderation log in your language, and delete the server (owner only).
- API: `PATCH` and `DELETE /api/v1/guilds/{guild_id}`, `GET /api/v1/guilds/{guild_id}/invites`, `DELETE /api/v1/invites/{code}`, and the `GUILD_UPDATE` gateway event. Creating and revoking invites now appear in the moderation log.

### Fixed
- Web: a deleted role no longer stays on members until the next reload.

## [0.7.0] - 2026-10-22
The Dusk layout, on the web and the desktop.

### Added
- Quick switcher: `Ctrl+K` or "Go to…" jumps to any channel, conversation or server.
- The members panel opens on demand (`Ctrl+U`) beside the conversation, or over it in narrow windows, and shows a member's profile.
- Each person gets their own avatar colour, names take their highest role's colour, and messages that mention you stand out.
- A "New" divider and a "new messages since…" bar mark what arrived since your last visit.
- Create a channel directly inside a category with its "+".
- Desktop: an Appearance section to choose the theme and reduce motion.

### Changed
- The navigator: servers, channels and your user card share one floating card, the conversation sits on its own sheet, and your card lines up with the message bar.
- Labels are in sentence case; transitions follow the Dusk motion, and stay still with reduced motion.

## [0.6.0] - 2026-10-22
Dusk, Mellow's own look.

### Changed
- A new visual identity, Dusk (ADR-0022): plum-tinted dark and light themes, apricot kept for what concerns you right now, softer nested corners, sentence-case type and more breathing room, on the web and the desktop. The new layout, French interface and transitions follow in the next releases.
- The app icon and the favicon show Melli on a dark dusk gradient tile.

## [0.5.0] - 2026-10-22
Melli on the desktop, and an app icon.

### Added
- Melli comes to the desktop app: the animated cloud lives in the Home button, worries when the connection drops and hops for unread direct messages, as on the web.
- Mellow has an app icon: Melli in the window, the macOS Dock and Finder, and the Linux app menu. The signed app installs it when it starts, so existing installs get it after their next update.

### Changed
- Melli is shown alone, without its cream tile, on the web and the desktop; the favicon too.

## [0.4.1] - 2026-10-21
File transfers hardened after the 0.4.0 audit.

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
