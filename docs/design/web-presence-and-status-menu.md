# Web client: presence statuses and the user card's status menu

DESIGN.md §2.3 ("Presence"), §5.3 (user card, status menu, custom status dialog), §5.7 (member rows) and §6.9 on the web client, in the Dusk direction ([ADR-0022](../adr/0022-give-mellow-its-own-visual-language-on-a-familiar-layout.md)). ROADMAP item 6. The protocol and server side is commit 2072683 (`ChosenStatus`, `custom_status`, `PUT /api/v1/users/@me/presence`). This page records the design decisions and the mapping to the code, so the desktop catch-up keeps the same shape.

## Brief

- **Who**: members of small self-hosted communities who already read Discord's presence at a glance, and who want to say "busy" or "back after lunch" without leaving the conversation; often on a phone, where the navigator is a drawer.
- **What the owner asked** (#roadmap, item 6): idle, do not disturb and invisible, a custom status, and a menu on the user card with "Copy user ID", as on Discord.
- **Rule**: the instance decides. The client shows your own pick at once (`presenceOf` reads `client.user.status`), but the menu waits for the instance's answer before closing, so a refusal is never hidden behind a closed menu. Others see invisible as offline; so does every avatar of yours, and only your own card names it "Invisible".

## Flow and states

```
user card (avatar + name button) ── click / Enter / Space ──> status menu (above the card)
   ├ Online | Idle | Do not disturb | Invisible ── pick ──> saving (items dimmed) ──> menu closes, focus on the card
   │                                                    └─> error at the menu's foot, menu stays
   ├ Set a custom status / Edit custom status ──> dialog ── Enter / Save status ──> saved, focus on the card
   │                                                    ├─ Clear status ──> cleared
   │                                                    └─ Esc / Cancel / scrim ──> nothing changes
   └ Copy user ID ──> menu closes, card's second line: check + "User ID copied" (2 s), announced
                  └─> clipboard refused: error at the menu's foot, menu stays
```

| State | What shows |
|---|---|
| No custom status | Card second line `@username`; menu item "Set a custom status". |
| Custom status | Card second line the sentence (truncated, full text in the tooltip); member rows the same in `type.body-sm`, `text.muted`; menu item "Edit custom status". |
| Invisible | Your avatar shows the offline ring; the card's label says "Invisible"; your sentence stays on your own card only. |
| Saving a status | Status items `aria-disabled` and dimmed; Copy and custom status stay usable. |
| Saving the custom status | Field read-only, buttons waiting; Esc and the scrim do nothing until the answer. |
| Too long | Helper "3 characters too many", `aria-invalid`, Save status disabled. Counted in Unicode characters of the trimmed text, as the instance counts (an emoji is one). |
| Refused by the instance | Hidden characters: "remove line breaks and invisible characters, then try again"; rate limit and network: the usual reasons. |
| Reduced motion | The menu appears without its lift; nothing else moves. |

## Design plan

- **Colour**: no new colour. The four `component.status.*` colours carry the conventions people already read; apricot stays out of presence (2.1), so the check that marks your status is in the item's text colour, not the accent. The menu is the ContextMenu surface (`surface.floating`, `state.selected` on hover).
- **Contrast**: muted text is 4.36:1 on the hover wash in light, so hovered hints and the card's hovered second line switch to `text.secondary` (`context-menu.item-hint-text-focus`, `user-panel.status-text-hover`); the status colours are checked on the menu and the card surfaces (3:1).
- **Type**: `type.menu-item` for status names and actions, `type.body-sm` in `context-menu.item-hint-text` for the two hints, `type.user-panel-name` and `type.body-sm` on the card.
- **Shape**: the statuses are SVG paths on a 20-unit grid with even-odd holes (`StatusIndicator.svelte`), so the surface behind shows through whatever its colour, hovered rows and the open card included, and they stay crisp at 10 and 12 px. New token `component.status.size-menu` (12 px) for the menu, where a 10 px crescent was too soft beside 14 px text.
- **Motion**: one answer to one action: the menu lifts in by `space.2xs` over `motion.duration.fast` with `motion.easing.enter` (`mw-lift-in`, the mirror of the drop-in used by menus that open downward).
- **Layout**: the menu is as wide as the card and sits `space.2xs` above it, so it reads as the card unfolding upward; shapes lead (a legend down the left edge), the check trails.

```
wide (navigator card)                      phone (navigator drawer, same structure)
┌──────────────────────────────┐           ┌──────────────────────────┐
│ ●  Online                  ✓ │           │ ●  En ligne              │
│ ◐  Idle                      │           │ ◐  Inactif               │
│ ⊖  Do not disturb            │           │ ⊖  Ne pas déranger     ✓ │
│    You won't get notifications│          │    Tu ne recevras pas…   │
│ ○  Invisible                 │           │ ○  Invisible             │
│    You'll look offline       │           │    Tu apparaîtras hors…  │
│ ──────────────────────────── │           │ ──────────────────────── │
│ ☺  Edit custom status        │           │ ☺  Modifier le statut…   │
│ ──────────────────────────── │           │ ──────────────────────── │
│ ⧉  Copy user ID              │           │ ⧉  Copier l'identifiant… │
└──────────────────────────────┘           └──────────────────────────┘
[(P⊖) paul / In a meeting…   ] ⚙           [(P⊖) paul / En réunion…] ⚙
```

## Defaults rejected

- **Coloured dots alone** (the common status picker): each status has its own shape, and the hints say what Do not disturb and Invisible do.
- **A toast for "Copied"**: there is no toast component, and the confirmation belongs where focus returns: the card's own second line says it for two seconds, and a polite live region announces it.
- **Discord's full custom status dialog** (emoji button, "Clear after" durations): one field, since emoji typed or pasted are kept by the instance, and timed clearing needs server support nobody asked for yet.
- **Closing the menu at once on a pick** (optimistic): it waits for the instance, so a rate limit or a network error is shown where the choice was made.
- **Repeating the custom status under "Edit custom status"**: removed after the first render, since the card right below already shows it.
- **An always-on "12/128" counter**: the helper says how many characters are left in words, and is read with the field rather than on every keystroke.

## Mapping to the code

| Piece | File |
|---|---|
| Status shapes | `web/src/components/StatusIndicator.svelte` |
| Avatar with status and its spoken name | `web/src/components/Avatar.svelte` (`presence` takes the four statuses and `invisible`) |
| User card, status menu, Copy user ID | `web/src/components/UserCard.svelte` |
| Custom status dialog | `web/src/components/CustomStatusDialog.svelte`, length rule in `custom-status.ts` (tested) |
| Menu keyboard (arrows, Home, End), shared with the server and message menus | `web/src/components/menu-keyboard.ts` (tested) |
| Member rows, conversation rows | `MemberList.svelte`, `ChannelSidebar.svelte` |
| Words | `presence`, `userCard`, `customStatus` and `members.memberLabel` in `web/src/i18n/en.ts` and `fr.ts` |

## Checked

Headless Chrome against a local instance built from the branch, seeded with members in every status: English and French, dark and light, 1280 px and 390 px, reduced motion. Keyboard: Enter opens the menu on your status, arrows move, Enter picks, Esc closes and focus returns to the card; the dialog saves with Enter and cancels with Esc. Over-long and refused drafts show their messages. Not checked: a real screen reader (VoiceOver, NVDA), 200 % text zoom, Windows forced colours.
