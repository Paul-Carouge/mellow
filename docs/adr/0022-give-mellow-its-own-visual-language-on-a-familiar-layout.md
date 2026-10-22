# 0022. Give Mellow its own visual language on a familiar layout

- Status: Accepted
- Date: 2026-10-22 (revised the same day after the owner's first review)

Supersedes [ADR-0012](0012-clone-discord-interface-with-own-brand.md). Accepted by the owner on 2026-10-22 after the second review.

## Context
ADR-0012 made Mellow a faithful Discord clone with its own brand: Discord's layout, pane sizes, density, cool-grey palette and interaction patterns, with apricot wherever Discord uses blurple. That removed the switching cost, and it is still the reason a Discord user finds their way around Mellow at once.

On 2026-10-22 the owner asked for more: a direction "better than Discord: cleaner, authentic, very beautiful, animated, with excellent UX and UI, able to compete with Discord". A clone can only ever be the second-best Discord. Several of the things ADR-0012 copied are also the things people find tiring in Discord: four flush panes of nearly identical greys, uppercase labels everywhere, borders and shadows to tell regions apart, and motion limited to colour swaps. Meanwhile Mellow already has an identity that Discord cannot copy: Melli, the cloud mascot (DESIGN.md §2.10), the apricot accent and its promise of a place you host yourself.

The first proposal (colours, type, shape and motion on Discord's exact layout) was reviewed by the owner the same day. They liked it, and asked for five more things: a fully French interface, microphone and headset controls in the user panel, a taller user panel aligned with the composer, a layout that no longer reads as Discord, and an app icon on a dark, beautiful background instead of Melli on white.

Constraints that frame the decision:

- Users must not relearn anything: servers, channels, messages, members and conversations stay, with the same names, shortcuts and behaviours.
- Every value stays a token in `design/tokens.json`, consumed by both the Slint desktop client and the Svelte web client, and later by Compose (Material 3 Expressive).
- WCAG 2.2 AA: contrast enforced by the token generator, 44 px targets on touch, keyboard and screen readers, every animation with a reduced-motion variant.
- Motion must be cheap. Melli already costs about 7 % of a core on macOS while visible; nothing else may animate while the app is idle.
- Only the fonts already bundled (Figtree), and no new crate or npm package.
- Whatever layout we choose must be buildable with Slint 1.18 and Svelte 5, and work from phone width to wide desktops.

## Decision
Keep Discord's concepts and interaction model, and give Mellow its own visual language and structure, called **Dusk**: the mellow hour, when the light is low and warm. The conversation is a lit sheet; the navigation is a card beside it; apricot is the low sun; Melli is the cloud.

What stays from ADR-0012:

- The concepts and their names: servers, channels in categories, direct messages under Home, members, the user panel, the quick switcher.
- Message grouping, the hover action bar, context menus, the reply reference, unread pills and red mention badges, the circle-to-squircle server icon, and every keyboard shortcut (`Ctrl+K`, `Alt+Up/Down`, `Ctrl+U`, `F6`, …).
- The brand rules: no blurple or any accent hue between 215° and 255° in OKLCH, no Discord logo, mascot or fonts.

### 1. Layout: the navigator and the side sheet
The window becomes **cards on a dusk ground** instead of four flush panes:

- **The navigator** is one floating card on the leading side holding the server strip (Home, servers, add), the channel list of the current server and, across its whole width at the bottom, the user card. Servers and channels read as one object: "where I am".
- **The sheet** is the conversation, the brightest surface. Its header shows the channel, its topic, a visible **"Aller à… Ctrl K"** button that opens the quick switcher, and the member button.
- **The side sheet** replaces the permanent member column. It opens on demand (member button or `Ctrl+U`, remembered per server) and shows the member list, a member's profile when one is chosen, and later threads and pinned messages. It docks beside the sheet when the window is wide enough and slides over it otherwise.
- Below the width where the navigator and a 480 px sheet fit, the sheet is full-bleed and the navigator opens as a drawer from the leading edge, as today.

Why this one: it changes the silhouette that makes a screenshot read as Discord (a 72 px column of circles, a flat 240 px list and a permanent member column) while every Discord habit still works: one click on a server, one click on a channel, the user panel bottom-left, `Ctrl+U` for members. It gives the conversation more width by default, and it costs no new toolkit feature: rounded clipped rectangles and the overlay pattern already used by the member list and the drawer.

### 2. Colour: one ground, cards, one lamp
The neutrals become a plum-tinted grey family (`ref.dusk`, OKLCH hue 322, very low chroma, steps named by lightness). The ground is the darkest tone; the navigator and side sheet sit one step up (`color.surface.navigator`); the sheet is the brightest in dark and white in light. Separation comes from tone and space, not lines. Apricot means "you" or "now" and nothing else: the primary action, Home, the selected-server pill, the unread divider and its "Nouveau" pill, mentions and jump bars.

### 3. Type, shape and motion
- Figtree only. Display sizes are tighter; messages get a 24 px line; category and group labels are sentence case at 13 px instead of uppercase; member groups show their count as a separate number.
- Radii 4, 8, 12, 16 and 24 px by hierarchy; nested corners are concentric. Things that float are rounder than things that sit.
- Every animation answers a user action or a state change and ends within 340 ms. Three curves: `standard` for colour and opacity, `enter`/`exit`, and `settle` (about 5 % overshoot, Melli's bounce) for the few things that land. Nothing loops except Melli and the typing dots while someone types.

### 4. The user card and the composer share one baseline
The user card (avatar, name, `@username`, microphone, headset, settings) is as tall as the composer's base row, and both sit the same distance above the bottom of their card, so their top and bottom edges line up across the window. Both come from the same tokens: `size.dock.height` (56 px) for the height and `size.dock.margin` (8 px) for the bottom gap, inside cards that share `size.sheet.inset`. The typing indicator moves above the composer, so nothing sits under it. When the composer grows (several lines, a reply bar, attachments), it grows upward: the base row and the user card stay aligned on their bottom edges and on the base row's height; only the composer's top moves.

### 5. Microphone and headset in the user card
The user card gets "Couper le micro" and "Couper le son" toggles beside the settings button, Discord's convention. A muted control shows a strike-through on its icon and turns `text.danger`, keeps its accessible name, reports `aria-pressed` (desktop: `accessible-checked`) and changes its tooltip ("Réactiver le micro"). Deafening also mutes the microphone, and undeafening restores the microphone to its previous state, as on Discord.

Voice does not exist yet (ROADMAP: voice and video come after the web client, with their own ADR). Until it ships, both buttons stay **hidden** in released builds; the user card keeps its new height so nothing moves when they appear. A control that does nothing would be a broken promise, and a disabled one would need an explanation on every hover. The voice ADR wires them; until then they exist in the mockup and in DESIGN.md.

### 6. The app icon on a dusk tile
Melli stops floating on a transparent (in practice white) background. Every icon (web favicon, window icon, Linux PNGs, macOS `.icns`) is Melli on a rounded tile with a vertical dusk gradient (`color.mascot.tile-top` to `tile-bottom`) and a low apricot glow behind the cloud (`color.mascot.tile-glow`): the cloud at sunset. The textured variant shown in the mockup (a few faint stars) is kept for large marketing artwork only, because a texture turns to noise at 16 and 32 px. The Home button in the server strip keeps Melli without a tile.

`cargo xtask tokens` already rasterises every app icon from the favicon SVG, so the change is the favicon template only (`xtask/src/favicon.rs`): it draws the tile and the glow from the new tokens and centres Melli at 90 % of its former size. The PNG sizes, the macOS 824/1024 grid, the `--check` pixel comparison and the desktop's embedded icons are unchanged.

### 7. Localisation: French and English
Today both clients are English-only, with strings in `.svelte`, `.slint` and Rust files. Both clients become fully bilingual, French and English, with the same copy rules (DESIGN.md §8):

- **Language choice:** the system language on desktop, the browser's `navigator.languages` on the web; French for any `fr-*` locale, English otherwise. User settings get "Langue : Système, Français, English", stored per device.
- **Desktop (Slint):** every user-visible string in `.slint` uses `@tr()`, with plurals (`@tr("{n} membre" | "{n} membres" % n)`). Translations are gettext `.po` files bundled at compile time (`slint_build::CompilerConfiguration::with_bundled_translations`, part of slint-build 1.18), so the binary stays self-contained and works offline. With bundled translations Slint selects the language from the system locale itself (through `sys-locale`, already in the dependency tree); the setting calls `slint::select_bundled_translation`. Strings built in Rust (connection notices, errors) move into `.slint` where they are plain text, or become typed enums that the `.slint` side words, so every string lives in one catalogue. `slint-tr-extractor` produces the `.pot` template; a CI check fails on untranslated strings.
- **Web (Svelte):** one typed catalogue per language (`web/src/i18n/en.ts` and `fr.ts`); the French one is typed against the English keys, so a missing translation fails `svelte-check`. Plurals, dates, times and sizes use `Intl` (`PluralRules`, `DateTimeFormat`, `NumberFormat`), which formats "2,4 Mo" and "14:32" natively. `<html lang>` follows the chosen language. No i18n library: the catalogue is a plain object and a small function.
- **Errors from the instance** are mapped from their codes in each client; text sent by the server is never shown as-is.
- **French voice:** tutoiement throughout (Mellow speaks like a friend who runs the server, as in English), sentence case, French typography (a narrow no-break space before « ; ! ? », a no-break space before « : » and inside « guillemets », typographic apostrophes), and Discord's established French terms where users already know them ("salon", "Marquer comme lu", "Messages privés").

Nothing is implemented yet: phase 2 extracts the strings, then translates them.

### Token changes
The token diff has been applied without removing or renaming any token a client reads, so both clients compile and pick up the new values at once. The structural changes (navigator, side sheet, aligned dock, microphone and headset, sentence-case labels, the transitions and localisation) need client code and are phase 2.

ADR-0012 gets the line `- Status: Superseded by 0022` when this ADR is accepted.

## Consequences
- Mellow no longer looks like a reskinned Discord, in screenshots or in structure, which also lowers the trade-dress risk ADR-0012 managed by hand. Discord users still find every concept, action and shortcut.
- Phase 2 work in both clients: the navigator card (the rail and the channel list move into one card with the user card under both), the side sheet (the web client already shows the member list as an overlay in narrow windows, the desktop client does not yet; it becomes the only mode, docked when wide), the "Aller à…" button, the dock alignment, the typing indicator above the composer, sentence-case labels (desktop: stop upper-casing in `live_rows.rs` and `shell_rows.rs`; web: drop `text-transform` from the `type-category` utility), `rail.pill-color-selected`, the member group count, the transitions, and localisation.
- The breakpoints change with the layout: the navigator needs 8 + 312 + 8 px beside a 480 px sheet, so the drawer takes over below 816 px (792 today), and the side sheet docks from 1064 px. Both are computed from the tokens, as today.
- Every icon now has a dark tile. The favicon is generated from the tokens like before; the committed PNGs and `Mellow.icns` were regenerated. Installs pick the new icon at their next start, through the existing icon writer.
- Slint does not export easing tokens (`cubicBezier` is skipped by the generator). Slint components write the same curve as `cubic-bezier(0.34, 1.3, 0.64, 1)` next to the token's duration.
- The desktop client still has no reduced-motion setting (DESIGN.md §2.8). Phase 2 must add it before the new transitions ship on desktop.
- Localisation doubles the copy to maintain: every pull request that adds a string adds it in both languages, and DESIGN.md §8 keeps the French counterparts next to the English ones.
- Android maps the same tokens onto Material 3 Expressive: `ref.dusk` seeds the surface container roles, apricot is `primary`, the settle moments use `motion.spring.settle-*`, the navigator becomes the channel panel of the three-panel model, and the side sheet becomes the members panel.
- Discord muscle memory around colour: the unread divider is apricot instead of red, and hovered servers no longer flash the accent. Mention badges stay red.

## Alternatives considered
- **Keep the faithful clone (ADR-0012) and polish it:** lowest effort and lowest risk, but it cannot meet the owner's goal; a clone is judged against the original and loses.
- **Dusk colours on Discord's exact layout (the first version of this ADR, "Classique" in the mockup):** a clear step up, but the owner found the layout still reads as Discord.
- **Palette first ("Palette" in the mockup):** a top bar with Home, a "Rust Lyon › # général" breadcrumb, a large "Aller à…" field and the user card; no channel list, channels reached through the breadcrumb menus and the quick switcher. The most distinctive and the most room for the conversation, but switching channel takes two actions instead of one, unread channels lose their always-visible list, and Discord users would have to relearn navigation. The quick switcher stays prominent in the chosen layout instead.
- **Servers as an accordion in one list (spaces with their channels nested):** one column instead of two, but it gets long with many servers and loses the one-click server switch.
- **A collapsible navigator (a focus mode showing only the server strip):** useful for reading, and compatible with the chosen layout later, but not needed to set Mellow apart; left for after phase 2.
- **Warm charcoal neutrals with apricot:** close to Melli's ink, but warm charcoal with a peach-orange accent is the look of several current AI and productivity products.
- **Deep navy neutrals with apricot:** striking, but navy sits too close to Discord's blue-grey family.
- **Glass, gradients or a translucent sidebar in the interface:** costly to render in Slint, hard to keep at AA contrast, and decoration rather than information. The one gradient in the identity is the app icon's tile, which is artwork, not interface.
- **A second typeface for display:** Figtree at heavier weights with tighter tracking already carries the display moments, and a new font adds size to both clients.
- **Microphone and headset shown before voice exists, as a stored preference or disabled:** keeps the panel complete, but a control with no effect (or a disabled one) confuses people; they appear with voice.
- **A textured icon tile:** richer at large sizes, but faint stars or grain become noise at 16 and 32 px, where the icon is seen most.
- **An i18n library (`svelte-i18n`, `fluent`):** pluralisation and formatting are covered by `Intl` on the web and gettext in Slint; a library would add a dependency for little.
