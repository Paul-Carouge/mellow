# Desktop: menus for people, messages and groups

M3 step 4 for the GPUI client (ADR-0051). The desktop opens the web's person, message and group
menus ([web-context-menus.md](web-context-menus.md), DESIGN.md §6.33) with the same items, drawn by
the existing `ContextMenu`. This file records where they open and what the desktop leaves out.

## Rules

`mellow-client-core::shared_logic::context_menus_people` ports `personMenuActions`,
`messageMenuActions` and `groupMenuActions` as pure functions with their tests. The message menu
rule moved there from `conversation/rules.rs`, so the desktop has one rule, not two. How an item
reads (`menuItemLook`) stays in the desktop, which owns the catalogue and the icons.

## Where they open

| Menu | Opened from | Pointer | Keyboard (Shift+F10, Menu key) |
|---|---|---|---|
| Person | a message's author name and avatar | at the pointer | the row's own menu is the message's |
| Person, with Profile | a member row of a server's side sheet | at the pointer | beside the focused row, first item focused |
| Person | a friend row | at the pointer | beside the row while one of its actions has focus |
| Person, with the conversation's ID and Mute | a one-to-one conversation's row | at the pointer | beside the focused row |
| Person | a group member row in the group's side sheet | at the pointer | none: the row takes no focus |
| Group | a group's row, the group's name in the chat header | at the pointer | beside the focused row (the header name takes no focus) |
| Message | a message row | at the pointer | beside the focused row, or beside the list before the row is measured |

Home keeps the open person or group menu and draws it over the window. The message list cannot
reach Home, so it dispatches the `OpenPersonMenu` action, which the shell hands on. Focus goes back
to the row (or what had focus) when the menu closes. A change that fails is said in the shell's
notice bar; one that works is read out by a status region.

The member profile in the side sheet gains the web's buttons under Send message: Add friend, Accept
or "Request sent", then Block; someone blocked shows Unblock alone. Block and Unblock share one
focus handle, so focus stays in place when one replaces the other. Remove friend and Block always
ask first in the Friends space's dialog, which Home now draws, so it shows wherever it was asked
from.

## Left out until their screens exist

- "Notification settings…" in the person and group menus: no per-place notification dialog yet
  (step 6). "Mute" mutes until turned back on, without the web's durations, and reads "Mute" (no
  ellipsis, since no dialog follows).
- Moderation entries: the web's person menu has none either; they come with their dialogs
  (step 6).
- Add reaction, Forward, End poll and Remove all reactions in the message menu (step 5); the
  existing reactions are offered one by one instead.
- A "Mention" item and menus on mention chips: the web has neither.

No token or string was added.
