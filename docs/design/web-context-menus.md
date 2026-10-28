# Web client: context menus

DESIGN.md §6.33 (ContextMenu) and the per-surface menus of §5.2, §5.3, §5.4, §5.6.1 and §5.7 on the web client, in the Dusk direction ([ADR-0022](../adr/0022-give-mellow-its-own-visual-language-on-a-familiar-layout.md)). ROADMAP item 9, the owner's wish: right-click (and long press, and the keyboard's menu key) on a channel, a member, a message or a server gives the actions that fit, instead of the browser's menu. No server or protocol change: every item is an action the web client could already do. This page records the decisions and where they live, so the desktop catch-up keeps the same shape.

## Brief

- **Who**: people who know Discord and right-click everything out of habit, on a laptop; the same people on a phone, where a long press is the habit; keyboard and screen-reader users, for whom Shift+F10 and the Menu key are the habit.
- **Job**: reach the actions of the thing under the pointer or the focus in one gesture, without hunting for a hover button, and without losing the browser's own menu where it is the useful one.
- **Constraint**: only what already works (YAGNI). Mute, notification settings, "Mark as read", "Mark unread", "Copy message link", Select text and server folders wait for the work that makes them possible (see Follow-ups).

## What opens Mellow's menu, and what keeps the browser's

| Gesture | Opens | Where |
|---|---|---|
| Right-click | Mellow's menu of the item | At the pointer, down and to the trailing side, turned back when the window has no room; it lifts in when it opens above the pointer. |
| Long press (touch) | The same | At the finger, when the finger lifts after `motion.delay.long-press` (500 ms) without drifting more than `component.context-menu.long-press-move-tolerance` (10 px). |
| Shift+F10, the Menu key (focused item) | The same | Under the focused item, or above it near the bottom; a message's menu opens where its More button would, a server's beside its icon. |
| More (`…`) on a message or a friend | The message's menu; the friend's two danger actions | Under the button, as before. |

The browser's menu stays, deliberately:

- **Inside text fields** (the composer, the inline editor, every settings field): Paste, spelling, emoji, autofill live there. Discord keeps the native menu in its composer too.
- **On links**, file cards included: open in a new tab, copy the link address, save the file.
- **On selected text**: Copy and the browser's search. A message's own Copy text is in its menu for the rest.
- **With Shift held** while right-clicking anywhere: the browser's menu on purpose (developers, extensions, "Inspect").
- **Where Mellow has no menu**: pending or failed messages (their Retry and Delete sit under them), the folded blocked messages, Home and "Add a server" in the strip, category headers, the chat header.

## Menus and their items

Items appear only when the reader may use them, from the same helpers the buttons use (`hasPermission` on the server snapshot, `myChannelPermissions`, `visibleChannelSections`, `canOpenServerSettings`, the relationship kind). Danger items come last, after a separator. The pure lists are `web/src/components/context-menus.ts`, tested in `context-menus.test.ts`.

| Surface | Items, in order |
|---|---|
| **Message** (right-click, long press, Shift+F10 on a focused message, its More button) | Add reaction · — · Reply · Edit message (own) · Copy text (when it has text) · Pin message or Unpin message (Manage messages, or anyone in a conversation) · Reactions (when it has some) · — · Remove all reactions (moderators) · Delete message (own, or Manage messages) |
| **Person**: a member row, a message's author (avatar or name), a friend row | Profile (member list only) · Message (not yourself, not someone you blocked) · — · Add friend, or Accept friend request and Ignore request, or Cancel request, or Unblock · — · Copy user ID · — · Remove friend (friends) · Block |
| **One-to-one conversation row** | The person's menu without Profile and Message, plus Copy channel ID after Copy user ID |
| **Group conversation row** | Rename group · — · Copy channel ID · — · Leave group |
| **Channel row** (the row and its gear) | Edit channel (when a settings page is visible to the reader) · — · Copy channel ID |
| **Server icon in the strip** | Invite people (accent, Create invite) · Server settings (when a page is visible) · — · Copy server ID · — · Leave server (not the owner) |
| **Server header** (unchanged) | Invite people · Server settings · Create channel · — · Leave server |
| **Friend row's More** (unchanged) | Remove friend · Block |
| **Status menu** (unchanged) | Online, Idle, Do not disturb, Invisible · — · Set or Edit custom status · — · Copy user ID |

Acting:

- Items that open something (a dialog, the editor, the composer for a reply, the emoji picker, a profile) close the menu first, then act; the dialogs are the existing ones (delete, pin, remove all reactions, remove friend, block, rename group, leave group, invite, server settings, leave server, channel settings).
- Items that wait (Message, Add friend, Accept, Ignore, Cancel request, Unblock, every Copy ID) keep the menu open with its items dimmed (`aria-disabled`, `aria-busy` on the menu); success closes it and is read out politely ("Friend request sent to Ana.", "Server ID copied."); a refusal stays at the menu's foot with the warning icon, in an alert, and the menu stays usable, as the status menu does.
- The strip's menu works on that server without opening it: invite, settings and leave take the server's ID (`createInvite(guildId)`, `leaveServer(guildId)`), which also fixed Invites settings creating an invite for the open server rather than the one being edited.

## Design plan

**Tokens.** No new colour: the menu is `component.context-menu.*` (`surface.floating`, radius 12, padding 6, items 32 px with radius 6, 44 px under a coarse pointer, `elevation.high`, quiet `state.selected` hover, `danger.fill` hover for danger items, apricot text only for Invite people). Three new tokens: `motion.delay.long-press` (500 ms, kept under reduced motion: it is a gesture, not an animation), `component.context-menu.long-press-move-tolerance` (10 px) and `component.context-menu.viewport-margin` (`space.2xs`, the least room a menu keeps from the window's edges at the pointer).

**Motion.** The existing pair: `mw-drop-in` under its origin, `mw-lift-in` above it, `motion.duration.fast` with the enter curve; reduced motion makes both instant. Nothing else moves.

**Layout.**

```
Right-click near the middle            Right-click near the bottom-right       Phone, long press (44 px items)
  ┌ pointer                                        ┌──────────────┐            ┌──────────────────────┐
  ▼┌──────────────┐                                │ Add reaction │            │ bob  Today at 09:41  │
   │ Add reaction │                                │──────────────│            │ Quelqu'un a… ┌──────────────┐
   │──────────────│                                │ Reply        │            │              │ Add reaction │
   │ Reply        │                                │ Copy text    │            │              │──────────────│
   │ Copy text    │                                │──────────────│            │              │ Reply        │
   │──────────────│                                │ Delete       │            │              │ Copy text    │
   │ Delete       │                                └──────────────┘▲ pointer   │              └──────────────┘
   └──────────────┘                         (turned left and lifted in)       └──────────────────────┘
```

**One component.** `ContextMenu.svelte` is the only menu of the web client: the popover (`popover="auto"`, light dismiss and Esc), `role="menu"`, the keyboard of `menu-keyboard.ts` (arrows wrap, Home and End, type-ahead on the first letter, accents ignored), the placement of `menu-placement.ts` (`placeMenu`: at a point, or at an element aligned to its start, end, or stretched to its width), focus back to the opener on close, the waiting items and the error foot. The server header, the status menu, the friend's More and the message's More moved onto it; the status menu keeps its own radio items as children. Each surface holds one menu for all its rows (`MessageList`, `ChannelSidebar`, `ServerRail`, `MemberList`, `FriendsPane`), so no row builds a menu it may never open; `PersonContextMenu.svelte` is the person's menu with its confirmation dialog, shared by four surfaces. `contextMenuTrigger` (an attachment, `context-menu-trigger.ts`) turns any element into a menu opener.

## Defaults rejected

| Default | Why not here | Instead |
|---|---|---|
| Focus the first item whatever opened the menu | After a right-click or a long press the first item looked chosen (and, after typing in the composer, showed the focus ring on "Add reaction"); desktop menus opened by a pointer preselect nothing | Pointer: the menu itself takes focus; an arrow or a letter moves to an item. Button or keyboard: the first item, or the checked one, as before |
| Open on the long-press timer, finger still down | A popover opened during a touch is light-dismissed by the finger's own `pointerup`, and the tap's compatibility mouse events close it again | Open when the finger lifts after the hold; the lift's `touchend` is cancelled, so the row is not also tapped |
| Replace the browser's menu everywhere | Paste, spelling, "Open link in new tab" and Copy on a selection are worth more than consistency | The four exceptions above, decided per event, plus Shift+right-click |
| A menu inside every row | Thousands of hidden menus in a long history; the message list already builds its action bar only on hover | One menu per surface, filled when it opens |
| A context-menu library | A dependency for 300 lines the platform covers (`popover`, `contextmenu`, Pointer Events) | The house component |
| Hover-only actions | Not reachable by touch or keyboard | Every hover button also lives in the menu, which a long press and Shift+F10 open |
| `aria-haspopup` on every row | A channel row is a link to the channel, not a menu button; screen readers would announce it as one | `aria-haspopup="menu"` only on buttons whose click opens a menu (More, the server header, the status button); rows keep their role, and Shift+F10 is listed in §7.2 |

## Accessibility

- WAI-ARIA menu pattern: `role="menu"` with an `aria-label` naming its subject ("Actions for Ana", "Actions for #general", "Message actions"); items `menuitem` (the status menu's `menuitemradio`); arrows wrap, Home, End, a letter jumps; Esc and a click outside close it and focus returns to the opener: the row, the message, the More button, the icon.
- Opening by keyboard puts focus on the first item, with the focus ring; by pointer, on the menu, without one.
- Items reach 44 px under a coarse pointer; the menu never leaves the window (`viewport-margin`), and a menu taller than the window starts at its top.
- Results of waiting items are read out politely from outside the popover (a hidden popover's live region is not read); errors are alerts inside it.
- Text at 200 %: items grow with their text (`min-height`, wrapping labels up to the 320 px maximum width).

## Verified, and what is left

- `npm run check`, `npm test` (placement, type-ahead, which items show for which permissions and relationships, the long-press decision), `npm run build`, `cargo xtask tokens --check`.
- Headless Chrome against a local instance at 1440, 820 and 360 px, dark and light, English and French: every menu above by right-click, the message menu by Shift+F10 (focus back on the message after Esc), by type-ahead and through to the delete confirmation, the message menu by an emulated long press at 360 px (44 px items, no focus ring), Copy server ID refused by the headless clipboard (the error at the menu's foot).
- Not verified on real devices: the long press on iOS Safari (no `contextmenu` event there; `-webkit-touch-callout: none` on messages; text selection starting under the finger is cleared when the menu opens) and on Android Chrome (its own long press is taken over and opens the menu on lift), the Menu key on Windows and Linux (Chrome also fires `contextmenu` after it; the menu's own handler stops the native one), VoiceOver and TalkBack reading.

## Follow-ups (need server or client work first)

- Mark as read (server, channel, conversation), Mark unread from a message, Mute and Notification settings: no client state or endpoint for them yet (ROADMAP item 13, notifications).
- Close a one-to-one conversation: the server has no "hide this conversation" endpoint.
- Copy message link, server or channel link: Mellow has no permalink route yet.
- Select text (F2) from DESIGN §5.4, the Enter shortcut that opens a focused message's menu, and the message keybinds (R, E, Ctrl+C): client-only, but not part of this item.
- Profile from a message's author outside a server's member list, and moderation (kick, ban, roles) from a member's menu: the actions exist in server settings; bringing them here needs the profile popover of §5.7.
- The author's name is not focusable, so its menu is pointer-only; keyboard users reach the same person from the member list or the conversation row.
