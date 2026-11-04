# Role mentions and `@everyone` on the web (ROADMAP item 18, ADR-0040)

Status: designed and built on `feat/web-role-mentions` (on top of `feat/role-mentions-server`), DESIGN.md v0.47. Web client: the role chip in `MessageMarkdown.svelte`, parsing in `markdown.ts` and `mentions.ts`, rules in `role-mentions.ts` (labels, plain text, who may mention what, suggestions, `@Name` to `<@&id>`), chip colours in `role-colors.ts` (`roleMentionChipColors`) and `name-colors.svelte.ts`, "mentions me" in `lib/reader-mentions.ts`, `MellowClient.knownRoles` and `mentionReaderIn`, the switch in `RoleSettings.svelte`, the draft and the log line in `server-settings.ts`. The server and protocol are ADR-0040's.

## Brief

A community server of 40 people, "Atelier". Paul, who runs it, writes "Réunion ce soir, @Modérateurs et @Équipe son : on prépare le live." and wants exactly those people pinged, and on restart nights "@everyone le serveur redémarre à 22 h.". Ana, a moderator, must see the first message light up for her as if she were named; Bob, who holds no role, must not get a ping, and must not be able to ping everyone himself. The job: **reach a group by its name, and see at a glance which group a message called**.

The instance decides who is reached (ADR-0040): `Message.mention_role_ids` lists the roles kept and the guild's id for `@everyone`; a role is kept when it is mentionable or the author holds `MENTION_EVERYONE` in the channel.

## Flow and states

1. **Writing.** `@` opens "Mentions correspondant à @" above the field: the people first (unchanged, v0.42), then `@everyone` with "Notifie tout le salon" when the writer holds "Mentionner @everyone et tous les rôles" in the channel, then the roles they may mention (mentionable ones, or every role with that permission), highest first, each with its colour dot and "Rôle". Typing narrows them like people (a name starting with the query, then a word of it, then a name holding it; case and accents ignored), ten roles at most. Nothing about roles in a conversation.
2. **Choosing a role** writes `@Équipe son ` in the field, readable while writing. On send, and when an edit is saved, the `@Name` of a role the writer may mention becomes `<@&id>`, outside code, not after a backslash, longest name first; a name that is someone's username, or "everyone", is never converted, so picking a person keeps naming that person. The length counter counts the converted text. Typing a role's name by hand converts it the same way, as typed `:name:` becomes an emoji.
3. **Reading.** `<@&id>` is a role chip (below). `@everyone` is an amber chip when the instance kept it; a member without the permission who types it sees it stay plain, as on Discord. A role the client does not know (deleted, or in a server the reader left) reads as plain `@rôle-supprimé`. Inside code the markup stays literal.
4. **Mentioned.** A message that names the reader, mentions a role they hold, or `@everyone` in their server takes the mention tint and bar, notifies them when their "Me notifier pour" level is "Mentions" (and through a mute), and counts in the channel's red badge. Their own message never does, even naming a role they hold. A forward never does (ADR-0039).
5. **Previews.** Reply lines, the pins panel, the delete and pin confirmations, the forward dialog's preview and browser notifications read `@Modérateurs` / `@rôle-supprimé`, never `<@&123>`; a renamed role renames its quotes.
6. **Setting it up.** Role editor, Display tab, under "Afficher les membres à part": "Autoriser tout le monde à @mentionner ce rôle" with "Les membres qui ont « Mentionner @everyone et tous les rôles » peuvent le mentionner quand même.". `@everyone` has no Display tab. The permission itself now reads "Mentionner @everyone et tous les rôles": "Prévenir tout le monde dans un salon en une fois, et mentionner n'importe quel rôle, même ceux que personne d'autre ne peut mentionner." The moderation log reads "Tout le monde peut désormais mentionner ce rôle" or "Seuls les membres autorisés à mentionner tous les rôles peuvent désormais le mentionner".

```
Réunion ce soir, [@Modérateurs] et [@Équipe son] : on prépare le live.
                  ^ blue wash,     ^ yellow wash, name darkened (light) or kept (dark)
                    name in the role's blue, 4.5:1 on its wash
[@everyone] le serveur redémarre à 22 h.      amber chip, the people's
Merci @rôle-supprimé                           plain text: nothing to point at

┌ Mentions correspondant à @ ────────────────┐   the composer's list, 6.58 rows
│ (A) ana  @ana                              │
│ (B) bob  @bob                              │
│  ⚇  @everyone  Notifie tout le salon       │   only with the permission
│  ●  @Modérateurs  Rôle                     │   the role's colour
│  ●  @Lecteurs  Rôle                        │   neutral dot: no colour
└────────────────────────────────────────────┘
```

## Design plan

- **Palette**: no new token. The role chip is computed from the role's colour (data, not a design value) against `color.surface.chat`: background = sheet + 20 % of the role colour (`ROLE_MENTION_WASH_WEIGHT`), text = `readableRoleColor` on that background (4.5:1, moved in OKLCH lightness only, so the hue stays the admin's). Measured: blue `#3498DB` 4.57 (dark) / 4.60 (light), yellow `#F1C40F` 6.36 / 4.53, purple `#6A1B9A` 4.53 / 6.49, teal `#1ABC9C` 4.98 / 4.52, pink `#E91E63` 4.53 / 4.55 (unit test: every colour, both themes, ≥ 4.5). Hover swaps text and background, the same ratio. Roles without colour and `@everyone` use `mention.chip-*` (7.24 / 7.18). Dots in the suggestions: the role colour, or `component.roles.neutral-color`, decorative (the name is there).
- **Type**: the chips keep the people's chip (`font-weight.medium`, `component.mention-chip.radius` and padding); the suggestion rows keep 6.58's `type-body` and `text.secondary` for the second word ("Rôle", the hint).
- **Shape**: chips are the people's chips; the suggestions' leading slot is the 24 px avatar box, holding a 12 px dot (`component.roles.row-dot-size`, the dot the role list and the member dialog use) or the 18 px `users` glyph (`component.context-menu.item-icon-size`), so every name starts on the same line.
- **Motion**: the chip's colour transition on hover (`motion.duration.fast`), zero under reduced motion like every token duration.
- **Layout**: nothing moves; the composer's list grows by the roles and scrolls inside its existing maximum height.

## Defaults rejected

- **One amber chip for every mention** (what the people's chip does): a role's colour is how members already recognise it in names and the member list; a blue "Modérateurs" chip says which group was called before the name is read. Amber stays for people and `@everyone`, "you and everyone, now".
- **Discord's translucent role colour at 10 % over whatever is behind**: on the mention-tinted row and the search result card the text contrast would depend on the row; an opaque, computed wash keeps 4.5:1 everywhere.
- **Raw markup in the field** (`<@&111536…>`): unreadable while writing. Mellow writes `@Name` and converts on send, as it already does for `:name:`.
- **A greyed chip for a deleted role**: a chip suggests something to point at; plain text says there is nothing left.
- **Showing every role in the suggestions to everyone**: offering a role that will not ping is a trap; the list shows what the writer can actually reach, as the instance decides.
- **The full "Notifie tous ceux qui voient ce salon"**: truncated at every width in the list's 320 px; "Notifie tout le salon" says the same in the room there is.

## Accessibility

- Chips are text (`@Modérateurs`), so screen readers read the role's name; colour never carries the meaning alone (the `@` and the name do). Contrast as above, checked by a unit test for both themes and by eye in headless Chrome.
- The suggestions keep the composer's combobox pattern (`aria-activedescendant`, a polite count, now "3 mentions" / « 3 mentions »); the dot and glyph are `aria-hidden`; the hint is text, with a `title` when it is cut.
- The switch is the existing `Switch`, labelled by its title and described by its sentence.
- Checked in headless Chrome (Playwright) against a local instance seeded with four people and four roles: French dark and English light at 1280 × 800, French light at 320 × 800; the chat, the suggestions as a member without the permission (two mentionable roles, no `@everyone`) and as the owner (everything), a sent role mention, the role editor and the moderation log after flipping the switch.

## Not done

- A role chip is not focusable and opens nothing (Discord shows the role's members on click); the people's chips do not either yet.
- The message editor still shows `<@&id>` for a role mention already sent (as it shows `<:name:id>` for emoji).
- `@here`, and message search's `mentions:` for roles (ADR-0040).
- A role of another server named in a forward shows its own name if the reader is in that server, `@rôle-supprimé` otherwise: roles are looked up across the reader's servers by id.
- The composer converts `@Name` to `<@&id>` by name when the message is sent, not by the role picked: of two roles with the same name (case ignored) the higher one is mentioned, and a role renamed or made unmentionable between the pick and the send leaves `@OldName` as plain text that notifies no one. Keeping the picked ids with the draft would fix both.
