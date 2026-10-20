# 0015. Hide channels a member cannot view, in snapshots and in the gateway fan-out

- Status: Accepted
- Date: 2026-10-20

## Context
Channel permission overwrites (ADR-0007 step 4) let a guild hide a channel from some members: staff rooms, private projects. Until now every member of a guild received every channel in its snapshots and every event of the guild over the gateway, serialized once per event (ARCHITECTURE §4.8). Once a channel can be hidden, a member without `VIEW_CHANNEL` must learn nothing about it: not its name, not its messages, not who types in it. Permission changes (an overwrite, a role's permissions, a member's roles) can hide or reveal channels for members who are online.

## Decision
- **REST**: a channel the member cannot view answers `NOT_FOUND`, like a channel of a guild they are not in. Guild snapshots (`GET /guilds/{id}`, `Ready`, `GUILD_CREATE`) list only the channels the member can view, read states only for those channels, and the audit log only entries about those channels (unlike Discord); entries about deleted channels are for the owner and administrators.
- **Gateway fan-out**: events about one channel (messages, typing, channel create, update and delete) go to the members who can view it. When `@everyone` can view the channel and no overwrite denies `VIEW_CHANNEL`, every member can view it and the event goes to the whole guild as before, serialized once. Otherwise the domain computes the set of members who can view it, with the shared computation, and the hub delivers the event, still serialized once, only to their sessions. The audience is computed from the channel's overwrites read again just before the write (and, for messages, before taking the send lock), so a failure leaves nothing half done and publishing after the commit cannot fail.
- **Permission changes**: after a change that can hide or reveal channels, each affected online member receives a fresh `GUILD_CREATE` snapshot, which replaces the guild on the client. For an overwrite change, the affected members are those who start or stop seeing the channel (viewers before and after); those who keep seeing it receive a `CHANNEL_UPDATE` with the new overwrites, and an unchanged overwrite sends nothing. A member's role change affects that member only. Editing a role's permissions or deleting a role (guild-wide `MANAGE_ROLES` only) affects every online member. A member whose snapshot cannot be built is skipped and catches up at their next `Ready`.
- **Clients** ignore message, typing and channel-update events for channels their latest snapshot does not list, so an event racing the snapshot that hid its channel cannot bring it back.

## Consequences
- Hidden channels never leave the server for members who cannot view them, whatever the client.
- A public channel costs two more queries per event (its overwrites read again, the guild's roles). A restricted channel adds the guild owner and two queries over every member and role assignment of the guild, unbounded by the member list cap. For messages this runs before the instance-wide send lock, not under it. To measure in the M4 load test (performance-optimizer) and cache per guild if needed.
- Permission changes cost one snapshot (four queries) per affected online member, serially in the request. Overwrite changes reach only the members whose view changes; role edits and deletions reach every online member, but need guild-wide `MANAGE_ROLES`.
- Accepted window: a message sent while its channel is being hidden can still reach a member whose `GUILD_CREATE` is on its way; their client drops it, as above.

## Alternatives considered
- **Send every channel and let clients hide them:** leaks names and topics of private channels.
- **Per-session permission cache in the hub:** faster fan-out for restricted channels, but every role, member and overwrite change must update it consistently; not worth it before measurements show the need.
- **Diff the visible channels per member on each change and send `CHANNEL_CREATE`/`CHANNEL_DELETE`:** needs the visibility before and after for each member; a fresh snapshot is simpler and always correct.
