# Web client: server settings

DESIGN.md §5.8 on the web client, in the Dusk direction ([ADR-0022](../adr/0022-give-mellow-its-own-visual-language-on-a-familiar-layout.md)). This page records the design decisions, so later work (channel settings, the desktop catch-up) keeps the same shape.

## Brief

- **Who**: the owner of a community server and the moderators they trust; they open the settings a few times a week, often on a phone, to fix one thing (a role, a troublemaker, a stale invite).
- **Primary job of each page**: Overview renames; Roles shapes the hierarchy and what each role may do; Members gives roles and removes people; Bans and Invites undo what was done; the moderation log tells who did what.
- **Rule**: the instance decides every action (ADR-0007). The client only hides sections and locks controls the instance would refuse (`server-settings.ts` mirrors `RoleManager` and `outranks`), and always shows why a control is locked.

## Layout

The user settings' layout, unchanged: on wide windows the navigation on the ground and the page as a sheet beside it, with the round close button and "Esc"; below the navigator breakpoint (816 px) the navigation is a page of its own and each section opens with a back button.

```
wide (>= 816 px)                                         phone (< 816 px)
+-------------------------------------------------------+ +----------------------+
| ground          +-----------------------------------+ | | Server settings  (x) |
|  Rust Lyon      | Roles                         (x) | | | Rust Lyon            |
|  Overview       | [Create role]                Esc  | | | Overview             |
| >Roles          | o Moderators  1  | o Moderators   | | | Roles                |
|  Invites        | o Helpers     1  |   ^Up  vDown   | | | ...                  |
|  ------         | o @everyone   4  | Display|Perms  | | +----------------------+
|  Moderation     | hint             | name, colours  | |   then, for a section:
|  Members        |                  | or switches    | | +----------------------+
|  Bans           |                  |                | | | <                (x) |
|  Moderation log | [ Careful, unsaved  Reset  Save ] | | | Roles                |
|  ------         +-----------------------------------+ | | list, then editor    |
|  Delete server                                        | | [unsaved bar]        |
+-------------------------------------------------------+ +----------------------+
```

- **Navigation**: the server's own settings under its name (Overview, Roles, Invites), then "Moderation" (Members, Bans, Moderation log), then "Delete server" for the owner only, in `text.danger`, last. A section shows only with its permission: Overview and Invites `MANAGE_GUILD`, Roles `MANAGE_ROLES`, Members any of `KICK_MEMBERS`, `BAN_MEMBERS`, `MANAGE_ROLES`, Bans `BAN_MEMBERS`, the log `VIEW_AUDIT_LOG`. The menu item shows when one section does, or for the owner.
- **Live**: the page reads the guild from the client state, so `GUILD_UPDATE`, `GUILD_ROLES_UPDATE` and member events show at once; a section whose permission is lost gives way to the next one; a deleted server, or losing every section, closes the settings.
- **Lists** (members, bans, invites, log): rows of `component.server-settings.row-*`, avatar, a name line, a muted second line, one trailing action. Each list starts with static skeleton rows (no shimmer), and a failure says what failed with "Try again".
- **Unsaved changes** (Overview, Roles): the bar of DESIGN.md §5.8 sits at the bottom of the page, sticky. Leaving the page (another section, back, close, Esc, "Delete server", another role) is refused while it shows: the bar turns red-bordered, says "Save or reset your changes before leaving." and shakes (not under reduced motion).
- **Roles**: the hierarchy with colour dots and member counts, locked rows with a lock; the editor with "Move up" and "Move down" (also `Ctrl+Shift+Up/Down` on a row), tabs "Display" (name, "No colour" with its name in view, then the presets in rows of up to ten, a preview of a name in the readable colour) and "Permissions" (Discord's groups, a switch per permission, "Sensitive" with an icon, locked switches explained once). A draft keeps only the user's own changes on top of a role changed meanwhile (`rebaseRoleEdit`), so another manager's change is never undone and our own save, echoed by the gateway, ends the edit. Moving a role saves at once.
- **Members**: search by display name or username (case and accents ignored), role chips, and "Manage" opening a dialog with the roles as switches (applied at once, as on Discord) and "Kick Ana" / "Ban Ana", each confirmed with the copy of §8; the ban dialog takes an optional reason.
- **Delete server**: the owner types the server's name before the danger button wakes up (§8: typed confirmation for irreversible actions affecting many people).

## Tokens

New group `component.server-settings` (rows, chips, loading rows), built from checked semantic pairs: chips are `text.heading` on `state.selected`, second lines `text.muted` on `surface.chat`. Everything else reuses `component.settings`, `roles`, `tabs`, `switch`, `unsaved-bar` and `dialog`.

## Defaults rejected

- **A data table for invites and bans**: tables collapse badly on phones and hide the one action each row has. Rows with two short lines and one button read the same on both sizes.
- **Remove buttons on role chips** (Discord's small ×): a 16 px target on a chip misses the 44 px floor. Roles change from the member's dialog, where each role is a full-width switch row.
- **A `<select>` to add a role**: it cannot show locked roles and why; switches can.
- **Raw JSON-ish audit entries** ("ROLE_UPDATE target=…"): each entry is one sentence in the reader's language, with the changed fields named ("Granted: Kick members"), and people or roles that are gone named as such.
- **Nickname permissions**: nicknames do not exist yet, so their switches would do nothing; their bits stay as stored.

## Follow-ups

- Channel settings (rename, topic, delete, permission overwrites with allow, inherit and deny) from a channel's context.
- A role's "Members" tab, drag-and-drop reordering, a colour preview on both themes with "Adjusted for readability", custom colours.
- Invite options (expiry, maximum uses) when creating from Invites; "delete recent messages" when banning (needs the API).
- The bans and member lists are capped at 1 000 and not paginated; the moderation log pages by 50.
