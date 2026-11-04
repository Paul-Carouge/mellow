# Long code blocks and "Copier le code" on the web (ROADMAP item 18)

Status: designed and built on `feat/web-collapse-long-code-blocks`, DESIGN.md v0.48. Web client only, no server change: `MessageCodeBlock.svelte` (rendered by `MessageMarkdown.svelte` for every `codeBlock` node), the rules in `code-blocks.ts` (line count, the fold threshold, the first lines), tokens `component.message.code-*` in `design/tokens.json`.

## Brief

Paul pastes a 42-line Rust patch in #général of "Atelier" and asks "Dites-moi si ça compile.". Today the patch takes the whole list: Ana, who was reading the conversation, has to scroll past 760 px of code to reach the question, and on a phone the message after it is three screens away. When she does want the code, she has to select 42 lines by hand, because a drag across a block picks up the line numbers of nothing and the text around it. The job: **keep a long paste from taking over the conversation, and make the code itself one click to take away.**

Assumption: messages are at most 4 000 characters, so the longest block is a few hundred short lines; nothing here needs virtualisation.

## Flow and states

1. **Short block (up to 20 lines).** Unchanged, except "Copier le code" at its trailing top.
2. **Long block (21 lines and more), folded.** The first 12 lines, the last two fading into the well, then a strip across the block's foot: caret down, "Afficher tout (42 lignes)". Only those 12 lines are in the page.
3. **Opened.** Every line, the strip now "Réduire" with a caret up; the block's first line has not moved, so reading goes on below line 12. When the list was at the latest message, it stops following it (the block grew under the reader) and "Tu consultes des messages plus anciens · Revenir au présent" shows, as after any scroll up.
4. **Folded again.** The strip stays under the pointer; what was above it scrolls down. A list at the latest message stays there.
5. **Copied.** The key reads "Copié" with a check for 2 s (`motion.delay.copied-label`, kept under reduced motion: it is a display time, not an animation) and "Code copié" is announced. The whole block is copied, folded or not, raw: no fade, no strip label.
6. **Copy refused** (clipboard denied, insecure context): "Copie impossible" with a warning glyph for 2 s; "Impossible de copier le code. Sélectionne-le et copie-le à la main." is announced. The text stays selectable, and opening the block shows it all to select.
7. **Gateway updates.** A reaction, an edit elsewhere, a role rename re-parse the message; the block keeps its place in the message's keyed list, so it stays open. Editing the message itself starts it folded again; so does a reload.
8. **Very long single lines.** They scroll inside the block, never wrap the layout or widen the message; the fade covers every column the block scrolls to.
9. **Where.** Wherever message bodies render in full: the message list (forwards included) and the search results, where the strip and the key are buttons, so a click on them does not open the result. Previews (reply lines, the pins panel, notifications, the delete, pin and forward dialogs) are text summaries and never show code.

```
Desktop, folded                                      hover or focus: the key shows
┌─────────────────────────────────────────────────────────────┬─────┐
│ fn main() {                                                 │ ⧉   │ 24 px key, the sheet's colour
│     println!("ligne 2 : on prépare le live");               └─────┘
│     …                                                             │
│     println!("ligne 11 : on prépare le live");  ░ fading          │
│     println!("ligne 12 : on prépare le live");  ░░ fading         │
├───────────────────────────────────────────────────────────────────┤
│ ▾ Afficher tout (42 lignes)                                       │ the whole strip is the button
└───────────────────────────────────────────────────────────────────┘

Phone (320 px, coarse pointer)
┌──────────────────────────┬──────┐
│ const url = "https://    │  ⧉   │ 44 px, always shown;
│ console.log(0);          └──────┘ long lines scroll under it
│ …                               │
├─────────────────────────────────┤
│ ▾ Afficher tout (25 lignes)     │ 44 px high
└─────────────────────────────────┘
```

## Design plan

- **Palette**: no new colour. The key is a piece of the sheet raised over the well (`color.surface.chat`, 1 px `border.divider`), the same key as the message action bar, so "a control that appears on hover" looks the same everywhere in the list. The glyph and the strip's words are `text.secondary` at rest and `text.normal` on hover; the strip takes `state.hover` on hover. Contrast pairs added and checked by `cargo xtask tokens`: `code-control-text` on the well and on the key (dark: `#C5BEC6` on `#1A141A` and `#221B23`; light: `#594E5B` on `#F8F4F8` and `#FFFFFF`), `code-control-text-hover` on `state.hover`, all at 4.5:1 or more.
- **Type**: the code keeps `type.code`; the strip is `type.body-sm` semibold with tabular figures, quieter than the message text so it reads as the block's own control, not as a line of the conversation; "Copié" uses the same.
- **Shape**: the block keeps `radius.sm` and its 1 px `surface.rail` border; the strip is cut from the block by the same 1 px line and takes its bottom corners; the key is `radius.xs`, 24 px, inset 4 px, which fits the shortest block (8 + 18 + 8 px) without growing it. Under a coarse pointer: key 44 px, strip 44 px, and a one-line block grows to hold the key.
- **Motion**: none for opening or folding (a growing block in a scrolling list is already movement enough, and an animated height would fight the list's anchoring); the key fades in on hover in `motion.duration.fast`, zero under reduced motion.
- **Layout**: thresholds in `code-blocks.ts`: a block shows whole up to 20 lines (`MAXIMUM_CODE_BLOCK_LINES_SHOWN_WHOLE`: at 18 px a line, under 380 px, half a laptop's list), folds to 12 (`FOLDED_CODE_BLOCK_VISIBLE_LINES`: a function's signature and first statements), so a fold always hides at least 9 lines, never a lone one. The fade is two lines (`code-fade-height`, 36 px). The lines keep room at their end for the key, so it covers nothing that fits.

## Defaults rejected

- **A fixed `max-height` with `overflow: hidden`** (the usual "show more"): the horizontal scrollbar would sit under the fold and the hidden lines would still be in the page for every message. Mellow renders only the shown lines and masks their last two.
- **A gradient from transparent to the block's colour** for the fade: it needs a colour per theme and per row tint (a mentioned or highlighted row). A mask reads only alpha, so it fades into whatever is behind.
- **A header bar with the language and a copy button on every block** (GitHub's): it adds a row to every one-line snippet people paste in chat. The language stays kept and unshown, as DESIGN.md already says; the key appears where the hand is.
- **"Afficher plus" as a small link under the block**: a 12 px target outside the block that reads as part of the message. The strip is the block's own foot, as wide as the block, and says how much is hidden.
- **Keeping the list pinned to the latest message while a block opens**: the list would push the opened code up and show its end, losing line 13 from under the reader. The block's top stays; the jump bar is there to go back.
- **Folding at 15 lines, Discord-like, with 15 shown**: a 16-line block would hide a single line behind a button.

## Accessibility

- Both controls are real buttons. The strip carries `aria-expanded` and `aria-controls` (the `<pre>`), and its name says how many lines open ("Afficher tout (42 lignes)"). The key's name is "Copier le code" (visually hidden text, and its tooltip), then "Copié" while confirmed; a polite live region in each block announces "Code copié" or the refusal. They follow the message's links and chips in the Tab order (7.2); the key is visible on keyboard focus (`focus-ring`), the strip draws its ring inside the block (`focus-ring-inset`).
- Screen readers read the shown lines only, then the strip, which says more is there; nothing hidden is read as if shown.
- Targets: 24 px with a fine pointer (WCAG 2.5.8), 44 px under a coarse pointer, the strip full width.
- Sizes are in `rem`, so the block, the key and the strip scale with the browser's text size; lines never wrap.
- Checked in headless Chrome (Playwright) against a local instance (42-line Rust block, 25-line block with a 200-character first line, short blocks): French dark and English light at 1280 × 800, French light at 320 × 800 with touch; opening the last block at the latest message keeps its top at the same pixel (395.8 → 395.8) and stops following the latest message; folding again keeps the strip under the pointer and returns to the latest message; "Copier le code" puts the raw text on the clipboard, shows "Copié" and fills the live region; Enter on the focused strip opens it, the focus ring shows; in the search results the strip opens and folds without opening the result.

## Not done

- Syntax highlighting (out of scope; the language is kept by the parser for it).
- The open state does not survive a reload or an edit of the message, by choice.
- Not checked: a real screen reader, Firefox and Safari (mask support is universal since 2023), real iOS and Android touch.
