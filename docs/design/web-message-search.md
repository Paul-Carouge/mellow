# Message search on the web (ROADMAP item 15)

Status: built on `feat/web-message-search` (2026-10-31), on top of `feat/message-search` (server, protocol and data layer, ADR-0029). DESIGN.md v0.29, sections 5.10, 5.12, 6.61 to 6.63 and 8.

The owner asked for search "in Elasticsearch mode": ranked results, filters, highlights, facets, fast and pleasant. The server gives relevance or newest order, highlight ranges, a total up to 1 000 and the ten largest channels and authors among the matches. This note is the interface on top.

## 1. Brief

- **Who**: people in a server of friends or a community, and anyone looking back through their conversations at Home. They remember a word, a person, roughly when, or that it had a link or a picture.
- **Job of the screen**: find the message, then read it in its conversation. Search is a way back into the chat, not a place to stay: every result opens its channel at the message.
- **Real content**: short chat lines in French and English, accents, emoji, links, pictures, mentions, code. Results come from several channels at once.

## 2. Two ways around the header (the owner's decision)

Item 12 turned "Go to… Ctrl K" into a search-looking field in the chat header. The first proposal replaced it with message search and kept the switcher behind `Ctrl+K` and a line in the search suggestions. **The owner chose to keep both** (2026-10-31): "Go to… Ctrl K" stays with its item 12 style and tokens, and message search sits beside it. What keeps them apart:

- **Words and shortcuts, not only icons.** "Go to… Ctrl K" (« Aller à… ») and "Search Rust Lyon · Ctrl F" (« Rechercher dans Rust Lyon ») each say what they do; the switcher's is a compact button at its content's width, search is the field that grows when used.
- **Collapse without two magnifiers.** Where the header lacks room, "Go to…" becomes a **compass** button (Phosphor `compass`, Fill; same name, "Go to a channel, server or person"): below `component.quick-switcher.trigger-collapse-width` (640 px of header) beside the search field, or below `size.chat.min-width` (480 px) beside search's magnifier, so on phones. Two identical magnifier buttons side by side would leave people guessing; the compass says "go somewhere", the magnifier keeps "search".
- **The switcher's door stays in the suggestions.** An empty search field still ends its list with "Go to a channel, server or person · Ctrl K", for people who reach for the field first.

| Header width | Quick switcher | Message search |
|---|---|---|
| Wide, results dock (window >= 1 224 px) and header >= 640 px | "Go to… Ctrl K", content width | The field, 208 px, 288 px focused or filled |
| Wide, a sheet docked beside a narrow chat (header < 640 px) | Compass button | The field |
| 816 to 1 223 px (results open over the chat) | "Go to… Ctrl K", 208 px, giving way to the topic | Magnifier button |
| Phones (header < 480 px) | Compass button | Magnifier button |

On phones the channel name keeps its room as with item 12 ("gene…" at 320 px; the header's buttons lose their gaps under a coarse pointer).

## 3. Design plan

### Tokens

| Token | Value | Role |
|---|---|---|
| `color.match.background` (new) | `ref.amber.match-dark` #5C4527 / `match-light` #F3D8A8 | Words a search found, and the message a result opened: the lamp held over the text. Amber, never apricot: apricot is "you" and "now" (ADR-0022), a match is neither. |
| `component.search.match-text` | `text.heading` | Matched words, semibold: the match never rests on colour alone. 8.3:1 dark, 12.2:1 light (contrast pairs checked by the generator). |
| `component.search.result-background` | `surface.chat` / light `surface.input` | A result is a slip of the conversation sheet laid on the side sheet's ground (`surface.navigator`); light needs the input tint since both surfaces are white. |
| `component.search.result-radius` / `-padding` / `-gap` / `-avatar-size` | `radius.md`, `space.sm`, `space.xs`, 32 px | Slip shape. |
| `component.search.field-width` / `-width-active` / `-min-width` | 208 / 288 / 144 px | The header field at rest, focused or filled (it grows into the topic's room), and the least it gives up. |
| `component.search.suggestions-*` | `surface.popover`, `radius.md`, 400 px wide, 420 px tall at most, 40 px options | The suggestion list under the field, in the context menu's family. |
| `component.search.panel-width` | 400 px | The results sheet; docked from 1 224 px (navigator + 480 px chat + 400 px sheet + insets). |
| `component.search.facet-*`, `filter-*`, `sort-*` | `surface.input` chips, `state.selected` for applied filters and the chosen sort, 28 px (44 px under a coarse pointer) | Facets narrow, applied filters show what narrows. |
| `component.message.highlight-background` | now `color.match.background` (was the mention wash) | The message a result opened. A mention row already wears the mention wash, so the old value would have been invisible there. |
| `motion.delay.jump-highlight` (new) | 2 000 ms | How long the opened message stays lit; kept under reduced motion, where it holds still and then goes. |
| `component.quick-switcher.trigger-*` (kept), `trigger-collapse-width` (new) | 208 px, 16 px magnifier; 640 px | "Go to… Ctrl K" keeps item 12's look; below 640 px of header beside the search field (480 px beside search's magnifier) it becomes a compass button. |

Type: the existing scale. The count titles the sheet in `type.heading-md` with tabular figures; results use the message styles (`type.message-author`, `type.message-timestamp`, `type.message`), so a result reads like the line it came from. Channel labels are `type.body-sm` semibold in `text.muted`, the facet group labels `type.body-sm` muted, chips `type.body-sm`.

Motion: the sheet slides in like the members sheet (`motion.duration.slow`, enter easing); suggestions drop in (`fast`); the field's growth eases over `base`; the opened message's wash holds for 40 % of `motion.duration.highlight` and then fades with the exit easing. Reduced motion: no slide or growth, and the wash holds still for `motion.delay.jump-highlight`.

### Layout

```
Wide (>= 1 224 px): field in the header, results docked
+---------+ +--------------------------------------------+ +--------------------------+
| strip + | | # general [Go to… Ctrl K][(o) pizza    (x)]| | 2 results            (x) |
| channels| |                     +--------------------+ | | [Relevance | Newest]     |
|         | |  messages…          | People             | | | (From Chloé Martin x)    |
|         | |                     |  (C) Chloé   chloe | | | Narrow to a channel      |
|         | |                     +--------------------+ | | (#dev 1) (#archive 1)    |
|         | |                                            | | # archive                |
|         | |                                            | | +----------------------+ |
|         | |                                            | | |(C) Chloé  Today 00:05| |
|         | |                                            | | | Archived ▓pizza▓ poll| |
|         | | [composer]                                 | | +----------------------+ |
+---------+ +--------------------------------------------+ +--------------------------+

816 to 1 223 px: "Go to… Ctrl K" and a magnifier in the header; the sheet opens over the chat with its own field
Phones (< 816 px): compass and magnifier buttons; the same sheet, full window, field at its top
+------------------------------+
| 11 results               (x) |
| [(o) pizza              (x)] |
| [Relevance | Newest]         |
| Narrow to a channel …        |
| # general                    |
| [slip] [slip] …              |
+------------------------------+
```

Everything is start-aligned. A result's Jump button sits at the slip's top end, shown on hover and focus with a mouse and always under a coarse pointer; a click anywhere on the slip that is not a link, a spoiler or a file opens it too.

### Principles

1. **The query text is the truth.** Suggestions write into it, chips remove a part of it, facets add `in:` or `from:` to it; what you see in the field is what is searched (`search-query.ts` mirrors the server's parser, so limits and filters agree).
2. **Lit, not boxed.** Matches get the amber wash and weight, inside the message's own formatting (bold, links, quotes keep theirs).
3. **Search leads back to the chat.** Opening a result puts you at the message in its channel, lit for a moment, with the results still docked beside it on wide windows.
4. **Calm when it fails.** Too fast, too long, nothing found: one plain sentence and the next step, never an error code.

### Defaults rejected

- **A yellow `<mark>`** (the browser's highlighter): replaced by a flattened amber tuned to the dusk surfaces, with weight as the non-colour cue.
- **A modal "search dialog" or command palette for messages**: results must sit beside the conversation so opening one keeps the context (Discord's model); the modal stays the switcher's.
- **Numbered pages** (Discord's 1 2 3 … under the results): the list loads the next 25 as its end comes into view, up to the 1 000 the server reaches, then says to refine.
- **A filter-builder form with dropdowns**: the query language plus suggestions keeps hands on the keyboard and teaches the filters by showing them.
- **Results as identical cards with a shadow**: slips have no shadow; the side sheet's ground and the conversation's tone separate them, as the sheet separates the chat from the ground.
- **The full filter list after every word**: it showed again after each choice and covered the chat; between words it now comes back only when you start typing a filter's name, and lists everything only in an empty field.
- **An illustration for "no results"**: Melli is spent on sign-in and the Friends space; here the sheet stays text, one title and one sentence.

## 4. Flow and states

1. Focus the field (click, `Ctrl+F` / `Cmd+F`, or the magnifier on narrower windows). Empty: the filters and the switcher's door. Typing a filter's first letters offers it; after `from:`, `mentions:` people of the server (Home: your conversations' people and friends), after `in:` its text channels (Home: conversations, written as their id), after `has:` link, file, image, pinned, after `before:`, `after:`, `during:` today, yesterday, a week ago, a month ago and the day typed once it is a real one.
2. Typing searches after a 400 ms pause (`SEARCH_TYPING_PAUSE_MILLISECONDS`), never while a filter's value is being typed; Enter or choosing a value searches at once. One request is in flight at a time; the latest wish waits for it, stale answers are dropped (the instance refuses overlapping searches).
3. The sheet opens with the results. States:

| State | What shows |
|---|---|
| Idle (sheet open, field empty) | "Search messages in Rust Lyon", one sentence on how words match, three examples (`"see you soon"`, `-spoiler`, `from:`) |
| Loading | Three skeleton slips after `motion.delay.skeleton`, `role=status` "Searching…" |
| Results | Count title ("1 000+ results" at the cap), sort (only when words are searched: filters alone are always newest first), applied filter chips with remove buttons, channel and author facets (hidden once that filter is applied or when only one value exists, five at most), the slips labelled by channel where the channel changes |
| More loading / failed | One skeleton slip at the end / the reason and Try again (only that page) |
| Reached 1 000 | "These are the first 1,000 results. Add words or filters to reach the others." |
| No results | "No messages match" and "Check the spelling, use fewer words or remove a filter.", with the applied filters to remove |
| Too long / too many parts | Checked before sending: "Searches are at most 256 characters…" or "…at most 16 words and filters…" |
| Rate limited | "Searching too often" and "You searched many times in a row. Wait a few seconds, then try again.", Try again |
| Failed | "Couldn't search messages: the instance could not be reached. Try again." and Try again |
| Blocked author | Folded as in the channel: "1 blocked message · Show"; Show reveals it for the sheet's life |

4. Opening a result: the channel or conversation opens, older pages load until the message is cached (`MellowClient.loadHistoryUntilMessage`, 20 pages of 50 at most, always next to the oldest cached message so the cache never has a gap), the sheet closes if it was over the chat, and the message scrolls to the middle, takes focus and is lit. The result's Jump reads "Opening…" meanwhile. If the message is more than 1 000 messages back: the channel opens and says "This message is more than 1,000 messages back in #archive. Scroll up to reach it."; deleted since: "This message was deleted, or you can no longer read it."; the conversation left the list: "This conversation is no longer in your list."
5. Close (×) empties the search and returns focus to the field (or the composer). Esc: closes the suggestions first; then, in the header field or the docked sheet, it empties the search; over the chat it closes the sheet and keeps the results for the magnifier. Opening the members sheet (`Ctrl+U`) puts the results aside; another server starts with an empty search.

## 5. Accessibility

- The field is a `combobox` owning a `listbox` (`aria-expanded`, `aria-controls`, `aria-activedescendant`); focus never leaves the field. A value's first option is active by itself (Enter picks it); a filter list has none until Down is pressed, so Enter on plain words searches. The count of suggestions is said politely with the keys to use.
- Results: the sheet is an `aside` named "Search results"; the count is announced politely once a search ends ("11 results for pizza."), "No messages match" likewise; loading regions are `role=status`, failures `role=alert`. Each slip is an `article` named by its author, with the channel in a hidden span; Jump is a real button named "Jump to Ada's message".
- The sort is a radio group of native radios; applied filters are a list of chips whose remove buttons say "Remove filter: From Ada"; facets are buttons named "#dev, 2 results".
- Matches are `<mark>` with weight as well as colour. Contrast: `text.heading` on the match wash 8.3:1 dark and 12.2:1 light, `text.normal` 5.9 and 9.3; slip text on `result-background` is checked by the generator.
- Targets: field, chips, sort and Jump reach 44 px under a coarse pointer. Keyboard: `Ctrl+F` / `Cmd+F` (replacing the browser's find while Mellow has focus, as Discord does), Esc as above, Tab through sort, chips, facets and Jump buttons.
- Reduced motion: no slide, no growth, the opened message holds its wash still.

## 6. Files

- `web/src/components/search-query.ts`: the query language (parts, filters, caret context, limits, edits), mirroring `domain::search_query`.
- `web/src/components/search-suggestions.ts`: filters, people, channels, kinds and days to suggest.
- `web/src/components/search-highlights.ts`: ranges normalized (clamped, merged, never cutting a surrogate pair) and applied to the parsed message (`match` nodes).
- `web/src/components/search-results.ts`: channel runs and facet → query.
- `web/src/components/message-search.svelte.ts`: the search state (debounce, one request in flight, pages, statuses).
- `web/src/components/MessageSearchField.svelte`, `MessageSearchPanel.svelte`, `QuickSwitcherTrigger.svelte` ("Go to…" or its compass); `Shell.svelte` docks or overlays the sheet and owns `Ctrl+F`; `ChatPane.svelte` and `FriendsPane.svelte` render the field or magnifier from a snippet; `MessageList.revealMessage` lights the opened message; `MellowClient.loadHistoryUntilMessage`.
- Tests: `search-query.test.ts`, `search-highlights.test.ts`, `message-search.test.ts`, `client.test.ts` ("Reaching a message far back").

## 7. Not done, and open questions

- **Filter names in French.** The server understands `from:`, `in:`, `has:`… only; Discord in French shows « de: », « dans: ». The French interface describes each English key. Accepting French keys would be a server change (both spellings parsed).
- **Matches inside code blocks and mention chips** are not lit (`shortcut:` comments in `search-highlights.ts`, the mention case covered by a test): a chip stays whole. Text, formatting, links, lists and every line of a quote are lit.
- **Performance at the limits**, left for later and to measure first: the sheet renders every loaded result (up to 1 000 slips, each parsed as Markdown), and opening a result up to 1 000 messages back renders all of them in the chat list, which is not virtualised.
- **The pins panel** could use `loadHistoryUntilMessage` to jump to pins further back; it still offers Jump only for loaded messages.
- **Recent searches** are not kept.
- **A date picker**: days come from suggestions or are typed as `YYYY-MM-DD`.
- Not checked on real devices: iOS and Android keyboards over the full-window sheet, screen readers (VoiceOver, NVDA) on the combobox, text at 200 %.
