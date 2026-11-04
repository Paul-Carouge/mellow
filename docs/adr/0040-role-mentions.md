# 0040. Role mentions and `@everyone`

- Status: Accepted
- Date: 2026-11-04

## Context
ROADMAP item 18 asks for role mentions (`@role`), "with a permission deciding who may mention each role" (owner, 2026-11-03, #roadmap). Mentions so far name people only: the server finds `@username` (or a one-word display name) in the text, stores who is mentioned in `message_mentions` and counts unread mentions from it (migration 0016). `MENTION_EVERYONE` (bit 5) exists since M1 but nothing uses it, and `@everyone` mentions no one. Discord answers both with the same pair: a per-role "Allow anyone to @mention this role" switch, and `MENTION_EVERYONE`, which lets its holders mention every role, `@everyone` and `@here`.

## Decision
- **Who may mention a role**: anyone who can send in the channel, when the role is mentionable (`Role.is_mentionable`, set by members who manage the role, recorded in the moderation log as `mentionable`); otherwise only members holding `MENTION_EVERYONE` in that channel (an overwrite can grant or deny it per channel; `ADMINISTRATOR` includes it). `@everyone` always needs `MENTION_EVERYONE` and is never mentionable (`400` on the `@everyone` role). Existing roles start not mentionable (migration 0035), as on Discord. The rule is the author's computed permissions (ADR-0007), checked when the message is sent and again when it is edited.
- **Syntax**: a role is written `<@&id>`, which the composer inserts when a role is picked from the `@` suggestions, so a role renamed later, or named with spaces, still resolves; `@everyone` is the word itself (any case), with the person rule's boundaries and outside code. `@everyone` never mentions a person, even one named "everyone". A mention the author may not make stays plain text that notifies no one, as on Discord. Direct messages and forwards mention no role.
- **Storage per role, not per holder**: `message_role_mentions (role_id, channel_id, message_id)`, `@everyone` stored as the guild's id (its `@everyone` role's id). Mentioning a role of 1 000 members writes one row. `Message.mention_role_ids` carries the ids to clients, which decide "mentions me" from the roles the reader holds, as they already do with `mention_ids`.
- **Unread mention counts** add, per channel, the messages after the read position that mention a role the reader holds now, or the guild, without counting a message twice or one the reader wrote: one more bounded range of the new table's primary key per channel, the reader's roles read from `member_roles`' primary key (checked by `EXPLAIN QUERY PLAN` in a test).

## Consequences
- A role mention reaches whoever holds the role when unread mentions are counted, not when the message was sent: someone given a role later sees older unread mentions of it, as on Discord. Only people who can view the channel are counted (unread counts are computed for viewable channels only).
- Older clients see `<@&id>` as text and ignore `mention_role_ids` and `is_mentionable`; older servers ignore role requests' `is_mentionable` because it is left out when off or unchanged.
- No `@here` yet (it needs who is online when the message is sent); message search's `mentions:` filter still names people only.

## Alternatives considered
- **Expanding a role mention into one `message_mentions` row per holder**: no change to counting, but one `@everyone` in a guild of 1 000 writes 1 000 rows in the send transaction and sends 1 000 ids to every client with the message.
- **A list of roles allowed to mention each role**: finer than Discord, but a new table and screen for a need the per-role switch plus `MENTION_EVERYONE` already covers.
- **Role mentions by name (`@Moderators`)**: role names may contain spaces and change; ids keep a mention stable.
