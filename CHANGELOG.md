# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- Video follows each watcher's connection: the server measures how much it can send to each person (transport-wide congestion control) and fits the cameras and screens they watch into 90 % of it and into their fair share of `[voice] video_upload_mbps`. Smaller tiles go down to a lower picture first, then pause, the large tile last; pictures go back up after 5 seconds with 20 % to spare. A paused tile is announced to its watcher (`VOICE_VIDEO_PAUSED`, additive), for the camera interface to come.
- When a camera or screen stops sending its larger picture (its sender's upload is short), its watchers fall back to the next smaller picture still arriving within about a second and a half, and go back up as soon as it returns.

### Changed
- `[voice] video_upload_mbps` no longer counts 10 Mbit/s per full-quality stream: it is shared among the people watching video, and each layer is costed at what it really sends (at least its target).

## [0.32.0] - 2026-11-02
The sound of shared screens (ADR-0037). No database migration; clients of `v0.31.0` keep working and share without sound until reloaded.

### Added
- Shared screens carry their sound: tick "Partager l'audio" in the browser's picker (a tab's sound in every Chromium browser, the whole system's on Windows). Only the people watching the screen hear it, at the sharer's volume; someone without the "Parler" permission shares the picture only.

### Fixed
- A burst of 1440p screen keyframes no longer cuts the sharer's voice: voice packets have their own budget (120 packets and 70 kB a second, twice that while sharing a screen's sound), apart from video's.

## [0.31.0] - 2026-11-02
Screen sharing in voice channels (ADR-0037), asked for before the camera interface. No database migration.

### Added
- Share your screen in a voice channel from "Voix connectée": 720p, 1080p or 1440p at 30 images per second, chosen on each device and changeable while sharing. People sharing show an "EN DIRECT" chip; click it to watch their screen large, in fullscreen if you like. The "Vidéo" permission decides who may share.
- `[voice] video_upload_mbps` now caps full-quality video: each full-quality stream counts 10 Mbit/s, and beyond the budget watchers get the half-size picture.

### Changed
- The full-quality video layer may carry 2560 × 1440 at up to 10 Mbit/s, and a participant sending video up to 1.5 MB/s.
- Losing the "Vidéo" permission while sharing stops the share at once, with a message.

### Known limits
- No sound from the shared tab or application yet; camera and screen cannot be shown at once.

## [0.30.0] - 2026-11-02
First half of video in voice channels (ROADMAP 16.5, ADR-0035): the server can relay cameras, but the web client cannot turn one on yet. This version migrates the database (migration 33, with the usual automatic backup); older versions refuse it afterwards.

### Added
- A "Vidéo" permission (`STREAM`), granted to `@everyone` in every server, in the role and channel permission editors: it will allow turning a camera on in voice channels.
- The server relays cameras in voice channels: VP8 simulcast in three qualities, one per watcher from the size of its tile, at most 9 cameras watched per person, 12 cameras per channel and 25 per instance. Each quality has a byte budget, silent cameras are turned off after 10 s, and keyframe requests are throttled.
- `[voice] video_upload_mbps` in `mellow.toml` (default 100); `0` turns cameras off. Other values take effect with bandwidth allocation, in a later version.
- The voice UDP socket asks the kernel for 4 MiB buffers each way and logs what it got.

### Changed
- Gateway frames from identified clients may weigh up to 52 KiB, and voice offers up to 48 KiB, to hold the video lines.

## [0.29.0] - 2026-11-02
Fixes and quick wins asked for by the owner (ROADMAP item 17). This version migrates the database (migration 32, with the usual automatic backup); older versions refuse it afterwards.

### Added
- Roles can be displayed separately: online members holding such a role are listed under its name, above everyone else, in the member list (role editor → Display).
- Avatars and banners are framed before upload: a round frame for the avatar, the profile card's 5:2 band for the banner, animated GIFs included. The server stores only the chosen part (avatars square, banners 5:2); without a choice it keeps the centred part, as clients already showed.
- User settings → Accessibility: "Always animate" plays avatars, server icons, custom emoji, banners and Melli even when the system asks to reduce motion; "Animate on hover" stays the default.
- The channel list, the member list and the search results can be resized by dragging or with the keyboard; widths are remembered on each device.
- The member list remembers, on each device, whether it was open for each server and group.
- ADR-0035 proposes video in voice channels, backed by a measured spike.

### Changed
- The soundboard's "Play for me" shows a speaker instead of headphones.
- The "You're viewing older messages" bar rests on the composer.

### Fixed
- Choosing Deny, Inherit or Allow on a permission low in the channel settings scrolled the whole settings window up and left the bottom of the screen empty; the same happened with the search results' sort choice.
- `pulldown-cmark` updated for RUSTSEC-2026-0433.

### Removed
- The Slint desktop app, its installer and its app icons (ADR-0036). None was released since 0.8.0; installed copies keep working with this server. Native clients come back after the web client.

## [0.28.0] - 2026-11-02
Stored files spread evenly, and cleaner code. No database migration.

### Fixed
- Stored files (attachments, avatars, icons, emoji, previews and sounds) all landed in one folder. They now spread over 256 folders (ADR-0034). The first start moves existing files once, safely and resumably; a later restart continues where it stopped. Going back to 0.27.0 afterwards needs the files moved back by hand: update forward instead.

### Changed
- Internal clean-up with no visible change: guild code, HTTP route tests, the web composer, channel sidebar, dialogs and client state split into smaller modules; the web client's protocol types are now checked against the server's at build time.

## [0.27.0] - 2026-11-02
Faster, lighter and sturdier. This version migrates the database (migrations 24 to 31, with the usual automatic backup); older versions refuse it afterwards.

### Added
- Every picture the server stores (avatars, banners, server and channel icons, attachments, animated GIFs) is compressed with no visible loss: AVIF, or lossless WebP when that is smaller, each checked against a perceptual quality score (ADR-0033). Attachments take about half the space; avatars and banners about a tenth. Display P3 photos keep their colours.
- Picture attachments show a small preview in the chat and open the full picture on click; they keep their place while loading.
- `mellow-server backup` writes a consistent copy of the database and stored files while the server runs.
- `mellow-server admin recompress-images` (`--dry-run`, then `--confirm` after a recent backup) recompresses pictures stored by earlier versions, one at a time in the background.
- HTTP/2 when Mellow serves TLS itself.

### Changed
- The web client downloads about 250 KB at startup instead of 1.2 MB (brotli and gzip prepared at build time), and settings and dialogs load when opened.
- Attachments are revalidated instead of downloaded again, and a busy instance no longer breaks pictures for good.
- The web client reconnects as soon as the network comes back, and retries on its own when the instance was unreachable; a short drop of up to 2 minutes resumes without reloading.
- Read state is sent at most once a second per channel; the message and member lists redo much less work.
- Avatars are stored at 256 px and banners at 960 px wide.
- The database syncs to disk at checkpoints instead of every commit: writes are about 39 times faster on a Linux server. A process crash still loses nothing; a power cut can lose the last few seconds of messages, never the database.

### Fixed
- Reloading the page many times no longer locks you out of live updates.
- A flood of sign-in attempts from one network no longer blocks sign-ins for everyone.
- Behind a reverse proxy, connections now have the same time limits as with built-in TLS, and the server always stops cleanly.
- Several client races: late answers overwriting newer edits or members, user updates copying every message.
- Picture uploads stuck on a slow client now time out; one unreadable file no longer stops the cleanup of the others.

## [0.26.0] - 2026-11-01
Soundboard and join sounds. This version migrates the database (migration 23, with the usual automatic backup); older versions refuse it afterwards.

### Added
- Soundboard: each server holds up to 48 sounds of at most 5.2 seconds, with a name and an emoji. Add them in Server settings → Soundboard: pick any audio file your browser can read, trim it, set its volume, and Mellow uploads a small WAV.
- Web: a soundboard button in the "Voice connected" panel plays a sound to everyone in your voice channel, at most one every 2 seconds; right-click a sound to listen to it alone. Others hear it at their own "Soundboard volume" (Settings → Voice), never while deafened, and never from someone they muted for themselves; the sound's emoji shows by the player's name.
- Join sounds: choose one of a server's sounds as yours; people in the channel hear it instead of the join chime when you arrive.
- New permission **Use soundboard**, per channel, given to everyone by default (existing servers included).

### Changed
- The **Manage emoji** permission is now **Manage expressions** and also covers sounds; roles that had it keep it.

## [0.25.0] - 2026-11-01
Cleaner voices. No database migration.

### Added
- Web: Settings → Voice → Noise suppression: "Advanced" removes fans, keyboards and room noise with RNNoise, running in your browser (the default where supported), "Browser" uses the browser's own filter, or turn it off. If Advanced cannot run or stops working, Mellow falls back to the browser's filter and says why.
- Web: "Enhanced voice", off by default: Natural, Streamer, Warm or Clear presets, Bass, Mids and Treble sliders, and a compressor and limiter for a steady, clean sound. Others hear your processed voice.
- Web: "Listen to myself" in the microphone test and during a call (use headphones), and changes apply at once during a call.
- Web: third-party licence notices, linked from Settings → About.

### Changed
- Server: the web client's Content Security Policy allows WebAssembly compilation (`'wasm-unsafe-eval'`), and nothing more, for the noise suppressor (ADR-0031).

## [0.24.0] - 2026-11-01
Voice comfort. No database migration.

### Added
- Web: User settings → Voice: choose your microphone and output device (no output choice in Safari), set input and output volume up to 200 %, pick voice activity or push-to-talk (record your key, set a release delay, optional press sounds), automatic or manual input sensitivity with a live level meter, turn the browser's noise suppression on or off, and test your microphone.
- Web: changing a device or a setting applies at once during a call; an unplugged microphone falls back to the default one and comes back when plugged in again.
- Web: click someone in a voice channel to set their volume (0 to 200 %) or mute them for you only; it is remembered in this browser.
- Server: `VOICE_SLOT_ASSIGNED` tells each listener whose voice arrives on which of its receive slots.

### Notes
- Push-to-talk works while Mellow's tab has the focus, as in any browser.

## [0.23.0] - 2026-10-31
Voice channels, audio only. This version migrates the database (migration 22, with the usual automatic backup); older versions refuse it afterwards. Voice needs one UDP port (50000 by default) reachable from the internet: forward it on your router and open it in the host firewall; the Docker kit now publishes `50000:50000/udp`.

### Added
- Voice channels, on by default: create one with "Voice" in "Create channel" (optional user limit, 1 to 25), click it to join, and see who is in it, with a ring around whoever is speaking.
- Web: a "Voice connected" panel above your user card with the connection quality and a hang-up button; you stay connected while you move around Mellow. The microphone and headset buttons now work (deafening also mutes you), also with Ctrl/Cmd+Shift+M and Ctrl/Cmd+Shift+D, and soft sounds play when people join or leave. Mellow reconnects by itself after a network change, and you can join without a microphone to listen.
- New permissions **Connect** and **Speak**, per channel, given to everyone by default; without Speak, the server does not relay your voice.
- Server: an audio relay (SFU) built into the binary (ADR-0030), at most 25 people per channel and 50 per instance, voice states and speaking events on the gateway, and an optional `[voice]` section in `mellow.toml` (`enabled`, `udp_port`, `public_address`, `max_participants`).

### Changed
- Server: gateway messages may be up to 32 KiB once signed in (4 KiB before), to carry voice offers.

### Notes
- Desktop builds released so far cannot read servers that contain a voice channel.

## [0.22.0] - 2026-10-31
Message search. This version migrates the database (migration 21, with the usual automatic backup) and builds a search index of every message, which takes a few seconds per million messages; older versions refuse the database afterwards.

### Added
- Search messages in a server, or in your conversations and groups from Home: results ranked by relevance (or newest first), matching words highlighted, counts per channel and per person you can click to narrow down. Accents and capitals do not matter, and the start of a word is enough.
- Filters: `from:`, `mentions:`, `in:`, `has:file`, `has:image`, `has:link`, `has:pinned`, `before:`, `after:`, `during:`, `"exact phrase"`, `-excluded` and `word*`, suggested as you type.
- Web: the search field sits next to "Go to… Ctrl K" in the channel header (Ctrl F, or the magnifier on phones); opening a result jumps to the message, loading older history if needed.
- Server: `POST /api/v1/guilds/{id}/messages/search` and `POST /api/v1/users/@me/messages/search`.

### Changed
- Server: deleting a channel or a server deletes its messages in batches, so other people's messages are never held up for long; the request itself can take a while on a very long history.
- Server: each database connection keeps a 32 MiB page cache (at most about 160 MiB in all).

## [0.21.0] - 2026-10-30
Notifications. No database migration.

### Added
- Web: browser notifications for mentions and for messages in conversations and groups, while Mellow is open in a background tab or an unfocused window (one per channel, updated; click to open it). Turn them on from the bar that asks once, or in User settings → Notifications, where you can also hide message previews and turn the sound off. Nothing is shown in Do not disturb.
- Web: mute a server, channel, conversation or group for 15 minutes to "until I turn it back on", and choose to be notified for all messages, only mentions or nothing, from their right-click menus. Muted rows are greyed out; mentions still get through. These choices are kept in this browser.
- Web: Melli rings a little bell when something arrives, and closes its eyes in Do not disturb. The tab title shows the count, as in "(3) Mellow".

### Fixed
- Web: a blocked person's message in a group no longer counts as an unread mention.

## [0.20.0] - 2026-10-30
Custom emoji. This version migrates the database (migration 20, with the usual automatic backup); older versions refuse it afterwards.

### Added
- Custom emoji per server, still or animated (50 of each, 256 KB per picture), managed in server settings → Emoji by members with the new **Manage emoji** permission (owners and administrators have it).
- Web: type `:name:` in the composer to get suggestions; custom emoji show in messages (large when a message holds only emoji), in the emoji picker and as reactions. With reduced motion, animated emoji stay still.
- Server: `GET`/`POST /api/v1/guilds/{id}/emojis`, `PATCH`/`DELETE /api/v1/guilds/{id}/emojis/{emoji_id}`, `Guild.emojis` and the `GUILD_EMOJIS_UPDATE` event; custom reactions are keyed `:<emoji id>`.

## [0.19.0] - 2026-10-30
Interface polish. No database migration.

### Changed
- Web: "Go to…" looks like a search field with a magnifier, in the same colours; phones get a magnifier button in the chat header, and the search ignores accents ("general" finds "général").
- Web: clearer permission badges ("Sensitive" as a label with a shield, coloured Deny and Allow choices) and role chips tinted with their role's colour; the owner's chip has a crown.
- Web: the emoji picker has a category bar that follows the scroll, full keyboard navigation (arrows across categories, Home, End, Page Up, Page Down) and 44 px targets on touch screens.

## [0.18.0] - 2026-10-30
Better invites. No database migration.

### Added
- Web: choose how long an invite lasts (30 minutes to 30 days, or never) and how many people may use it (1 to 100, or no limit), in the Invite people dialog and in server settings. A new invite lasts 7 days with no use limit by default.

### Changed
- Invite codes read like `AE8C-78A9-K2QF`, shown in large in the Invite people dialog with a copy button. When joining, case, spaces and dashes do not matter, and O, I and L are read as 0, 1 and 1. Codes made before this version still work.
- Server: joining with an invite is also limited to 10 attempts per 10 minutes per account, besides the limit per address.

## [0.17.2] - 2026-10-28
No database migration.

### Fixed
- Web: a message longer than 4 000 characters failed with only "Not sent.". The composer now counts the characters left near the limit and holds a message that is too long back with a sentence saying to split it, and a message the instance refuses shows why next to "Not sent.".

## [0.17.1] - 2026-10-28
No database migration.

### Added
- `mellow-server admin set-administrator --username <name>` makes an existing account an instance administrator (`--revoke` takes it away), for example an account first created through the web sign-up.

## [0.17.0] - 2026-10-28
Who creates servers. No database migration.

### Changed
- Only instance administrators (accounts created with `mellow-server admin create-user --administrator`) can create servers, unless `mellow.toml` says `guild_creation = "everyone"`. **If other accounts on your instance should keep creating servers, add that line before updating.** This includes your own account if you did not create it with `--administrator`. Existing servers are not affected, and everyone can still join servers with an invite. In the web client, "Add a server" then offers only joining.

## [0.16.0] - 2026-10-28
Channels, categories and server creation. No database migration.

### Added
- Create categories, from the server menu or "Create channel", and give a new text channel a topic.
- Reorder channels and categories by dragging them (mouse, or a long press on a touch screen), or from their menus with Move up, Move down and Move to category. Requires Manage channels.
- Delete a channel (type its name to confirm) or a category (its channels stay), and rename a category.
- "Create a server": pick and crop an icon, start from a template (Friends, Community, Study group, Gaming or Blank) in your language, and see a live preview.
- Server: `PUT /api/v1/guilds/{id}/channels/order`, and `initial_categories` in `POST /api/v1/guilds`.

### Changed
- Channel and category names are kept as you type them, capitals and spaces included (no more lowercase with dashes). Existing names do not change.
- Server: names refuse the Unicode line and paragraph separators.

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
