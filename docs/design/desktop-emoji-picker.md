# Desktop: the emoji picker and reactions

M3 step 5 for the GPUI client (ADR-0051). The desktop ports the web's emoji picker (DESIGN.md
§6.52, with the category bar of [web-interface-polish.md](web-interface-polish.md) and the
server's own emoji of [web-custom-emoji.md](web-custom-emoji.md)) and the reaction entry points
left out since step 3. Nothing new is designed here: the web's sections, keys, words and tokens
(`component.emoji-picker.*`, `component.reaction.*`, `component.action-bar.*`).

## Rules and storage

- client-core's `shared_logic::emoji_picker` holds the UI-independent rules with the web's tests:
  the sections (`picker_sections`: results with server emoji first, else recent, the open
  server's emoji and the eight categories, never an empty one), the grid keys
  (`next_emoji_cell`, the web's `emoji-grid.ts`) and the recent emoji and quick reactions
  (`remember_emoji`, `recent_emoji_from`, `quick_reactions`). `display_emoji` now puts back the
  selectors of every catalogue emoji, as the web's does.
- Recently used emoji are the account's, latest first, at most 16, in the preferences file
  (`recent-emoji=<user id>:<JSON list>`, four accounts at most), read back only as emoji the
  picker knows. Built-in emoji picked in the picker, a quick reaction or the composer's `:`
  suggestions are remembered; server emoji are not, as on the web.
- Which items the message menu shows stays client-core's `message_menu_actions`: with
  `ADD_REACTIONS` (and the history readable), the three quick reactions as items, then Add
  reaction; existing reactions are still joined and taken back from their pills.

## Where it opens

```
 composer: the smiley at the base row's end, or Ctrl+E (Cmd+E)    hover bar (ADD_REACTIONS)
                                   ╭────────────────────────────╮  ╭──────────────────────────╮
                                   │ ⌕ Find the perfect emoji   │  │ 👍 ❤️ 😂 ☺ │ ↩ ⧉ ⋯ │
                                   │ ◷ ☺ 🐾 🍲 ⚽ ✈ 💡 ♥ ⚑       │  ╰──────────────────────────╯
                                   │ RECENTLY USED              │  reactions:  [👍 3] [🔥 1] [☺]
                                   │ 🔥 👍                       │  (the add pill shows with the
                                   │ PEOPLE                     │   hover bar, keeping its room)
                                   │ 😀 😃 😄 😁 😆 😅 🤣 😂    │
                                   ╰────────────────────────────╯
 right edges lined up with the button, below it when the window has room, else above
```

- The picker is a reusable entity (`crate::emoji_picker::EmojiPicker`): the host gives it an
  anchor, the recent emoji and the open server's emoji (none in a conversation), and decides what
  a pick does and where focus goes. A pick for the composer writes the emoji (or `:name:`) and a
  space at the caret and focuses the field; a pick for a message reacts and gives focus back to
  the button that opened it. Esc gives focus back too; a click outside leaves it where the click
  put it, and a second click on the same button closes the picker instead of reopening it.
- While the picker reacts to a message, that message's hover bar and add pill stay drawn.
- Three Tab stops inside, kept inside until Esc: the search, the category bar (arrows, Home, End,
  Enter or Space) and the emoji (arrows across sections keeping the column, Home and End,
  Ctrl+Home and Ctrl+End, Page Up and Page Down, Enter or Space); Down from the search enters the
  emoji, Up from their first row goes back, Enter in the search picks the first result. The bar
  and the grid each keep one focus and point screen readers at their active item.
- The rows are a GPUI virtual list, so opening it draws only what shows. It drops in over the
  fast duration; under reduced motion it appears at once, and animated server emoji hold still.

## Left out

- Sticky section headings (GPUI's list has none) and the phone sheet form.
- Skin tones: the web has none.
