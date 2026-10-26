# 0025. Friends, friend requests and blocking

- Status: Accepted
- Date: 2026-10-26
- Amends: the direct-message rule of ADR-0007 (`direct_message_permissions`)

## Context
The owner asked for Discord's friends (ROADMAP "Next, in order" item 7): friend requests, a friend list with presence, removing, blocking, and a Friends space above the direct messages. Until now a direct message could only be opened and written while both people share a guild, so accounts cannot be found or spammed by id, and presence only reaches people who share a guild. Friends change who may reach whom, so the rules are written down here.

## Decision
- **Relationships** are private to each user. Each user has at most one relationship with another user: `friend`, `incoming_request`, `outgoing_request` or `blocked`. Both sides are stored and written in one transaction, so a friendship or a pending request is always symmetric; a block is one-sided.
- **Sending a request** names a username (case-insensitive, as at sign-in), never an id. If that person already asked you, the request accepts theirs. A missing account, yourself, and someone who blocked you all answer the same "cannot send" error, so a request tells nothing about blocks; whether an account exists stays as discoverable as Discord makes it, bounded by a per-user rate limit and a cap on relationships.
- **Answering**: the recipient accepts (both become friends) or refuses; the sender cancels; either friend removes the friendship. Each change reaches both people's sessions (`RELATIONSHIP_ADD`, `RELATIONSHIP_REMOVE`).
- **Blocking** replaces any friendship or request with a one-sided `blocked` row. The blocked person only sees the friendship or request disappear, never the block. While either has blocked the other, no direct message can be opened or written between them, and no friend request can be sent either way. In guilds nothing changes on the server: the web client collapses messages from people you blocked, as Discord does. Unblocking removes the row.
- **Direct messages** may be opened and written by two people who share a guild or are friends, unless either blocked the other. `direct_message_permissions` stays the only decision, with "may reach each other" in place of "shares a guild". A conversation whose people can no longer reach each other stays readable, as before.
- **Presence** of friends reaches each other even without a shared guild; invisible still shows as offline. Accepting a request sends both their current presence.
- **Limits**: caps on friends, outgoing and incoming requests (below), and friend requests are rate limited per user.

## Consequences
- One new table (migration 18), REST routes under `/api/v1/users/@me/relationships`, `Ready.relationships` and two gateway events; older clients ignore them.
- Caps (after the security review): at most 1 000 friends and outgoing requests per user, and 200 requests waiting for one person's answer, past which new requests to them are refused like a block; blocks never count, so blocking always works and strangers cannot fill someone's list.
- Renames and new avatars reach friends who share no guild; ending a friendship sends each side the other's "offline" unless a shared guild keeps them informed.
- People who share a guild can still infer a block: a refused conversation can only mean one, as on Discord.
- The event hub keeps each online user's friend ids to fan presence out to them, updated on every relationship change.
- Group conversations (ROADMAP item 8) will reuse "may reach each other" for every pair of participants.

## Alternatives considered
- **Requests by user id:** ids appear in guilds, so anyone in a shared guild could already be asked; usernames are what people share, and ids would let anyone probe accounts quickly.
- **Telling the sender they are blocked:** Discord does not, and it would turn a block into harassment feedback.
- **Hiding blocked users' guild messages on the server:** it would change history per viewer and break shared context; a client-side collapse is what people expect.
