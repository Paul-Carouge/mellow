# Provider embeds on the web: YouTube, YouTube Music, GitHub, music services, Vimeo, Twitch, Wikipedia

Status: designed 2026-11-05 on top of v0.45.0 (link previews of DESIGN.md v0.49, ADR-0041, `docs/design/web-link-previews.md`). The foundation is built: [ADR-0044](../adr/0044-provider-embeds.md), the `LinkPreview.embed` union (`crates/mellow-protocol`, `web/src/lib/protocol.ts`), its storage (migration 0039), the fetcher's JSON path and the players' `frame-src`. YouTube fills `youtube_video` and `youtube_music_track` from its oEmbed endpoint (ADR-0044), and slices E1 and E2 are built (`web/src/components/embeds/`: `EmbedCard`, `VideoEmbed`, `MusicCard`, `embed-players.svelte.ts`); GitHub (repositories, issues, pull requests) and Wikipedia fill their embeds, and slice E3 and the Wikipedia half of E5 are built (`GitHubCard.svelte`, `WikipediaCard.svelte`, `linguist-colors.ts`); the other providers and slices are not yet. Section 9 is the proposal as written; where they differ (field names, every player origin allowed at once), ADR-0044 and the protocol are what holds.

## 1. Brief

Ana posts in #général of "Atelier": a YouTube link at 1:30 into a talk, a Short, a YouTube Music track for the evening, the `rust-lang/rust` repository, a pull request she reviewed, a Spotify album, a Bandcamp EP, a Twitch clip and a Wikipedia article. Today each is the same card: a site name, a blue title, three lines of description and, for YouTube only, a player. Paul has to open a tab to know whether the repository is alive, whether the pull request is merged, or who sings the track.

The job: **say what is behind a link in the terms of its kind** (a repository has stars and a language, a track has an artist and plays, a pull request has a state) and **let it play where the conversation is**, while keeping ADR-0041's promise: the reader's browser talks to nobody but the instance until the reader presses play.

Assumptions: the instance can fetch the providers' public APIs and oEmbed endpoints (section 9). Most posts are YouTube, GitHub and Spotify; Deezer is common among French users; Apple Music, SoundCloud and Bandcamp are rarer but share one card.

## 2. Principles for this feature

1. **Facts, not logos.** Each provider is named in words on the card's first line (the site name), as today. No provider logo, no provider colour on Mellow's controls. The page's `theme-color` keeps its one place: the 4 px pill.
2. **One structure, many kinds.** Every embed is the existing card (`surface.sidebar`, 1 px `border.divider`, `radius.md`, `space.sm` padding, pill, 432 px at most). Kinds change what sits inside, never the frame, so a conversation full of links still reads as one system.
3. **Nothing from the provider before a press.** Covers, avatars, thumbnails and posters come from the instance (`/link-previews/{id}/image`). The iframe is created on the click, from an address the client builds out of a checked id, never from a URL the page or the server sent.
4. **One player at a time in the list.** Opening a player returns any other open embed player in the message list to its poster (unmounting the iframe stops its sound). Attachments already play one at a time (ADR-0038); embeds follow.
5. **Mellow's colours for states.** States (open, merged, closed, draft) are carried by a word, an icon and a shape, coloured with Mellow tokens. GitHub's green and purple never appear.

## 3. Flow and states

1. **Sent.** The message shows its text at once, the link included.
2. **Preview arrives** (`MESSAGE_LINK_PREVIEWS_UPDATE`). If it carries `embed` data the kind's layout shows; otherwise the generic card of today (graceful fallback, also for older instances). Picture boxes have their final size before the picture loads.
3. **Poster** (players): the instance's picture fills the box, a 56 px key in its centre (the existing `play-button` tokens), a start-time chip when the link has one.
4. **Playing**: the provider's iframe replaces the poster in the same box (videos) or the card's body (music), focus moves into it, a "Fermer le lecteur" / "Close player" key appears on the card's first line.
5. **Another player opens**: this one goes back to its poster; focus is not moved from where the reader is.
6. **Picture failed** (twice): videos keep their stage and key; music keeps a `cover-placeholder` square with the `music-notes` glyph; GitHub keeps a `radius.sm` square with the owner's initial; Wikipedia drops the thumbnail.
7. **Offline when pressing play** (`navigator.onLine === false`): the iframe is not created; one line under the box, `text.danger` with `warning-circle`: "Hors ligne : le lecteur ne peut pas se charger." / "You're offline, so the player can't load." The key stays.
8. **Provider refuses inside its iframe** (video private, region-locked, embedding disabled): the provider's own message shows in the iframe; Mellow cannot read it. The card's title link still opens the page.
9. **Edited message**: the new set of previews; an open player stays while its link is still in the message.
10. **Reduced motion**: nothing here animates beyond the key's colour; autoplay after an explicit press is the reader's request and stays.
11. **Phone (coarse pointer)**: every key is 44 px; the card is the column's width; layouts below.

## 4. Kinds and layouts

Widths: desktop card at most 432 px (`max-width`, unchanged). At 320 px the message column is about 232 px, so the card's inner width (after the 12 px padding, the 4 px pill and its 8 px gap) is about 196 px.

### 4.1 (a) YouTube video and Shorts, with start time

Video: the existing 16:9 box. Short (`/shorts/{id}`, or `is_short` from the instance): a 9:16 box at most `short-player-width` (270 px) wide, so 480 px high; the card shrinks to the player (`width: fit-content`) and its text wraps to the player's width.

Start time: read from `t=` / `start=` / `#t=` (`90`, `90s`, `1m30s`, `1h2m3s`), bounded to 24 h. The poster shows a chip at its bottom leading corner, "Commence à 1:30" / "Starts at 1:30" (`type.body-sm`, `start-chip-*`), and the player opens with `?start=90&autoplay=1`. The key's name becomes "Lire la vidéo YouTube : {titre}, à partir de 1 min 30 s".

```
Desktop, video with start time               Desktop, Short (9:16)
┌───────────────────────────────────┐        ┌───────────────────────┐
│▌ YouTube                    [ × ] │        │▌ YouTube              │
│▌ RustConf 2026: Async closures    │        │▌ Melli makes coffee   │
│▌ ┌───────────────────────────────┐│        │▌ ┌───────────────────┐ │
│▌ │       poster (cover)          ││        │▌ │                   │ │
│▌ │            ( ▶ )              ││        │▌ │   poster (cover)  │ │
│▌ │ [Commence à 1:30]             ││        │▌ │       ( ▶ )       │ │  270 × 480
│▌ └───────────────────────────────┘│        │▌ │                   │ │
└───────────────────────────────────┘        │▌ └───────────────────┘ │
  [ × ] "Fermer le lecteur" only while        └───────────────────────┘
  playing                                      card = player width + padding

320 px, video                     320 px, Short
┌──────────────────────────┐      ┌──────────────────────────┐
│▌ YouTube                 │      │▌ YouTube                 │
│▌ RustConf 2026: Async    │      │▌ Melli makes coffee      │
│▌ closures                │      │▌ ┌──────────────────────┐│
│▌ ┌──────────────────────┐│      │▌ │                      ││
│▌ │   ( ▶ )  196 × 110   ││      │▌ │  ( ▶ )   196 × 348   ││
│▌ │[Commence à 1:30]     ││      │▌ │                      ││
│▌ └──────────────────────┘│      │▌ └──────────────────────┘│
└──────────────────────────┘      └──────────────────────────┘
```

### 4.2 (b) YouTube Music track

A track, not a video: the cover large, the title, the artist, and an explicit "Écouter" / "Listen" button. YouTube's terms forbid playing audio without its visible player, so the button opens **the visible YouTube player** (the same `youtube-nocookie` embed, 16:9) **under** the track row, inside the card. The row stays above it, so the reader keeps the track's identity while it plays. The button turns into "Masquer le lecteur" / "Hide player" (`caret-up`), which unmounts the iframe.

Cover: `cover-size` 96 px at most, `clamp(cover-size-min 64 px, 30 %, cover-size 96 px)` of the card's inner width, square, `radius.sm`, the instance's picture cropped to fill (YouTube Music thumbnails are often 16:9 with the square art in the middle: `object-fit: cover` keeps the art).

```
Desktop, before                               Desktop, playing
┌──────────────────────────────────────┐      ┌──────────────────────────────────────┐
│▌ YouTube Music                       │      │▌ YouTube Music                       │
│▌ ┌────────┐  Les Cités d'or (link)   │      │▌ ┌────────┐  Les Cités d'or          │
│▌ │ cover  │  Noëlle Dupuis           │      │▌ │ cover  │  Noëlle Dupuis           │
│▌ │ 96×96  │                          │      │▌ │        │  [⌃ Masquer le lecteur]  │
│▌ └────────┘  [▶ Écouter]   3:42      │      │▌ └────────┘                          │
└──────────────────────────────────────┘      │▌ ┌──────────────────────────────────┐│
                                              │▌ │   YouTube player, 16:9, focused  ││
                                              │▌ └──────────────────────────────────┘│
                                              └──────────────────────────────────────┘
320 px
┌──────────────────────────┐
│▌ YouTube Music           │
│▌ ┌──────┐ Les Cités d'or  │
│▌ │ 64   │ Noëlle Dupuis  │
│▌ └──────┘                │
│▌ [▶ Écouter]       3:42  │   button and duration move under the row
└──────────────────────────┘   when the text column would fall under 120 px
```

Without an artist from the instance (no `embed`), a YouTube Music link falls back to today's video card with its player.

### 4.3 (c) GitHub repository, issue and pull request

**Repository banner.** GitHub's `og:image` for a repository is a generated social card that repeats the name in a picture; Mellow shows the facts instead.

- Line 1: the site name "GitHub".
- Line 2: the owner's avatar (`owner-avatar-size` 40 px, `radius.sm`, not a circle: DESIGN keeps circles for Mellow people), then the title link "rust-lang / rust": owner in `text.link` at 400, name in `text.link` at 600 (`type.body-lg`), one link, wrapping at the slash when needed. An archived repository adds an "Archivé" / "Archived" badge (the draft style below).
- Line 3: the description (`type.body`, `text.normal`, 3 lines).
- Line 4: up to five topics as chips (`topic-*`: `state.selected`, `text.secondary`, `type.body-sm`, `radius.xs`, `space.3xs` × `space.xs` padding), wrapping. More than five: the first five only (the page has them all).
- Line 5: the stats row, `type.body-sm`, `text.secondary`, tabular figures, icons 16 px in `text.muted`, `stat-gap` (16 px) apart, wrapping on phones: language (a 10 px dot in the language's linguist colour with a 1 px `border.divider` ring, then its name), `star` and the compact star count, `git-fork` and the compact fork count, `scales` and the licence's SPDX id. Compact numbers use `Intl.NumberFormat(locale, { notation: "compact" })` ("104 k", "13,6 k" in French; "104K", "13.6K" in English); the exact number is in the accessible name and the tooltip ("104 238 étoiles").

```
Desktop, repository                                   320 px
┌──────────────────────────────────────────────┐      ┌──────────────────────────┐
│▌ GitHub                                      │      │▌ GitHub                  │
│▌ ┌────┐ rust-lang / rust                      │      │▌ ┌────┐ rust-lang /       │
│▌ │ av │                                      │      │▌ │ av │ rust              │
│▌ └────┘                                      │      │▌ └────┘                  │
│▌ Empowering everyone to build reliable and   │      │▌ Empowering everyone to  │
│▌ efficient software.                         │      │▌ build reliable and      │
│▌ [compiler] [language] [rust] [hacktoberfest]│      │▌ efficient software.     │
│▌ ● Rust   ☆ 104 k   ⑂ 13,6 k   ⚖ Apache-2.0  │      │▌ [compiler] [language]   │
└──────────────────────────────────────────────┘      │▌ [rust]                  │
                                                      │▌ ● Rust   ☆ 104 k        │
                                                      │▌ ⑂ 13,6 k ⚖ Apache-2.0   │
                                                      └──────────────────────────┘
```

**Issue and pull request.** Same frame; the state badge leads.

- Line 1: "GitHub", then the repository path "rust-lang/rust" in `text.muted` (two words, not a meta string).
- Line 2: the state badge (`state-badge-*`, 24 px, `radius.full`, 16 px icon, `type.body-sm` semibold), then "#12345" in `text.muted`, tabular.
- Line 3: the title link (`type.body-lg` semibold, `text.link`).
- Line 4: the body's first lines as plain text (3 lines), when the instance has them.
- Line 5: author and dates, `type.body-sm`, `text.secondary`: "ana-dev l'a ouverte le 3 nov. 2026" / "ana-dev opened this on Nov 3, 2026" (pull requests merged: "fusionnée le 5 nov." / "merged on Nov 5"), then `chat-circle` and the comment count. Nothing more, so the line stays one line on desktop.

| State | Badge (fill, text, icon) | Word FR / EN |
|---|---|---|
| Issue open, PR open | `positive.wash`, `text.positive`, `record` (a dot in a ring) for issues, `git-pull-request` for PRs | « Ouverte » / "Open" |
| PR draft | no fill, 1 px `border.strong` edge, `text.muted`, `git-pull-request` | « Brouillon » / "Draft" |
| PR merged | `state.selected`, `text.heading`, `git-merge` | « Fusionnée » / "Merged" |
| Issue closed as completed | `state.selected`, `text.heading`, `check-circle` | « Terminée » / "Completed" |
| Issue closed as not planned, PR closed unmerged | `danger.wash`, `text.danger`, `x-circle` | « Non prévue » / "Not planned"; « Fermée » / "Closed" |

The word always shows, the icon differs per state, and draft is the only outlined badge: colour never carries the state alone.

```
Desktop, pull request
┌──────────────────────────────────────────────┐
│▌ GitHub  rust-lang/rust                      │
│▌ (⑂ Fusionnée)  #12345                       │
│▌ Stabilise async closures                    │
│▌ This stabilises `async || {}` and the       │
│▌ `AsyncFn*` traits, closing the tracking …   │
│▌ ana-dev l'a ouverte le 3 nov. 2026   ◯ 42   │
└──────────────────────────────────────────────┘
```

### 4.4 (d) One music card for Spotify, Deezer, SoundCloud, Apple Music, Bandcamp

Same row as YouTube Music (cover, title link, artist, "Écouter"), with the item's kind in the artist line when it is not a track: "Album de Noëlle Dupuis", "Playlist de Ana", "Épisode de {podcast}". After the press, the row gives way to the **provider's own player at its own height**; the card's first line keeps the provider name and gains "Fermer le lecteur" (×), so the reader can get back to the quiet card.

| Provider | Link forms (checked) | Player address the client builds | Height (px) |
|---|---|---|---|
| Spotify | `open.spotify.com/(intl-xx/)?{track,album,playlist,episode,show}/{22 base62}` | `https://open.spotify.com/embed/{kind}/{id}?utm_source=mellow` (`&theme=0` in the dark theme) | track, episode 152; album, playlist, show 352 |
| Deezer | `www.deezer.com/(xx/)?{track,album,playlist}/{digits}` (short `link.deezer.com` links resolved by the instance, which sends the canonical one) | `https://widget.deezer.com/widget/{dark,light}/{kind}/{id}` | track 150; album, playlist 300 |
| SoundCloud | `soundcloud.com/{user}/{track}`, `soundcloud.com/{user}/sets/{set}` (path segments `[A-Za-z0-9_-]`) | `https://w.soundcloud.com/player/?url=https%3A//soundcloud.com/{path}&auto_play=true&visual=false` | track 166; set 450 |
| Apple Music | `music.apple.com/{cc}/{album,playlist,song}/{slug}/{id}(?i={track})` | `https://embed.music.apple.com/{cc}/{kind}/{slug}/{id}(?i=…)` | song or `?i=` 175; album, playlist 450 |
| Bandcamp | `{artist}.bandcamp.com/{track,album}/{slug}`; the numeric id comes from the instance (the page's `bc-page-properties`) | `https://bandcamp.com/EmbeddedPlayer/{album,track}={id}/size=large/bgcol={card}/linkcol={link}/tracklist=false/artwork=small/transparent=true/` | track 120; album 120 (tracklist off) |

Heights are provider facts, kept as named constants beside the address builders (`MUSIC_PLAYER_HEIGHT_PIXELS`), not design tokens. Where a provider takes colours (Deezer's theme, Bandcamp's `bgcol`/`linkcol`, Spotify's `theme`), the client passes the current theme's card and link colours read from the tokens, so the player sits in the card instead of on it.

```
Desktop, before (Spotify album)               Desktop, playing
┌──────────────────────────────────────┐      ┌──────────────────────────────────────┐
│▌ Spotify                             │      │▌ Spotify                       [ × ] │
│▌ ┌────────┐  Mellow Hour (link)      │      │▌ ┌──────────────────────────────────┐│
│▌ │ cover  │  Album de Noëlle Dupuis  │      │▌ │                                  ││
│▌ │ 96×96  │                          │      │▌ │  Spotify's player, 352 px high,  ││
│▌ └────────┘  [▶ Écouter]   12 titres │      │▌ │  focused                         ││
└──────────────────────────────────────┘      │▌ │                                  ││
                                              │▌ └──────────────────────────────────┘│
                                              └──────────────────────────────────────┘
320 px: as YouTube Music (cover 64, button under the row); the player takes the full
inner width (196 px); providers' players are responsive down to about 180 px.
```

### 4.5 (e) Vimeo and Twitch

They reuse (a): 16:9 box, poster from the instance, the 56 px key, focus into the player.

- Vimeo: `vimeo.com/{digits}(/{hash})` → `https://player.vimeo.com/video/{id}?dnt=1&autoplay=1(&h={hash})` (`dnt=1`: Vimeo sets no tracking cookie).
- Twitch video `twitch.tv/videos/{digits}` → `https://player.twitch.tv/?video=v{id}&parent={host}&autoplay=true`; clip `clips.twitch.tv/{slug}` or `twitch.tv/{channel}/clip/{slug}` → `https://clips.twitch.tv/embed?clip={slug}&parent={host}&autoplay=true`; channel `twitch.tv/{channel}` → `https://player.twitch.tv/?channel={channel}&parent={host}`. `{host}` is `location.hostname`; Twitch refuses non-HTTPS parents other than `localhost`, so on plain-HTTP instances the key opens the page in a new tab instead (the key's name says so: "Ouvrir la chaîne {nom} sur Twitch"). A channel link never claims "en direct": the instance cannot know at fetch time, and a stale claim is worse than none.

### 4.6 (f) Wikipedia summary

The thumbnail layout of today, with the article's summary instead of the page's `meta description`: the site name ("Wikipédia", plus the language when it differs from the interface: « Wikipédia en anglais » / "French Wikipedia"), the title link, the extract in `type.body`, `text.normal`, up to `wikipedia-extract-lines` (6) lines, the thumbnail (80 px) at the trailing top. No player, no key. Disambiguation pages show the title and "Page d'homonymie" / "Disambiguation page" as the extract.

```
Desktop                                         320 px
┌──────────────────────────────────────────┐    ┌──────────────────────────┐
│▌ Wikipédia                    ┌──────┐   │    │▌ Wikipédia     ┌──────┐  │
│▌ Crépuscule                   │thumb │   │    │▌ Crépuscule    │thumb │  │
│▌ Le crépuscule est la lueur   │ 80   │   │    │▌ Le crépuscule └──────┘  │
│▌ atmosphérique présente avant └──────┘   │    │▌ est la lueur            │
│▌ le lever ou après le coucher du soleil, │    │▌ atmosphérique présente  │
│▌ quand celui-ci se trouve sous …         │    │▌ avant le lever …        │
└──────────────────────────────────────────┘    └──────────────────────────┘
```

## 5. Design plan

- **Palette**: no new colour. Badges use `positive.wash`/`text.positive`, `danger.wash`/`text.danger`, `state.selected`/`text.heading`, an outline of `border.strong`; topics `state.selected`/`text.secondary`; the start chip `media.stage`/`media.text`; the language dot is the only outside colour besides the pill, and it always has its name beside it.
- **Type**: no new style. Titles `type.body-lg` semibold `text.link`; secondary lines `type.body` and `type.body-sm`; numbers tabular.
- **Shape**: the card `radius.md`; pictures and covers `radius.sm` (the card's 12 px radius minus its 12 px padding would be 0; `radius.sm` keeps them reading rounded, as the current previews do); chips `radius.xs`; badges `radius.full` (they are states, like the "New" pill).
- **Layout**: one column inside the card; a leading cover or avatar row for music and repositories; players take the column's full width under the row. Wrapping is intrinsic (flex-wrap with a `text-min-width` basis), never a viewport media query, since container widths depend on the resizable columns.
- **Motion**: none of its own. The key's hover colour (`motion.duration.fast`), as today. The iframe appears in place; no fade.

Anti-slop review of this plan. First draft and what changed:
- First draft put each provider's logo and brand colour on its card (Spotify green, YouTube red, GitHub's mark). Revised to words and the page's pill: logos would turn the chat into a brand wall, need assets and licences per provider, and put a second accent on every card.
- First draft had a card-wide gradient taken from the cover's colours behind music cards (Spotify's look). Rejected: decoration without function, a slop tell, and contrast impossible to guarantee.
- First draft used middle-dot meta strings ("Rust · ☆ 104k · ⑂ 13.6k · MIT"). Revised to icon-led stats separated by space: each figure has its glyph, the dots added nothing.
- First draft put a play overlay on the music cover. Revised to an explicit "Écouter" button: a cover with a triangle reads as a video, and the button's word tells the reader that a player will load.

## 6. Tokens to add (`component.link-preview.*` in `design/tokens.json`)

Existing tokens stay. New ones, with their aliases:

| Token | Value | Use |
|---|---|---|
| `description-lines` | 3 (number) | Description clamp (today a literal `line-clamp-3`) |
| `wikipedia-extract-lines` | 6 | Wikipedia extract clamp |
| `short-player-width` | 270 px (16.875rem) | Width of a Short's 9:16 box (built as `short-player-max-height` × 9/16, the token that already existed) |
| `start-chip-background` | `{color.media.stage}` | Start-time chip on the poster |
| `start-chip-text` | `{color.media.text}` | Its text (15.84:1) |
| `start-chip-radius` | `{radius.xs}` | |
| `start-chip-inset` | `{space.xs}` | Distance from the poster's corner |
| `cover-size` | 96 px (6rem) | Music cover, maximum |
| `cover-size-min` | 64 px (4rem) | Music cover on narrow cards |
| `cover-radius` | `{radius.sm}` | Covers and owner avatars |
| `cover-placeholder` | `{component.link-preview.picture-placeholder}` | Cover before load or after failure |
| `text-min-width` | 120 px (7.5rem) | Text column's flex basis before the button wraps under the row |
| `action-icon-size` | 16 px (1rem) | Icons in "Écouter", "Masquer le lecteur" |
| `owner-avatar-size` | 40 px (2.5rem) | GitHub owner avatar |
| `stat-text` | `{color.text.secondary}` | Stats row |
| `stat-icon` | `{color.text.muted}` | Stats glyphs |
| `stat-icon-size` | 16 px | |
| `stat-gap` | `{space.md}` | Between stats |
| `language-dot-size` | 10 px (0.625rem) | Language colour dot |
| `language-dot-ring` | `{color.border.divider}` | 1 px ring so dark linguist colours stay visible on the dark card |
| `language-dot-fallback` | `{color.border.quote}` | Unknown language |
| `topic-background` | `{color.state.selected}` | Topic chip |
| `topic-text` | `{color.text.secondary}` | 7.21 dark / 5.64 light |
| `topic-radius` | `{radius.xs}` | |
| `topic-padding-block` | `{space.3xs}` | |
| `topic-padding-inline` | `{space.xs}` | |
| `state-badge-height` | 24 px | |
| `state-badge-radius` | `{radius.full}` | |
| `state-badge-icon-size` | 16 px | |
| `state-open-background` / `-text` | `{color.positive.wash}` / `{color.text.positive}` | 6.16 dark / 4.64 light |
| `state-merged-background` / `-text` | `{color.state.selected}` / `{color.text.heading}` | 12.03 / 12.03 |
| `state-completed-background` / `-text` | `{color.state.selected}` / `{color.text.heading}` | same pair, different icon and word |
| `state-closed-background` / `-text` | `{color.danger.wash}` / `{color.text.danger}` | 5.17 dark / 4.97 light |
| `state-draft-border` / `-text` | `{color.border.strong}` / `{color.text.muted}` | Edge 5.27 / 3.30 on the card; text 7.62 / 5.45 |
| `close-player-icon` | `{color.text.secondary}` | The × key on the first line |
| `cover-placeholder-icon` | `{color.text.muted}` | The `music-notes` glyph of a missing cover (added in E2) |
| `player-error-text` | `{color.text.danger}` | The offline line under a player (added in E1) |

Contrast pairs to declare in `$extensions.org.mellow.contrast-pairs` (all computed with the generator's formula on 2026-11-05): `text.positive` on `positive.wash` (4.5), `text.danger` on `danger.wash` (4.5), `text.heading` on `state.selected` (exists), `text.secondary` on `state.selected` (exists), `text.muted` on `surface.sidebar` (exists), `border.strong` on `surface.sidebar` (exists, 3.0), `media.text` on `media.stage` (3.0 exists for the play glyph; add 4.5 for the chip's text).

Icons to add to `design/icons` (Phosphor 2.1.1 Fill): `git-fork`, `git-pull-request`, `git-merge`, `check-circle`, `record`, `scales`, `music-notes`, `caret-up` (if missing). Existing: `star`, `chat-circle`, `x-circle`, `play`, `x`, `warning-circle`.

## 7. Accessibility

- **Tab order per card**: the title link, then the card's action (the play key, "Écouter", or "Fermer le lecteur"), then the player once open. Pictures repeat the title link for the pointer only (`tabindex="-1"`, `aria-hidden`), as today.
- **Names**: the card is a `role="group"` named by its title ("Les Cités d'or, Noëlle Dupuis, YouTube Music"). Keys: "Lire la vidéo YouTube : {titre}, à partir de 1 min 30 s"; "Écouter {titre} de {artiste} sur Spotify" (visible label "Écouter"), with the description "Charge le lecteur de Spotify" / "Loads Spotify's player" (`aria-describedby`, and the tooltip), since pressing it contacts a third party; "Masquer le lecteur" with `aria-expanded="true"` and `aria-controls` on the player. Iframes are titled "Lecteur Spotify : {titre}" / "Spotify player: {title}".
- **Focus**: opening moves focus into the iframe (as today); closing returns it to the key that opened it. When another player opens, the closed one does not steal focus.
- **Stats and badges**: each stat is a `<span>` with an accessible text ("104 238 étoiles", "13 612 forks", "Langage : Rust", "Licence : Apache-2.0"); glyphs are `aria-hidden`. Badges are text with an `aria-hidden` icon. Topics are a list named "Sujets" / "Topics".
- **Targets**: keys are 32 px with a mouse and 44 px under a coarse pointer (`size.hit.*`); the play key covers the whole box.
- **Text**: every provider text is plain, never HTML or Markdown; long titles and descriptions wrap (`break-words`) and never widen the message; text scales with rem tokens to 200 % (the cover keeps its size, the text column wraps under it).
- **Colour**: no information by colour alone (state word and icon, language name, provider name).
- **Motion**: none added.

## 8. Strings (French with tutoiement, English)

| Key | Français | English |
|---|---|---|
| `playYouTubeShort` | Lire le Short YouTube : {titre} | Play YouTube Short: {title} |
| `startsAt` | Commence à {temps} | Starts at {time} |
| `startsAtSpoken` | à partir de {durée} | starting at {duration} |
| `listen` | Écouter | Listen |
| `listenOn` | Écouter {titre} de {artiste} sur {service} | Listen to {title} by {artist} on {service} |
| `loadsPlayer` | Charge le lecteur de {service} | Loads {service}'s player |
| `hidePlayer` | Masquer le lecteur | Hide player |
| `closePlayer` | Fermer le lecteur | Close player |
| `playerTitle` | Lecteur {service} : {titre} | {service} player: {title} |
| `albumBy` / `playlistBy` / `episodeOf` | Album de {artiste} / Playlist de {nom} / Épisode de {podcast} | Album by {artist} / Playlist by {name} / Episode of {podcast} |
| `trackCount` | {n} titre(s) | {n} track(s) |
| `offlinePlayer` | Hors ligne : le lecteur ne peut pas se charger. | You're offline, so the player can't load. |
| `stars` / `forks` | {n} étoiles / {n} forks | {n} stars / {n} forks |
| `language` / `licence` | Langage : {nom} / Licence : {id} | Language: {name} / Licence: {id} |
| `topics` | Sujets | Topics |
| `archived` | Archivé | Archived |
| `stateOpen` / `stateDraft` / `stateMerged` / `stateCompleted` / `stateNotPlanned` / `stateClosed` | Ouverte / Brouillon / Fusionnée / Terminée / Non prévue / Fermée | Open / Draft / Merged / Completed / Not planned / Closed |
| `openedBy` | {auteur} l'a ouverte le {date} | {author} opened this on {date} |
| `mergedOn` | fusionnée le {date} | merged on {date} |
| `comments` | {n} commentaire(s) | {n} comment(s) |
| `wikipediaIn` | Wikipédia en {langue} | {language} Wikipedia |
| `disambiguation` | Page d'homonymie | Disambiguation page |
| `playVimeo` / `playTwitchVideo` / `playTwitchClip` | Lire la vidéo Vimeo : {titre} / Lire la vidéo Twitch : {titre} / Lire le clip Twitch : {titre} | Play Vimeo video: {title} / Play Twitch video: {title} / Play Twitch clip: {title} |
| `watchTwitchChannel` / `openTwitchChannel` | Regarder {nom} sur Twitch / Ouvrir la chaîne {nom} sur Twitch | Watch {name} on Twitch / Open {name}'s channel on Twitch |

French typography: U+202F before `:` and `?`, U+00A0 inside numbers ("104 238"), as the rest of the catalogue. Plurals through `i18n/plural.ts`.

## 9. What the instance must send (protocol and ADR)

The current `LinkPreview` (`url`, `title`, `description`, `site_name`, `theme_color`, `image`) cannot carry stars, a state or an artist. Proposal for the main session and the protocol owner (`mellow-protocol` first, same commit on server and client, all optional so older clients ignore it):

```ts
interface LinkPreview { /* existing fields */ embed?: LinkEmbed }

type LinkEmbed =
  | { kind: "youtube_video"; video_id: string; is_short: boolean; duration_seconds?: number }
  | { kind: "youtube_music_track"; video_id: string; artist: string; album?: string; duration_seconds?: number }
  | { kind: "github_repository"; owner: string; name: string; description?: string; stars: number; forks: number;
      language?: string; topics: string[]; licence_spdx?: string; is_archived: boolean }
  | { kind: "github_issue" | "github_pull_request"; owner: string; repository: string; number: number;
      state: "open" | "draft" | "merged" | "completed" | "not_planned" | "closed"; author: string;
      opened_at: string; merged_at?: string; comments: number; body_excerpt?: string }
  | { kind: "music"; provider: "spotify" | "deezer" | "soundcloud" | "apple_music" | "bandcamp";
      item: "track" | "album" | "playlist" | "episode" | "show"; artist?: string; track_count?: number;
      duration_seconds?: number; bandcamp_id?: string; canonical_url: string }
  | { kind: "vimeo_video"; video_id: string; hash?: string }
  | { kind: "twitch"; item: "video" | "clip" | "channel"; id: string }
  | { kind: "wikipedia_article"; language: string; is_disambiguation: boolean };
```

- `image` stays the one picture: the cover (music), the owner's avatar (repositories; the author's avatar is not shown for issues), the poster (videos), the thumbnail (Wikipedia). Same pipeline, same route, same permission check.
- The client re-checks every id with the same strict patterns as `youTubeVideoIdOf` before building an address, so a compromised or buggy instance cannot point an iframe anywhere else.
- Sources the instance fetches (through the ADR-0041 guard: public addresses only, bounded sizes and times, one fetch per link per day): GitHub REST (`api.github.com/repos/{o}/{r}`, `…/issues/{n}`, `…/pulls/{n}`; unauthenticated, 60 requests an hour per instance, so on a 403 fall back to the generic card and cache the failure for an hour), YouTube oEmbed (title, author: "Artist - Topic" becomes "Artist"), Spotify, SoundCloud and Vimeo oEmbed, Deezer's public API, Apple Music and Bandcamp page `<meta>` (Bandcamp's numeric id from `bc-page-properties`), Wikipedia's REST `page/summary`.
- **ADR-0044 "Provider embeds"** (amends ADR-0041) decides: the providers' API hosts the fetcher may call; `frame-src` gains `https://open.spotify.com https://widget.deezer.com https://w.soundcloud.com https://embed.music.apple.com https://bandcamp.com https://player.vimeo.com https://player.twitch.tv https://clips.twitch.tv` (each only when its slice ships); iframes get `sandbox="allow-scripts allow-same-origin allow-presentation allow-popups allow-popups-to-escape-sandbox"`, `referrerpolicy="strict-origin-when-cross-origin"` and a per-kind `allow` (`autoplay; encrypted-media; picture-in-picture; fullscreen` for video, `autoplay; encrypted-media` for music); one player at a time; a GitHub token in `mellow.toml` is out of scope until rate limits are measured. `security-auditor` reviews E0 (outbound requests, CSP).

## 10. Slices (one PR each)

| Slice | Owner | Files | Acceptance criteria | Size |
|---|---|---|---|---|
| E0 | Server and protocol | `crates/mellow-protocol` (`LinkEmbed`), `crates/mellow-server` link preview fetcher and CSP, ADR-0044, wire samples, `web/src/lib/protocol.ts` | Each kind filled from a recorded fixture of each provider (no live network in tests); fallbacks to the generic card on any provider error; CSP lists only shipped origins | L |
| E1 | Web | `link-previews.ts` (+ tests: start time parser, Shorts, provider id patterns, pill contrast), `MessageLinkPreviews.svelte` split into `components/embeds/EmbedCard.svelte` (frame, pill, first line, close key) and `VideoEmbed.svelte`, tokens of section 6 (via WP0 or its own token PR), `embed-players.svelte.ts` (one open player at a time) | Shorts 9:16 at most 270 × 480; `?t=1m30s` shows "Commence à 1:30" and starts there; opening a second video returns the first to its poster; white `theme-color` falls back to the dusk pill; nothing requested from YouTube before the press (Playwright `page.on("request")`) | M |
| E2 | Web | `components/embeds/MusicCard.svelte` (shared row), YouTube Music branch | Cover 96 (64 at 320 px), "Écouter" opens the visible 16:9 player under the row, "Masquer le lecteur" removes it and returns focus; without `artist`, today's video card | S |
| E3 | Web | `components/embeds/GitHubCard.svelte`, `components/embeds/linguist-colors.ts` (generated table of the 60 most used languages from GitHub Linguist, MIT, with attribution) | Repository: avatar, owner / name, description, five topics max, language dot and name, compact stars and forks with exact numbers spoken, SPDX licence; issue/PR: badge word + icon per state, never GitHub colours; 320 px wraps without overflow | M |
| E4 | Web | `MusicCard.svelte` providers, address builders and heights in `link-previews.ts` | Spotify, Deezer, SoundCloud, Apple Music, Bandcamp each open their player at the listed height after the press, themed where the provider allows; nothing requested before the press | M |
| E5 | Web | `VideoEmbed.svelte` (Vimeo, Twitch), `components/embeds/WikipediaCard.svelte` | Vimeo with `dnt=1`; Twitch with `parent` = page host, new tab on plain HTTP; Wikipedia extract six lines, language named when it differs | S |

Every web slice: Playwright at 1440 dark and light, 390 dark touch, 320 light touch, French and English; `npm run check`, `npm test`, `npm run build` without warnings; DESIGN.md 5.4 "Link previews" and this note updated; the in-flight `fix/youtube-links-play-inline` worktree lands before E1.

## 11. Defaults rejected

- **Provider logos and brand colours on cards and buttons** (Discord, Slack): a wall of brands, second accents everywhere, licences per logo. Words on the first line and the page's pill instead.
- **Loading the provider's iframe as soon as the message shows** (Discord does for Spotify and YouTube thumbnails): every scroll would tell Spotify or Google who reads the channel. Click to load, poster from the instance (ADR-0041).
- **Audio-only playback of YouTube Music**: forbidden by YouTube's terms. The visible player opens under the track row.
- **GitHub's state colours** (green open, purple merged, red closed): GitHub's identity, and purple is close to the hue range Mellow excludes. Words, icons and Mellow tokens, with the draft badge outlined.
- **GitHub's social card picture** (`og:image`) for repositories: a picture of text that repeats the title, unreadable at 432 px and invisible to screen readers. The facts as text.
- **Exact star counts on the card** ("104 238"): noise at a glance; compact on screen, exact for screen readers and in the tooltip.
- **A play triangle over the music cover**: reads as a video; "Écouter" says what happens.
- **Cover-colour gradients behind music cards**: decoration, contrast not guaranteed, a slop tell.
- **Several players playing at once**: two sounds over each other; one at a time.
- **A per-provider card design**: five music cards that each look like their brand. One shared card, the provider's own player only after the press.

## 12. Not done

- A setting to turn embeds into plain links, or to always load a given provider's players (Discord has neither for players); a per-message "remove embeds" for one's own messages (already listed for link previews).
- GitHub gists, commits, releases, discussions, GitLab, Codeberg and Forgejo repositories (Forgejo is close to Mellow's audience; it would reuse `GitHubCard` with its own API, worth a later slice).
- Playlists with their track lists outside the providers' players; podcast chapters; live state for Twitch channels.
- Embeds in search results and the pins panel (link previews are not there yet either).
- Pausing an attachment's video when an embed starts, and the reverse, across iframes (no signal without each provider's script API).
- Not verified: provider players' behaviour inside the proposed `sandbox` (to test per provider in E1 to E5), Bandcamp's id extraction on custom domains, Apple Music in Firefox, Twitch on a real HTTPS instance.
