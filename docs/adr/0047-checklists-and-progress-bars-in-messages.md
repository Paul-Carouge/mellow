# 0047. Checklists and progress bars in messages

- Status: Accepted
- Date: 2026-11-05

## Context
ROADMAP item 18 ends with the owner's wish for "polls, checklists and progress bars, with their permissions" (#roadmap). Polls are done (ADR-0043). A checklist is shared state like a poll: a title and items the channel ticks together, where who ticked what and when matters. A progress bar is a label and a value its author moves along ("3/5", "40 %"). Discord has neither; their closest equivalents are bots. Both must work in guild channels and in conversations, survive pins, history pages and lost answers, and never let an older copy undo a newer change.

## Decision
- **Both are messages**, like a poll: `POST /api/v1/channels/{channel_id}/messages` with `checklist: {title, items}` or `progress_bar: {label, value, maximum}` instead of text, files, a reply, a forward or a poll. A checklist has a title of 1 to 300 characters and 1 to 25 items of 1 to 200 characters (one line each, the message rules for invisible and control characters apply; items may repeat). A progress bar has a label of 1 to 300 characters, a `maximum` from 1 to 1 000 000 and a `value` from 0 to `maximum`; clients show it as a percentage when `maximum` is 100, "n/m" otherwise. Neither can be edited as text or forwarded (ticks belong to their channel); both can be pinned, replied to, reacted to and deleted like any message. The send validates its nonce like any other.
- **Permission**: new bit 23, `SEND_CHECKLISTS` ("Créer des listes et des barres de progression"), checked with `SEND_MESSAGES` by the domain's permission function (ADR-0007), channel-scoped, granted to every existing `@everyone` by migration 0040 and in `DEFAULT_EVERYONE`, as `SEND_POLLS` was. In conversations every participant who may write there has it (`direct_message_permissions`: not across a block).
- **Ticks**: whoever may write where the checklist is (`VIEW_CHANNEL`, `READ_MESSAGE_HISTORY` and `SEND_MESSAGES`: `may_tick_checklist_items`) ticks with `PUT …/messages/{message_id}/checklist/items/{item_id}/check` and unticks with `DELETE`. A ticked item keeps who ticked it and when (`checked: {user_id, at_unix_milliseconds}`); ticking it again changes nothing, so the first tick stays. A read-only channel's checklists stay its writers', and someone blocked in a conversation cannot keep notifying the other with ticks.
- **Editing**: the author while still allowed to write there, or whoever holds `MANAGE_MESSAGES` in a guild channel (`may_edit_tracker`), adds an item at the end (`POST …/checklist/items`), renames one (`PATCH …/checklist/items/{item_id}`, keeping its tick), removes one (`DELETE …/checklist/items/{item_id}`, never the last), or replaces a progress bar's label and value (`PATCH …/progress-bar` with the same body as at creation). Item ids are never reused (`next_item_id`), so a late tick cannot land on an item added after a removal.
- **Revisions instead of counts**: every change bumps the row's `revision` in the same short write transaction (single writer, ADR-0024), which reads the result back before committing. Answers and the events `MESSAGE_CHECKLIST_UPDATE` and `MESSAGE_PROGRESS_BAR_UPDATE` carry the whole checklist or bar (at most 25 items) with its revision, to the message's audience. Clients keep the copy of the highest revision, whatever brought it (an event, an answer, a pin's `MESSAGE_UPDATE`, a history page), so a copy read before a later change never undoes it. Unlike poll votes, nothing is counted from events: the "3/5" is computed from the ticks.
- **Storage** (migration 0040): `checklists` (title, revision, next item id), `checklist_items` (text, who ticked and when, both or neither) and `progress_bars` (label, value, maximum, revision), all going with their message (`ON DELETE CASCADE`). History, pins and search results get them in at most three queries per page.
- **Rate limit**: ticks and edits share the reactions' write budget.

## Consequences
- Who ticked what is visible to readers of the channel; checklists are not anonymous, as polls are not.
- A checklist's events carry the whole list: at most 25 items of 200 characters, about 6 KB, once per change, serialised once per fan-out.
- Message search does not find a checklist's title or items, nor a bar's label, yet; older clients see an empty message.
- The web client ticks optimistically and undoes a tick only when the instance refuses it (an `ApiError` with a status) and no newer revision came meanwhile; after a lost answer it settles from the instance's copy.

## Alternatives considered
- **Counts from events, as polls**: a tick is not a count but a state (who and when), and item edits change the list itself; ordered deltas would need the same ordering a revision gives for free.
- **A checklist as a message's Markdown (`- [ ]`) ticked by editing**: only the author could tick, and edits would race.
- **A progress bar computed from a checklist**: the owner asked for both; a checklist already shows its own bar.
- **Separate permissions for checklists and progress bars**: one bit covers both, as one feature ("listes et progression") in the role editor.
