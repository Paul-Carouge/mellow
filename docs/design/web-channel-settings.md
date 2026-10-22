# Web client: channel settings and icons

DESIGN.md §5.8 ("Channel settings, web") and §6.54 on the web client, in the Dusk direction ([ADR-0022](../adr/0022-give-mellow-its-own-visual-language-on-a-familiar-layout.md)). The server API is #70 (`feat/guild-and-channel-icons-api`). This page records the design decisions and the mapping to the code, so the desktop catch-up keeps the same shape.

## Brief

- **Who**: the owner of a community server and the moderators they trust, often on a phone, fixing one channel: making `#annonces` read-only for everyone but the moderators, hiding a staff channel, renaming, giving a channel or the server a recognisable picture.
- **What the owner asked** (2026-10-22): a channel's name, topic and permissions; an icon for channels and for the server, "like a profile picture", cropped automatically or with a chosen square, compressed because it is shown tiny; server icons may be animated GIFs.
- **Rule**: the instance decides (ADR-0007). The client hides sections and locks controls the instance would refuse (`channel-settings.ts` mirrors `set_permission_overwrite`, `delete_permission_overwrite` and `can_target_role_in_overwrites`) and always says why a control is locked. The instance also does the compression (256 px server icons, animated kept with a still; 96 px channel icons, always still); the browser never re-encodes the picture, so an animated GIF stays animated.

## Flow and states

```
channel row (hover / focus / selected) ── gear ──┐
chat header ─────────────────────────── gear ──┤
                                                v
                  Channel settings (sheet) ── Overview ── icon: Upload ─> file picker ─> Choose the square ─> Save icon
                                          │             └ Remove icon (at once)
                                          │             name, topic ─> unsaved-changes bar ─> Save changes
                                          └─ Permissions ─ list: roles, @everyone, members
                                                          ├ Add a role or member (dialog, at once, all Inherit)
                                                          ├ Remove (at once; not @everyone)
                                                          └ Deny / Inherit / Allow per permission ─> unsaved-changes bar
Server settings ── Overview ── Server icon (same editor and dialog), then the name
```

| State | What shows |
|---|---|
| Section without permission | Hidden; the sheet closes when none is left, or when the channel is deleted or hidden from the reader. |
| Unsaved changes | The bar of §5.8; leaving the page, another target, "Add", back, close or Esc shakes it. |
| Role at or above the reader's highest | Its row has a lock; its editor says "You can only change roles below your highest role." and every option is locked. |
| Permission the reader lacks in the channel | Its control is locked (dimmed, not focusable); one sentence above the list says why and the controls are described by it. |
| Members still loading | "Loading members…" in the add dialog; the roles are already listed. |
| Nothing to add | "Every role and member you can manage is already in the list." / no match for a search. |
| Picture opening | "Opening the picture…" on the stage; Save is disabled. |
| Unreadable, too large, not a picture | Refused before the dialog for type and size (8 MB); an unreadable picture says so in the dialog; refusals of the instance (unreadable, too large, rate limited, busy, no permission) show in the dialog, which keeps the square so a retry needs no new pick. |
| Uploading / removing | The button says "Uploading icon…" / "Uploading…" or "Removing icon…", the area is `aria-busy`; the instance answers once compressed (a second or two for a large GIF). |
| Saved | "Icon saved." / "Icon removed." / "Changes saved", announced through a status region. |
| Icon fails to load | The initials or `#` come back. |

## Layout

The server settings' sheet, shared through `SettingsSheet.svelte` (navigation on the ground, page as a sheet, or one after the other below 816 px).

```
wide                                                          phone
+-------------------------------------------------------+     +-------------------------+
| ground       +---------------------------------------+ |     | <                   (x) |
|  #annonces   | Permissions                       (x) | |     | Permissions             |
|  Overview    | Choose who can see and use #annonces… | |     | intro                   |
| >Permissions | [+ Add a role or member]          Esc | |     | [+ Add a role or member]|
|              | Roles and members | o Moderators  Remove| |  | Roles and members       |
|              | o Moderators      | General             | |  |  o Moderators       (lock)|
|              | o @everyone       | View channel        | |  |  o @everyone            |
|              | (A) Ana           | [x Deny|- Inherit|v Allow]| (A) Ana             |
|              |                   | …                   | |  | o Moderators            |
|              | [ Careful… Reset  Save changes ]       | |  | View channel            |
|              +---------------------------------------+ |  | description             |
+-------------------------------------------------------+     | [x Deny|- Inherit|v Allow]|
                                                              +-------------------------+

Choose the square (dialog, 440 px wide or the window minus 32 px)
+------------------------------------------+
| Choose the square                        |
| Drag the picture and zoom until…         |
| +--------------------------------------+ |
| |░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░░| |   ░ picture under color.scrim
| |░░ +------------------------------+ ░░| |   square framed in white
| |░░ |   kept square (drag, wheel,  | ░░| |
| |░░ |   pinch, arrows, + and −)    | ░░| |
| |░░ +------------------------------+ ░░| |
| +--------------------------------------+ |
| Arrow keys move the picture, …           |
| Zoom [-----o--------]       Centre it    |
| At its real size                         |
|  (o) In the list   [o] Selected          |   server: 48 px circle and squircle
|  or: [i] general / [i] general (selected)|   channel: a read and a selected row
|                     Cancel  [Save icon]  |
+------------------------------------------+
```

## Mapping

| Piece | Code | Tokens |
|---|---|---|
| Sheet layout (server and channel settings) | `SettingsSheet.svelte`, `SettingsNavItem.svelte` | `component.settings.*` |
| Channel settings and its sections | `ChannelSettings.svelte`, `ChannelOverviewSettings.svelte`, `ChannelPermissionSettings.svelte`, `AddOverwriteTargetDialog.svelte` | `component.roles.*` (list, rows, permission rows), `component.channel-settings.*` |
| Rules: sections, tri-state, drafts, locks, addable targets | `channel-settings.ts` (tested) | — |
| Icon editor and crop dialog | `IconEditor.svelte`, `IconCropDialog.svelte`, `icon-crop.ts` (tested) | `component.icon-picker.*` |
| Server icon in the strip | `ServerRail.svelte` | `component.rail.*` |
| Channel icon and gear in the row; icon in the chat header | `ChannelSidebar.svelte`, `ChatPane.svelte` | `component.channel-row.image-radius`, `trailing-icon-size`, `settings-*` |
| API | `api.ts` (`setGuildIcon`, `setChannelIcon`, `updateChannel`, `setPermissionOverwrite`, …), `protocol.ts` (`iconUrl`, `CHANNEL_SCOPED_PERMISSIONS`) | — |

Edits show when the gateway brings them (`GUILD_UPDATE`, `CHANNEL_UPDATE`), never from the REST answer, as in the server settings; an overwrite draft rebases on any change made meanwhile (`rebaseOverwriteEdit`, like `rebaseRoleEdit`), so our own save echoed by the gateway ends it and someone else's change to another bit is kept.

## Tokens

- `component.channel-settings`: the override control (32 px, `radius.sm`, `border.strong` outline at 3:1, options in `text.muted`, the chosen one on `state.selected` with `text.heading`, glyphs `text.danger` / `text.heading` / `text.positive`) and the 20 px avatar of member rows. Two contrast pairs are checked by the generator: `text.positive` and `text.danger` on `state.selected`, at least 3:1 (graphical objects), in both themes.
- `component.icon-picker`: preview size (80 px), the channel preview's radius (`radius.lg`, the row's 4 px at 20 px scaled up), the stage (`surface.code`, `radius.md`), the frame (2 px white on any picture, `space.lg` from the stage's edge), the scrim outside the square (`color.scrim`), the sample labels.
- `component.channel-row`: `image-radius` (`radius.xs`), `trailing-icon-size`, and the gear (`settings-button-size` 24 px, 44 px under a coarse pointer, `settings-icon` / `-hover`).

## Defaults rejected

- **Discord's filled red / grey / green squares** for the three states: three saturated fills per row, ten rows deep, is the loudest thing on the page. Dusk keeps one lamp: the chosen option is filled with the neutral selected tone and named in words; only its glyph takes the colour, so the page can still be scanned for green and red without shouting.
- **"Allow / Inherit / Deny"** (DESIGN.md before v0.18): Discord's users look for Allow on the trailing side; the order follows Discord, the words stay ours.
- **A circular mask in the cropper**, as every avatar cropper has: server icons are circles at rest but squircles when hovered or selected, and channel icons are rounded squares, so a circle would lie about the corners. The square is framed, and the real shapes are shown at their real size underneath, which is what the owner worried about ("shown tiny").
- **Re-encoding or cropping in a canvas before upload**: it would freeze animated GIFs and double the compression. The browser only measures; the instance cuts the square.
- **A drop zone with a dashed border** on the Overview: one button does it, and the page has nothing else to drop.
- **Circles for channel icons**: circles are people in Mellow (avatars, presence); a place gets a rounded square, like Discord's server folders.
- **A trash icon beside "Remove"**: the word is enough, and a red glyph next to the target's name read as a warning about the role itself.

## Follow-ups

- Category settings (their overwrites, and syncing channels with their category, as on Discord).
- "Delete channel" in the channel settings (the API exists; the owner did not ask for it in this step).
- The channel icon in the welcome hero, the quick switcher and mentions of the channel.
- Real upload progress (bytes sent): `fetch` exposes none; the button shows an indeterminate state.
