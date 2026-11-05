# 0043. Polls in messages

- Status: Accepted
- Date: 2026-11-05

## Context
ROADMAP item 18 asks for "polls, checklists and progress bars, with their permissions" (owner's #roadmap). Polls come first: they are what Discord has, they need the server (votes are shared state), and checklists can reuse their storage later. Discord's polls set the expectations: a question, up to ten answers, one or several choices, a duration, votes anyone who reads the channel can change until the end, the author ending it early, and a permission to create them.

## Decision
- **A poll is a message**: `POST /api/v1/channels/{channel_id}/messages` with `poll: {question, answers, allows_multiple_answers, duration_hours}` instead of text, files, a reply or a forward. Question 1 to 300 characters, 2 to 10 answers of 1 to 55 characters each (the message rules for invisible and control characters apply), a duration of 1, 4, 8, 24, 72, 168 or 336 hours. A poll message cannot be edited or forwarded; it can be pinned, replied to, reacted to and deleted like any message.
- **Permission**: new bit 22, `SEND_POLLS` ("Créer des sondages"), checked with `SEND_MESSAGES` by the domain's permission function (ADR-0007), channel-scoped, granted to every existing `@everyone` by migration 0038 and in `DEFAULT_EVERYONE`, as on Discord. In conversations every participant who may write there creates one (`direct_message_permissions`: not across a block).
- **Votes**: whoever reads a guild channel (`VIEW_CHANNEL` and `READ_MESSAGE_HISTORY`), or a participant who may still write in a conversation (`may_vote_in_poll`: someone blocked cannot keep notifying with votes, as with reactions), votes with `PUT …/messages/{message_id}/poll/answers/{answer_id}/votes/@me` and takes it back with `DELETE`, until the poll ends; a single-choice poll moves the vote. `MESSAGE_POLL_VOTE_ADD` and `MESSAGE_POLL_VOTE_REMOVE` carry the voter and the answer to the message's audience, as reactions do; REST answers carry each answer's count and whether the reader voted for it (`me_voted`). `GET …/poll/answers/{answer_id}/voters` lists up to 100 voters.
- **The end**: a poll ends at its `expires_at` with no background work (servers and clients compare times), or when its author ends it early (`POST …/messages/{message_id}/poll/end`, a `MESSAGE_UPDATE` with `ended_at`). Votes are refused once it has ended.
- **Storage**: `polls` (one row per message: question, choice mode, expiry, early end), `poll_answers` (2 to 10 rows) and `poll_votes` (one row per voter and answer, primary key `(message_id, user_id, answer_id)`, an index on `(message_id, answer_id)` for counts); all go with the message (`ON DELETE CASCADE`). History, pins and search results get their polls in two queries per page (the polls, then their answers with counts and the reader's votes). Clients take counts from vote events, not from a `MESSAGE_UPDATE` (a pin's or an early end's may have read them before a vote).

## Consequences
- Counts are sent to everyone at once; the web client may hide them until the reader votes or the poll ends, as Discord does, but they are not secret. Who voted for what is visible to readers of the channel, as on Discord: polls are not anonymous.
- A vote is a write in the single writer, like a reaction; votes share the reactions' rate limit.
- Message search does not find a poll's question or answers yet; older clients see an empty message.

## Alternatives considered
- **A poll as a separate object linked to a message**: one more lifecycle (orphans, permissions) for nothing a message does not already give.
- **Anonymous polls**: Discord's are not; a later option if asked, with its own choice of who may see voters.
- **A background job closing polls**: nothing has to happen at the end except refusing votes, which a time comparison does.
