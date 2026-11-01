# Soundboard and join sounds on the web (ROADMAP stage 16.4)

Status: web client built on `feat/soundboard-and-join-sounds`, DESIGN.md v0.33, against the contract of ADR-0032 while the server half was written in parallel. Checked with unit tests, the development gallery (`web/dev/voice.html#view=soundboard`, `#view=sound-settings&dialog=add`) and an in-browser render of the upload path; not yet in a real two-person call.

## Brief

The owner wants voice to "work exactly like Discord", in Mellow's look: a server keeps short sounds, anyone connected to a voice channel can play one for everyone there, and each member may pick one of them as the sound others hear when they arrive. The people are small self-hosted communities on a browser tab, mostly with a headset; the job of the soundboard is a quick laugh during a call without stopping the talk, and the job of the settings page is to turn any clip someone has (an MP3 from a meme site, a phone recording) into a short sound in under a minute.

Contract (ADR-0032): sounds are played by each client from `VOICE_SOUND_PLAYED`; the instance accepts only canonical mono 48 kHz 16-bit PCM WAV of at most 5.2 s, so the browser decodes, trims, mixes, applies the volume and encodes; 48 sounds per server; names of 2 to 32 characters, not unique; an optional emoji as a reaction key; `USE_SOUNDBOARD` (bit 20, channel-scoped, on by default) to play and to choose a join sound; `MANAGE_EXPRESSIONS` (bit 17, formerly "Manage emoji") to manage them; 2 s between two sounds of one member.

## Flows and states

**Playing (voice panel → soundboard).** The "Voix connectée" panel gains a soundboard button (`speaker-high`) beside Disconnect, the place Discord gives it in its voice panel. It opens a light-dismiss popover above the button (a sheet from the bottom when the window cannot hold the navigator and the popover side by side).

- Header: "Sons de {server}" and, when you have one, "Ton son d'arrivée ici : Ta-da".
- A grid of three columns: each sound is its emoji (or the speaker glyph) and its name on up to two lines, hyphenated. Your join sound's tile also carries an outline (`border.strong`), never only that.
- A click or Enter plays it for everyone in the channel (`voice_play_sound`). Arrows move between sounds (one Tab stop, the emoji picker's grid keys); Home and End go to the row's ends.
- Right-click, long press, Shift+F10 or the Menu key: "Écouter pour moi" (headphones; plays on this device only) and, with `USE_SOUNDBOARD` in the server, "Définir comme son d'arrivée" / "Retirer le son d'arrivée", which waits in the menu and announces the result. A pointer also finds "Écouter" as a headphones button on the tile's end (on hover; always on touch screens).
- Blocked: every tile dims (still focusable, `aria-disabled`, the menu still works) and the footer says why, in the order a person can act on it: not connected yet ("Les sons se jouent une fois connecté."), no `USE_SOUNDBOARD` in the channel, no `SPEAK` in the channel, deafened. Self-mute does not block, as on Discord.
- Cooldown: after your sound, the footer says "Patiente un instant entre deux sons." over a bar that drains for the 2 s (apricot fill on the meter's track; still under reduced motion). The cooldown starts on the click and is pushed back to 2 s after the instance's echo, so the next frame always reaches it in time and is never silently dropped. Both lines keep their room, so the grid never jumps.
- Empty: "{server} n'a pas encore de son." then, with `MANAGE_EXPRESSIONS`, "Importes-en un…" and "Ajouter des sons" (opens Server settings at Soundboard); without, "Quelqu'un qui gère les expressions peut en ajouter."
- Leaving voice closes the popover; Esc closes the menu first, then the popover, focus back on the button.

**Hearing.** On `VOICE_SOUND_PLAYED` for the channel you are connected to, the sound plays on this device: downloaded once (`GET /api/v1/sounds/{id}`), decoded, kept while a server still lists it, through the chime's shared Web Audio context, so on the output chosen for voices, at output volume × "Volume du soundboard". Overlapping sounds overlap. Nothing plays while deafened or from someone you muted for yourself. The player's row in the channel list shows the sound's emoji after their name for the sound's length (at least 1.5 s), with a small drop-in that reduced motion removes; it is shown, never announced, like the speaking ring.

**Join sounds.** When someone comes into your channel (joining or moving in) with a `join_sound_id` the server still lists, their sound plays instead of the arrival chime, and their emoji shows by their name. Your own join sound replaces your own join chime once your state has arrived. Deafened, muted for you, an unknown sound, or a download that fails: the usual chime plays, so an arrival is never silent.

**Managing (Server settings → Soundboard, `MANAGE_EXPRESSIONS`).** After Emoji, with the same page grammar.

- A sentence on what sounds are, the accepted files and name lengths, then "Importer un son" (dimmed with "Ce serveur compte 48 sons…" once full).
- The list: "Sons" with its count "8 / 48" (read "8 sur 48 utilisés"; `text.danger` and "· Complet" when full). Each row: the emoji on a `surface.field` tile, the name, the duration ("1,6 s", plus ", son d'arrivée" for yours), then Écouter (headphones), Modifier (pencil) and Supprimer (trash), all 44 px.
- "Ajouter un son" dialog: the file is read first ("Lecture de …"; a file over 20 MB or one the browser cannot decode gets one sentence and only Cancel). Then the trimmer: "Partie à garder" with "5,2 s gardées", a waveform of 96 bars where the kept part is lit apricot (Dusk's lamp: the part that will play) and the rest muted; a click or a drag centres the selection there; "Début" and "Fin" sliders set it to the hundredth (keyboard, screen readers, `0:03,2` as their value text); the selection never exceeds 5.2 s nor drops under 0.1 s. "Volume" 0 to 100 % and "Écouter" / "Arrêter", which plays exactly what will be uploaded. Then the name (prefilled from the file name, `_` and `-` as spaces, cut to 32) and the emoji (the composer's picker, server emoji first; "Changer" and "Retirer l'emoji"). "Ajouter le son" renders, encodes and uploads; refusals become sentences (48 reached, 10 uploads per 10 minutes, a file the instance refused, network).
- "Modifier {name}": the same name and emoji fields, saved together (the instance replaces both). A new cut is a new sound.
- Delete asks "Supprimer {name} ?" and says it leaves everyone's soundboard and stops being anyone's join sound; focus then goes to the next row's first button, else the upload button.
- Loading: none, the list comes with the server (`Guild.sounds`, kept by `GUILD_SOUNDS_UPDATE`). Offline: the actions fail with the network sentence; playing needs a connected voice session anyway.

**Permissions and log.** The role editor and channel overwrites gain "Utiliser le soundboard" under Salons vocaux (voice channels only in a channel's editor). Bit 17 reads "Gérer les expressions": emoji and sounds. The moderation log says "Ana a ajouté le son Klaxon", "… a modifié le son …" with "Nom : …, devenu …" and "Emoji : aucun, devenu :parrot:" lines, and "… a supprimé le son …".

**Voice settings.** "Volume du soundboard", 0 to 100 %, under the output volume, default 70 %: uploads are not normalised and are usually mastered louder than speech, so a friend's airhorn should not drown the voices; Discord's slider stops at 100 % too.

## Design plan

No new token: the soundboard is built from existing component tokens, so `design/tokens.json` stays untouched in this stage (the designer's half was limited to `web/`, DESIGN.md and this folder). Aliases worth adding later as `component.soundboard.*` are marked ⟶.

| Element | Tokens used |
|---|---|
| Popover surface, border, radius, shadow, padding, width, gap, heading, muted text | `component.emoji-picker.*` (⟶ `soundboard.background` …) |
| Tile | `custom-emoji.settings-picture-background` (`surface.field`), `emoji-picker.cell-radius`, `emoji-picker.cell-background-hover`, `size.hit-web-min`; join outline `custom-emoji.settings-name-border-hover` (`border.strong`); dimmed `button.disabled-opacity` |
| Cooldown bar | `voice-meter.track`, `voice-meter.level-heard` (apricot), `voice-meter.height`, `voice-meter.radius` |
| Waveform | `voice-meter.track` behind, `voice-meter.level-heard` kept bars, `voice-meter.level-quiet` the rest; height 2 × `size.hit-web-min` |
| Played-sound emoji by a participant | `reaction.emoji-size` (16 px), glyph `voice-participant.name-text-speaking`; `mw-drop-in` at `motion.duration.fast` with `motion.easing.settle` |
| Settings rows | `server-settings.*`, `custom-emoji.settings-picture-*` |

Motion: two state answers only, the popover lifting in (`mw-lift-in`, as menus opened from the foot of the window) and the participant's emoji dropping in. The cooldown bar is a timer, not decoration, and holds still under reduced motion.

Layout:

```
Wide (navigator beside the chat)          Phone (drawer)
┌ navigator ─────────┐                    ┌──────────────────────────┐
│ …channels…         │ ┌ Sons de Rust ─┐  │ …channels…               │
│  Ana  🎺           │ │ Ton son : Ta-da│ ├──────────────────────────┤
│                    │ │ [📯Klaxon][👏 ]│ │ Sons de Rust Lyon        │
│                    │ │ [🎺 ][🔉 ][🐐 ]│ │ [📯 ][👏 ][🎉 ]          │
│                    │ │ ▔▔▔▔▔▔ reason  │ │ [🎺 ][🔉 ][🐐 ]          │
│ Voix connectée 🔉 ☎│◄┘               │ │ reason / hint            │
│ Paul           🎙 🎧│                  └ sheet from the bottom ───┘
└────────────────────┘
```

## Defaults rejected

- **Discord's coloured tiles and big emoji-only buttons**: Dusk keeps one lamp; tiles sit on `surface.field` like the emoji settings' pictures, and apricot only marks what is "now" (the cooldown, the kept part of a waveform).
- **A countdown number on each tile**: one bar and one sentence for the whole grid say the same thing once.
- **Hiding the soundboard without permission**: the button stays and says why, as Discord greys it, so people learn the rule instead of looking for a missing feature.
- **A separate "Join sound" setting page**: the choice lives where the sounds are; settings show it on the row.
- **Uploading the original file**: the instance stores one format only (ADR-0032); the browser renders exactly what "Écouter" played.
- **A drag handle pair on the waveform as the only trimming control**: not reachable by keyboard or screen readers; the sliders are the accessible path and the waveform is a pointer shortcut.

## Accessibility

- Popover `role="dialog"` named by its heading; the sounds are one Tab stop with arrow keys; each sound is a button named by its sound ("…, ton son d'arrivée"), described by the block reason and the keyboard help; its menu is the shared ContextMenu (menu pattern, type-ahead, Esc).
- Dimmed sounds stay focusable (`aria-disabled`). Every reason is in words. 44 px targets for tiles, buttons, sliders (`RangeField`).
- The waveform is `role="img"` with the file's name; the Start, End and Volume sliders carry value texts; the kept length is a polite live region. Name errors use `aria-invalid` and `aria-errormessage`.
- Results (join sound set or removed, sound added, edited, deleted) are announced in polite live regions; played sounds and arrivals are not (they would talk over the call), but the arrival announcement of stage 16.2 stays.
- Reduced motion: no drop-in, no lift, the cooldown bar holds.

## Components

- `SoundboardPopover.svelte`, `SoundSettings.svelte`, `AddSoundDialog.svelte` (trimmer), `EditSoundDialog.svelte`, `SoundDetailsFields.svelte` (name and emoji), `SoundEmoji.svelte` (a sound's picture).
- `soundboard-audio.ts`: download, decode and cache, play through the shared context, decode an upload, render the selection with `OfflineAudioContext`.
- Pure rules in `lib/soundboard.ts` (tested): WAV encoding, selection rules, waveform peaks, names, when the soundboard is blocked, cooldown, who is heard, join cue, gain.
- Client: `MellowClient.soundboardBlock`, `playSound`, `soundIndicators`, `ownJoinSoundId`, `createSound`, `updateSound`, `deleteSound`, `setJoinSound`, and the three new events.

## Not done, to validate

- A real call with two browsers: sync of sounds, the join sound timing (yours plays only if your voice state arrives before the media connects; otherwise the chime plays), overlapping sounds, the output device on Chromium.
- No preloading: a sound someone never played downloads on first play, so its first play may start a little late (ADR-0032 accepts it).
- The waveform shows the whole file at once: for a long song, the 5.2 s window is a sliver; a zoom would help if people upload long files.
- Safari: `decodeAudioData` support for Ogg Vorbis/Opus varies; such files get the "can't read" sentence.
- Icons: the soundboard button and the participant's fallback glyph reuse `speaker-high`, and preview uses `headphones`, because the icon set lives in `crates/mellow-desktop/icons`; Phosphor `waveform` (soundboard) and `play` (preview) would read better.
