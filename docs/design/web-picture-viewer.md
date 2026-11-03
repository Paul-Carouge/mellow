# Picture viewer on the web (ROADMAP item 18, second slice)

Status: designed on `feat/web-picture-viewer`, DESIGN.md v0.43; built in `PictureViewer.svelte` (logic in `picture-viewer.ts`), opened from `MessageAttachments.svelte`. Web only: no server, protocol or migration change. Tokens `component.picture-viewer.*` and three `color.media.*` additions are in `design/tokens.json`; icons `caret-left` and `arrow-square-out` were added to `design/icons`.

## Brief

Today a picture in a message is a link that opens the file in a new browser tab, which throws people out of the conversation for a glance at a screenshot. The owner wants Discord's viewer: the picture large over the app, the next ones of the same message one key away, and the usual actions at hand. Mellow's people are small communities sharing screenshots of code, photos of an evening, memes; the job is "look closer, then get back to the chat", and sometimes "keep this one" or "send this link to someone".

Real content: in "Rust Lyon" #général, Ana posts four pictures in one message: `capture-écran-2026-11-03-fan_out.png` (2880 × 1800, 1,2 Mo), `soirée.jpg` (4032 × 3024, 3,4 Mo), `chat.gif` (480 × 270, 860 Ko), `schéma-gateway.webp` (1200 × 900, 240 Ko). Paul opens the second from the message list; later he finds the first through message search.

Scope (decided with the owner, not widened here): pictures only (PNG, JPEG, GIF, WebP, AVIF, the types `PREVIEWABLE_IMAGE_TYPES` lists); opened from the message list and the search results panel, both through `MessageAttachments.svelte`. Videos and audio wait for a server change; forwarding does not exist yet.

## Concept: the darkroom

The viewer is Mellow's third media surface, after the stream view and the call view, and it speaks their grammar: the picture on the **dark stage** (`color.media.*`, dark in both themes, "as a cinema is"), and a **band of controls under it**, never laid over it. Discord floats its actions over the corners of a translucent scrim; Mellow keeps the picture whole and the controls in one place a thumb can reach.

- **The room is opaque** (`color.media.stage`), not a translucent scrim. The light theme's scrim (55 %) over a light app would tint every photo grey-pink; an opaque room keeps the picture's own colours, and gives every caption and control a known background, which the token generator can check (it refuses contrast pairs on translucent colours). The room fades in, so it still reads as something opened over the app.
- **One band at the bottom on every width.** File name and the sender's line at the start, the position ("2 / 4") with previous and next in the middle, then download, copy link, open the original and close. On a phone the caption wraps to its own row and the controls sit under the thumb.

## Layout

Wide window (1280 × 800, dark or light theme alike: the room is the same in both):

```
+----------------------------------------------------------------------+
|                                                                      |
|            +--------------------------------------------+            |  room: media.stage
|            |                                            |            |  stage-padding 16 px
|            |                                            |            |  around the picture
|            |        soirée.jpg, fitted to the room      |            |
|            |        (never larger than its pixels)      |            |
|            |        click: actual size                  |            |
|            |                                            |            |
|            +--------------------------------------------+            |
|                    [ (…) Chargement de l'image… ]                    |  status, only when slow
+----------------------------------------------------------------------+
| soirée.jpg                       [<]  2 / 4  [>]      [v] [⧉] [↗] [x] |  band, min 64 px
| Ana · Aujourd'hui à 14:32 · 3,4 Mo                                   |
+----------------------------------------------------------------------+
   caption (start, flex 1)        position (centre)    actions (end)
```

Phone (390 × 844, pointer coarse):

```
+------------------------------+
|                              |
|                              |
| +--------------------------+ |  16 px from the edges
| |                          | |
| |   soirée.jpg, fitted     | |
| |                          | |
| +--------------------------+ |
|                              |
|                              |
+------------------------------+
| soirée.jpg                   |  caption row (wrapped)
| Ana · Aujourd'hui à 14:32 ·… |
| [<] 2 / 4 [>]   [v][⧉][↗][x] |  controls row, 44 px each
+------------------------------+
  + env(safe-area-inset-bottom)
```

- The band is a flex row with `flex-wrap: wrap`: the caption is `flex: 1 1 var(--mw-component-picture-viewer-caption-min-width)` (240 px), so it takes a row of its own exactly when the controls no longer fit beside it; no breakpoint, it also works at 200 % text. At 320 px the action group wraps once more under the position.
- Centring the position: on wide windows the band is a three-column grid in effect (`1fr auto 1fr` behaviour with the caption start-aligned and the actions end-aligned); once wrapped, the position goes to the start of the controls row and the actions to its end.
- The picture: `max-width: 100%`, `max-height: 100%` of the stage (the room minus the band and `stage-padding`), `width` and `height` auto, so it is fitted and **never upscaled past its own pixels** (a 480 × 270 GIF stays 480 × 270). Rounded `picture-radius` (the list's `radius.md`).
- One picture in the message: no previous, next or position; the caption and the actions only.
- Safe areas: the room uses the full viewport (`viewport-fit=cover` is set); the band adds `env(safe-area-inset-bottom)` under its padding, the stage `env(safe-area-inset-top)` above the picture.
- Text: the file name in `type-body-lg` (`name-text`), middle-truncated with `fileNameStartAndEnd` (the full name in its tooltip); the second line in `type-body-sm` (`meta-text`); the position in `type-body` with `tabular-nums` so "10 / 12" does not shift the arrows.

## States

| State | What shows | Announced |
|---|---|---|
| Opening | The preview the list already shows (`/preview`, in the browser's cache), drawn at the **full** picture's fitted size when the attachment has `width` and `height` (it is softer for a moment, never misplaced); without them, at its own size. The full file (`/api/v1/attachments/{id}`) loads in a second `<img>` stacked on it. | Dialog name and description (below). |
| Loading, slow | After `status-delay` (340 ms) without the full file: the status notice at the stage's lower edge, centred, "Chargement de l'image…". A cached picture never shows it. | `role="status"` (polite). |
| Loaded | The full picture replaces the preview (instant swap, no cross-fade). | Nothing more. |
| Full file failed, preview shown | One automatic retry with a fresh element (as `retryImageOnceThenMarkFailed` does), then the notice: warning sign, "L'image en pleine taille ne s'est pas chargée. Vérifie ta connexion, puis réessaie." and "Réessayer". The preview stays, the actions keep working. | Polite. |
| Nothing could load | The stage shows the notice alone, centred: "Impossible d'afficher cette image. Vérifie ta connexion, puis réessaie, ou télécharge-la." and "Réessayer". Download and open the original stay in the band. | Polite. |
| Offline | The same failure sentences: they already say to check the connection. Coming back `online` does not retry by itself; "Réessayer" does. | |
| Actual size | See Zoom. | Toggle state (`aria-pressed`). |
| Link copied | The copy button's icon becomes `check` and its name and tooltip "Lien copié" for `copied-label-duration` (2 s), then back. | "Lien copié" in the polite region. |
| Copy refused | The notice: "Impossible de copier le lien. Autorise l'accès au presse-papiers et réessaie." (no Réessayer button: the copy button is right there). | Polite. |
| Another picture | Previous or next swaps the picture and caption at once; the new one goes through Opening again; the zoom returns to fit. | "Image 3 sur 4 : chat.gif" in the polite region. |
| Message deleted or picture removed while open | The viewer keeps the attachment list it was opened with; a picture the instance no longer serves goes to "Nothing could load". Closing returns focus to the message list's tab stop when the clicked picture is gone. | |
| Long file name | Middle truncation keeps the extension; full name in the tooltip and the dialog's accessible name. | |

The notice (`status-*` tokens): an opaque `media.tile` chip, `radius.sm`, `space.xs` padding, a 20 px icon (`warning-circle` for failures, none while loading) in `status-text`, the sentence, then "Réessayer" as an outlined button (`retry-border`, 1 px, filled with `control-background-hover` when hovered, 44 px tall). Opaque, so it reads over any picture even where it overlaps the picture's lower edge.

## Interactions and keyboard

**Opening.** The inline picture stays an `<a href>` to the full file, so middle-click, Ctrl or Cmd-click, "Copy link address" and "Open in new tab" keep working. A plain primary click (no modifier key) and Enter on the focused link call `preventDefault()` and open the viewer on that picture, with the message's other pictures as its set (in message order; file cards are skipped). The viewer loads lazily (`LazyComponent`), like the other heavy dialogs.

**Closing.** Esc (the native `cancel`), the close button, or a click on the room outside the picture and the band. A click on the picture itself never closes. Focus returns to the picture link that opened the viewer (the one that was clicked, even after moving to another picture: the list's scroll position stays where the reader left it).

**Previous and next.** The two caret buttons, and Left / Right anywhere in the dialog. They **wrap** (after 4 / 4 comes 1 / 4): no disabled button to lose focus on, and the position says where you are.

**Zoom: one toggle, fit or actual size.** Kept in this slice because the commonest picture in Mellow is a screenshot of code or of a game, and fitting a 2880 px capture into a laptop window halves its text; reading it should not need a new tab.
- Only when the picture's natural size is larger than its fitted box. Then the picture is a toggle button, named "Taille réelle", `aria-pressed`, with the `zoom-in` cursor (`zoom-out` while at actual size). A picture that already shows at its own size is a plain image: nothing to toggle, default cursor.
- At actual size the stage becomes a scroll container (`overflow: auto`): wheel, trackpad, scrollbars and a finger pan it natively; no custom drag code. The point that was clicked stays under the pointer (scroll to `clickRatio × naturalSize − pointerOffset`); from the keyboard (Enter or Space on the toggle), the picture's centre is centred.
- At actual size, the arrow keys are left to the browser so the keyboard pans too; previous and next stay on their buttons. Back to fit (a click, Enter or Space), Left / Right change pictures again.
- No zoom steps, no pinch handling: the page keeps the browser's own pinch-zoom on phones (the viewport allows it), which covers finer zoom.

**Actions** (all in the band, in this order):
1. Download: `<a href="/api/v1/attachments/{id}" download="{file_name}">`, icon `download-simple`, named "Télécharger soirée.jpg" (the existing `messages.download`), tooltip "Télécharger".
2. Copy link: copies `new URL(attachmentUrl(attachment), location.origin).href`, the absolute link that works for any member of the instance; icon `copy` (two sheets), named "Copier le lien".
3. Open the original: `<a href target="_blank" rel="noopener noreferrer">`, icon `arrow-square-out`, named "Ouvrir l'original dans un nouvel onglet", tooltip "Ouvrir l'original". The instance serves pictures `inline`, so the tab shows the picture.
4. Close: icon `x`, named "Fermer".

**Keyboard order.** The dialog opens with focus on "Fermer" (the dialog's name and description are read first, and Enter gets you out at once); Tab then cycles: picture toggle (when zoomable), previous, next, download, copy link, open the original, close. Native `showModal()` keeps focus inside and makes the app inert.

| Key | Action |
|---|---|
| Esc | Close (at any zoom) |
| Left / Right | Previous / next picture (wrapping); at actual size, pan instead |
| Enter / Space on the picture | Fit / actual size |
| Tab / Shift+Tab | Controls in band order |

**Swipe: out of this slice.** A horizontal swipe fights with panning at actual size and with the browser's pinch and back gestures, and the previous and next buttons are already under the thumb in the band. Worth revisiting if phone use asks for it.

## Motion

| Moment | What moves | Duration and curve | Reduced motion |
|---|---|---|---|
| Open | The room fades in; the picture fades in and grows from `enter-scale` (96 %) | `motion.duration.base` (200 ms), `motion.easing.enter`: no settle overshoot, a photo should not bounce | Appears at once (the motion token is 0 ms; component duration aliases keep their value, so they are not used) |
| Close | Instant (an exit fade needs the dialog kept mounted while it plays; not in this slice) | | |
| Control hover | Background to `control-background-hover` | `motion.duration.fast`, standard | Colour kept (0 ms) |
| Previous / next, fit / actual size, preview to full | Nothing: an instant swap | | |

Only `opacity` and `transform` animate (DESIGN.md §2.8 cost rules). The in-app "Images animées" choice governs pictures, not UI motion; UI motion follows the system preference through the tokens. An animated GIF or WebP plays in the viewer as it does in the list: opening it is an explicit request to see it, and the instance has no still frame for attachments yet (see Not done).

## Accessibility

- The `<dialog>` is labelled by the file name (`aria-labelledby`) and described by the caption's second line and the position ("Ana · Aujourd'hui à 14:32 · 3,4 Mo", "Image 2 sur 4").
- The picture's `alt` is the file name, as in the list (no alt text exists yet); as the toggle, its name is "Taille réelle" with `aria-pressed`.
- Every control has a French accessible name (below) and a tooltip; icons are decorative (`aria-hidden`).
- The position is visible as "2 / 4" and read as "Image 2 sur 4" (`aria-hidden` on the visible text, an `sr-only` twin).
- One polite live region for loading, failures, "Lien copié" and picture changes.
- Contrast (checked by `cargo xtask tokens`, the room being the same in both themes): `media.text` on `media.stage` 15.8:1, `media.text-secondary` on `media.stage` 7.6:1, `media.text` on `media.control-hover` 10.7:1, `media.focus` (sky 300) on `media.stage` 8.7:1 and on `media.control-hover` 5.9:1, `media.text-secondary` as the retry outline on `media.tile` 6.6:1. Nothing is laid on the picture except the opaque notice, so "over any picture" holds by construction.
- Focus ring: `focus-ring` (`color.media.focus`, the dark theme's sky in both themes): the light theme's darker sky would vanish on the dark room. Use `outline-color: var(--mw-component-picture-viewer-focus-ring)` with the usual width and offset.
- Targets: 44 px for every control whatever the pointer (`control-size`), like the call view's band.
- Text at 200 %: the band grows (`band-min-height` is a minimum) and wraps; the picture's stage shrinks to what is left.

## Strings (French, `strings.pictureViewer`)

| Key | Text |
|---|---|
| `previous` | Image précédente |
| `next` | Image suivante |
| `position(index, count)` (visible) | `${index} / ${count}` |
| `positionLabel(index, count)` | `Image ${index} sur ${count}` |
| `pictureChanged(index, count, fileName)` | `Image ${index} sur ${count} : ${fileName}` |
| `caption(author, timestamp, size)` | `${author} · ${timestamp} · ${size}` (timestamp from `messageTimestampLabel`: « Aujourd’hui à 14:32 », « Hier à 09:12 », or the short date and time) |
| `actualSize` | Taille réelle |
| download (reuse `messages.download(fileName)`) | Télécharger soirée.jpg |
| `downloadTooltip` | Télécharger |
| `copyLink` | Copier le lien |
| `linkCopied` | Lien copié |
| `copyLinkFailed` | Impossible de copier le lien. Autorise l’accès au presse-papiers et réessaie. |
| `openOriginal` | Ouvrir l’original dans un nouvel onglet |
| `openOriginalTooltip` | Ouvrir l’original |
| close (reuse `common.close`) | Fermer |
| `loading` | Chargement de l’image… |
| `fullSizeFailed` | L’image en pleine taille ne s’est pas chargée. Vérifie ta connexion, puis réessaie. |
| `pictureFailed` | Impossible d’afficher cette image. Vérifie ta connexion, puis réessaie, ou télécharge-la. |
| retry (reuse `common.tryAgain`) | Réessayer |

The French strings use the typographic apostrophe (’) as the rest of `fr.ts`. English (`en.ts`): "Previous picture", "Next picture", "Picture 2 of 4", "Picture 3 of 4: chat.gif", "Actual size", "Download", "Copy link", "Link copied", "Couldn't copy the link. Allow clipboard access and try again.", "Open the original in a new tab", "Open the original", "Loading the picture…", "The full-size picture didn't load. Check your connection, then try again.", "Couldn't show this picture. Check your connection, then try again, or download it."

## Sender and date: in

The caption's second line names the sender and the date. In the search results panel the viewer hides the result it came from, and a picture without its sender is a picture without context; Discord shows the same. It costs two props for `MessageAttachments` (the author's shown name, as the message header shows it, and `created_at`), which both callers already have.

## Tokens

New semantic colours (the `color.media` family, the same in both themes):
- `color.media.text-secondary` (`ref.dusk.715`): second lines on the stage.
- `color.media.control-hover` (`ref.dusk.312`): a control's hover and pressed wash on the stage.
- `color.media.focus` (`ref.sky.300`): the focus ring on the stage.

New contrast pairs: `media.text-secondary` on `media.stage` (4.5) and on `media.tile` (3), `media.text` on `media.control-hover` (4.5), `media.focus` on `media.stage`, `media.tile` and `media.control-hover` (3).

`component.picture-viewer.*`:

| Token | Value | Use |
|---|---|---|
| `background` | `color.media.stage` | The opaque room |
| `stage-padding` | `space.md` (16 px) | Around the fitted picture |
| `picture-radius` | `component.attachment.image-radius` (12 px) | The picture's corners |
| `band-min-height` | 64 px | The band under the picture |
| `band-padding-inline` / `band-padding-block` | `space.md` / `space.xs` | Inside the band |
| `band-gap` / `band-row-gap` | `space.md` / `space.2xs` | Between the band's groups / its wrapped rows |
| `caption-min-width` | 240 px | Below this the caption takes its own row |
| `name-text` | `color.media.text` | File name |
| `meta-text` | `color.media.text-secondary` | Sender · date · size |
| `position-text` | `color.media.text` | "2 / 4" |
| `control-size` | `size.hit.web-min` (44 px) | Every band control |
| `control-icon-size` | `component.chat-header.icon-size` (24 px) | Their icons |
| `control-gap` | `space.2xs` | Between controls of a group |
| `control-radius` | `radius.sm` | Their shape |
| `control-icon` | `color.media.text` | Icons |
| `control-background-hover` | `color.media.control-hover` | Hover and pressed wash |
| `focus-ring` | `color.media.focus` | Focus outline |
| `status-background` / `status-text` | `color.media.tile` / `color.media.text` | The loading and failure notice |
| `status-radius` / `status-padding` / `status-gap` / `status-icon-size` | `radius.sm` / `space.xs` / `space.xs` / 20 px | Its shape |
| `retry-border` / `retry-border-width` | `color.media.text-secondary` / 1 px | "Réessayer" in the notice |
| `status-delay` | `motion.delay.loading-notice` (340 ms) | Before "Chargement de l’image…" |
| `copied-label-duration` | `motion.delay.copied-label` (2 s) | "Lien copié" |
| `enter-scale` | `component.dialog.enter-scale` (0.96) | Picture growth on open |

Icons: `caret-left` (new, Fill, the twin of `caret-right`), `caret-right`, `download-simple`, `copy`, `check`, `arrow-square-out` (new, Fill), `x`, `warning-circle`. `Icon.svelte`'s `IconName` union needs the two new names.

## Defaults rejected

- **Discord's translucent scrim with actions floating in the corners.** Corners are far from a thumb and the actions land on the picture; Mellow's media surfaces put the controls in a band under the stage, and the room is opaque so its contrast can be checked.
- **The dialog's settle overshoot on open.** It suits a card landing; on a photograph a bounce reads as a toy. The picture grows with the plain enter curve.
- **A zoom slider, zoom steps or a custom pan-and-drag.** One toggle and native scrolling cover the case that matters (reading a big screenshot), with no gesture code to maintain.
- **Swipe between pictures.** Out, for the conflicts above; the arrows are in thumb reach.
- **Disabled previous / next at the ends.** Wrapping keeps focus on a working button.
- **A thumbnail strip of the message's pictures.** Messages rarely carry more than four; the position and arrows say enough.
- **A spinner over the picture.** The preview is shown at once, and a sentence appears only when the full file is actually slow.
- **Left out before building:** a separate "Taille réelle" button in the band (the picture is the toggle, with its own name and keyboard access), and a top close button duplicating the band's.

## Not done, to validate

- Videos are shown since DESIGN.md v0.45 ([web-inline-media.md](web-inline-media.md#in-the-picture-viewer-designmd-v045)); sounds stay in the list. Forwarding, and "Copier l’image" (writing a picture to the clipboard needs PNG conversion for most types).
- A still frame for animated attachments under reduced motion: the instance serves `/still` for avatars, icons and emoji, not attachments; the viewer plays them as the list already does.
- Transparent pictures with dark ink sit on the dark room and can be hard to see (Discord has the same); a checkerboard or light room toggle waits for a request.
- The phone back gesture does not close the viewer (no other dialog does either).
- To check once built: VoiceOver and NVDA reading of the dialog's name, description and the "Image 3 sur 4 : chat.gif" announcement; panning at actual size with a trackpad, a mouse wheel and a finger; Safari's handling of `download` on same-origin links; 320 px width and 200 % text wrapping.
