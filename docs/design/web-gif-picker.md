# The GIF picker and GIFs in messages on the web (ROADMAP item 18)

Status: designed and built on `feat/web-gif-picker`, on top of `feat/gifs-server` (ADR-0042: the instance hands the browser Klipy's key and an opaque customer id, and keeps each member's starred slugs), DESIGN.md v0.51. `GifPicker.svelte` (loaded lazily by `ChatPane.svelte` through `LazyComponent`), `MessageGif.svelte` (rendered by `MessageItem.svelte`), the rules in `lib/klipy.ts` (requests, answer checks, message detection) and `components/gif-picker.ts` (search pause, stale-answer guard, arrow keys, favourites' order, tile shape), tokens `component.gif.*` in `design/tokens.json`, and the Phosphor icons `gif` (Fill), `star` (Fill) and `star-outline` (Bold).

## Brief

Ana wants to answer Paul's "on a fini la release" with a GIF, as she would on Discord. The job: **find a GIF in two keystrokes, send it in one, and find the ones you love again**, while following Klipy's terms to the letter (ADR-0042): the browser talks to Klipy itself, results are shown in Klipy's order without filtering, media is loaded from the returned addresses, the search field reads "Search KLIPY" in every language, and "Powered by KLIPY" is visible.

Assumption: most picks come from a one-word search or from favourites; a member stars a handful of GIFs, rarely the 200 the instance allows.

## Flow and states

1. **The instance has GIFs** (`GET /api/v1/gifs/settings` answered at the session's first `Ready`): the composer gains a GIF button before the emoji one. Without a key (404) nothing about GIFs shows anywhere.
2. **Open.** The picker opens beside its button (a sheet at the bottom on phones), the search focused. It loads the member's favourites from the instance first: with at least one, it opens on **Favoris**; otherwise on **Tendances**.
3. **Loading.** Nine placeholder blocks in row shapes (no shimmer: motion only answers actions), "Chargement des GIF…" for screen readers.
4. **Browsing.** Justified rows of tiles in Klipy's order; each tile shows the blurred preview Klipy sends, then the small animated WebP. Scrolling near the end asks for the next page by itself; "Plus de GIF" stays as the keyboard's way.
5. **Search.** Typing pauses 300 ms before Klipy is asked; Enter asks at once (it never sends a GIF). The tabs give way to the results; clearing the field brings them back at once. An answer for an earlier query is dropped (a request counter, plus an abort).
6. **Pick.** A click or Enter on a tile closes the picker, sends the GIF as its own message through the outbox (a reply in progress goes with it, the text being written stays), calls Klipy's share trigger with the search that found it, and puts focus back in the composer.
7. **Star.** The star at a tile's trailing top (shown on hover and focus, always under a coarse pointer, always when starred) changes at once and comes back if the instance refuses. Past 200: "200 favoris au maximum : retire une étoile pour ajouter celui-ci." Too many changes: "Trop de changements d’un coup. Attends un instant, puis réessaie." Unstarring inside Favoris keeps the tile in place (its star turns hollow) so nothing jumps under the pointer; the list is rebuilt the next time the tab opens.
8. **Empty.** Search: "Aucun GIF pour « zzz ». Essaie un autre mot." Favourites: "Pas encore de favori. Mets une étoile sur un GIF pour le retrouver ici." Trending (unlikely): "KLIPY n’a pas de tendances pour le moment. Cherche plutôt un GIF."
9. **Failures**, in the list's place: offline ("Tu es hors ligne. Les GIF reviendront avec la connexion.", retried by itself on `online`), rate-limited (Klipy's 429: a test key allows about 100 requests an hour per instance), key refused (401/403: "Préviens l’administrateur de l’instance."), anything else ("Impossible de joindre KLIPY…" with "Réessayer"), favourites not loaded ("Impossible de charger tes favoris." with "Réessayer"). A failed next page says the same under the tiles, with a link-styled "Réessayer".
10. **Ads.** Klipy's ad objects are HTML for a web view, which this page never renders. If one comes (ads are off on the owner's key), it keeps its place as a neutral tile, "Sponsorisé", named "Contenu sponsorisé de KLIPY", not focusable and not a link: its fields give no safe target to link to.

```
Desktop (432 × 480 at most, beside the GIF button)      Phone (sheet, 70 % of the window)
┌──────────────────────────────────────────┐            ┌────────────────────────────┐
│ ⌕ Search KLIPY                           │            │ ⌕ Search KLIPY             │
│ ★ Favoris   Tendances                    │            │ ★ Favoris   Tendances      │
│ ─────────── ‾‾‾‾‾‾‾‾‾ ────────────────── │            │ ────────────────────────── │
│ ┌──────────────★┐┌──────┐┌──────────────┐│ 96 px      │ ┌─────────★┐┌──────┐┌────┐ │
│ │               ││      ││              ││ rows       │ └──────────┘└──────┘└────┘ │
│ └───────────────┘└──────┘└──────────────┘│            │ ┌────┐┌───────────┐┌─────┐ │
│ ┌─────────┐┌────────────────────────────┐│            │ └────┘└───────────┘└─────┘ │
│ └─────────┘└────────────────────────────┘│            │ …                          │
│ ┌──────────────┐┌────────────┐┌────────┐ │            │                            │
│ │ Sponsorisé   ││            ││        │ │            │                            │
│ └──────────────┘└────────────┘└────────┘ │            │                            │
│ (status line)          Powered by KLIPY  │            │           Powered by KLIPY │
└──────────────────────────────────────────┘            └────────────────────────────┘
```

## GIFs in messages

- A message whose whole text (trimmed) is one `https://static.klipy.com|static1|static2/…` address ending in `.webp`, `.gif`, `.jpg`, `.png` (a picture) or `.mp4`, `.webm` (a muted looping video) is shown as the GIF itself, with no link text. Anything more (words, a second address, `<…>`, another host, `http:`) keeps the link, as older clients show it.
- **Size:** Klipy's address carries no size, so the box is `component.gif.message-height` (224 px) high and as wide as the picture's shape makes it (a square until it loads), at most `attachment.image-max-width` and the column. The height never changes on load, so the list does not move.
- **Motion:** it plays as custom emoji do (`isPictureAnimationWanted(true)`): always, unless reduced motion is on and "Toujours animer" is off. Kept still, the first frame is painted on a canvas (an `<img>` cannot pause) and a "GIF ▶" key (`component.gif.play-button-*`, the media stage, "Lire le GIF", `aria-pressed`) plays that one GIF on demand.
- **Failure:** a GIF Klipy no longer serves, or a page whose policy does not open Klipy (an instance without a key), gives the link back.
- **Star in messages: not done.** Klipy's media addresses (`/ii/{hash}/{xx}/{yy}/{random}.webp`) do not contain the slug, and favourites are kept by slug, so a message cannot be starred from its address. Stars live in the picker only. A later option: send `…/md.webp#klipy=slug` (the fragment never reaches Klipy) and star from it; that changes what ADR-0042 calls "the media URL Klipy returned", so it needs the owner's word.
- Everything else (reply lines, pins, notifications, search snippets) still reads the address.

## Design plan

Within Dusk (ADR-0022); nothing new in the palette.

- **Colour.** The picker is the emoji picker's floating surface (`surface.floating`, `border.divider`, `elevation.high`). Tiles sit on `attachment.image-placeholder` (`state.hover`) until their picture shows. The star and play keys are opaque `media.stage` chips (dark in both themes) so their glyphs keep contrast over any picture: `media.text` hollow star, **apricot** (`media.speaking`, the lamp) filled star, since apricot is "yours" in Dusk. Focus inside a tile uses the stage's sky ring (`media.focus`). Credit and status lines in `text.muted` and `text.secondary`.
- **Type.** Figtree as everywhere: the search in `type.body`, tabs in `type.body` semibold (the tabs' grammar of 6.45), status and credit in `type.body-sm`.
- **Shape and spacing.** `radius.md` outside, `space.xs` padding, tiles `radius.xs` (concentric), 4 px gaps, 96 px rows.
- **Layout concept: a contact sheet.** Every row is the same height and each tile takes a width from its shape (flex-grow by aspect ratio, clamped between 1:2 and 2.5:1), so GIFs keep their framing and the sheet reads in Klipy's order, row by row, left to right; a spacer stops the last row from stretching.
- **Motion.** The picker drops in like the emoji picker (`mw-drop-in`, the sheet rises on phones); the star fades in on hover in `duration.fast`. Nothing else moves but the GIFs, which follow "Toujours animer" and reduced motion: still JPEG tiles in the picker, the canvas frame in messages.

### Defaults rejected

| Default | Why rejected | Instead |
|---|---|---|
| Masonry columns (Tenor, Giphy, Discord) | Columns read top to bottom, so the visual order is not Klipy's, which its terms require; arrows across columns are guesswork | Justified rows of one height: order is reading order, Up and Down land on the nearest tile |
| A square grid with cropped tiles | Crops faces and captions, which are what a GIF says | Tiles take their shape within bounds; only extreme shapes are cropped |
| GIFs in messages sized after load | Each GIF would push the list as it arrives (the address has no size) | A fixed height, the width following the shape |
| Starring from messages by guessing the slug from the address | The address holds no slug; a guess would star the wrong GIF | Stars in the picker only, for now |
| A shimmer skeleton | Motion that answers nothing | Still placeholder blocks in row shapes |
| Rendering Klipy's ad HTML | Arbitrary third-party HTML in the page, against the policy and the ADR | A neutral tile in its place |

## Accessibility

- The picker is a `role="dialog"` popover named "Sélecteur de GIF", light-dismissed (Esc, a click outside); focus starts in the search and returns to the GIF button only if it was in the picker.
- Tabs are a `tablist` (one Tab stop, Left and Right switch); the list is their `tabpanel`, or a `region` named "GIF pour « chat »" while searching, `aria-busy` while loading.
- Tiles: one Tab stop (roving), then the active tile's star. Left and Right follow Klipy's order, Up and Down reach the nearest tile of the row above or below, Home and End the ends, Up from the first row returns to the search, Down from the search enters the tiles. Each tile is named by Klipy's title ("GIF sans titre" without one), shown as its tooltip too.
- Stars are named "Ajouter « titre » aux favoris" / "Retirer « titre » des favoris"; the change is announced ("Ajouté aux favoris."), refusals are said in the status line under the tiles.
- Under a coarse pointer the search, tabs, stars, play key and "Plus de GIF" are 44 px.
- Contrast pairs added to the generator: star icons and the play key on the stage chip, credit and status text on the picker, the ad's text on its tile.

## Privacy and safety

- Klipy requests carry no cookie and no page address (`credentials: "omit"`, `referrerPolicy: "no-referrer"`); media `<img>` use `referrerpolicy="no-referrer"`. Klipy still sees the member's address (ADR-0042).
- Every field of Klipy's answer is checked: slugs as the instance accepts them, media only `https` on Klipy's three hosts without credentials or port, sizes positive integers, blurred previews only base64 picture data URIs (they go into a CSS `url()`), titles cut at 200 characters. A broken item is skipped, never the page.

## Checked

- Vitest: Klipy answer parsing and validation, media choice for tiles, stills and messages, request addresses, locale, failures (429, 403, broken body, network, abort), share trigger body, message GIF detection, debounce, stale guard, arrow keys over rows, favourites' order and toggling, tile shapes, the client's GIF settings per session.
- Playwright on a throwaway instance with `[gifs] klipy_app_key = "mock-key"` and Klipy's hosts intercepted (fixtures from ffmpeg), under the server's real Content-Security-Policy: button shown, trending opens with an ad tile in place, star (PUT), next page on scroll (48 tiles), a slow "ca" answer dropped after "cat", empty and rate-limited states, arrows then Enter sends, the share trigger posted with `q`, the message shows the GIF at 224 px high, a Klipy address with words stays a link, reopening lands on Favoris, Esc returns focus to the button, light theme, reduced motion (canvas frame and play key, JPEG tiles), phone sheet with 44 px stars; no policy violation.

## Not done

- No real Klipy key: field names and behaviours come from docs.klipy.com (read 2026-11-05), not from live answers. To verify with a key: CORS on `api.klipy.com` for the share trigger's JSON `POST` (it needs a preflight), the items endpoint's `slugs` parameter and order, `locale` codes, whether `md.webp` is the right weight for messages (Klipy's median is about 636 KB).
- Stars on GIFs in messages (see above); the GIF shown as "GIF" in reply lines, pins and notifications instead of its address; Klipy's categories and recent items; stickers, clips, memes.
