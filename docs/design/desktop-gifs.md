# Desktop: the GIF picker and GIFs in messages

M3 step 5 for the GPUI client (ADR-0051). The desktop ports the web's GIF picker and message GIFs
([web-gif-picker.md](web-gif-picker.md), ADR-0042, DESIGN.md §6.80) with the same rules, tokens
(`component.gif.*`) and strings (`gifs.*`). This file records how the desktop does it and what it
leaves out.

## Klipy, called by the app

- `mellow-client-core::shared_logic::klipy` ports `lib/klipy.ts` and the picker rules of
  `components/gif-picker.ts` with their tests: answer checks (slugs, `https` media on Klipy's three
  hosts only, positive sizes, titles cut at 200 characters, a broken item skipped, never the page),
  request addresses, Klipy's `locale` (POSIX tags too), the message GIF rule, arrow keys over
  justified rows, the favourites' order and paging, a tile's shape.
- `mellow-client-core::KlipyClient` is the app's own HTTP client for Klipy, never the instance's:
  no Mellow token, no cookie, no referrer, `https` only, redirects only to Klipy's hosts (three at
  most), 15 s timeouts, pages read up to 2 MiB and pictures up to 24 MiB. It asks only for
  addresses built by the rules above.
- The session asks `GET /api/v1/gifs/settings` at a `Ready` until it knows them; `404` keeps GIFs
  off without a word. Favourites go through the instance's routes, as on the web.
- Klipy's pictures load through the picture pipeline (`PictureFile::Klipy`), decoded off the main
  thread within its size and frame caps, with six downloads at a time, one per address however
  many views ask. Their bytes stay in the loader's memory cache of files and their decoded frames
  in the store's, never on disk: Klipy's terms forbid copies and caches beyond what a browser
  keeps. A GIF played after being kept still is decoded again from memory, and its message box
  keeps the shape it last had while it does.

## The picker

The web's layout and states: Klipy's search ("Search KLIPY"), then the Favoris and Tendances tabs,
then the contact sheet, then the favourite status line and "Powered by KLIPY". It opens beside the
composer's GIF button (`popover_position_beside_anchor`, the emoji picker's placement), on
Favourites when the member has some, else on Trending; a tab or a search chosen meanwhile wins.

- **Contact sheet.** `gif_picker::layout::justified_rows` works out the web's `flex-wrap` by hand
  (GPUI's flexbox does not wrap with grow): each tile's basis is the row height times its shape,
  the free space goes by shape, and the last row keeps its tiles near their shape (the web's
  spacer). A tile reached by the keyboard scrolls its row into view by the shortest distance
  (`scrollIntoView({ block: "nearest" })`), set as an offset since the rows sit inside the grid.
- **Lazy pictures.** Only rows within half a sheet of the visible part draw their picture (the
  web's `loading="lazy"`); the next page is asked for when the end is half a sheet away, and "Plus
  de GIF" stays for keyboards.
- **Motion.** Tiles play the small animated picture; under reduced motion they show the small JPEG.
  Where the web plays Klipy's small MP4 (no small WebP), the desktop takes the next animated
  picture Klipy offers (`picture_tile`: small GIF, extra small or medium WebP); MP4 GIFs come with
  step 9.
- **Requests.** Typing pauses 300 ms; Enter searches at once; going back to the words already
  searched asks nothing. Each request bumps a generation and replacing it drops the previous task,
  which cancels its network request; a late answer never changes the sheet.
- **Failures.** The web's sentences. "Offline" is said when Klipy cannot be reached while the
  instance's link is down too, and the request is made again once the session reconnects (the
  web's `online`).
- **Stars.** As on the web: optimistic, put back on refusal with the reason (200 at most, too many
  changes), shown on hover, on the active tile while the sheet or its star has focus, and always
  when starred.
- **Picking** sends the GIF's medium address as its own message through the outbox, with the reply
  in progress; the text being written stays. Klipy's share trigger is called with the search that
  found it. Focus goes back to the composer; Esc or a click outside closes the picker, and focus
  goes back to the GIF button when it was inside.

## GIFs in messages

`messages::MessageGif`: a message whose whole text is one Klipy picture address, on an instance
with GIFs, shows the GIF in a box always 224 px high (no jump when it loads), as wide as its shape
up to 400 px (a square until decoded), letterboxed. Under reduced motion it shows its first frame
and the "GIF ▶" key (`aria` toggled, the stage's sky focus ring) plays that one. A picture Klipy no
longer serves gives the link back. Clip addresses (`.mp4`, `.webm`) keep their link until video
playback (step 9).

## Keyboard and screen readers

Focus starts in the search. Down enters the tiles (one Tab stop, the active tile announced as the
grid's active descendant), Left and Right follow Klipy's order, Up and Down reach the nearest tile
of the next row, Home and End the ends, Up from the first row returns to the search, Enter or Space
sends. Tab from the sheet reaches the active tile's star; Tab stays inside the picker. The tabs are
the shared `Tabs` (Left and Right). Counts and star changes are read out politely.

## Left out

- Klipy's blurred previews under loading tiles (base64 pictures the desktop does not decode yet):
  tiles show the placeholder colour until their picture arrives.
- The star icon in the Favoris tab (the shared tabs take text only).
- The phone sheet: the desktop window keeps the popover, narrowed to the window.
- Categories: the web has none (Klipy's categories are listed as "Not done" in the web's note).

No token, icon or string was added.
