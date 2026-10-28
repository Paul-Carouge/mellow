# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.15.0] - 2026-10-28
Context menus. No database migration.

### Added
- Web: right-click, a long press on a touch screen, or Shift+F10 and the Menu key open Mellow's own menu on messages, people (member list, message authors, friends), channels, servers in the strip, conversations and groups, with the actions you are allowed to use. The browser's menu stays in text fields, on links, on selected text, and with Shift held.

### Changed
- Web: a message's More button opens the full message menu (reply, edit, copy text, pin, reactions, delete).

### Fixed
- Web: Invites in server settings created the invite for the open server instead of the server being edited.
- Dependencies: `pulldown-cmark` 0.13.5 (RUSTSEC-2026-0414; reached only through the desktop app's UI toolkit).

## [0.14.0] - 2026-10-28
Group conversations, a web client for every screen, and installing it as an app. This version migrates the database (migration 19, with the usual automatic backup); older versions refuse it afterwards.

### Added
- Group conversations of up to 10 people: a "New message" picker at Home opens a conversation with one friend or creates a group with several; anyone in a group adds their friends, renames it or leaves it; its owner removes people, and the next person takes over when the owner leaves.
- Web: install Mellow from Chrome, Edge or Android Chrome as its own app window, with its icon and colours. There is no service worker: nothing from your conversations is stored for offline use.

### Changed
- Web: every screen works from 320 px phones to large monitors, in landscape and with text at 200 %: nothing is cut off or scrolls sideways, touch targets reach 44 px, content stays clear of notches and the home indicator, the soft keyboard keeps the composer visible on Android, menus open above their button near the bottom of the screen, and long file names keep their extension.
- Server: the web app manifest is served as `application/manifest+json`, and the content security policy refuses workers (`worker-src 'none'`).

### Known limits
- Groups have no system messages yet ("Ana added Bob"). On iPhone the keyboard may still cover the composer. Not yet checked on real devices.

## [0.13.0] - 2026-10-26
Friends. This version migrates the database (migration 18, with the usual automatic backup); older versions refuse it afterwards.

### Added
- Friends: send a friend request by username, accept, ignore or cancel it, remove a friend, block and unblock; a Friends space at Home (Online, All, Pending, Blocked, Add friend) with a badge for requests waiting for you; friend and block actions in a member's profile.
- Friends can write to each other and see each other's status without sharing a server.
- Messages from people you blocked fold in chats ("2 blocked messages — Show"), also in pins and replies; a conversation with someone you blocked waits until you unblock them.

### Changed
- Direct messages open between people who share a server or are friends, unless either blocked the other; someone you block is never told, they only see the friendship or request end.

## [0.12.0] - 2026-10-26
Statuses and the user card menu. This version migrates the database (migration 17, with the usual automatic backup); older versions refuse it afterwards.

### Added
- Web: pick online, idle, do not disturb or invisible, and a custom status of up to 128 characters, from a menu on your user card, which also copies your user ID; your pick is kept across devices and reconnections.
- Web: idle, do not disturb and offline have their own shapes (crescent, bar, ring) wherever presence shows, and custom statuses appear under names in the member list.
- Server: `PUT /api/v1/users/@me/presence`; connecting clients get each online member's status and custom status. Invisible users look offline to everyone else.

### Known limits
- No automatic idle after inactivity yet. Custom statuses refuse the flags of England, Scotland and Wales (their tag characters can hide text). The desktop app shows idle and do not disturb as online.

## [0.11.0] - 2026-10-23
Unread channels and mention badges. This version migrates the database (migration 16, with the usual automatic backup); older versions refuse it afterwards.

### Added
- Web: red mention badges on channels, servers, conversations and Home, with "99+" from 100; channels you have not opened since you connected now show as unread too.
- Server: the server finds who a message mentions ("@username", or a one-word display name, outside code) among those who can see the channel, and tells each client its unread channels and mentions when it connects.

### Changed
- Web and server: a display name two people share no longer mentions either of them, so copying someone's name cannot take their mentions; "**@name**" and "||@name||" now mention.

### Known limits
- Messages sent before this version count as mentioning no one, though the web client still highlights them; reading a channel on another device clears its badge here at the next reconnection.

## [0.10.0] - 2026-10-23
Server speed under load. This version migrates the database (migration 15, with the usual automatic backup); older versions refuse it afterwards.

### Changed
- Server: writes go through a single database connection in arrival order, next to four read-only ones. Under many simultaneous actions the slowest 1 % of requests is 3 to 6 times faster and throughput is 13 to 22 % higher; a write still waiting after 5 seconds fails, as before (ADR-0024).
- Server: messages in private channels reach their readers about 4 times faster, and changing a role's permissions with 1 000 members online takes about 9 ms instead of 150 ms.

## [0.9.2] - 2026-10-23

### Fixed
- Web: confirming a message's deletion, pin or unpin, or the removal of all its reactions in their dialog now does it; the dialog closed and nothing happened (deleting worked only with Shift+click).

## [0.9.1] - 2026-10-23
Robustness and speed.

### Fixed
- Web: a deleted message could come back after a late answer, and deleting it again failed; messages are now deleted in their own channel and a message already gone counts as deleted.
- Web: deleting the focused message no longer leaves the message list out of keyboard reach, and cancelling a dialog opened from a message's actions no longer loses the focus.
- Web: no gap in a channel's history after a reconnection; the pinned messages panel loads again after one; a send on a dead connection becomes "Not sent" after 30 seconds instead of staying "Sending…".
- Server: one address trying many login names could fill the rate limiter and lock everyone out of sending and reconnecting; sign-ins, gateway connections and signed-in limits now count apart.
- Server: members past the first 1 000 of a server now show as online when you connect.
- Security: `ctutils` updated for RUSTSEC-2026-0385.

### Changed
- Web: the message list does about 85 % less work per new message, reaction or edit in a long channel, and uses 40 % less memory.
- Server: history pages load several times faster when messages have attachments.

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
