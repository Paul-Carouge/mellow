# Web: YouTube's volume slider, and a remembered volume

Status: built 2026-11-05 on top of v0.47.0 (provider embeds of [ADR-0044](../adr/0044-provider-embeds.md), [web-provider-embeds.md](web-provider-embeds.md) 4.1 and 4.2; inline media of [ADR-0038](../adr/0038-play-audio-and-video-attachments-inline.md)).

## Brief

The owner, 2026-11-05: "Volume slider sur youtube qui est mieux géré : là il disparaît quand on le survole pour aller choisir le volume si pas mis en grand écran, et aussi se souvenir du volume que l'on met sur les vidéos pour pas le refaire à chaque fois."

Two jobs: a reader can set a YouTube video's volume without going full screen, and the volume they set once is where the next video starts, YouTube or an attached file.

## 1. Why the slider disappeared

YouTube's embedded player (as served in November 2026) has its speaker key in the top bar; pointing at it opens a vertical slider under it, with a gap of about 24 px between the key and the slider. Measured in Chrome on the real `youtube-nocookie` player, moving the pointer from the speaker down to the slider in 2 px steps every 16 ms (a scripted, real pointer path):

| Player size (CSS px) | Slider after the pointer reaches it |
|---|---|
| 394 × 222 (Mellow's card before, 432 px card) | closed (twice) |
| 426 × 240 | closed |
| 480 × 270 | stays open |
| 854 × 480, 1280 × 720 | stays open |
| 270 × 480 (a Short) | stays open |

So below 270 px of height, YouTube closes its own slider as the pointer crosses the gap; full screen worked because it is large. Nothing of Mellow's covers the iframe (no overlay, no `pointer-events` change, `overflow: hidden` only clips the rounded corners). 480 × 270 is also the smallest 16:9 size YouTube's player documentation recommends for a player with controls.

**Decision:** a card holding a 16:9 YouTube player (a video, or YouTube Music's player) is wider than other link previews, so its player gets `component.link-preview.video-player-width` (480 px) when the column has room: the card is 480 px plus its padding, border, pill and gap (518 px), computed from the tokens in `EmbedCard` (`hasVideoPlayer`). Shorts keep their 270 × 480 box (it works). The box is the final size from the poster on, so nothing moves on play.

**Rejected:** our own volume control laid over YouTube's (it would cover YouTube's controls and break its terms); the YouTube IFrame API script (a third-party script on the page and a CSP change, for what `postMessage` does alone); widening every link preview (other cards keep the file card's 432 px, so text cards still line up with files).

**Not solved:** a column narrower than 518 px (a narrow window, the side columns opened wide) still gives a smaller player, where YouTube's slider still closes. The keyboard always works there (the player has focus on play: up and down arrows change the volume), and the remembered volume makes the slider needed less often. Touch devices have no hover.

## 2. A remembered volume

One volume per browser, `localStorage` key `mellow.media-volume`: `{ "volume": 0 to 1, "isMuted": boolean }`, checked on the way in (anything else is ignored). Shared by every player, since a reader thinks "my videos are too loud", not "my YouTube videos are too loud".

- **Attachments** (`<video>` and `<audio>` in messages, the video in the picture viewer): the player starts at the remembered volume and muted state; each `volumechange` is remembered (`lib/media-volume.ts`, attachment `playAtRememberedVolume`).
- **YouTube** (videos, Shorts, YouTube Music's player): the embed is loaded with `enablejsapi=1&origin={this page}`; the page talks to it with `postMessage` only (`components/embeds/youtube-player.ts`): on the iframe's `load` it sends `listening`; on `onReady` it sends `setVolume` (0 to 100, rounded) and `mute` or `unMute`; then every `infoDelivery` carrying `volume` and `muted` is remembered. Reports of the player's own starting volume that arrive after the commands are skipped until the player reports the volume it was sent, so a new player never overwrites the memory with YouTube's 100. The wait also ends at the first report of a third volume (neither the starting one nor the one sent): the reader moved it, or YouTube refused the command (volume 0 unmuted, a browser keeping the player muted), so such a player still remembers the reader's changes.
- **Trust:** a message counts only if its origin is `https://www.youtube-nocookie.com` and its source is this player's window; data must be a JSON string under 64 KiB with the expected shape (integer volume 0 to 100, boolean muted). The CSP is unchanged (`frame-src` already allows the player; `postMessage` needs no `connect-src`). The `origin` parameter tells YouTube the instance's origin, which the referrer policy (`strict-origin-when-cross-origin`) already sends.

States: nothing remembered (players start as before); storage blocked or full (players work, nothing is remembered, one console warning); a player that never reports (nothing changes); several players (one plays at a time; the last change wins).

Shortcut: the volume is per browser, not synced across devices; move it to a server setting when settings sync.

## 3. Tokens

| Token | Value | Use |
|---|---|---|
| `component.link-preview.video-player-width` | 480 px (30rem) | A YouTube 16:9 player's width when the column has room; its card adds the card's own padding, border, pill and gap |

No colour, copy or new component: the controls are YouTube's and the browser's.

## 4. Checked

- The measurements above, in Chrome (headed, throwaway profile) on the real player, from a local page.
- `postMessage` round trip on the real player: `onReady`, then `infoDelivery {volume: 100, muted: false}`, then `setVolume(30)` answered by `infoDelivery {volume: 30}`.
- Unit tests: storage (corrupt values, blocked storage, writes only on change), message parsing (origin, source, shape, sizes), the follow logic (skips the starting volume, whole steps, stops waiting when the player never reports the volume it was sent).

To check on a real device: Safari (an iPhone ignores `volume` on media elements; mute still applies), Firefox, a screen reader announcing the restored volume.
