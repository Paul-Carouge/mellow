# Desktop (GPUI): the conversation view

M3 step 3 (ADR-0051). `crates/mellow-desktop/src/conversation/` fills the shell's conversation slot with the open channel's or conversation's messages and the composer. It ports `ChatPane.svelte`, `MessageList.svelte`, `message-actions.ts`, `messageMenuActions` (`context-menus.ts`), `PinnedMessagesPanel.svelte` and the parts of `client.svelte.ts` they call, on the message rows of PR #292 and the composer of PR #291. It adds no token: every length and colour comes from DESIGN.md's existing components (`component.message.*`, `component.action-bar.*`, `component.jump-bar.*`, `component.pins.*`, `component.composer.typing-*`, `component.unread-divider.*`).

## Brief

People who keep Mellow open all day and read more than they write. The view's job is to put them at the right message (the latest, or the first one they have not seen), keep the rows they are reading still while history loads or messages arrive, and make the everyday actions (reply, edit, react, copy) one gesture away with the mouse and with the keyboard.

## Structure

```
 conversation slot (below the chat header)                       pins panel, from the header's pin
 ┌──────────────────────────────────────────────────────────┐    ┌───────────────────────────────┐
 │ ╭ 3 new messages since today at 14:32 ── Mark as read ╮  │    │ 📌 Pinned messages          ✕ │
 │ history start: hero / skeletons / "Couldn't load" + Try   │    │ ╭ avatar Ana · Today at 9:02 ╮│
 │ ── Today ─────────────────────────────────── (day rule)   │    │ │ the pinned text, 4 lines   ││
 │ ◯ Ana  Today at 14:30                    ╭✎ ⧉ ⋯╮ (hover)  │    │ │ [Jump] [Unpin]             ││
 │   message rows, author groups, reactions, replies         │    │ ╰────────────────────────────╯│
 │ ── New ─────────────────────────── (first unread)         │    └───────────────────────────────┘
 │   …                                                       │     420 wide, under the header's
 │ ╭ You're viewing older messages ── ⌄ Jump to present · 3 ╮│     trailing end, as on the web
 │ Ana is typing…                                  (24 high) │
 │ Couldn't edit the message: …           (last action error)│
 │ ╭ composer, base row as tall as the user card (56) ─────╮ │
 └──────────────────────────────────────────────────────────┘
```

- `conversation/mod.rs`: `ConversationView`, owned by `DuskShell`, told the place by `set_place` and applying it in its next render (which has the window): the previous channel's visit ends (`trim_history`, read positions sent), the composer switches drafts, the history is asked for. It turns `ComposerEvent`s into the session's requests and keeps the visit's unread marker, the menu, the confirmation dialog and the pins panel.
- `conversation/rows.rs`: rows from client-core's `build_message_rows` (author groups, day and unread dividers) drawn as `MessageItem` with `MessageBodyCache`; each row's key and version, so the virtual list splices and remeasures only what changed (`rules::list_update`); the hover bar and right-click.
- `conversation/chrome.rs`: the list with its keys, the unread bar, the jump bar, the typing line, the action error and the composer.
- `conversation/pins_panel.rs`: the pins popover.
- `conversation/rules.rs`: the pure rules, with the web's tests ported.
- `session/messages.rs`: the requests (history pages, sends with files, edits, deletions, reactions, pins, typing, read positions) with the web's guards, and `SendRefused`.

## Behaviour

- **The list** is GPUI's `list` with `ListAlignment::Bottom` and `FollowMode::Tail`: it opens at the latest message and follows it while there; scrolling up stops following and back at the end follows again. The history start (hero, skeletons after `motion.delay.skeleton`, or the failure with "Try again") is item 0, so older pages land between it and the first old row, whose index shift keeps the rows being read in place; a page arriving while the reader is inside the history start re-anchors on the first old row.
- **Older history** loads when the top is within `component.message-history.load-distance` (800), one page at a time, never retried by itself after a failure (the web's rule), and a short first page keeps loading until the list fills or reaches the start.
- **Read state**: the unread marker is read once the messages are here, before anything is marked read; the list opens on the "New" divider and stays at the latest message when the unread ones fit. The channel is marked read while its latest message shows in a window in front, and when Esc is pressed in an empty composer or "Mark as read" on the unread bar. Positions reach the instance at most once per second (`READ_STATE_SEND_DELAY`), and at once when the channel is left.
- **Sending** goes through `Session::send_message` with the uploaded files' ids (client-core's outbox now carries them). A send the instance refuses for good gives its text back to an empty composer with the reason above it; when the composer holds something else, the message stays failed in the list with Retry and Delete, so nothing typed is lost.
- **Actions**: the hover bar has Edit (own) or Reply, Copy text, Delete while Shift is held (a click on it skips the confirmation, as on the web) and More; More, a right-click and Shift+F10 open the menu: the existing reactions to toggle, Reply, Edit, Copy text, Pin or Unpin, Copy message ID, then Delete. Permissions come from client-core's ADR-0007 bits only to adapt the interface. Delete and Pin/Unpin confirm in a `ModalDialog`; failures show above the composer, only in the channel they belong to.
- **Keyboard**: the list is one Tab stop; Up and Down move between messages (the focused row shows its ring and its hover bar, which keeps its buttons reachable by Tab); Page Up and Page Down scroll most of a screen; Ctrl/Cmd+Home and End go to the start and the present; Esc jumps to the present and puts focus in the composer; a printable key goes to the composer with its character.

## Defaults rejected

- **A side-sheet pins panel.** The track asked for the side sheet; the tokens (`component.pins.*`, a 420 wide card with radius, border and shadow) and the web draw a popover under the header's pin button, and the side sheet's docking is the member list's, remembered per server. The panel stays a popover, the same as the web.
- **Marking a channel read when it opens.** The web does; the desktop marks it when its latest message shows, so a long unread run opened on its divider stays unread until the reader gets to the end or presses Esc.
- **A typing frame on the gateway.** There is none in the protocol: typing goes through `POST /channels/{id}/typing`, as on the web.
- **Quick reactions in the hover bar and "Add reaction".** Both need the emoji picker (step 5); the menu toggles the reactions a message already has, and the pills toggle through REST with client-core's optimistic rules.

## Accessibility

- The list is a `List` named "Messages in #general", each message a `ListItem`; the hover bar is a `Toolbar` named "Message actions" with labelled buttons; the history start is a named group, its loading a `Status`, its failure an `Alert`; the typing line, the unread bar and copies are polite `Status` regions; the pins panel is a `Dialog` that takes focus and closes with Esc.
- Focus is visible on rows (inset ring), bar buttons, the jump bar and "Mark as read"; nothing is reached by pointer only (Shift+F10 opens a row's menu).
- No motion was added; colour changes go through `colour_transition`, which reduced motion turns into cuts.

## Not yet (named in the code with `shortcut:`)

Forwards, quick reactions, the emoji and GIF pickers, polls, checklists and progress bars as cards, link previews (steps 5); folding blocked authors' runs (ADR-0025); the delete dialog's message preview; who reacted (reaction tooltips and dialog); a jump to a pinned message not loaded yet (loading history up to it); the typing dots' animation; drag-and-drop over the whole conversation (the composer takes drops); new messages read out by screen readers.
