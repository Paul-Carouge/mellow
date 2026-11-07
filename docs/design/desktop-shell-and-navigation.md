# Desktop (GPUI): the signed-in shell and navigation

M3 step 2 (ADR-0051). The GPUI desktop client ports the web's shell (`Shell.svelte`, `Navigator.svelte`, `ServerRail.svelte`, `ChannelSidebar.svelte`, `UserCard.svelte`, `MemberList.svelte`, `QuickSwitcher.svelte`, `ColumnResizeHandle.svelte`) onto the Dusk tokens. [`desktop-dusk-navigator.md`](desktop-dusk-navigator.md) records the Slint-era mapping of the mockup; this file records the GPUI shell, so the message list and composer (step 3) and later steps build on it.

## Brief

People who live in Mellow all day, mostly on a laptop or a desktop screen, switching between a handful of servers and conversations. The shell's job is to say where you are and get you anywhere in one gesture: a click on the rail or the list, `Ctrl+K`, or `Alt+Up/Down`. Everything else (voice, profiles, settings) comes in later steps and keeps its place.

## Structure

```
 ≥ 1064 px, members docked                                  816–1063 px: members over the sheet
 ┌───────────────────────────────────────────────────────┐   ┌───────────────────────────────────┐
 │ title band (macOS) · connection banner · sign-out error │   │ ╭navigator╮ ╭sheet─────╮░░░░░░░░░ │
 │ ╭navigator──────╮ ┃ ╭sheet──────────────╮ ┃ ╭members╮ │   │ │          │ │       ╭members─╮ │
 │ │rail│channels   │ ┃ │header 48          │ ┃ │Members│ │   │ ╰──────────╯ ╰───────╰─────────╯ │
 │ │ 72 │ 240–360   │ ┃ │conversation slot   │ ┃ │240–320│ │   └───────────────────────────────────┘
 │ │╭user card 56──╮│ ┃ │╭composer frame 56─╮│ ┃ │       │ │    < 816 px: the sheet alone, full-bleed;
 │ │╰──────────────╯│ ┃ │╰──────────────────╯│ ┃ │       │ │    "Open navigation" brings the navigator
 │ ╰────────────────╯   ╰────────────────────╯   ╰───────╯ │    as a drawer from the leading edge, and
 └───────────────────────────────────────────────────────┘    the members cover the window.
   ┃ = resize handles in the gaps
```

- `shell/mod.rs`: `DuskShell`, its layout (`ShellLayout`), the keys, the column widths and the overlays' state. One new shell per session (`MainWindow`), so nothing of a previous account stays.
- `navigation.rs`: the place (Home with an optional conversation, or a server with its channel), pure and tested. The desktop also remembers the channel last opened in each server and the place itself across launches, as Discord does; the web opens the first server at each load.
- `shell/conversation_sheet.rs`: the chat header and **`DuskShell::conversation_slot`**, the slot the message list and composer replace. It shows the sentence an empty conversation opens with and the composer's frame, so the dock lines up with the user card (ADR-0022 §4). Replace the body and the frame; keep the slot's flex sizing and the `composer` debug selector, which the layout test reads.
- Preferences (`preferences.rs`): every change goes through `change_preferences`, which saves the whole set; keys `place`, `channel`, `member-sheet`, `channels-width`, `members-width`, each validated on the way in and bounded to 200 entries.

## Tokens

Only existing tokens: `component.rail.*`, `component.channel-row.*`, `component.category.*`, `component.sidebar-header.*`, `component.user-panel.*`, `component.chat-header.*`, `component.member-row.*`, `component.member-list.*`, `component.side-sheet.*`, `component.column-resize.*`, `component.quick-switcher.*`, `component.voice-participant.*`, `size.sidebar-width{,-min,-max}`, `size.member-list-width{,-min,-max}`, `size.mobile-panel-peek`, `size.dock.*`, `color.scrim`, the breakpoints. The status menu's choices use `component.status.size-menu`.

## Motion

The rail's squircle settles (`component.rail.morph-duration`, settle curve) and its pill moves on `motion.duration.base`; drawers and the member overlay slide 16 px and fade (`motion.duration.slow`, enter curve); the quick switcher fades in (`component.dialog.enter-duration`). All run through `animation_duration`, so reduced motion turns them into cuts. Nothing moves while idle.

## Defaults rejected

- A permanent member column: the member sheet opens on demand and is remembered per server, as on the web.
- Sign out in the menu bar: it moved to the user card's menu, where people look for their account; the menu bar keeps Quit only. The web keeps it in Settings, which the desktop does not have yet.
- Microphone, headset and settings buttons on the user card before voice and settings exist: the card keeps its height and shows only what works.
- A "Go to… Ctrl K" pill: the header has a magnifier button named "Go to"; the shortcut is the main way in.
- Dead buttons on voice channels and members: their rows are read, not pressed, until joining (step 7) and profiles (step 4) land.

## Accessibility

- Every row and button has a role and a name with its state ("general, 3 mentions, unread", "Rust Lyon, unread"); categories report expanded and collapse with Left and Right; the member toggle reports pressed; the status choices are radio items with a check, never colour alone.
- Tab walks every control; drawers, the member overlay and the quick switcher keep Tab inside and give focus back when they close; Esc closes the topmost layer, then a focused docked member sheet.
- Keys (DESIGN.md §7.2, `Cmd` on macOS): `Ctrl+K`, `Alt+Up/Down`, `Alt+Shift+Up/Down`, `Ctrl+Alt+Up/Down`, `Ctrl+U`. Resize handles are window splitters: arrows, Home, End, double-click to reset, Esc to cancel a drag.

## Not yet (named in the code with `shortcut:`)

Group member sheet, profiles from the member sheet, the Friends space, Melli on the Home button, animated avatars and icons, `F6` region cycling and roving focus inside lists, joining voice from a row, the server header menu and its dialogs, custom status, Esc marking a channel read, and marking the open channel read (step 3).
