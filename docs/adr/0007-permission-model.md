# 0007. Use a bitfield permission model with role and channel overwrites

- Status: Accepted
- Date: 2026-10-19

## Context
Communities need roles, per-channel restrictions and moderation. Users coming from Discord already understand role-based permissions with channel overrides.

## Decision
Permissions are a 64-bit bitfield. A member's effective permissions in a channel are computed by one pure function: owner gets everything; otherwise the union of `@everyone` and the member's roles; `ADMINISTRATOR` grants everything; then channel overwrites are applied in order `@everyone`, roles (denies then allows), member. Role hierarchy (`position`) limits who can manage whom. Every domain use case checks permissions through this function, and the gateway uses it to filter events.

## Implementation status
- M1.3a: `domain::permissions::guild_permissions` implements steps 1, 3 and the @everyone part of 2 (owner, administrator, @everyone role). The @everyone role shares its guild's id, as on Discord. Non-members get `NOT_FOUND`, never `MISSING_PERMISSIONS`, so guild existence is not revealed. Named roles, role hierarchy and channel overwrites arrive with M2.
- To do with M2: filter `GET /guilds/{id}` channels by `VIEW_CHANNEL` once overwrites exist; check bans when joining with an invite; decide whether invites created by a member who left or was kicked stay valid (they do today, as on Discord).

## Consequences
- Familiar mental model for users; simple and fast to compute.
- One function to test exhaustively and to audit.
- 64 bits leave room for future permissions (voice, threads).

## Alternatives considered
- **ACL per resource:** more flexible, much harder to understand for community admins.
- **Fixed roles (admin/mod/member):** too limited for real communities.
