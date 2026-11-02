# Screen sharing on the web (ROADMAP stage 16.6, ADR-0037)

Status: web client built on `feat/screen-sharing`, DESIGN.md v0.39, on the voice session's screen sharing logic (`client.voice`: `canShareScreen`, `isSharingScreen`, `screenShareProblem`, `screenShareQuality`, `startScreenShare`, `stopScreenShare`, `watchStream`, `streamTracks`). Checked with unit tests, the development gallery (`npm run dev`, then `/dev/voice.html#stream=share`, see below) and Playwright on the gallery, never with a real call between two browsers yet.

## Brief

The owner wants screen sharing before the camera interface ("je veux ça d'abord"). Mellow's people are small communities hanging out in a voice channel: someone shows code, a document, a game or a holiday slideshow to the others. The jobs: start and stop sharing without hunting for it, always know when your screen is out there, see at a glance who shares, and watch one screen large while staying in the call.

Real content: "Rust Lyon", Ana sharing `fan_out.rs` in Salon, Chloé sharing in Jeux, Paul (you) in Salon.

## Flows and states

**Share.** The voice panel gains a second row, as Discord's panel has its video buttons: a split button, "Partager l'écran" (the browser's picker opens from that click) and the quality half ("1080p", a caret). Shown only when `canShareScreen` (connected, and the browser can choose codecs). While sharing, the share half reads "Arrêter le partage" on an apricot wash ("you, now"), a line under it says "EN DIRECT Tu partages ton écran" (also read out), and your own row in the channel shows the chip. The browser's own "Stop sharing" ends it the same way.

**Quality.** The quality half opens a menu above the panel (the one menu of 6.33, radio items with a check): 720p, 1080p (default) and 1440p, each at 30 images per second, with a hint ("30 i/s, le plus léger", "un texte net", "pour les grands écrans"). It applies at once while sharing and is remembered on the device. Its accessible name carries the value: "Qualité du partage : 1080p".

**Problems** (one line in the panel, with a close button, polite live region): cancelled picker "Partage d'écran annulé." (quiet: no warning sign), unsupported browser, stopped by the instance, missing Vidéo permission, too many videos in the channel or on the instance, video turned off on the instance. Each says what happened and what to do.

**Who shares.** Under a voice channel, a person sharing gets an "EN DIRECT" chip at the end of their row, and "partage son écran" after their name for screen readers. In your call, the chip is its own button ("Regarder l'écran de Ana"), apricot fill on hover; your own chip, and chips in a channel you are not connected to (or with a browser that cannot show video), are shown only, with a tooltip ("Ton écran est partagé", "Rejoins Jeux pour regarder").

**Watch.** The stream view takes the conversation sheet's place (the chat stays mounted underneath, `inert`, so its scroll and draft are kept). Header in the chat header's grammar: the streamer's avatar, "Écran de Ana", the chip, then Plein écran (Fullscreen API on the view, shown only where the page may go fullscreen) and Arrêter de regarder. Below, a dark stage with the picture letterboxed (`object-fit: contain`, muted autoplay inline video). Focus moves into the view and returns to the chip when it closes; Esc closes it while focus is in it (in fullscreen, Esc leaves fullscreen first).
- Connecting: the streamer's avatar on the stage and "Connexion à l'écran…", until the track arrives and its first frame has loaded.
- Ended: they stopped, left, or you left the call: "Ana a arrêté de partager son écran." on the stage for `motion.delay.stream-ended` (3 s, kept under reduced motion as reading time), then the view closes. Sharing again meanwhile cancels the close.
- Phones: the header keeps the navigation button, so the panel stays reachable.

## Design plan

Tokens (new, both themes checked by `cargo xtask tokens`):
- `color.media.stage` (`dusk.160`) and `color.media.text` (`dusk.932`), the same in both themes: a screen is watched in the dark, as in a cinema, so its own colours read true. Contrast pair `media.text` on `media.stage`.
- `component.live-badge.*`: the mention chip's apricot wash and text, `accent.fill` and `text.on-accent` on hover, 16 px high, `radius.xs`, `type.badge`. Contrast pair `mention.chip-text` on `state.hover` (the row's hover wash).
- `component.stream-view.*`: `background` (the sheet), `stage`, `stage-radius` (`radius.md`, concentric in the sheet's `radius.lg`), `stage-inset` (`space.xs`, none in fullscreen), `stage-text`, `avatar-size` 64, `ended-close-delay`.
- `component.voice-panel.share-*`: `height` (`button.height-small`, 44 px under a coarse pointer), `radius` (`radius.sm`), `split-gap` (2 px), rest `state.selected` / `text.heading`, hover `neutral.fill` / `text.on-neutral`, sharing `mention.chip-background` / `mention.chip-text`, sharing hover `accent.fill` / `text.on-accent`.
- `motion.delay.stream-ended` 3000 ms.
- Icons (Phosphor 2.1.1, Fill): `monitor-arrow-up`, `corners-out`, `corners-in`.
- Motion: colour transitions only (`motion.duration.fast`, 0 under reduced motion); nothing moves on its own.

```
Navigator, wide                         Sheet while watching
+--+-----------------------------+      +-----------------------------------------+
|  | ))) Salon                   |      | (A) Écran de Ana [EN DIRECT]    [⤢] [x] |
|  |     (A) Ana     [EN DIRECT] |      | +-------------------------------------+ |
|  |     (B) Bob          [mic/] |      | |                                     | |
|  |     (P) Paul    [EN DIRECT] |      | |      picture, object-fit: contain   | |
|  | ))) Jeux                2/5 |      | |      on the dark stage              | |
|  |     (C) Chloé   [EN DIRECT] |      | |                                     | |
|  +-----------------------------+      | +-------------------------------------+ |
|  | |||| Voix connectée [♪] [⏏]  |      +-----------------------------------------+
|  |  Salon / Rust Lyon          |
|  | [▣ Arrêter le partage][1080p ^]     Connecting / ended: the streamer's avatar
|  | [EN DIRECT] Tu partages ton écran   centred on the stage, one sentence under it.
|  +-----------------------------+
|  | (P) Paul    [mic][hp][gear] |
+--+-----------------------------+
```

## Defaults rejected

- **Discord's red "LIVE" pill.** In Dusk, red is danger and counts; apricot is "you, now", and a share is "now". The chip uses the mention chip's wash, not the apricot fill, so a channel with three people sharing does not shout; it turns to the fill only under the pointer, when it is an action.
- **A modal or picture-in-picture player over the whole app.** A screen is watched while talking and chatting in the navigator: the view takes only the sheet, the panel and the participants stay reachable, and the chat comes back exactly as it was.
- **A spinner while the picture comes.** The streamer's face says whose screen is coming and the sentence says what happens; no lone spinner (DESIGN.md 1.5).
- **The quality hidden in settings.** People choose it per share (a slideshow versus a large monitor), so it sits on the button, keeps its value visible, and changes live.
- **Removed before shipping:** a second hint colour on the stage (one sentence is enough) and a viewer count (the protocol does not say who watches).

## Not done

- Watching a share in a channel you are not in: the chip says to join first (Discord joins and opens in one click).
- Text at 200 %: the panel's share label truncates on the narrowest navigator; the full label stays the button's accessible name.
- A real two-browser check (picker, frames arriving, fullscreen on Safari and Firefox, the 10 s stop) remains to do on the test instance.
