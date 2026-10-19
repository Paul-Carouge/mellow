# 0007. Use a bitfield permission model with role and channel overwrites

- Status: Accepted
- Date: 2026-10-19

## Context
Communities need roles, per-channel restrictions and moderation. Users coming from Discord already understand role-based permissions with channel overrides.

## Decision
Permissions are a 64-bit bitfield. A member's effective permissions in a channel are computed by one pure function: owner gets everything; otherwise the union of `@everyone` and the member's roles; `ADMINISTRATOR` grants everything; then channel overwrites are applied in order `@everyone`, roles (denies then allows), member. Role hierarchy (`position`) limits who can manage whom. Every domain use case checks permissions through this function, and the gateway uses it to filter events.

## Consequences
- Familiar mental model for users; simple and fast to compute.
- One function to test exhaustively and to audit.
- 64 bits leave room for future permissions (voice, threads).

## Alternatives considered
- **ACL per resource:** more flexible, much harder to understand for community admins.
- **Fixed roles (admin/mod/member):** too limited for real communities.
