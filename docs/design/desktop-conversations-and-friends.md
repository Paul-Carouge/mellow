# Desktop: Home, friends and group conversations

M3 step 4 for the GPUI client (ADR-0051). Home is the conversations entry of the server rail. The
desktop ports the web's Friends space and group conversations as they are
([web-friends.md](web-friends.md), [web-group-conversations.md](web-group-conversations.md),
DESIGN.md §5.6, §5.6.1, ADR-0025, ADR-0026); this file records how the GPUI shell holds them.

## Layout

```
 wide (navigator beside the sheet)                         < 816 px (navigator in a drawer)
 ╭navigator──────────────╮ ╭sheet─────────────────────╮    ╭sheet──────────────────────╮
 │rail│Direct messages + │ │👥 Friends │Online All     │    │≡ 👥 Friends          Go to │
 │    │▌Friends        2 │ │           │Pending² Blocked│    │Online All Pending² Blocked │
 │    │ (Ana)• Ana       │ │           │Add friend Go to│    │Add friend                  │
 │    │ (AB)  Book club  │ │ ONLINE                 3   │    │ ONLINE                  3  │
 │    │       3 members  │ │ (Ana)• Ana    ◯msg ◯more   │    │ (Ana)• Ana    ◯msg ◯more   │
 │╭user card──────────╮  │ │ …                          │    │ …                          │
 ╰───────────────────────╯ ╰───────────────────────────╯    ╰───────────────────────────╯
```

- The navigator's Home list: "Friends" first (its badge counts requests waiting), then the
  conversations by last activity (client-core's order). One-to-one rows show the other person's
  presence on the avatar; group rows show `GroupAvatar` and "N members" on a second line, never
  presence. "+" beside "Direct messages", and the empty list's "New message", open the people picker.
- The Friends space fills the sheet when Home has no conversation open. Its tabs sit in the chat
  header after a divider where the navigator is docked, and on their own row under the header in
  the drawer layout, as on the web. The tab chosen survives a visit to a conversation (the pane
  lives as long as the shell).
- A group's side sheet (`Ctrl+U` or the header's members toggle, remembered per conversation like a
  server's): Add friends (apricot), Rename group, Leave group (red, last), then everyone with the
  owner said in words; the owner removes others from their row.

## Tokens

Only existing tokens: `component.friends.*` (tabs, rows, round actions, result lines),
`component.people-picker.*`, `component.group-avatar.*`, `component.member-row.*`,
`component.side-sheet.*`, `component.context-menu.{accent,danger}-text` for the group actions. No
token was added.

## Behaviour kept from the web

- Rows act at once except Remove friend and Block, which confirm (destructive pattern, Cancel takes
  focus); Unblock, Accept, Ignore and Cancel do not. A row waiting for the instance dims its actions
  only. When an action takes the focused row away, focus moves to the row now at its place, else
  to the selected tab.
- Add friend: two forms, by name and by friend code, the answer said under the field (polite
  status) with the web's outcome words (sent, already sent, accepted theirs, already friends) and
  refusals (unknown, rate limited, limit reached), never telling a block apart from an unknown name.
- The people picker: friends with a search ignoring case and accents, picked friends as chips
  (Backspace in an empty search takes the last back, Enter picks the first match), the room left
  (10 places), one friend opens the conversation, several create a group with an optional name, a
  group adds people one request at a time and keeps the rest picked on a refusal.
- Rename (empty brings back the names), Leave (says who owns it next, or that it goes with you),
  Remove from group: the web's dialogs and sentences.

## Public entry point

`shell::OpenDirectConversation { user_id }` is a GPUI action any view inside the shell dispatches
(context menus, profile cards): the shell opens or creates the one-to-one conversation, shows it
and focuses the composer; a failure shows in a notice bar under the connection banner.

## Defaults rejected

- Reusing the underline `Tabs` primitive: the web's Friends tabs are filled pills with an apricot
  "Add friend" and a count badge on Pending, which the underline tabs cannot show.
- A separate "Friends" screen outside the sheet: it stays the sheet's content so the navigator,
  the dock alignment and the quick switcher behave as everywhere else.

## Left out (named in the PR)

Melli in the empty tabs (the desktop has no Melli yet), the person and group context menus on rows
(wave 4b), muted conversations (notifications track), the "removed you" and "left" announcements.
