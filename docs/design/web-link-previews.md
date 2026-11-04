# Link previews and the YouTube player on the web (ROADMAP item 18)

Status: designed and built on `feat/web-link-previews`, on top of `feat/link-previews-server` (ADR-0041: the server fetches, stores and announces previews), DESIGN.md v0.49. `MessageLinkPreviews.svelte` (rendered by `MessageItem.svelte` under the files), the rules in `link-previews.ts` (layout, YouTube id, picture address, the page's colour), tokens `component.link-preview.*` in `design/tokens.json`, the Phosphor `play` icon, and one line of the server's Content-Security-Policy (`frame-src https://www.youtube-nocookie.com`).

## Brief

Ana posts "Le site du langage : https://www.rust-lang.org/" in #général of "Atelier", then a YouTube link for the evening's watch party. Today both are bare blue addresses: Paul has to open a tab to know whether the link is worth it, and the video takes him out of Mellow. The job: **say what is behind a link before anyone clicks it, and let a video play where the conversation is**, without the reader's browser ever talking to the site (the instance fetched everything) and without YouTube learning anything before someone presses play.

Assumption: most links are articles, repositories and videos; a message carries up to three previews (the server's cap).

## Flow and states

1. **Sent.** The message shows its text at once, the link included (nothing is hidden, as on Discord). No skeleton: most links get a card within a second or two, and a placeholder for links that never get one would be a lie.
2. **Preview arrives** (`MESSAGE_LINK_PREVIEWS_UPDATE`). The card appears under the text and the files. Its picture box has its final size before the picture loads, so a list at the latest message stays there (the list's `ResizeObserver` re-pins the bottom; checked: 0 px from the bottom after a preview landed).
3. **Card with a wide picture** (landscape at least 1.6:1 and 300 px wide: social banners, article heroes): the picture large under the text.
4. **Card with a small or square picture** (logos, avatars): an 80 px thumbnail at the trailing top.
5. **Card without picture**: the pill, site name, title, description.
6. **A link that is itself a picture** (the server found no page, only an image): the picture alone, with the attached pictures' maxima, opening the link.
7. **YouTube**: the card's picture in a 16:9 box with the play key. Pressed, the privacy-enhanced player takes the same box and plays; focus moves into it. Several videos can be open at once.
8. **Picture fails** (twice, as attachments retry once): the picture goes, the card stays; a picture-only preview disappears, the link in the text remains. A YouTube box keeps its stage and its play key.
9. **Nothing to show** (a page with only a site name, or a link that is not http(s)): no card.
10. **Suppressed** (`<https://…>`): the server previews nothing. **Forwards** copy no preview. **Edits** bring the new set of previews; a player stays open while its link is still in the message.

```
Desktop, wide picture                        Desktop, thumbnail
┌───────────────────────────────────┐        ┌───────────────────────────────────┐
│▌ GitHub                           │        │▌ MDN Web Docs             ┌─────┐ │
│▌ GitHub - rust-lang/rust: …  link │        │▌ <iframe> HTML inline …   │ img │ │ 80 px box
│▌ Empowering everyone to build     │        │▌ The <iframe> HTML element│     │ │
│▌ ┌───────────────────────────────┐│        │▌ represents a nested …    └─────┘ │
│▌ │           picture             ││        └───────────────────────────────────┘
│▌ └───────────────────────────────┘│
└───────────────────────────────────┘ 432 px at most, the file card's width

YouTube, before and after the click          Phone (320 px)
┌───────────────────────────────────┐        ┌──────────────────────────┐
│▌ YouTube                          │        │▌ YouTube                 │
│▌ Me at the zoo                    │        │▌ Rick Astley - Never     │
│▌ ┌───────────────────────────────┐│        │▌ Gonna Give You Up       │
│▌ │        picture (cover)        ││        │▌ ┌──────────────────────┐│
│▌ │            ( ▶ )  56 px key   ││        │▌ │       ( ▶ )          ││ 16:9 at any width
│▌ │      whole box = the button   ││        │▌ └──────────────────────┘│
│▌ └───────────────────────────────┘│        └──────────────────────────┘
└───────────────────────────────────┘
  then: the player in the same 16:9 box, focused
```

## Design plan

- **Palette**: no new colour. A card is a piece of the ground set into the lit sheet, exactly the file card (`surface.sidebar`, 1 px `border.divider`): what a message brings in from elsewhere (a file, a page) looks the same. The page's own `theme-color` is the only colour from outside, and it goes on a 4 px pill, decoration only, never checked for contrast (DESIGN.md 1.4: information is in the words). Without one, the pill is `border.quote`, the dusk of quotes and reply spines: a page is quoted, not lit, and apricot stays the lamp. The YouTube key is a disc of `media.stage` with a `media.text` glyph (`media.control-hover` hovered), opaque so it reads on any picture, and its focus ring is `media.focus`, as on every dark stage. Contrast pairs added and checked by `cargo xtask tokens` in both themes: site name, title and description on the card (4.5:1), the play glyph on its disc at rest and hovered (3:1).
- **Type**: site name `type.body-sm` in `text.muted` (where it comes from, said quietly), title `type.body-lg` semibold in `text.link` (the one thing to act on), description `type.body` in `text.normal`, three lines at most with its line breaks kept. The title falls back to the host name ("www.rust-lang.org", whose page has no `og:title`), so every card has its link.
- **Shape**: the card `radius.md` like the file card; pictures `radius.sm` inside it, one step smaller so the corners nest. The pill is fully rounded and inset by the card's padding, rather than a border on the card's edge: on a 12 px corner an edge bar bends into a crescent, and inset it reads as Mellow's quote bar.
- **Layout**: the text column and the thumbnail share a row; a large picture or the player takes the column's full width below. Sizes are set before load: the thumbnail through `min(80px, 80px × w / h, w px)`, the large picture `min(100%, w px)` with its `width` and `height`, YouTube a fixed 16:9 box with the picture cropped to fill (a 4:3 `hqdefault` would otherwise jump when the 16:9 player replaces it).
- **Motion**: none of its own. Cards appear without fade (a message arriving is already the event, DESIGN.md 1.3); the key's hover colour uses `motion.duration.fast`, zero under reduced motion.

## Defaults rejected

- **Discord's bar on the card's edge, square corners**: Mellow's cards are rounded; the pill inside reads as its quote bar and stays straight.
- **Apricot when the page has no colour**: the lamp means you or now (1.2); a quoted page is neither.
- **YouTube's red play button**: their mark, and a red that DESIGN.md keeps for danger and mention counts. The dark disc is the media stage's own key, as on Mellow's videos and screens.
- **A loading skeleton per link**: the server may never produce a card (a page built by JavaScript, a refused address); a skeleton that then vanishes moves the list twice.
- **Hiding the text when it is only the link**: Discord keeps it; the address is what was written and is selectable.
- **A thumbnail for every picture**: banners at 80 px are unreadable; **a large picture for every one**: a square logo would take 400 px of the conversation.
- **Loading the YouTube player or its thumbnail from YouTube**: the reader's address would reach Google on every scroll. The thumbnail comes from the instance; only a click loads YouTube.

## Accessibility

- Each card has one Tab stop, its title link, opening in a new tab with `rel="noopener noreferrer"`. The pictures repeat it for the pointer only (`tabindex="-1"`, `aria-hidden`). A picture-only preview is one link whose name is "Image de www.rust-lang.org" / "Picture from www.rust-lang.org".
- The play key is a real `<button>` covering the whole 16:9 box (394 × 222 px on desktop, more than 44 px everywhere), named "Lire la vidéo YouTube : {titre}" / "Play YouTube video: {title}"; Enter or Space plays. The player that replaces it is titled "Vidéo YouTube : {titre}" and takes the focus, so keyboard users land in it.
- Text scales with the browser (rem tokens); long titles and descriptions wrap (`break-words`), never widen the message.
- Checked in headless Chrome (Playwright) against a local instance fetching real pages (rust-lang.org, GitHub, MDN, Wikipedia, two YouTube videos, two picture links): French dark and English light at 1280 × 900, French light at 320 px with touch; no request to YouTube before the click, the embed loaded with the referrer `http://127.0.0.1:18080/` under the server's new policy and "Me at the zoo" played; Tab from the title to the play key, Enter plays and focus is in the iframe; a preview arriving while the list is at the latest message keeps it there.

## Not done

- A user setting to hide link previews (Discord's "Show website preview info"), and removing previews from one's own message.
- Opening a picture-only preview in the picture viewer: the viewer takes attachments (their URLs, sizes and names); a picture link opens the link instead.
- Previews in the search results and the pins panel.
- Players for Spotify, Deezer and others (each needs its origin in `frame-src`), and pausing an attachment when a YouTube video starts (the iframe gives no signal without YouTube's script API).
- The page's colour is shown as given: a white `theme-color` (rust-lang.org) is a bright pill in the dark theme and nearly invisible in the light one, as on Discord.
- Not checked: Rick Astley's video says "Cette vidéo n'est pas disponible" in headless Chrome (its owner restricts embeds or headless playback), Firefox and Safari, a real screen reader, real iOS and Android touch.
