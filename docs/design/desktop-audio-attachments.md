# Sounds played in the desktop app, and role mentions in its composer (M3 step 5)

Status: built on `feat/desktop-audio-attachments-and-slash-menu`, after the web's inline media ([web-inline-media.md](web-inline-media.md), ADR-0038, ADR-0045) and role mentions ([web-role-mentions.md](web-role-mentions.md), ADR-0040). Desktop code: `attachment_audio.rs` (the one sound playing, its download and remembered volume), `messages/audio_attachment.rs` (the card), `composer/suggestions.rs` (the role rows); engine: `mellow-media`'s `clip_decoder` and `clip_player`; client-core: `RestClient::download_in_ranges`. New token `component.attachment.audio-volume-width` (72 px), new icon `pause` (Phosphor Fill).

## Brief

Same people and content as the web note: in "Rust Lyon" #general, Ana posts `mémo-vocal.mp3` (12 s) and `note.wav`, which the instance stores as Ogg Opus (ADR-0045). The job: press play without leaving the conversation, and keep the file when the app cannot play it. The web relies on the browser's `<audio>` controls; GPUI has none, so the desktop draws Dusk's own, with the same rules: nothing loads before play, nothing autoplays, one sound at a time, and a failure is never retried by itself.

## Layout

```
+--------------------------------------------------+   the file card it was: max 432 px,
| [♪] mémo-vocal.opus                              |   surface, radius.md, apricot glyph
|     94,3 Ko                                      |
| [>] 0:03 / 0:12  ━━━━━━o──────────  [🔊] [━━o─] |   play, clock, seek, mute, volume (72 px)
+--------------------------------------------------+
```

- The player row sits under the name row inside the card, as the web's native player does. The seek bar takes what is left; it appears once the duration is known (after the first play), so an unplayed card shows play and `0:00` only, like a `preload="none"` player.
- The clock is `m:ss` (`h:mm:ss` past an hour) in `type-body-sm` and the size's secondary colour, so it never competes with the name.
- Cannot play (a type, codec or size the app does not decode, or a file that breaks part way): the player gives way to the web's note, "Couldn't play this file here. Download it to open it in another app." (`playback-note-text`). No output device: the player stays and the note says to plug something in and press play again.

## Decisions and rejected defaults

- **Dusk's controls, not a media-player look.** Play, mute and the two sliders are the existing `IconButton` and `RangeField` (with its label kept for screen readers but not drawn); no new button style, no waveform. A waveform would be the one expressive choice worth spending, and the web does not have it either.
- **No "loading" spinner.** While the file arrives the play button stays in place, disabled, and its name says "Loading mémo-vocal.opus…": the card never changes height.
- **Ranges, not one download.** The file comes through `bytes=N-` requests (206), at most 8 MiB per answer as the instance allows, so a large file does not hold a download slot for long (ADR-0038); the whole file is then kept in memory while it plays, up to 64 MiB, beyond which the card offers the file.
- **Stereo stays stereo.** Voices play in mono through the engine's mixer; a sound attachment has its own output path, resampled straight to the device's rate.
- **One remembered volume** (`media-volume`, `media-muted` in the preferences file), as the web's `mellow.media-volume`; moving the slider unmutes.
- **Role suggestions** follow the web note exactly: people first, then `@everyone` ("Notifies the whole channel", only with the permission) and the mentionable roles, each with its colour dot (or the neutral dot) in the avatar's 24 px slot and "Role" as a secondary word; a picked role writes `@Name` and is sent as `<@&id>`.

## Accessibility

- The card is a group named "Audio mémo-vocal.opus"; the toggle is "Play …" / "Pause …"; the seek bar is a slider named "Position in …" whose value reads "0:03 of 0:12"; mute is a toggle button; the volume slider reads its percentage. All are Tab stops with the Dusk focus ring; the sliders take arrows, Page Up/Down, Home and End.
- Nothing moves on its own: the seek bar only advances while the person's own sound plays, and reduced motion changes nothing here since there is no animation.
- Contrast pairs are the file card's, already checked (`cargo xtask tokens`: 226 pairs pass in both themes).

## Verified

Headless GPUI tests: the card shows its player, at least the desktop hit size tall, until a file is refused, then the note; the engine plays a WAV to its end on a synthetic output, seeks while paused and reports a refused file with its key, hands back to its commands during a long skip ahead, and survives a decoder that panics; Ogg Opus decodes from its pre-skip to its last granule. Not looked at on screen: no signed-in run of the desktop app has been looked at yet (development notes), and this session could not capture the screen.

## Not done

- Sounds keep playing when their row scrolls far out of the list (the web's element goes with the row).
- Ogg Vorbis, surround files and files over 64 MiB show the note instead of playing; there is no download button on the desktop yet (M3 step 9).
- Seeking in Ogg Opus (and ADTS AAC) decodes again from the start. The player checks its commands every 64 silent packets, so pause, stop and another sound still answer at once, but a far seek in a very long or hostile file keeps a core busy while it plays (unmeasured).
- No output device choice: sounds use the system's default until the desktop has voice settings (M3 step 7).
- "#" channel references: neither the web nor the protocol has them, so the composer does not suggest them.
