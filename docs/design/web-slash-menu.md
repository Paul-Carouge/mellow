# The "/" menu and formatting shortcuts on the web (ROADMAP item 18)

Status: designed and built on `feat/web-slash-menu`, DESIGN.md v0.50. Web client: rules in `web/src/components/slash-commands.ts` (finding the "/co" being typed, ranking the commands with English aliases, writing each command, toggling marks on a selection, the one changed range written for undo), the rows and keys in `Composer.svelte`, the wider list in `ComposerSuggestions.svelte`, words in `i18n/en.ts` and `i18n/fr.ts` (`chat.slashCommands`). No server change.

## Brief

Léa writes in "Atelier" and knows Discord, not Markdown. She wants a title over tonight's plan, a list of three songs, a word in bold, a spoiler for the setlist's surprise. Today she has to know that `##`, `-`, `**` and `||` exist. Paul, who knows them, wants `Ctrl+B` on a selected word, as in every word processor.

The ROADMAP asks for "a '/' menu with live formatting while typing (Notion style)". This slice is the menu and the shortcuts. **Live formatting inside the field comes later**: the composer is a `textarea`, which cannot show bold or a heading, and replacing it with a rich editor is its own slice (DESIGN.md section 9).

## Flow and states

1. **Opening.** `/` at the start of the message, or at the start of a line (only spaces before it), opens "Mise en forme et commandes" above the field with all sixteen commands. A `/` anywhere else (after a word, in `et/ou`, in `https://`, in `/usr/bin/`) is text and opens nothing.
2. **Narrowing.** Each letter typed after the `/` narrows the list live: "/co" → Code, Bloc de code; title "Commandes correspondant à /co". Case and accents are ignored ("/souligne"), and English names work in French ("/bold" finds Gras), as do Markdown's words (`h1`, `ul`, `ol`, `blockquote`, `url`). A space, or a word that names nothing ("/usr"), closes the list; Enter then sends the text as written, as on Discord.
3. **Choosing.** Up and Down (wrapping), Enter or Tab, a click or a tap; Esc hides the list until the word changes. Choosing removes the typed "/query" and writes the command:
   - headings, quote, lists: the line starts with `# `, `## `, `### `, `> `, `- ` or `1. ` (the spaces before the `/` go too), caret after it;
   - bold, italic, underline, strikethrough, spoiler, inline code: an empty pair (`**|**`) with the caret inside;
   - code block: ```` ``` ```` on its own line, the caret on an empty line, the closing fence below (the language stays empty: typing it after the opening fence works);
   - link: `[texte](https://)` with "texte" selected, ready to be typed over;
   - `shrug` and `tableflip`: `¯\_(ツ)_/¯` and `(╯°□°)╯︵ ┻━┻` in place of the command (escaped so the shrug's backslash and underscores survive the parser).
4. **Undo.** The command is written through `execCommand("insertText")` on the one range that changed, so `Ctrl+Z` gives back the "/query" in one step (checked in Chrome: two undos after "/co" → Code → "x" return to "/co"). Where the browser refuses, the text is set directly and undo is lost, never the text.
5. **Shortcuts on a selection.** `Ctrl+B` bold, `Ctrl+I` italic, `Ctrl+U` underline, `Ctrl+Shift+X` strikethrough (`Cmd` on macOS). They wrap the selection and keep it selected; pressed again they take the marks off (around the selection or at its ends; a run of stars tells italic from bold). Spaces at the selection's edges stay outside the marks, where Markdown needs them. With no selection they write an empty pair, except `Ctrl+U`, which stays the member list's shortcut (Shell) when nothing is selected.

```
┌ Mise en forme et commandes ────────────────────────────┐   416 px at most, full width on phones
│ H₁  Titre 1  Grand titre de section               [#]  │   active row: context-menu focus wash
│ H₂  Titre 2  Titre de section moyen              [##]  │
│ B   Gras     Mise en avant forte                 [**]  │   [..] = the Markdown, as inline code
│ ⌖   Spoiler  Caché jusqu’à ce qu’on le révèle    [||]  │       shows in messages
│ ‹/› Code     Du code dans une phrase              [`]  │
│ …   (scrolls inside 18.5 rem)                          │
└────────────────────────────────────────────────────────┘
┌ ⊕  /                                              ☺  ┐   the composer, unchanged
```

At 320 px the description gives way first (truncated), then the name and the Markdown chip.

## Design plan

- **Palette**: no new colour. Rows are 6.58's context-menu rows; the description is `text.secondary` (already the mention rows' second word, AA on the popover and on the focus wash in both themes); the Markdown chip is `component.message.code-background` with `text.normal`, the pair inline code already uses in messages (declared contrast pair).
- **Type**: `type-body` for name and description, `type-code` for the chip.
- **Shape**: the 24 px leading box of the mention rows holds an 18 px glyph (`component.context-menu.item-icon-size`); the chip takes `radius.xs` and `space.3xs` padding like inline code.
- **Size**: one new token, `component.composer-suggestions.command-list-width` (416 px): the emoji and mention list's 320 px cut every description after two words.
- **Icons**: fifteen Phosphor 2.1.1 additions in `design/icons`. The formatting glyphs (`text-h-one/two/three`, `text-b`, `text-italic`, `text-underline`, `text-strikethrough`, `code`, `code-block`, `list-bullets`, `list-numbers`, `link-simple`) use **Bold**: their Fill versions cut the glyph out of a solid square and read as toolbar keys. `quotes`, `smiley-meh` (shrug) and `smiley-angry` (table flip) use Fill; the spoiler reuses `eye-slash`.
- **Motion**: none added. The list appears and changes with the text, like the `@` and `:` lists.

## Defaults rejected

- **Notion's large block menu** (two-line rows with previews, sections "Basic blocks", "Inline"): Mellow's composer already has one list model for `@` and `:`; a third look for the same gesture would be three things to learn. The menu is the same list, one line per command.
- **A hidden format** (the menu applies bold that the field then shows as bold): the field is plain text, and the message is Markdown. Showing the Markdown each command writes, in the same chip inline code gets in messages, turns the menu into a teacher: after a few uses Léa types `**` herself. This chip is the menu's one distinctive element.
- **Discord's slash commands** (bots, arguments, a different list): Mellow has no bots yet; the two fun commands Discord users expect (`/shrug`, `/tableflip`) are kept, without arguments.
- **"Petit texte" (`-# `)**: `markdown.ts` does not render Discord's subtext, so the command would write text that shows as typed. It comes with the parser.
- **Opening the menu on any `/`**: paths, URLs and "et/ou" would open it all day. Only a `/` that starts a line opens it, as in Notion.
- **`Ctrl+U` always underlining**: it is already the member list's shortcut, as on Discord. It underlines only a selection, where toggling the member list would make no sense.

## Accessibility

- The list keeps the composer's pattern: the field owns the listbox (`aria-controls`, `aria-activedescendant`, `aria-autocomplete="list"`, no `role="combobox"` on a multi-line field), a polite count ("16 commandes. Haut et Bas pour choisir, Entrée ou Tab pour appliquer." / "16 commands. Up and Down to choose, Enter or Tab to apply."), the glyph `aria-hidden`; each option reads its name, its description and its Markdown.
- Rows are 32 px under a fine pointer and `size.hit.web-min` (44 px) under a coarse one, as 6.58's rows; a click or tap chooses without taking focus from the field.
- Nothing animates, so reduced motion has nothing to stop.
- The shortcuts are announced by the text change itself; selection is kept so a second press can undo.

## Checked

- Unit tests (`slash-commands.test.ts`, 26): query detection (start, after a newline, after spaces; never mid-line, mid-word, in URLs or paths), ranking with aliases and accents, every command's text and selection, the shrug and a code block read back through the message parser, shortcut mapping, mark toggling (stars run), and the changed range.
- Headless Chrome (Playwright) against a local instance, Vite on `localhost:5173`: French dark and light at 1280 px, English dark at 1280 px, French dark and English light at 320 px; "/co" → Code then undo, "/bold" in French with Tab, "/lien" with "texte" selected and typed over, "/bloc", Esc, "Salut /gras" and "https://exemple.fr/gras" open nothing, `Ctrl+B` twice, `Ctrl+U` on a selection (underlines, member list untouched) and without one (member list toggles), `Ctrl+Shift+X`, a click on "Citation" keeps focus in the field, `/shrug` sent shows `¯\_(ツ)_/¯`.

## Not done

- Live formatting inside the field (Notion style), the ROADMAP's second half: needs a rich editor in place of the `textarea`.
- The message editor (edit mode) has no menu and no shortcuts yet.
- Numbered lists are written (`1. `) but `markdown.ts` shows them as typed text, not as an ordered list; subtext (`-# `) is not offered until the parser renders it.
- Firefox and Safari untried (Firefox supports `execCommand("insertText")` in text fields since version 89; Safari does).
