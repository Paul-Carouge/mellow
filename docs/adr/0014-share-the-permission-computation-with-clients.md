# 0014. Share the permission computation with clients

- Status: Accepted
- Date: 2026-10-20

## Context
ADR-0007 puts the permission computation in one pure function in `mellow-server::domain`. Until M2 the only input was the `@everyone` role, so the server sent each member a precomputed `my_permissions` per guild and clients never computed anything. Named roles change that: editing one role, or giving it to a member, changes the permissions of every member holding it. The server would have to recompute and push a fresh value to each affected member, and once channel overwrites exist (ADR-0007 step 4) a value per channel too. Clients still need these permissions to hide what the member cannot do (the composer of a read-only channel, a "Manage roles" entry, a hidden channel).

## Decision
- The pure computation moves to `mellow-protocol` (`permissions::compute_guild_permissions`, and later the channel step). It has no I/O and depends only on wire types, so it fits the protocol crate (ADR-0001).
- The server's `domain::permissions` stays the only place that **decides**: every use case calls it, and it calls the shared computation. Handlers, the gateway and storage still never decide on their own.
- Clients call the same computation on the roles and member role ids they hold, only to adapt the interface. A wrong client result can hide or show a button, never grant anything: the server checks every request.
- The server keeps sending `Guild::my_permissions` in snapshots (`Ready`, `GUILD_CREATE`, `GET /guilds/{id}`), and clients recompute it after role and member events (`GUILD_ROLES_UPDATE`, which carries the guild's full role list, and `GUILD_MEMBER_UPDATE`).

## Consequences
- Role events stay one serialized event per guild; fan-out does not grow with the number of members a role change affects.
- Server and clients cannot drift apart: they run the same code, and the exhaustive tests live next to it.
- Changing the computation is a protocol change: it ships in the same commit for server and clients, like any wire type.

## Alternatives considered
- **Push recomputed permissions to each affected member:** one query and one event per member for each role edit, and per channel once overwrites exist; costly on large guilds and easy to get wrong.
- **Duplicate the computation in the client core:** two implementations of the most security-relevant function would drift.
