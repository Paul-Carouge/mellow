# 0026. Group conversations

- Status: Accepted
- Date: 2026-10-27
- Amends: [ADR-0025](0025-friends-and-blocking.md) (who may write in a group)

## Context
ROADMAP "Next, in order" item 8 asks for Discord's group direct messages and a "New message" picker. A conversation is already a channel outside any guild with a list of participants (ARCHITECTURE §4.5); one-to-one conversations add a pair so each pair has exactly one. A group needs an owner, an optional name, and rules for who joins, leaves and writes.

## Decision
- **A group** is a conversation with an owner (`group_conversations`, migration 19) and a name (`channels.name`, empty for none). It holds at most 10 people, its creator included. Every new group is a new conversation, even with the same people, as on Discord.
- **Creating** a group names 2 to 9 people, each a friend of the creator (ADR-0025); a request with anyone else answers `NOT_FOUND`, like an unknown account. Every participant receives `DIRECT_MESSAGE_CHANNEL_CREATE` at once, before any message.
- **Adding**: any participant adds a friend of theirs while the group has room; the newcomer reads the whole history, as on Discord. They receive `DIRECT_MESSAGE_CHANNEL_CREATE`, the others `DIRECT_MESSAGE_CHANNEL_UPDATE`.
- **Leaving and removing**: anyone leaves; only the owner removes someone else. The person who goes receives `DIRECT_MESSAGE_CHANNEL_DELETE` and can no longer read it; the others receive `DIRECT_MESSAGE_CHANNEL_UPDATE`. When the owner leaves, the participant with the lowest id becomes owner. When the last person leaves, the conversation and its messages are deleted.
- **Renaming**: any participant, up to 100 characters; an empty name shows the participants' names.
- **Writing**: every participant writes, reacts and attaches. Unlike one-to-one conversations, a group stays open when two of its people stop being friends or one blocks the other: the web client folds the blocked person's messages, as in guilds (ADR-0025). `direct_message_permissions` stays the only decision; for a group, "may reach each other" is simply "is a participant".
- **The "New message" picker** opens a one-to-one conversation for one person, or creates a group for several; it lists the user's friends.

## Consequences
- One new table, routes under `/api/v1/group-conversations`, an `owner_id` and a `name` on `DirectMessageChannel`, and two gateway events (`DIRECT_MESSAGE_CHANNEL_UPDATE`, `DIRECT_MESSAGE_CHANNEL_DELETE`); older clients ignore them and show a group as a conversation with several recipients.
- Groups appear in `Ready` even before their first message.
- Unread counts and mentions follow conversations: every unread message from others counts.
- After the security review: creating a group, or adding someone, is refused like a stranger (`NOT_FOUND`) when the newcomer and anyone in it blocked each other either way, so a mutual friend cannot pull someone into a room with the person they blocked; a block placed inside an existing group leaves it open. Adds and removals are decided inside the write transaction (the actor still in the group, the owner for removals). Unread counts in conversations leave out people the reader blocked.
- No account deletion exists; one added later must hand the groups an account owns to another participant first (the owner's foreign key has no `ON DELETE`).
- shortcut: no system messages ("Ana added Bob") yet; members see who joined from the participant list only.

## Alternatives considered
- **Reusing the one-to-one rule (every pair must reach each other):** one block or unfriending would silence the whole group; Discord keeps groups open.
- **Letting anyone sharing a guild be added:** would let strangers pull people into private groups; friends-only matches Discord and the owner's Friends space.
- **Giving the leaving owner's role to the earliest joiner:** needs join times the participants table does not keep; the lowest id is deterministic and rarely matters.
