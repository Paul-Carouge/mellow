# Desktop: polls, checklists, progress bars and the "/" menu

M3 step 5 for the GPUI client (ADR-0051). The desktop ports the web's polls
([web-polls.md](web-polls.md), ADR-0043), checklists and progress bars
([web-checklists-and-progress.md](web-checklists-and-progress.md), ADR-0047) and the composer's
"/" menu ([web-slash-menu.md](web-slash-menu.md)). Nothing new is designed here: the web's cards,
words, rules and dialogs, drawn with the desktop's Dusk primitives and the existing
`component.poll.*` tokens. No token or icon was added.

## Rules

The UI-independent rules live in client-core, with the web's tests ported:

- `shared_logic::poll_form` (`poll-form.ts`): the draft, its problems (empty, too long, a repeat
  on its later rows), what is sent, two to ten rows.
- `shared_logic::poll_view` (`poll-view.ts`): results (rounded shares, leaders), time left
  (minutes, hours, days from 48 hours, always rounded down), the end time, the outbox preview.
- `shared_logic::tracker_form` (`tracker-form.ts`, `tracker-view.ts`): the checklist and
  progress bar drafts, whole numbers from 0 to the maximum, outbox previews.
- `shared_logic::slash_commands` (`slash-commands.ts`): the "/query" that starts a line (never
  inside a word, an address or a code block), ranking with English names and keywords, and what
  each command writes and selects.
- `state::outbox` carries a poll, checklist or progress bar (`MessageCard`) through the outbox,
  so a card shows as it will arrive, then "Not sent" with Retry and Delete.
- `context_menus_people::message_menu_actions` offers "End poll" to the author of an open poll,
  in the danger section before Delete.

Conversation permissions now match the server's `direct_message_permissions` (reactions, polls
and checklists were missing in client-core).

## Cards

```
┌──────────────────────────────────────────┐   432 px at most, the poll's well
│ Lunch?                                   │   heading-md
│ Choose one answer                        │   hidden once ended
│ ┌──────────────────────────────┬───────┐ │
│ │▓▓▓▓▓▓ ● Pizza                │2 votes│ │   your answer: apricot edge and fill;
│ │▓▓▓▓▓▓                        │  67 % │ │   the column opens who voted
│ └──────────────────────────────┴───────┘ │
│ 3 votes · Ends in 3 hours   Remove my vote│  exact date in the tooltip
└──────────────────────────────────────────┘
```

- A single-choice poll is one Tab stop (your answer, else the first); arrows move without
  voting; Space, Enter or a click votes. A multiple-choice poll is a group of checkboxes.
- The bar grows from nothing when results show and follows each vote over
  `motion.duration.slow`; at once under reduced motion (the duration token is 0).
- One poll clock per conversation view ticks once a minute while an open poll is drawn, and
  stops at the next tick once none is (a test drives it on the executor's clock).
- Checklists: rows are checkboxes 44 px tall with "Ticked by Ada, …" under a ticked item; "Edit
  list" turns the rows into fields with Remove and a new-item field (one checklist at a time).
  Progress bars: the level and "Update", which opens the dialog filled in.
- A level says its new value through a polite live region once it changes, never the first one.

## Dialogs

`ModalDialog` with `TextField`s: question or title with its character counter, answer or item
rows (Enter goes to the next and adds one after the last, never sends), Add and Remove, "Allow
multiple answers" (`Switch`) and the duration (`ChoiceGroup`, one radio row per duration).
Repeats and too long texts are said as typed; missing ones on the action, which focuses the
first field to fix.

## The "/" menu

A separate list from the ":" and "@" suggestions (`composer/slash_menu.rs`), 416 px wide, with
the glyph, the name, the description and the Markdown chip. The poll is offered with "Send
polls", the checklist and the progress bar with "Send checklists". Not in edit mode.

## Not done

- The web's compact results column under 360 px of card; the duration as a drop-down.
- Undo of a command in one step (the field is rewritten whole), and Ctrl+B / I / U / Shift+X.
- Focus after removing a checklist item goes to the new-item field rather than the next item.
- Not looked at on screen signed in; not tried with a screen reader.
