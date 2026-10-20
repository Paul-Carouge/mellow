# 0015. Hide channels a member cannot view, in snapshots and in the gateway fan-out

- Status: Accepted
- Date: 2026-10-20

## Context
Channel permission overwrites (ADR-0007 step 4) let a guild hide a channel from some members: staff rooms, private projects. Until now every member of a guild received every channel in its snapshots and every event of the guild over the gateway, serialized once per event (ARCHITECTURE §4.8). Once a channel can be hidden, a member without `VIEW_CHANNEL` must learn nothing about it: not its name, not its messages, not who types in it. Permission changes (an overwrite, a role's permissions, a member's roles) can hide or reveal channels for members who are online.

## Decision
- **REST**: a channel the member cannot view answers `NOT_FOUND`, like a channel of a guild they are not in. Guild snapshots (`GET /guilds/{id}`, `Ready`, `GUILD_CREATE`) list only the channels the member can view, and read states only for those channels.
- **Gateway fan-out**: events about one channel (messages, typing, channel create and update) go to the members who can view it. When `@everyone` can view the channel and no overwrite denies `VIEW_CHANNEL`, every member can view it and the event goes to the whole guild as before, serialized once. Otherwise the domain computes the set of members who can view it, with the shared computation, and the hub delivers the event, still serialized once, only to their sessions.
- **Permission changes**: after a change that can hide or reveal channels (setting or removing an overwrite, editing or deleting a role, giving or taking a role), each affected online member receives a fresh `GUILD_CREATE` snapshot, which replaces the guild on the client. A member's role change affects that member only; the other changes affect every online member of the guild. A channel deleted while hidden from a member still sends them nothing.

## Consequences
- Hidden channels never leave the server for members who cannot view them, whatever the client.
- Public channels keep the existing cost: one query for the channel and one serialization per event.
- A restricted channel costs one more query per event, listing the guild's members and their roles. This is bounded by the member list cap and measured before large guilds become a target (performance-optimizer, M4 load test).
- Permission changes cost one snapshot query per affected online member. They are rare, need `MANAGE_ROLES` and are rate limited.

## Alternatives considered
- **Send every channel and let clients hide them:** leaks names and topics of private channels.
- **Per-session permission cache in the hub:** faster fan-out for restricted channels, but every role, member and overwrite change must update it consistently; not worth it before measurements show the need.
- **Diff the visible channels per member on each change and send `CHANNEL_CREATE`/`CHANNEL_DELETE`:** needs the visibility before and after for each member; a fresh snapshot is simpler and always correct.
