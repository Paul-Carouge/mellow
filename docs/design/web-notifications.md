# Notifications on the web (ROADMAP item 14)

Status: built on `feat/web-notifications`, DESIGN.md v0.28. Web only: no server, protocol or migration change.

## Brief

The owner wants "browser notifications for mentions and direct messages, and Melli reacting to them", and "Melli improved for notifications". Mellow is a self-hosted Discord alternative for small communities; its people keep the web client open in a tab or as an installed app while they do something else. The job of this work is to tell them, quietly and only when it matters, that someone is talking to them, and to let them quieten a busy server without leaving it.

Real content: a mention of `@ana` in `#general` of "Rust Lyon", a message in a one-to-one conversation with Bob, a burst of six messages in the group "Weekend crew", a meme channel the reader mutes for 8 hours.

## Flows and states

**Who is told (`web/src/components/notifications.ts`, tested).** A message alerts the reader when all of these hold:

- it is new (not a replay that `Ready` already counted, not an edit) and someone else wrote it, and its author is not blocked;
- the reader's status is not Do not disturb;
- the place allows it: the channel's "Notify me about" choice, else its server's, else the default ("Only mentions" in a server, "All messages" in a conversation). "All messages" alerts for everything, "Only mentions" when the message mentions the reader (`Message.mention_ids`), "Nothing" never;
- if the channel, its server or the conversation is muted, only a mention gets through (and never with "Nothing"), as on Discord: "direct mentions are never muted" (DESIGN.md §5.2).

**How they are told.** Where the reader is looking decides the channel of the alert:

| Reader | Alert |
|---|---|
| Looking at that channel (tab visible, window focused, channel open) | None: they see it arrive |
| In Mellow, another channel open, window focused | In the app: Melli's chime on Home, the soft sound if on; the badge and title count as always |
| Tab hidden or window not focused | A browser notification (if enabled and allowed), plus the chime and the sound |

**Browser notifications.** One per channel: a burst replaces the channel's notification (`tag` = channel id) and counts it ("Weekend crew, 6 new messages"), silently, so the system does not ring six times. Title: the person for a one-to-one conversation, the group's name, or `#general · Rust Lyon`. Body: "Ana: see you at 7?" (the first 120 characters as plain text, spoilers replaced by "spoiler", custom emoji as `:name:`, "Sent a file" for attachments alone), or "New message from Ana" when previews are off. Icon: the app icon (avatars need the session cookie). Clicking it focuses the tab, opens the channel and closes the notification; opening the channel any other way closes it too. They are `silent`: the sound, when on, is Mellow's own.

**Asking permission.** Never on load. Two gestures ask: the switch in User settings → Notifications, and a one-time bar at the top of the window, shown once connected while the browser has not been asked and the bar was never dismissed: bell icon, "Get a notification when someone mentions you or writes to you.", "Turn on notifications", and a close button ("Not now"). Either answer hides the bar for good on this browser. A browser without the Notification API (Safari on iOS outside an installed app) or on Android (Chrome there refuses to build a notification without a service worker) never shows the bar, and the settings say notifications are not available; should another browser refuse to build one, the first refusal marks them unsupported for the session. Mellow keeps no service worker (`worker-src 'none'`, docs/design/web-responsive-and-installable.md).

**User settings → Notifications** (App settings group, before About):

- A first sentence: "While your status is Do not disturb, nothing notifies you. These settings stay in this browser."
- "Desktop notifications" switch, with the permission explained under it: asking ("Your browser will ask you once."), allowed ("Mentions and direct messages show on your desktop while Mellow is in the background."), blocked (switch locked off: "Your browser blocks notifications from this site. Allow them in the site settings next to the address, then come back."), unsupported (locked: "This browser can't show notifications from Mellow.").
- "Show message previews": "Show the start of the message. When off, a notification only says who wrote."
- "Sound": "Play a soft chime when a mention or a direct message arrives."

**Per place: Mute and Notify me about.** From the server icon's menu (strip and header), a channel row's menu, a group's menu and a one-to-one conversation's menu: "Mute…" (or "Unmute", at once, read out "Rust Lyon unmuted.") and "Notification settings…". Both open the same dialog, "Notifications for Rust Lyon", focused on its first or second group:

- "Mute": Don't mute, For 15 minutes, For 1 hour, For 8 hours, For 24 hours, Until I turn it back on; while a timed mute runs, its end is a row of its own ("Until 18:40", described "Now"), checked.
- "Notify me about": All messages, Only mentions, Nothing (each with one line), plus "Same as the server" for a channel (its description names the server's choice).
- "Save" applies both; Cancel keeps everything. Settings are kept per user id in `localStorage` (try/catch; a blocked storage keeps them for the session only).

**Muted places.** A muted channel or conversation row takes `text.channel-muted`, loses its unread pill and heading colour, and shows a 16 px bell-slash after its name (colour is never the only sign); its accessible name gains ", muted". Mention badges still show in a channel (a mention is never muted); a muted conversation shows no badge, since every message of a conversation counts there. A muted server loses its rail pill and keeps its mention badge. A mute ends by itself at its time (one timer for the earliest end, no polling).

**Tab title and favicon.** The title reads "(3) Mellow" while mention badges and unmuted conversation badges add up to 3 ("(99+) Mellow" beyond 99), "Mellow" otherwise. The favicon gets a red dot (`badge.background`) in its bottom trailing corner while the count is above zero: the generated `favicon.svg` with one circle added, as a `data:` URL (`img-src` allows it).

**Melli.**

- `chime` (new, one-shot): when an alert arrives, Melli on Home squashes, hops and lands once, with happy eyes, while a small bell swings beside its head and fades (`component.melli.chime-duration`, 1400 ms). A burst during the chime does not restart it. Under reduced motion there is no chime: the resting pose already says it.
- `notification` (changed): after the chime, Melli rests happy (eyes "^ ^", blush, sparkles) and floats as at idle, instead of hopping every two seconds for as long as anything is unread. It now shows for unread conversations and unread mentions anywhere. The hop moved to the moment something arrives, which is when it means something.
- `do-not-disturb` (new): eyes softly closed, no blush, a small bell-slash above its head, the puffs still, and a slow breath (`sleep-duration`). Still pose: the same face and bell-slash. It wins over `notification`; connection trouble still wins over it.

## Design plan

Tokens (all under existing roots; no new colour):

- `component.melli.chime-duration` 1400 ms.
- `component.notice-bar.invite-background` (`mention.background`), `invite-text` (`text.normal`), `invite-bar` (`mention.bar`), `invite-bar-width` 2 px: the permission bar speaks in the "for you" apricot wash with the mention highlight's leading bar, so it is never mistaken for the amber connection warning.
- `component.channel-row.muted-icon-size` 16 px (the trailing bell-slash), `muted-icon` (`text.channel-muted`).
- Icons: Phosphor Fill `bell` and `bell-slash` (2.1.1).

Type and layout follow DESIGN.md: the dialog is a §6.35 dialog with two §6.55 choice groups; the settings page uses the role editor's switch rows (label, description, switch at the trailing edge).

```
Wide (>= 816 px)                                    320 px
+------------------------------------------------+  +------------------------+
|| (bell) Get a notification when someone        |  || (bell) Get a          |
||  mentions you or writes to you.               |  ||  notification when    |
||               [Turn on notifications]   (x)   |  ||  someone mentions you |
+------------------------------------------------+  ||  or writes to you.    |
| +-navigator-+ +-sheet----------------------+   |  || [Turn on notif.] (x)  |
| | (Melli)   | | # general                  |   |  +------------------------+
| | # general | |                            |   |  | = # general            |
| | # memes (bell-slash), greyed             |   |  |                        |
```
On a wide window the sentence and the button share one line; the bar wraps the button under the sentence when they do not fit.

## Defaults rejected

- **Asking for permission at sign-in or on load** (the commonest pattern): a prompt out of context is mostly refused, and Chrome then hides it for good. Mellow asks only from a gesture, after saying what it is for.
- **A notification per message**: a lively group would flood the desktop. One per channel, replaced and counted.
- **Melli hopping for as long as something is unread**: busy for hours, against "calm" (§2.1). The hop now answers an arrival, once.
- **A "zzz" sleeping Melli for Do not disturb**: sleeping is reserved for idle presence; Do not disturb is a choice to be left alone, so Melli closes its eyes under a bell-slash, awake but quiet.
- **A bundled sound file**: an asset to license, cache and fetch. The chime is two short sine notes made with Web Audio, original and a few lines.
- **A server setting for notification choices now**: per-device storage first (`shortcut:` in code); syncing needs a server setting later.
- **Greying a muted row by colour alone**: a bell-slash and ", muted" say it too (WCAG 1.4.1).

## Components

All in `web/src/components/` unless said otherwise.

- `NotificationSettings.svelte` (settings page), `NotificationSettingsDialog.svelte` (per place), `NotificationPermissionBar.svelte` (the one-time bar).
- `Melli.svelte`: `chime` prop (a counter: each increase plays the arrival once), states `do-not-disturb`; `melli.ts`'s `homeMelliState` takes Do not disturb and attention.
- Logic: `web/src/lib/notification-settings.ts` (types, validation of what storage holds, mute ends), `notifications.ts` (who is told, how, the text, what the dialog saves), `notification-preferences.svelte.ts` (the reader's settings as state, the mute timer, re-read on another tab's `storage` event and on return after sleep), `notifier.svelte.ts` (browser notifications and their coalescing, sound, title, favicon), `chime-sound.ts` (Web Audio, unlocked on the first `pointerdown`, `pointerup` or `keydown`). Each has its Vitest file beside it.

## Not done, to validate

- Chrome on Android and Safari on iOS cannot show these notifications without a service worker; they get the in-app chime, sound, title and badges only. Push notifications (ROADMAP "Later") will need a worker and an ADR.
- Settings live in one browser; another device keeps its own.
- "Mark as read" in menus and a read-state event across devices are still item 5's and 9's "Not yet".
- Check with a screen reader that the permission bar is announced once, and on a real macOS and Windows desktop that clicking a notification brings the window forward.
