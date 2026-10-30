# 0028. Give servers their own emoji, stored as guild images

- Status: Accepted
- Date: 2026-10-30

## Context
The owner wants Discord's custom emoji (ROADMAP item 13): a server uploads small pictures with a name, and its members use them in messages and as reactions. Three things already exist that this must fit: guild and channel icons (migration 0014, ADR-0021), stored as sealed files (ADR-0020), compressed losslessly and served only to members; reactions keyed by a Unicode emoji string (migration 0012); and the permission model (ADR-0007). Owner's decisions of 2026-10-30 set the limits, the syntax and the permission.

## Decision
- **Permission.** A new guild-scoped bit, `MANAGE_EMOJIS = 1 << 17`, needed to add, rename and delete a server's emoji. It is not channel-scoped (an overwrite cannot carry it) and not in `DEFAULT_EVERYONE`; the owner and `ADMINISTRATOR` hold it through the existing rule. No migration grants it: existing roles get it only when someone ticks it.
- **Storage.** Migration 0020 adds `guild_emojis (id, guild_id, name)`. An emoji's picture is a `guild_images` row with the **same id**, so it is served by the existing icon routes (`GET /api/v1/icons/{id}` and `/still`) to members of its guild only, kept by the orphan sweep, and deleted with its guild. An emoji is animated when its image has a still frame.
- **Pictures.** The upload is the raw body, at most **256 KB**; the centred square is kept (no crop parameter), fitted to **128 px** and stored as lossless WebP. An animated GIF or WebP stays animated, with a still first frame, like an animated server icon. Processing goes through `ProfileImages::store` (dimension checks before decoding, the processing permit held inside `spawn_blocking`).
- **Limits.** Per server, **50 still and 50 animated** emoji, counted in the insert's transaction. Names are **2 to 32 ASCII letters, digits and `_`**, unique per server whatever the case (`COLLATE NOCASE` unique index); a taken name is `VALIDATION_FAILED`.
- **API** (`/api/v1`, guild writes rate limit): `GET /guilds/{id}/emojis` (members), `POST /guilds/{id}/emojis?name=…` (picture as body, `201` with the emoji), `PATCH /guilds/{id}/emojis/{emoji_id}` `{ "name" }`, `DELETE /guilds/{id}/emojis/{emoji_id}` (`204`). `Guild.emojis` carries the list (absent from older servers, read as empty), and `GUILD_EMOJIS_UPDATE { guild_id, emojis }` sends the whole new list to every member after each change. Each change is audited: `emoji_create`, `emoji_update`, `emoji_delete`, target the emoji, with a `name` change.
- **In messages.** Discord's syntax, `<:name:id>` and `<a:name:id>` when animated; the web composer turns `:name:` into it. The server stores the text as written and does not check it: a message may name any emoji, and clients show the picture only when they know the id (they are members of its server), `:name:` otherwise.
- **As reactions.** A custom emoji reacts under the key **`name:id`** (the id in its one decimal spelling) in the existing `message_reactions.emoji` column; Unicode keys are unchanged, and no Unicode emoji contains `:`. Adding a reaction with a custom key needs the emoji to exist and to belong to the **server of the channel** (`domain::permissions::may_react_with_custom_emoji`): no custom emoji in conversations or in another server's channels, so everyone who sees the reaction can load its picture. The key takes the emoji's **current** name, whatever name the client sent. Listing, taking back and moderating reactions accept any well-formed key, so reactions with a renamed or deleted emoji can still be removed. `ADD_REACTIONS`, the 20-emoji cap and everything else in `may_react` apply as before.
- **Renaming and deleting.** A rename changes the emoji's name only: messages keep the text they were written with, and reactions keyed under the old name stay under it (new reactions use the new name, so the same picture may show as two reactions). Deleting removes the emoji and its picture: messages keep their text (clients show `:name:`), and reactions with it **stay as they are**, shown by their key's name; they can be taken back but not added to.

## Consequences
- No new route or access rule serves emoji pictures; the icon tests and the orphan sweep already cover them.
- A reaction key stays readable after a rename or deletion, at the cost of a rename splitting old and new reactions with one picture. Rewriting every reaction key at rename time would be an unbounded write inside the single writer's transaction (ADR-0024); revisit if owners rename often.
- Members of another server, and conversation participants, see `:name:` for a custom emoji in a message: the picture is members-only, like icons.
- Older clients skip `GUILD_EMOJIS_UPDATE`, ignore `Guild.emojis`, read the new audit actions as `unknown`, and show a custom reaction's key as text.

## Alternatives considered
- **A separate emoji route and image table.** More code for the same rule (members of the guild read it); reusing `guild_images` keeps one sweep and one access check.
- **Keying reactions by id only.** Stable across renames, but a deleted emoji's reactions would have no name to show. `name:id` keeps the name, as Discord's reaction routes do.
- **Allowing custom reactions anywhere the reactor is a member of the emoji's server.** Other viewers of the channel might not be members and could not load the picture; the same-server rule is the simplest one that is always safe.
- **Validating `<:name:id>` in message text.** It would refuse messages quoting emoji of servers the reader left, for no gain: the text is harmless and clients decide how to show it.
