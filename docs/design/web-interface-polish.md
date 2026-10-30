# Interface polish on the web (ROADMAP item 12)

Status: built on `feat/web-interface-polish` (2026-10-30), DESIGN.md v0.26. Web only; no server or protocol change.

The owner asked for three things, in their words: the "Go to…" bar as a search with a nice magnifier, "but I love the current style and colours"; nicer permission badges; a better emoji picker. Each is a refinement of something that already works, so the rule for this pass was to keep every existing surface, colour and behaviour, and to spend the change where the eye or the keyboard actually stumbles.

## 1. The quick switcher reads as a search field

**Header, wide windows.** The "Go to… Ctrl K" button keeps its `surface.input` fill, `radius.sm`, `type.body` in `text.secondary` (heading on hover) and its `kbd`. What changes is its shape and order, so it reads as a field:

```
before   [ Go to…  Ctrl K ]
after    [ (o) Go to…                 Ctrl K ]
           |   placeholder tone          |
           magnifier, text.muted         shortcut pinned to the end
```

- A leading magnifier (`component.quick-switcher.trigger-icon-size`, 16 px, `trigger-icon` = `text.muted`, `text.secondary` on hover).
- A resting width of `component.quick-switcher.trigger-width` (208 px), the words pushed to the start and the shortcut to the end, so it has the proportions of a field. It gives way before the topic does, down to its content (`flex: 0 1 208px; min-width: max-content`).
- It stays a `button` with its accessible name ("Go to a channel, server or person") and `aria-keyshortcuts="Control+K"`: the field itself is in the switcher, so typing starts there.

**Header, phones.** Phones had no way into the switcher without a keyboard. A magnifier `IconButton` with the same name now sits before the header's other buttons. Under a coarse pointer the header's buttons are 44 px targets, so the gaps between them go (`pointer-coarse:gap-0`): at 320 px the channel name stays visible ("gene…") instead of vanishing.

**The switcher.** Its large field gets the same magnifier at its type size (`input-icon-size`, 24 px, `text.muted`) before the text, with `input-gap` (12 px). The focus ring belongs to the box, so it surrounds the icon too. Same dialog, colours and results as before.

**The magnifier.** Search uses Phosphor's **Bold** magnifier, an open lens, everywhere on the web (header, switcher, emoji picker, the overwrite dialog). The Fill magnifier's solid lens reads as a target at 16 px; the icon README lists it with the other Bold exceptions.

**Search.** The switcher now ignores accents both ways ("general" finds « général », « ZOÉ » finds Zoé), through `lib/search-text.ts`, which the emoji catalogue now shares.

## 2. Permission badges and role chips

Three places show a permission or a role as a small label; each now has one clear treatment.

| Where | Before | After |
|---|---|---|
| Role settings → Permissions, "Sensitive" (Administrator, Ban members, Manage roles…) | A warning circle and loose red words | A **tag**: `shield-warning` (Fill, 14 px) and the word in `type.category`, `text.danger` on `color.danger.wash`, 22 px tall, `radius.xs` (`component.permission-badge.*`) |
| Server settings → Members, role chips | Neutral chip with a colour dot | The chip takes a quiet wash of its own role colour: `color-mix(in oklab, role 18 %, chip-background)` (`chip-role-tint`), dot kept; "Owner" gets a crown (`chip-owner-icon`, `text.secondary`) instead of no mark |
| Channel settings → Permissions, Deny / Inherit / Allow | The chosen segment on `state.selected`, only its glyph coloured | A chosen **Deny** on `color.danger.wash` in `text.danger`; a chosen **Allow** on `color.positive.wash` in `text.positive`; **Inherit** unchanged. The word and glyph still say it without colour |

New colour tokens: `ref.red.wash-dark/-light`, `ref.green.wash-dark/-light` (flattened tints, like the apricot washes), and the semantic `color.danger.wash` and `color.positive.wash`. Contrast pairs added and checked by `cargo xtask tokens`: `text.danger` on `danger.wash` (5.17 dark, 4.97 light), `text.positive` on `positive.wash` (6.16 dark, 4.58 light), `text.heading` on `danger.wash`. The role tint was checked by hand at its extremes (white and black role colours, both themes): `text.heading` stays at 6.5:1 or more. The unused `channel-settings.override-*-icon` tokens are gone.

## 3. The emoji picker

```
wide (352 x 400 popover)                    phone (bottom sheet, coarse pointer)
+--------------------------------------+    +------------------------------+
| (o) Find the perfect emoji           |    | (o) Trouve l'emoji parfait   |
| [clk] smi paw bwl bal pln bul hrt flg|    | [clk] smi paw bwl bal pln bu>|  <- scrolls sideways
| Recently used                        |    | Utilisés récemment           |
| 🥳 👍                                |    | 🥳                           |
| People                               |    | Personnes                    |
| 😀 😃 😄 😁 😆 😅 🤣 😂               |    | 😀  😃  😄  😁  😆  😅      |  <- 44 px cells, 28 px emoji
+--------------------------------------+    +------------------------------+
```

- **Category bar** under the search, hidden while searching: Recently used (when there is one), People, Nature, Food & drink, Activities, Travel & places, Objects, Symbols, Flags, as Phosphor Fill glyphs (`clock`, `smiley`, `paw-print`, `bowl-food`, `soccer-ball`, `airplane-tilt`, `lightbulb`, `heart`, `flag`): never emoji, so the bar reads as controls and not as more emoji to pick. Each button has the category's name as label and tooltip. The current category sits on `state.selected` in `text.heading` (`component.emoji-picker.category-*`, 32 px, 44 px under a coarse pointer).
- **Following the list.** The current category is the last whose heading has reached the top of the list, or the last of all once the list is scrolled to its end (Flags is too short to reach the top). A click scrolls to the category (smoothly; at once under reduced motion) and moves the emoji's Tab stop to its first emoji, so Tab lands there.
- **Keyboard.** Tab order: search, the bar (one Tab stop, Left and Right wrap, Home, End), the emoji (one Tab stop). In the grid, Up and Down keep the column across categories and short last rows; Up from the first row returns to the search; Home and End go to the row's ends, with Ctrl to the first and last emoji; Page Up and Page Down jump to the previous and next category, its heading going to the top. A focused emoji is never left under the sticky heading. The rules are pure functions in `web/src/components/emoji-grid.ts`, tested.
- **Phones.** Under a coarse pointer the cells are 44 px and the emoji grow to 28 px (`emoji-size-coarse`); the bar keeps 44 px targets and scrolls sideways, the current category kept in view.
- **Search field.** Its magnifier now leads it, as in the switcher.
- **Removed:** the clock before "Recently used": the bar names it now.
- **Speed.** About 320 emoji, all rendered, as before. Opening the picker measured 26 ms before and 25 ms after (median of 10 opens in headless Chrome, from the click to two frames later, so two frames are included); a search keystroke 34 ms both times. No virtualisation needed.

## Copy

| Situation | English | Français |
|---|---|---|
| Emoji picker help (screen readers) | "Arrows to move, Page Up and Page Down to change category, Enter to choose, Esc to close." | « Flèches pour te déplacer, Page précédente et Page suivante pour changer de catégorie, Entrée pour choisir, Échap pour fermer. » |
| Emoji category bar | "Categories" | « Catégories » |

The magnifier button on phones reuses "Go to a channel, server or person" (« Aller à un salon, un serveur ou une personne »).

## Defaults rejected

- A pill-shaped "⌘K" search box in a new colour, the generic SaaS header search: the owner asked to keep today's style and colours, so only the shape, the icon and the order changed.
- Saturated filled badges per permission, or "SENSITIVE" in capitals: loud, and capitals are a template tell. A quiet wash and a shield say "careful" without shouting.
- Emoji as the category icons (Discord's way): they compete with the emoji to pick; the rest of Mellow speaks Phosphor.
- A preview footer naming the hovered emoji: the catalogue's search words are not names ("red" for ❤️), so the footer would read badly.
- Skin tones: they need modifier data per emoji, a stored choice and care in reactions' keys; not cheap, so not now.

## Checked and not checked

Checked with headless Chrome against a local instance built from the branch, in English dark and French light, at 1280 × 860 and 320 × 640 (with touch emulation for the 44 px sizes): the header field, the switcher, the phone magnifier, the picker's bar, scrolling, keyboard moves and Esc, the Sensitive tag, role chips and the override states. Not checked: real iOS and Android touch, a screen reader, text at 200 %, Firefox and Safari (`color-mix` needs Safari 16.2 and Firefox 113).
