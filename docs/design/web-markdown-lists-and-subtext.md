# Web client: numbered lists, subtext and nested lists

Status: done, 2026-11-05, design pass WP5 (finding C1 in `web-design-pass-2026-11.md`). The "/" menu already offered "Liste numérotée", which wrote `1. ` that messages then showed as plain text; Discord's subtext `-#` and nested lists were missing too (DESIGN.md 4.5 listed them as "Not yet").

## Parsing (`web/src/components/markdown.ts`)

Still a typed tree, never HTML, and still linear on hostile text (three new cases in the "hostile text" tests).

- **Numbered list**: a line starting with one to nine digits, `.` or `)`, a space and text. The list keeps the first item's number (`start`); later numbers are ignored, as in CommonMark and on Discord, so `1.` `1.` `1.` counts 1, 2, 3. "1.5 kg", a ten-digit number and a bare "2. " stay text.
- **Bullets** and numbers do not mix: switching from `-` to `1.` at the same level starts a new list. `-` and `*` still share one list, as before.
- **Nesting**: an item indented by two spaces or a tab (or more) nests under the item above, one level only; deeper indentation flattens to that level. One space before the marker ends the list, and an indented item with no item above stays text, so old messages render as before.
- **Subtext**: `-# ` then text, at line start, with inline formatting, mentions and links inside; also inside quotes.
- The tree changed shape: `MarkdownList { start: number | null; items: { children, sublists }[] }` and a `subtext` block. Notification previews, search highlights and mention detection read the new shapes (tests for each).

## Rendering (`MessageMarkdown.svelte`)

| Element | Tokens and rule |
|---|---|
| Bullet list | `space.md` inline start, `disc` outside, as before |
| Nested bullets | `circle`: a level down reads as a level down without another colour or size |
| Numbered list | `<ol start>`, `decimal` outside, markers in tabular figures; inline start is `space.md` plus one `ch` per digit past the first of the **last** number, so the marker ends where the paragraph text starts and "10." sits flush with the text column while "9." aligns on its dot |
| Subtext | `type.message-subtext` (12/16), `text.muted`; "(modifié)" follows its last word when it ends the message, as on a paragraph |

Rejected defaults: `list-style-position: inside` (wrapped lines would run under the number), a fixed wider margin for every numbered list (one-digit lists would sit visibly deeper than bullets), a different colour or weight for markers (numbers are content, they keep the text's colour), and a slash command for subtext (Discord has none; out of scope).

Contrast: `text.muted` is already a declared pair on `surface.chat`, `state.hover-message`, `mention.background` and `component.search.result-background` in both themes (`cargo xtask tokens --check`), which covers subtext on a plain, hovered, mentioned and search-result row. No new token.

## Checked

Headless Chrome against a throwaway instance, French, before (origin/main) and after: 1440 dark, 1440 light, 390 dark touch, 320 light touch. Not checked: a screen reader (native `ol`/`ul` semantics, `start` read as the first number), Firefox and Safari marker metrics, text zoomed to 200 % (margins are in `rem` and `ch`, so they scale).
