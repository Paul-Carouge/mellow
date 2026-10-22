# Desktop: the Dusk navigator layout

Phase 2 of ADR-0022 for the Slint desktop client. The reference is `design/mockups/dusk-direction.html`, default "Navigateur" layout: its CSS and markup are the specification, measured element by element in headless Chrome. This file records how the desktop maps it, so later work keeps it.

## Structure

```
 wide (≥ 1064 px, side sheet docked)                           816–1063: side sheet over the sheet
 ┌──────────────────────────────────────────────────────┐       ┌──────────────────────────────────┐
 │ 16                                                    │       │ ╭navigator╮ ╭sheet─────╮░░░░░░░░ │
 │ ╭navigator 312╮ 8 ╭sheet (flex)──────╮ 8 ╭side 240╮ 8 │       │ │         │ │       ╭side sheet╮ │
 │ │strip│channels│   │header 48         │   │Members │   │       │ │         │ │  scrim│        │ │
 │ │ 72  │  240   │   │messages          │   │        │   │       │ ╰─────────╯ ╰───────╰────────╯ │
 │ │╭user card 56╮│   │typing 24         │   │        │   │       └──────────────────────────────────┘
 │ │╰────────────╯│   │╭composer 56─────╮│   │        │   │        < 816: sheet full-bleed, navigator
 │ ╰──────────────╯   │╰────────────────╯│   ╰────────╯   │        as a drawer from the leading edge,
 │ 8                  ╰──────────────────╯                │        side sheet over the whole sheet
 └──────────────────────────────────────────────────────┘
```

- The ground is `color.surface.rail`. Cards are inset `size.sheet.inset` (8). The mockup's grid adds an empty top-bar row and an empty side-sheet column, each with its gap, so the top inset is 16 and the trailing inset is 16 while the side sheet is not docked. The desktop copies that, so screenshots match.
- The navigator (`navigator.slint`) is one card: the server strip, the channel list (which reaches 12 px back into the strip so its unread dots sit just after the server icons) and the user card across both.
- The user card and the composer's base row are both `size.dock.height` (56) high and `size.dock.margin` (8) above the bottom of their card, so their edges line up. The typing row (24) sits above the composer; more lines grow the composer upward, up to half the smallest window, then the field scrolls.
- Breakpoints: `8 + 312 + 8 + 480 + 8 = 816` (navigator), `816 + 240 + 8 = 1064` (docked side sheet), computed from tokens in `app.slint`.

## Tokens

Everything comes from `design/tokens.json`. Added for this work: `component.user-panel.toggle-strike-length`, `component.message.reply-height`, `reply-spine-inline-size`, `reply-spine-block-size`, `reply-spine-radius`. Easing curves are written next to their duration token (`motion.easing.standard` `cubic-bezier(0.2, 0, 0, 1)`, `enter` `cubic-bezier(0, 0, 0, 1)`, `settle` `cubic-bezier(0.34, 1.3, 0.64, 1)`), since Slint globals cannot hold an easing.

## Motion

`MotionPreference` (`ui/preferences.slint`) holds the reduced-motion choice and the spatial durations (`fast`, `base`, `slow`, `settle`), which drop to zero when motion is reduced. Colour transitions keep `motion.duration.fast`. Transitions shipped: rail pill height (`base`), squircle (`settle` with the settle curve), hover colours (`fast`), channel switch (list fades in and rises 8 px, title cross-fades, `base`), jump bar (rises 8 px and fades, `base`), side sheet and drawer (slide 16 px and fade, `slow`), mute strike (`fast`). Nothing runs while idle except Melli, which shows its still pose under reduced motion.

## Defaults rejected

- A "Match system" motion option: Slint 1.18 does not expose the OS setting, and an option that cannot follow the system would lie. The stored choice says so in Appearance.
- A permanent member column: the side sheet opens on demand and remembers its state per server.
- Moving focus into a docked side sheet: focus moves in only when the sheet lies over the conversation, so typing keeps going to the composer otherwise.
- Showing "Go to… Ctrl K", the microphone and the headset in releases before the quick switcher and voice exist: they show in development builds only, and the header and user card keep their size.

## Content

- People: default avatar tints (`color.avatar.tint-1`…`8`, FNV-1a over the id's decimal digits, modulo 8) and role colours on author and member names (highest coloured role, OKLCH lightness moved to 4.5:1), shared with the web client (`person_style.rs`).
- Mentions: `@username` or a one-word display name of someone known, in the mention chip's text colour; a message naming you gets the mention highlight (`mentions.rs`, the web's `mentions.ts` rules).
- Unread: the "New" divider above the first message someone else wrote after the read position at opening, and "N new messages since HH:MM" with "Mark as read" (also `Esc` in the composer).
- Typing: "Ana is typing…" above the composer, with the waving dots only while someone types.
- The quick switcher (`Ctrl+K`, "Go to… Ctrl K"), and "+" on categories (`parent_id`).

## Known differences from the mockup

- Mention chips have no fill: `StyledText` has no background colour, so only the chip's text colour shows.
- No picture previews and no download button on file cards: the desktop client has no authenticated attachment download yet (client-core has no download call).
- No tooltips (Slint 1.18 has none built in) and no shadows (the token generator does not emit shadows for Slint).
- The member list groups online and offline members, not roles: the protocol has no "show separately" role flag. Profiles show no `@username@instance` line and no banner colour: member rows do not carry them yet.
- Author and timestamp are centred on their line rather than aligned on a shared baseline (about 1 px apart); Slint has no line-height, so multi-line text uses the font's own line spacing.
- New messages appear in place rather than rising: rows are recreated whenever the model is rebuilt, so an entrance animation would replay on every update.
- The desktop does not send typing events of its own yet; it shows other people's.
