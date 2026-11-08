# Desktop: the menus of places and their notification settings

M3 step 4 for the GPUI client (ADR-0051). The desktop ports the web's menus of servers, channels,
categories and the narrow chat header ([web-context-menus.md](web-context-menus.md), DESIGN.md
§6.33), with mutes and "Notify me about" ([web-notifications.md](web-notifications.md)). Nothing
new is designed here: the web's items, order, words and dialog, on the desktop's `ContextMenu`,
`ModalDialog` and `ChoiceGroup`.

## Rules and storage

- Which items a menu shows comes from client-core's `shared_logic::context_menus_places` (the
  web's `channelMenuActions`, `categoryMenuActions`, `serverMenuFactsOf`, `serverMenuActions`,
  `chatHeaderMenuActions` and the notification section), pure and tested with the web's cases.
  The desktop says through the facts which actions it cannot carry out yet, so sections and
  separators stay right.
- Mutes and levels are the account's, kept in the preferences file in the web's JSON shape
  (`notification_settings_of`), so the notifier, the window title, the Dock and menu bar badge,
  the rail and the rows read the same choices. The notifier waits for the earliest end of a
  running mute and then drops the mutes that ended, as the web's `refreshClock`.

## Where each menu opens

```
 rail icon: right-click at the pointer;     channel row / category: right-click at the pointer;
 Menu key or Shift+F10 beside the icon      Menu key or Shift+F10 under the row, first item focused
 ╭──╮╭───────────────────────╮              ▸ # general         ╭────────────────────────╮
 │RL││ Mute…               ⊘ │              ╭────────────────╮  │ Copy category ID     ⧉ │
 ╰──╯│ Notification sett…  ◌ │              │ Mute…        ⊘ │  ╰────────────────────────╯
     │───────────────────────│              │ Notification…◌│
     │ Copy server ID      ⧉ │              │────────────────│
     │───────────────────────│              │ Copy channel ID│
     │ Leave server       ⇥  │ (red)        ╰────────────────╯
     ╰───────────────────────╯
 < 816 px: the chat header folds Go to and the pins into "More options" (⋯), plus Add friends
 in a group; the member toggle stays.
```

Focus goes back to the row or button that opened the menu. A menu whose place goes away (a
server left, a channel deleted or hidden) closes.

## Muted places

As the web's `sidebarRowClasses` and `mutedMark`: a muted channel or conversation row takes
`component.channel-row.text-muted-channel`, ends with a bell-slash
(`component.channel-row.muted-icon`, `muted-icon-size`), never shows unread, and its name is read
"general, muted". A muted conversation shows no mention badge; a muted channel keeps its mentions.
A muted server shows no unread pill on the rail and is read "Books, muted"; its mentions still
count. Home counts only conversations not muted.

## Tokens

Only existing tokens: `component.context-menu.*`, `component.dialog.*`, `component.choice.*`,
`component.channel-row.{text-muted-channel,muted-icon,muted-icon-size}`,
`component.text-field.label-{text,gap}`, `component.settings.section-gap`,
`component.chat-header.*`. No token was added.

## Accessibility

- Every menu is named "Actions for #general" / "Actions for Rust Lyon" / "More options"; items
  are menu items with their own names; Go to carries its shortcut for screen readers.
- The notification dialog's groups are radio groups named by their legends; focus starts on the
  checked choice of the group the menu item named ("Mute…" the mute group, "Notification
  settings…" the level group). Copy, Unmute and Save are announced in a polite status line.
- Leave server is the destructive pattern of DESIGN.md §8: Cancel takes focus; a failure stays
  in the dialog in words; after leaving, focus goes to the composer.
- Motion is the menu's and dialog's own, through `animation_duration`, so reduced motion holds.

## Defaults rejected

- A submenu of mute durations, as Discord's: the web opens the dialog, which also says how long
  a running mute lasts and what the place notifies about; one way for both clients.
- Disabled items for invites, settings, moves and deletion: left out until their screens exist,
  as the track's scope asks, rather than shown and refused.

## Left out (named in the code with `shortcut:`)

Invite people, server, channel and category settings, create channel and category, move up and
down, move to category, rename and delete (step 6); join and leave voice, and the voice rows'
menu (step 7); the server header's menu (the desktop's header is a heading, not a button yet).
The group and one-to-one conversation menus belong to the people menus.
