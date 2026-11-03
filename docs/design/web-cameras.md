# Cameras on the web (ROADMAP stage 16.5, ADR-0035)

Status: web client built on `feat/web-camera-interface`, DESIGN.md v0.41, on the voice session's camera logic (`client.voice`: `canUseCamera`, `isCameraOn`, `isCameraStarting`, `cameraTrack`, `cameraProblem`, `startCamera`, `stopCamera`, `setCameraDevice`, `cameraDevices`, the settings preview, `watchCameras`, `videoTracks`, `pausedVideoUserIds`, and `isCallViewOpen` with `openCallView` / `closeCallView`). Checked with unit tests (`call-view.test.ts`, `voice-session.test.ts`), the development gallery (`npm run dev`, then `/dev/voice.html#call=grid`, see below) and Playwright on the gallery in French, light and dark, at 1280 px and 390 px and with text at 200 %; and two headless Chromes through a release build (see Not done).

## Brief

The owner wants cameras in voice channels "exactly like Discord", in Mellow's own Dusk look (ADR-0035 plan item 3). Mellow's people are small communities hanging out in a voice channel; turning the camera on is how an evening chat becomes a face-to-face one. The jobs: turn the camera on or off at once and always know it is on; pick which camera; see yourself before others do; see who has a camera from the channel list; look at everyone's faces, or one face large, while staying in the call; and understand why a picture is missing.

Real content: "Rust Lyon", Salon: Ana and Paul (you) with cameras, Bob without (muted), Chloé sharing `fan_out.rs`, Dimitri Kowalczyk-Lefèvre whose picture the instance paused; Jeux with Chloé's camera seen from outside the call. Cameras as Chrome names them on a Mac: "Caméra FaceTime HD", "Logitech BRIO".

## Flows and states

**Turn the camera on.** The voice panel's video row starts with a camera split button, before the share's: an icon half (`video-camera`) named "Allumer la caméra" and a caret named "Caméra : Caméra FaceTime HD" (the chosen one). A click turns the camera on and opens the call view, where your own tile says "Allumage de ta caméra…" while the browser asks for the permission (`isCameraStarting`; the half is named "Allumage de la caméra…" and `aria-busy`). Once on, the half is on the apricot chip wash ("you, now") and named "Éteindre la caméra"; "Ta caméra est allumée" is read out. A click while it starts cancels it. Your camera replaces your shared screen and the reverse (ADR-0037: one video per person), so the two halves are never apricot together.

**Choose the camera.** The caret opens **Options de la caméra** above the panel (the one menu of 6.33): the group "Caméra" with "Caméra par défaut" and each camera by name, a check on the chosen one. Before any camera was allowed, the browser names none: the default item says "Les noms s'affichent une fois une caméra autorisée". The choice is remembered on the device and switches the camera that is on at once.

**Problems** (one line in the panel with the warning sign and a close button, polite live region), each saying what happened and what to do: blocked by the browser ("Autorise-la dans les réglages du site… puis rallume-la"), no camera found, another application holds it, browser unable to send it, unplugged while on, stopped by the instance (Vidéo permission taken away, or nothing sent for 10 s), missing Vidéo permission, too many cameras in the channel or on the instance (12 and 25, ADR-0035), video turned off on the instance.

**Who has a camera.** Under a voice channel, a `video-camera` mark (14 px, `voice-participant.state-icon`) after the state icons, and "caméra allumée" after the name for screen readers. In your call it is its own button, "Voir la caméra de Ana" (your own: "Ta caméra est allumée"), which opens the call view; elsewhere it is shown only, with the tooltip "Rejoins Jeux pour la voir".

**Open the call view.** The voice channel row you are connected to now opens the call view (its accessible name ends "tu es dedans. Afficher l'appel"), as clicking the channel does on Discord; joining never opens it, so voice-only evenings keep the chat. Turning your camera on and a camera mark open it too.

**The call view** takes the conversation sheet's place, like the stream view (the chat stays mounted underneath, `inert`). Header in the chat header's grammar: the speaker glyph and the channel's name, then the layout toggles "Grille" (`squares-four`) and "Intervenant" (`user-focus`), the one in use pressed (`aria-pressed`, selected wash), and "Revenir à la discussion" (Esc while focus is in the view; focus returns to what opened it).
- **The stage** is the stream view's dark stage (`color.media.*`, dark in both themes). One tile per person, 16:9, rounded `radius.sm` inside the stage's `radius.md`: people with a picture first, then the others, each in join order. Nine tiles at most (the video receive slots); the others line up under them as 32 px avatars ("Encore 3 dans l'appel"), with the speaking ring and their names in tooltips.
- **Grid** (default): the column count that gives the tiles their largest size in the stage (`gridShapeFor`), rows centred, the last one too, as on Discord. One tile asks the instance for the large layer, 2 to 4 for the medium one, 5 to 9 for the small one (ADR-0035).
- **Speaker view**: one large tile, the person picked or else the last person who spoke (not you, as on Discord; it stays on them through silences and follows the next speaker), else the first picture; under it a strip of up to 8 tiles 96 px high, scrolling sideways when narrow. The large tile asks for the large layer, the strip for the small one.
- **A tile**: the picture filling it (a camera cropped to fill, a shared screen letterboxed with the "EN DIRECT" chip in its corner); your own camera mirrored, as a mirror shows you. Without a picture, the avatar (80 px, 40 px on small tiles) on `color.media.tile`. The name chip sits at the lower start corner (`media.label-background` behind `media.text`), after a crossed microphone when muted, "Paul (toi)" for you. Whoever talks gets Dusk's lamp: a 2 px apricot ring inside the tile's edge (`color.media.speaking`, the dark theme's indicator in both themes since the stage is dark), fading over `motion.duration.fast`.
- **The tile is a toggle**, "Mettre Ana en avant" (`aria-pressed`, described by her states: "Ana, micro coupé, caméra allumée"): it switches to the speaker view with her large; again, and the speaker view follows whoever talks. Focus follows the tile when it moves. A shared screen's tile is "Regarder l'écran de Chloé" instead: it opens the stream view with its sound band and fullscreen; closing that comes back to the call view, which steps aside meanwhile (unmounted, so it asks for no picture).
- **Paused**: when the instance pauses someone's picture for your connection (`VOICE_VIDEO_PAUSED`), the tile shows their avatar and "Connexion trop lente" with `wifi-slash`; in the strip, too short for a sentence, the mark joins the name chip.
- **Your camera starting**: your tile shows your avatar and "Allumage de ta caméra…".
- **Alone, camera off**: your avatar fills the stage; the band's camera button is the obvious next step.
- **The control band** under the stage, as tall as the header (the stream view's rhythm): microphone (`aria-pressed`, Ctrl+Shift+M), camera ("Allumer la caméra" / "Éteindre la caméra", `video-camera-slash` while off, the apricot wash while on), share the screen, and disconnect (warming to danger), 44 px whatever the pointer, so a phone needs no drawer to mute or turn the camera on.
- Phones: the header keeps the navigation button; the grid stacks tiles in one column when that makes them largest.

**Settings, Voix → Vidéo.** A section after the microphone test: "Caméra" (a native select: "Par défaut" and each camera; a remembered camera that is unplugged reads "Appareil débranché" with a sentence, as for microphones), the preview on the call view's stage, and "Tester la caméra" / "Arrêter le test", opened only from that click, as the microphone test is (no camera light just for visiting the page). "Toi seul vois cet aperçu, inversé comme dans un miroir ; les autres te voient à l'endroit. Il s'arrête quand tu quittes cette page." While your camera is on in a call, the preview shows it and says so, without a test button. A refused or missing camera says why, in the settings' error colour.

## Design plan

Tokens (new, both themes checked by `cargo xtask tokens`):
- `color.media.tile` (`dusk.235`), `color.media.label-background` (`ref.alpha.media-label`, the stage's dusk at 72 %), `color.media.speaking` (`apricot.300`), the same in both themes as the rest of `color.media`. Contrast pairs: `media.text` on `media.tile` (4.5), `media.speaking` on `media.stage` and on `media.tile` (3, non-text).
- `component.call-view.*`: `background` (the sheet), `stage`, `stage-radius` (`radius.md`), `stage-inset` and `stage-padding` (`space.xs`), `tile-gap` (`space.xs`), `tile-background`, `tile-radius` (`radius.sm`), `tile-text`, `label-*` (24 px chip, `radius.xs`, `space.2xs` padding, 14 px icons), `speaking-ring` and its width (2 px, the navigator's), `avatar-size-large` 80, `avatar-size-small` 40, `strip-tile-height` 96, `overflow-avatar-size` 32, `control-bar-height` (the header's), `control-size` (44 px), `control-gap`, `control-radius`, `control-*` colours aliasing `voice-panel.share-*` (rest, hover, active apricot, active hover), `hang-up-text-hover`, `layout-background-selected`.
- `component.voice-panel.share-*` now dress the whole video row (camera and screen); their descriptions say so.
- Icons (Phosphor 2.1.1, Fill): `video-camera`, `video-camera-slash`, `squares-four`, `user-focus`.
- Type: `type-heading-md` for the channel in the header, `type-body-sm` on tiles and chips; no new style.
- Motion: the speaking ring's fade only (`motion.duration.fast`, 0 under reduced motion); layout changes are instant, nothing moves on its own.

```
Wide: the sheet while the call view is open
+----------------------------------------------------------+
| ))) Salon                              [▦][◎]  [x]       |
| +------------------------------------------------------+ |
| |   +--------------+  +--------------+                 | |
| |   |   Ana        |  | [EN DIRECT]  |   grid: columns | |
| |   |  (camera)    |  |  Chloé's     |   for the       | |
| |   |[Ana]         |  |[Chloé] screen|   largest 16:9  | |
| |   +--------------+  +--------------+   tiles, last   | |
| |   +--------------+  +--------------+   row centred   | |
| |   |   (D)        |  |  Paul        |                 | |
| |   |~ Connexion   |  |  (mirrored)  |                 | |
| |   |[Dimitri…]    |  |[Paul (toi)]  |                 | |
| |   +--------------+  +--------------+                 | |
| |          +--------------+                            | |
| |          |  (B) [mic/ Bob]   apricot ring: speaking  | |
| |          +--------------+                            | |
| |              (I) (J) (K) (L)  past nine tiles         | |
| +------------------------------------------------------+ |
|              [mic] [cam] [screen] [hang up]              |
+----------------------------------------------------------+

Speaker view                           Panel (240 px navigator)
+--------------------------------+     +------------------------------+
| +----------------------------+ |     | ▮▮ Voix connectée    [♪] [⏏] |
| |                            | |     | Salon / Rust Lyon            |
| |   Ana, large (picked or    | |     | [cam][^]  [▣ Partager l'écran][1080p ^]
| |   last speaker)            | |     | (!) Trop de caméras…       x |
| |[Ana]                       | |     +------------------------------+
| +----------------------------+ |
| [Chloé][Dimitri][Paul][Bob] →  |     Phone: one column, the header's
+--------------------------------+     navigation button, the same band.
```

## Defaults rejected

- **Discord's green speaking border and its blurple buttons.** In Dusk the lamp is apricot ("happening now"), the same ring as beside avatars in the navigator; the buttons keep the panel's video row look, so the camera reads as the share's sibling.
- **Opening the call view on every join**, as Discord does. Most evenings are voice only; the chat stays where it was, and the call view opens from the channel you are in, a camera mark, or your own camera.
- **A camera preview that starts by itself in the settings.** The camera light coming on just for opening Voix would alarm people; the preview starts from "Tester la caméra", as the microphone test does.
- **Controls hovering over the pictures.** Hidden from keyboards and touch until found, and over faces; the band sits under the stage, as the stream view's sound band does.
- **A separate grid algorithm per breakpoint.** One pure function picks the column count that makes the largest tiles for the stage's actual size, so the same code lays out a phone, a laptop and a wide monitor.
- **A third half for the camera's label in the panel.** The narrowest navigator (240 px) keeps "Partager l'écran" whole only if the camera is an icon; its accessible name and tooltip say the action, the apricot wash says the state.
- **Left out before building:** a "Personne n'a allumé sa caméra" sentence on the stage (the avatar tiles already say it, and the band's camera button is the answer), and the camera's own live line in the panel ("EN DIRECT" belongs to screens; your tile and the apricot button show a camera).

## Gallery

`/dev/voice.html#call=grid` (you and Ana with cameras, Bob without, Chloé's screen, Dimitri paused, Ana and Bob speaking), `#call=speaker`, `#call=many` (twelve people: nine tiles and the avatars), `#call=alone` (you alone, camera off; Chloé's camera seen from outside the call in Jeux), `#call=starting` (your camera waiting for the browser), `#camera=open` (the camera's choice), `#cameraproblem=denied|not-found|in-use|unsupported|ended|stopped|missing_permissions|too_many_cameras|video_unavailable`, `#view=settings&camera=preview` (Vidéo with the preview), each with `&theme=dark|light` and `&wide=0`. Cameras are canvases drawn by the gallery.

## Not done

- The layout (grid or speaker) is not remembered between openings, and there is no "hide people without a camera" filter (Discord has one); both wait for the owner's use.
- A tile's fullscreen and picture-in-picture: a camera can be put forward, a screen watched in fullscreen from the stream view.
- The settings preview says nothing while the browser asks for the camera (the button stays "Tester la caméra" until it opens).
- Two-browser check (2026-11-03, two headless Chromes through a local release build, the camera replaced by a moving 1280 × 720 canvas): Ana's camera reaches Bob's call view at 640 × 360 and 30 fps (two tiles, so medium, `l`), the sidebar's camera icon opens it, clicking a text channel closes it, coming back shows the camera again, and turning it off removes the icon. Not yet tried: a real camera and its permission prompt, pausing on a throttled link, Firefox and Safari.
