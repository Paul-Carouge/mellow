# Desktop: message rendering

The GPUI desktop client's message components (`crates/mellow-desktop/src/messages`, ADR-0051), ported from the web client's `MessageItem.svelte`, `MessageMarkdown.svelte`, `MessageCodeBlock.svelte`, `MessageReactions.svelte`, the pictures of `MessageAttachments.svelte`, `reply-references.ts` and the dividers of `MessageList.svelte`. DESIGN.md §5.4 stays the specification; this file records how GPUI maps it, so the message list and later work keep it. No token was added: every colour, size and duration comes from `design/tokens.json`.

## Structure

```
 gutter 72                        padding-trailing
 ┌────┬────────────────────────────────────────┐
 │    │ ╭─ reply spine ─ (A) Ana  quoted text…  │  reply line (component.message.reply-height)
 │ (L)│ Léo  Today at 05:58 PM  📌              │  header: author (role colour), time, pin
 │    │ @Paul can you check… ▓▓▓▓▓▓ (edited)    │  body: one GPUI text per paragraph
 │    │ [👍 3] [:party: 1]                      │  reaction pills
 └────┴────────────────────────────────────────┘
 continued rows: no avatar or header; the time of day shows in the gutter while hovered.
```

- A row is `MessageItem` with a `MessageItemContent` the list builds from client-core's `build_message_rows` row and a `MessageBodyCache`. The cache parses each version of a message once (keyed by id, `edited_at` and whether `@everyone` was kept; the outbox by nonce) and words each quoted original once; the list bumps its sources generation when people, emoji, roles or the language change. A virtualised list therefore builds a row from cached values only.
- Dividers are separate elements the list places above a row: `DayDivider` (with the apricot line and "New" pill when new messages start that day) and `UnreadDivider`.
- The gallery (`MELLOW_COMPONENT_GALLERY=1`) shows a sample conversation laid out by `build_message_rows`, with working reaction pills, Retry and Delete. `cargo run -p mellow-desktop --example gallery_picture -- out.png dark|light` draws the gallery without a screen, for screenshots.

## How GPUI draws message text

- A paragraph, heading, subtext or list item is one `StyledText` with highlight runs, so it wraps as one paragraph. Bold, italic, underline, strikethrough, link colour, spoiler and search-match washes and mention chips are runs; inline code and server emoji switch to the monospace family by range.
- Links and spoilers are click ranges (`InteractiveText`). A link opens in the OS browser only when `safe_http_url` accepts it (http and https, no user info), checked again at click; a masked link's tooltip shows its real address. A hidden spoiler draws its text in the bar's colour, keeps its links inactive and reveals on click; revealed spoilers and opened code blocks are kept per row while it is drawn.
- GPUI text holds no inline boxes, so a server emoji keeps room with monospace no-break spaces (Source Code Pro advances 0.6 em, so the count covers the emoji's side) and its picture is painted over that room after the text; its tooltip is `:name:`. A picture that cannot load shows the missing-emoji outline.
- Role mention chips use client-core's `role_mention_chip_colours` on `color.surface.chat`, memoised per colour and theme; people and `@everyone` use the amber chip.

## Known differences from the web

- Mention chips have no radius or padding (text runs have neither): a thin space each side stands in for the padding, and chips do not change colour on hover.
- Inline code, "(edited)" and server emoji in text use the message font size (a text has one size): the web sets code at 14 px and "(edited)" in the timestamp type.
- Server emoji painted in text show their still frame; reactions and pictures animate unless motion is reduced.
- Message text is not selectable yet: gpui-base's selection layer works on plain text only, and a window-wide selection layer belongs to the shell.
- Links and spoilers inside text are reached by pointer only; the keyboard reaches the row's buttons (reactions, code block keys, Retry, Delete). The list's roving row focus should give rows a way to open their links.
- In the headless picture (`gallery_picture`), bold and semibold text draw at the regular weight in every primitive, not only messages, possibly because the embedded `Figtree[wght].ttf` is a variable font whose weight axis is not picked. To check in a real window.
- Not drawn yet: the hover bar and message menu, the inline editor, forwards, polls, checklists, progress bars, link previews, GIFs, videos and sounds, the picture viewer and downloads (files show as cards without a link), and "who reacted" in reaction tooltips (the tooltip names the emoji and count).
