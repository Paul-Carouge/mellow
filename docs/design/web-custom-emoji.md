# Custom emoji on the web (ROADMAP item 13)

Status: built on `feat/web-custom-emoji`, DESIGN.md v0.27. Server, protocol and data layer: ADR-0028.

## Brief

The owner wants Discord's custom emoji: a server imports small pictures, still or animated, and its members use them by typing `:name:`, from the picker and as reactions. Limits come from the instance: 50 still and 50 animated per server, 256 KB per upload, names of 2 to 32 ASCII letters, digits and `_`, unique per server whatever the case. Pictures are members-only (the icon routes), so a client can show a picture only for emoji of the servers it is in.

People: members of small self-hosted communities who already know Discord. The job is the same as there: drop a server's in-jokes into messages without thinking about it, and let a moderator fill the server's set quickly.

## Flows and states

**Managing (Server settings → Emoji, `MANAGE_EMOJIS`).**

- "Upload emoji" opens the file picker (several files at once, PNG, JPEG, GIF, WebP). Each picture uploads at once, one after another, named after its file (accents dropped, other characters made `_`, `_2` added when the name is taken). Discord does the same: renaming afterwards is cheaper than a form before each upload.
- While it uploads, its row shows the local preview at half opacity, `:name:` and "Adding…"; the others say "Waiting for the others…".
- A picture the client knows is wrong never leaves: not a picture, over 256 KB, or a PNG/JPEG while still emoji are full. The instance's refusals become sentences: 413 too heavy, 409 no room for this kind, 400 a picture it could not use (names are checked before sending), 429 too many pictures in a short time (the budget is shared with avatars and icons). After a 429 the files still waiting fail with the same reason, so nothing hammers the instance; each failed row has "Try again" (named again, in case the name was taken meanwhile) and a dismiss button.
- The list is split in two, "Still emoji" and "Animated emoji", each heading carrying its own count ("12 / 50", read "12 of 50 used"): the quota is per kind, so the count sits where the kind is. A full kind turns its count `text.danger` and adds "· Full" (never colour alone). Both full: "Upload emoji" is dimmed (still focusable) and a sentence says why.
- Each row: the picture on a `surface.field` tile (transparent emoji keep one shape), the name as an in-place field (edge only on hover, focus ring on focus), and Delete (trash, 44 px). Enter or leaving the field saves; Esc puts the name back (a second Esc closes the settings). Invalid or taken names are refused before asking, with the reason under the field and `aria-invalid`. The rename shows when the gateway's list arrives; the field keeps the typed name meanwhile.
- Delete asks "Delete :party:?" and says what happens: messages show the name instead; reactions stay and can still be taken back. Focus moves to the next row's name, else the upload button.
- Empty: "No emoji yet. Upload a picture and everyone here can use it as :name:." A kind with none: "No still emoji yet." / "No animated emoji yet. A GIF or WebP that moves becomes one."
- Loading: the list comes with the server (`Guild.emojis`, kept live by `GUILD_EMOJIS_UPDATE`), so there is no list to wait for; the waiting states are per upload.

**Writing.**

- Typing `:` then two name characters at the start of the text or after a space opens the suggestions above the composer: the open server's emoji (names starting with the query first, then containing it), then built-in emoji by their search words, ten at most. Up and Down move (wrapping), Enter or Tab inserts, Esc hides them until the word changes, a click inserts without taking focus. The field points at the active option (`aria-activedescendant`, `aria-autocomplete="list"`); a polite live region says how many there are.
- Choosing writes `:name:` and a space, not the markup: the field stays readable, and on send every `:name:` of the open server's emoji (whatever the case, outside code, not after a backslash) becomes `<:name:id>` / `<a:name:id>`. The length counter counts the converted text, which is what the instance limits. Edits are converted the same way when saved.
- The composer gains an emoji button (smiley, trailing, before the phone send button) and `Ctrl+E` / `Cmd+E`, both opening the picker; a pick goes in at the caret.

**Picker.** After "Recently used", a section titled with the server's name holds its emoji; its button in the category bar is the server's first emoji. Search finds them by name before the built-in set. Cells name them `:name:` for screen readers and in a tooltip.

**Rule for other servers' emoji: hidden.** The picker and the suggestions offer only the open server's emoji, and none in conversations. Reactions with a custom emoji are only allowed in channels of its own server, and an emoji written elsewhere would show as `:name:` to anyone outside that server. One rule for both, nothing offered that the instance or the readers would refuse.

**Reading.**

- `<:name:id>` and `<a:name:id>` show the picture (22 px in a line, `alt` and tooltip `:name:` with the name its server gives it, never the name the markup claims) when the reader is in its server; otherwise the text `:name:`. A picture that fails to load falls back to the same text. Previews (reply lines, pinned panel, dialogs) show `:name:`.
- A message of 1 to 30 emoji (custom or built-in) and spaces, in one paragraph, shows them at 48 px, as on Discord.
- Animated emoji play, except under reduced motion, where the still frame is served (`/still`). Hidden windows cost nothing: browsers do not paint or advance images in a hidden page, so no visibility listener was added.

**Reactions.** A custom reaction (key `:id`) shows the emoji's picture under its current name; chips, tooltips ("Ana reacted with :party:") and the Reactions dialog's tabs all use the same appearance. A reaction whose emoji was deleted keeps a chip with a dashed outline where the picture was, named "a deleted emoji"; it can be joined and taken back, never started anew (the picker no longer offers it).

## Tokens

New `component.custom-emoji.*`: `inline-size` 22 px, `jumbo-size` 48 px, `missing-border` (`text.muted`), `missing-border-width` 1 px, `missing-radius` (`radius.xs`), `suggestion-width` 320 px, `suggestion-max-height` 296 px, `settings-picture-size` 40 px, `settings-picture-background` (`surface.field`), `settings-picture-radius` (`radius.sm`), `settings-picture-padding` (`space.2xs`), `settings-name-max-width` 320 px, `settings-name-border-hover` (`border.strong`), `settings-count-full-text` (`text.danger`). The suggestions reuse the context menu's surface, rows and focus colours, and the picker's emoji size.

## Defaults rejected

- **A staging form before each upload** (name field, then "Add"): one more step for the common case. Upload at once, rename in place.
- **Inserting the raw markup in the composer**: `<:party_parrot:1234567890123456789>` in a textarea is noise; `:name:` reads as what people typed, and the send path converts it.
- **Showing other servers' emoji greyed out in the picker**: more to scan, and a choice that only leads to a refusal or a `:name:` for others.
- **A single "12 / 50 still · 3 / 50 animated" line at the top**: the counts belong to the two lists they limit.
- **A spinner while emoji "load"**: they arrive with the server; only uploads wait, and they say so in their row.

## Components

- `CustomEmojiImage.svelte`: the picture, still under reduced motion, `:name:` on failure.
- `ReactionEmoji.svelte`: a chip's emoji (built-in, picture, or the deleted outline).
- `EmojiSuggestions.svelte`: the composer's listbox.
- `EmojiSettings.svelte`: the settings page.
- Logic in `custom-emoji.ts` (tested): suggestion query and ranking, `:name:` conversion, plain text, jumbo detection, reaction appearance, upload naming and checks, failure sentences, rename checks; `markdown.ts` parses the markup into a `customEmoji` node.

## Not done, to validate

- Recently used and quick reactions keep built-in emoji only.
- The message editor shows the markup of emoji already sent (only new `:name:` are converted on save).
- Unicode emoji have no `:shortcode:` of their own; the suggestions show their first English search word.
- Check with a screen reader (VoiceOver, NVDA) that the composer's active option is read on Up and Down.
