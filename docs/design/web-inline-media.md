# Videos and sounds played inline on the web (ROADMAP item 18)

Status: built on `feat/inline-audio-and-video`, DESIGN.md v0.44, after the server half (ADR-0038: sniffed `video/*` and `audio/*` types, served inline with byte ranges). Web only: `MessageAttachments.svelte`, its rules in `attachment-preview.ts`, `PLAYABLE_VIDEO_TYPES` and `PLAYABLE_AUDIO_TYPES` in `lib/protocol.ts`. New tokens `component.attachment.video-*` and `playback-note-text`; new icon `file-audio` (Phosphor Fill).

## Brief

The owner asked for "audio files played inline by clicking them, whatever their common format" and videos watched in the app. Mellow's people are small communities: a voice memo for the meeting, a clip of last night's game, a phone recording of a band rehearsal. The job is "press play without leaving the conversation", and sometimes "keep this file".

Real content: in "Rust Lyon" #general, Ana posts `clip.mp4` (640 × 360, 122 Ko) and `portrait.webm` (a phone held upright), then `mémo-vocal.mp3` (12 s) and `note.wav`, then an old `old-codec.mp4` whose MPEG-4 Part 2 video no browser decodes.

Constraints that shape the design (ADR-0038): every range request decrypts the whole file on the instance and counts against 120 downloads per minute per user. So **nothing loads until someone presses play** (`preload="none"`), nothing autoplays, and a failure is never retried by itself.

## Concept: a small screen, and a file that speaks

- **A video is a small screen of the media stage.** The same dark `color.media.stage` as the stream view, the call view and the picture viewer, dark in both themes, so a letterboxed phone video sits on the cinema's black and not on the chat's ground. Its name, size and download sit in a **band under the picture, never over it**: the picture viewer's grammar at list scale. Discord lays the name over the video's top edge on hover; Mellow keeps the picture whole and the actions where they always are.
- **A sound is a file card that plays.** It keeps the download card it was (`surface.sidebar`, `radius.md`, apricot glyph, name as the download link, size, download icon) with the browser's own audio player under the name row. The glyph becomes Phosphor's `file-audio`, so a sound is recognised before its name is read.
- **Native controls**, not a custom seek bar: they are keyboard and screen-reader accessible in every browser, follow the system's colour scheme (`<meta name="color-scheme">`), offer speed and picture-in-picture where the browser has them, and cost no code. Their look is the browser's; the container around them is Mellow's.

## Layout

Wide window (cozy list, video box within the image maxima of 400 × 300):

```
 ana  Aujourd'hui à 16:11
 Le rendu de la démo, et la version portrait
 +----------------------------------------+
 |                                        |   media.stage, radius.md
 |      picture letterboxed (contain)     |   16:9 box: 400 x 225
 |  [>] 0:00  ───────────────  [vol][⛶][⋮] |   native controls
 +----------------------------------------+
 | clip.mp4                 122,2 Ko  [↓] |   band: name link, size, download
 +----------------------------------------+
 Mon mémo vocal pour la réunion
 +--------------------------------------------+
 | [♪] mémo-vocal.mp3                     [↓] |   the file card, max 432 px
 |     94,3 Ko                                |
 | ( [>] 0:00 / 0:12  ━━━━━━━━━━  [vol] [⋮] ) |   native <audio>, full card width
 +--------------------------------------------+
```

Phone (390 px): the video box and the card take the column's width (`min(100%, …)`), the box keeps 16:9; the band and the controls stay one row (the name truncates in the middle, extension kept).

- **The box never moves.** Nothing is fetched before play, so the video's own size is unknown: the box is 16:9 (`min(100%, image-max-width, image-max-height × 16 / 9)`), or the stored width and height when an instance sends them (pictures do; videos not yet). A portrait or square video plays letterboxed on the stage: no layout shift while reading, and the list stays pinned.
- The band: `video-band-padding-inline` (8 px) and `-block` (4 px), the name in `type-body` (`video-name-text`, the stage's text), the size in `type-body-sm` (`video-size-text`), the download icon in `video-download-icon` with `video-download-background-hover` behind it. 32 px targets, 44 px under a coarse pointer, as the file card.
- Focus on the stage uses `color.media.focus` (the light theme's focus colour would not show on the dark stage), drawn inside the box.

## States

| State | What shows | Notes |
|---|---|---|
| Pending (still in the outbox) | The download card without link, as for any file | Nobody else can read it yet. |
| Ready, not played | Video: the stage box with the browser's play control, nothing loaded. Sound: the card with its player at 0:00 | No request is made: the list costs nothing however many videos it holds. |
| Playing | The browser's controls; seeking issues range requests (`206`) | Starting one video or sound pauses the one playing before, anywhere in the app (one at a time, as on Discord). |
| Cannot play | The **download card** in its place (glyph `file` for a video, `file-audio` for a sound), with under the size: "Impossible de lire ce fichier ici. Télécharge-le pour l'ouvrir dans une autre application." (`playback-note-text`) | Covers both an undecodable codec (HEVC outside Safari, MPEG-4 Part 2) and a file the instance did not serve; no automatic retry, since each try decrypts the whole file. If the player had the focus, it moves to the card's name link, which the note describes (`aria-describedby`). |
| Uploaded before ADR-0038 | The download card | Its stored type is still `application/octet-stream`. |
| Removed from the page | The browser pauses it | Scrolling far or switching channel stops the sound. |

## Accessibility

- The video is named "Vidéo clip.mp4", the sound "Audio mémo-vocal.mp3"; the band's name link reads "Télécharger clip.mp4". The duplicate download icons stay out of the Tab order (`aria-hidden`, `tabindex="-1"`), as on file cards.
- Native controls are reachable with Tab and operable with Space, arrows and the browser's own shortcuts; focus rings are drawn in the stage's focus colour inside the video box.
- Uploads come without caption tracks, so the Svelte `a11y_media_has_caption` check is silenced on that one element, with the reason beside it.
- No motion is added; nothing autoplays (DESIGN.md 7.6). Text at 200 %: the band's name truncates and the size keeps its place; the card's note wraps.
- Contrast: `media.text` and `media.text-secondary` on `media.stage`, `text.secondary` on `surface.sidebar`, all already checked pairs.

## Defaults rejected

- **A custom player** (Mellow-drawn play button, seek bar, volume): Discord draws its own. It would need keyboard, screen-reader, buffering and touch handling the browser already does well; the native player is the KISS answer and stays accessible. Revisit if the owner wants a waveform for voice memos.
- **`preload="metadata"`** for a poster frame and a duration: on this instance each video in view would cost one or more whole-file decryptions and eat the download budget just by scrolling.
- **The name over the video's top edge**, as on Discord: hides the picture's top and differs from Mellow's other media surfaces, which keep controls in a band.
- **A grey or themed box** behind the video: a letterboxed phone video on the chat's ground reads as a broken layout; the stage's dark is the media convention throughout Mellow.

## In the picture viewer (DESIGN.md v0.45)

The band's "Agrandir" icon (`corners-out`, between the size and the download icon, named "Agrandir la vidéo clip.mp4") pauses the list's player and opens the picture viewer on that video, going on from the same second and playing (from the start when it had ended). A video reached with Left or Right does not play by itself (`preload="none"`, as in the list): it holds a 16:9 box until played. The viewer's set is the message's pictures and list-playable videos, in message order. A video there fills the room (grown past its own pixels, as a player does; pictures still never are), with the browser's controls; no "Taille réelle" toggle. Left and Right still move between items unless the focus is on the video, where they seek. "Chargement de la vidéo…" appears after the usual delay, only for the video that autoplays, until its metadata arrives; a video this browser cannot play shows "Impossible de lire cette vidéo ici. Télécharge-la pour l’ouvrir dans une autre application." without "Réessayer" (it would fail again; download stays in the band). A video sent with no other media still opens the viewer alone. Previous and next became "Précédent" and "Suivant", and the position "2 sur 4", since an item may be a picture or a video.

## Not done

- A video's real size or a poster frame from the instance (would need probing at upload, ADR-0038 does no transcoding); the 16:9 box letterboxes portrait videos meanwhile.
- Chrome paints its unloaded `preload="none"` video in its own grey under the controls' gradient, not in the stage colour; only the band shows `media.stage` until the first frame.
- The light theme's native audio player is Chrome's light grey pill on the light card: its controls are dark and readable, the pill's edge is faint.
