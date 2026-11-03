# Watching a stream, and the share's sound (web, ADR-0037)

Status: built on `feat/web-stream-watching-and-sound-source`, DESIGN.md v0.40, on top of [web-screen-sharing.md](web-screen-sharing.md). Checked with unit tests and the development gallery (`npm run dev`, then `/dev/voice.html#…`, see below) in headless Chromium, light and dark, at 1280 px, 390 px and with text at 200 %. Not checked yet on a real Linux desktop, in Firefox or in Safari, nor between two real browsers.

## Brief

The owner, 2026-11-03: "a better interface to watch a person's stream; make sure the sound is really shared, because last night there was no way to get sound; it must also work on Linux from the browser, whether the sound of one application or of the whole system."

What was established before designing: the server and the client relay a screen's sound correctly. The failure is the browser's picker. Chromium offers a **tab's** sound on Linux and macOS, a tab's or the **entire screen's** on Windows and ChromeOS, never a window's; Firefox and Safari offer none. Chromium on Linux cannot capture an app's or the system's sound through `getDisplayMedia` at all. What a page can do everywhere is open an **audio input** with `getUserMedia`; PipeWire and PulseAudio list a "Monitor of …" input for every output, which is the whole system's sound, and a virtual output made with two `pactl` commands gives one app's sound.

Two people, two jobs:
- **The sharer** must be able to get sound into the share on their system, and must always know whether sound goes out.
- **The watcher** must be able to set the stream's loudness apart from the sharer's voice, know whether the stream has sound at all (so silence is not mistaken for a broken setting), and understand every state of the picture.

Real content: "Rust Lyon", Ana sharing `fan_out.rs` in Salon, Paul (you) watching or sharing; inputs as Chrome lists them on Linux with PipeWire: "Micro interne", "Monitor of Audio interne Stéréo analogique", "Monitor of Partage-Mellow".

## Flows and states

**Choose the share's sound (sharer).** The quality half of the split button now opens **Options du partage**: two radio groups, each with a visible label, in the one menu of §6.33.
- *Qualité*: 720p, 1080p, 1440p, unchanged.
- *Son*: "Écran partagé" (what the picker gives; hint "Si tu coches « Partager l'audio »", left out in Firefox and Safari, whose picker has no such box), then each audio input by its name, the system's sound first (monitors, stereo mixes, BlackHole, Loopback; hint "Tout ce que joue l'ordinateur"), then the others (no hint). The list is read when the menu opens; it is empty until the microphone is allowed, which joining a call already asks.
- On Linux only, after a separator: "Partager le son d'une application", which opens the help dialog.
- The choice is remembered on the device (`mellow.screen-share-sound`) and applies at once during a share (`replaceTrack` on the screen sound line, the previous input stopped). The picker's own sound stays captured meanwhile, so going back to it needs no new picker.

**Know what goes out (sharer).** Under "EN DIRECT Tu partages ton écran", one more line: "Son : écran partagé", "Son : Monitor of Audio interne…" (the whole name in the tooltip) or "Sans son" with a crossed speaker.

**No sound at the start (sharer).** When a share starts with nothing to send, one quiet notice below (no warning sign: nothing failed), read out once with the "Sans son" line:
- Chromium on Windows and ChromeOS: "Pour en ajouter, partage un onglet ou l'écran entier en cochant « Partager l'audio », ou choisis une source sonore."
- Chromium elsewhere: "Ton navigateur ne partage que le son d'un onglet (coche « Partager l'audio de l'onglet »). Pour une application ou tout l'ordinateur, choisis une source sonore."
- Firefox and Safari: "Ton navigateur ne partage pas le son de l'écran. Choisis une source sonore pour en ajouter."
- Then "Choisir une source sonore" (opens the share's options) and, on Linux, "Partager le son d'une application". Close dismisses it; choosing a source with sound removes it. Choosing "Écran partagé" again later does not bring it back: that is the person's own choice. The browser is told apart by `navigator.userAgentData` (Chromium only), its platform, and the user agent's "Linux" for the help; nothing else is sniffed.
- The chosen input cannot open, or is unplugged mid-share: "Impossible d'ouvrir la source sonore choisie. Ton partage utilise le son de l'écran partagé, s'il en a." with the warning sign, and the share falls back to the picker's sound.
- Stopping the share, leaving or moving channel, or the instance refusing the screen stops every input opened for it.

**Help on Linux (dialog).** "Partager le son sous Linux": *Tout ce que joue l'ordinateur* (choose "Monitor of …"; warns that it also carries the call's voices unless Mellow plays on another output such as headphones), then *Une seule application*, four numbered steps (a true sequence): create the "Partage-Mellow" output, loop it back to the speakers so the app is still heard, send the app to it in the sound settings or pavucontrol, choose "Monitor of Partage-Mellow" here. Each command sits in a well with its own "Copier" button ("Copié" for 2 s, announced "Commande copiée"). A footnote: they work with PipeWire and PulseAudio and last until a restart.

```
pactl load-module module-null-sink sink_name=mellow_partage sink_properties=device.description=Partage-Mellow
pactl load-module module-loopback source=mellow_partage.monitor
```

**Watch (watcher).** The stream view keeps its header and dark stage, and gains a **sound band** under the stage, as tall as the header, so the stage sits between two equal bands:
- the stream's mute (speaker / crossed speaker, `aria-pressed`, "Couper le son du partage"), its volume slider "Volume du partage" (0 to 200 %, value read as "150 %" and shown beside it), both 44 px tall;
- at the end, whether sound comes: "Avec son", "Sans son" (crossed speaker; tooltip and one announcement "Aucun son ne vient de l'écran de Ana."), or "Sourdine activée" while you are deafened. Nothing while it is not known yet (the first 5 s of the picture).
- The volume and mute are per sharer, kept in the browser (`mellow.stream-sound-settings`), apart from the person's voice volume and "Rendre muet pour moi"; output volume and deafen still apply (ADR-0037, amended).
- In fullscreen the band stays; the keyboard shortcuts for the microphone and deafen keep working there.

**Stage states.** Connecting (avatar, "Connexion à l'écran…"); not coming after 10 s ("L'image de Ana n'arrive pas. Vérifie ta connexion, ou ferme puis rouvre son écran."); paused by the instance for a slow connection (`VOICE_VIDEO_PAUSED`: the frozen frame is hidden behind the avatar and "Ta connexion est trop lente pour cette image. Elle reviendra dès que possible."); live; ended (unchanged: one sentence, then the view closes). Every stage sentence goes through the view's polite live region. The band stays (empty) when the share ends, so nothing jumps.

## How "Avec son" is known

The instance gives a watched screen's sound a receive slot at its first packet. The session reads that slot's level from an `AnalyserNode` on a copy of its track (the existing `meterLevel` port), **once every 500 ms**, never per frame, and only while a stream is watched. A level above about -50 dBFS (RMS 0.003) counts as heard; heard within the last 5 s means "Avec son", nothing for 5 s once the picture is there means "Sans son" (`streamSoundStatus`, a pure function). Reading a copy means your own volume and mute do not change the answer.

## Design plan

No new colour, type or icon: this extends the stream view and the voice panel with what they already speak.
- **Palette**: the stage stays `color.media.*` (dark in both themes); the band is the sheet (`surface.chat`), its status in `text.muted` (4.5:1 on the sheet in both themes, already a contrast pair); the slider keeps `component.slider.accent` (apricot fill). The help's wells are `surface.field` with a `border.divider` edge, as the invite code's.
- **Type**: `type-body-sm` with tabular figures for "100 %", `type-category` for the menu's group labels (sentence case, not capitals), `type-code` for the commands.
- **Shape and spacing**: the band is `size.chat.header-height` (48 px) with `space.xs` gaps; controls `size.hit.web-min` (44 px) whatever the pointer, since a stream is often watched from the sofa; the slider at most 160 px and shrinking first on narrow sheets, the status never truncated.
- **Motion**: none added. Status changes are text swaps; the "Copié" label uses `motion.delay.copied-label`, held under reduced motion.

Tokens (both themes checked by `cargo xtask tokens`): `component.stream-view.sound-bar-height`, `sound-bar-gap`, `control-size`, `volume-width`, `sound-status-text`, `slow-connect-delay` (→ `motion.delay.stream-slow`, 10 s, kept under reduced motion as a wait), `component.screen-sound-help.command-*`.

```
Wide: the sheet while watching                        Narrow navigator: the panel while sharing
+------------------------------------------------+    +------------------------------+
| (A) Écran de Ana [EN DIRECT]          [⤢] [x] |    | ▮▮ Voix connectée    [♪] [⏏] |
| +--------------------------------------------+ |    | Salon / Rust Lyon            |
| |                                            | |    | [▣ Arrêter le partage][1080p^]|
| |      picture, letterboxed on the stage     | |    | [EN DIRECT] Tu partages ton… |
| |                                            | |    | (×) Sans son                 |
| +--------------------------------------------+ |    | Ton navigateur ne partage …  |
| [🔊] [=====o-----] 100 %            Sans son   |    | Choisir une source sonore  x |
+------------------------------------------------+    | Partager le son d'une app…   |
                                                      +------------------------------+
Options du partage (menu above the panel)
+------------------------------+
| Qualité                      |
|  720p    30 i/s, le plus…    |
|  1080p   30 i/s, un texte… ✓ |
|  1440p                       |
| ---------------------------- |
| Son                          |
|  Écran partagé             ✓ |
|  Monitor of Audio interne…   |
|  Monitor of Partage-Mellow   |
|  Micro interne               |
| ---------------------------- |
|  Partager le son d'une app…  |   (Linux only)
+------------------------------+
```

## Defaults rejected

- **Discord's hover-revealed controls over the picture.** They hide from keyboards and touch until found, and cover the bottom of a shared document, where text often is. The band is always there, under the picture, the same height as the header.
- **An animated level meter.** A meter redrawn every frame costs main-thread time for the whole watch and draws the eye from the picture. The question watchers have is binary ("is there sound?"), so two words, sampled twice a second.
- **A third button in the share row for the sound.** The narrowest navigator (240 px) cannot hold three halves; the options menu already exists, so it grows a second labelled group.
- **The Linux commands inline in the panel (`<details>`).** In a 240 px card, 100-character commands would wrap into unreadable fragments and push the user card off screen; a dialog has room for numbered steps and Copy buttons, and is loaded only when opened.
- **Sniffing the browser by its user agent.** The advice follows what the picker can do: Chromium is the only browser with `userAgentData`, and its platform says whether the entire screen's sound is offered. The user agent is read only to tell a Linux desktop for the help.
- **A separate "theatre mode".** The stream view already takes only the sheet: the navigator, with the participants and the voice panel, stays beside it on wide windows and one tap away on phones. Fullscreen covers the rest.
- **The stream's sound following the person's volume** (as in v0.32.0). People lower a loud talker and want their film loud, or the reverse; Discord keeps them apart for that reason.
- **Removed before shipping:** the hint "Une entrée audio" under ordinary inputs (the name already says it), and a "Réessayer" button on a picture that is not coming (closing and reopening the view does the same, and says so).

## Gallery

`/dev/voice.html#stream=share` (a share without sound: the "Sans son" line and the notice), `&source=monitor` (sharing a monitor), `&quality=open` (the options), `&help=1` (the Linux dialog), `#stream=live` ("Sans son" after 5 s), `&sound=on` ("Avec son"), `#stream=paused`, `#stream=connecting` (the "not coming" sentence after 10 s), with `&theme=dark|light` and `&wide=0`.

## Not done, and to check on real devices

- A real Linux desktop: that Chrome and Firefox list "Monitor of …" inputs (they do with PipeWire and PulseAudio as far as known), that the two `pactl` commands give "Monitor of Partage-Mellow", and the sound quality through Opus (whether music arrives in stereo is not checked).
- Firefox and Safari: the advice's wording, `getUserMedia` with an exact device id and every processing off, and the meter on a received track.
- Two real browsers: "Avec son" and "Sans son" on real shares, the paused state on a throttled connection, and that switching inputs mid-share is heard at once by the watchers.
- The sharer's own microphone mute does not stop the share's sound (it is not a voice, ADR-0037); if a person shares their microphone as the share's sound, it is heard even while muted. The menu lists microphones because some people play music through an interface they see as an input.
