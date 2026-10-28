# Web client: channels, categories and server creation

DESIGN.md §5.2 ("Create a server"), §5.3 (channel sidebar), §5.8 (channel settings) and §8 on the web client, in the Dusk direction ([ADR-0022](../adr/0022-give-mellow-its-own-visual-language-on-a-familiar-layout.md)). ROADMAP item 10. The server API and the web data layer are on `feat/channels-and-categories`; this page records the design decisions and where they live, so the desktop catch-up keeps the same shape.

## Brief

- **Who**: the person who just installed an instance and makes the first server for their friends, club or class; then the owner and moderators who tidy the channel list as the server grows, on a laptop and on a phone.
- **Job**: start a server that already looks like the group it is for (a picture, a name, a sensible set of channels) in one dialog; then add, name, move and remove channels and categories where Discord users look for those actions.
- **Owner's decisions (2026-10-28)**: names are kept as typed (capitals and spaces, 1–100 characters), with no "lowercase with dashes" hint anywhere; categories are created from the server menu and from "Create channel" (a type choice, as on Discord), with an optional topic for text channels; channels and categories reorder by drag and drop with a keyboard and single-pointer alternative (WCAG 2.2 2.5.7), only with `MANAGE_CHANNELS`; delete channel and delete category with a confirmation; rename a category; the "Create a server" dialog gains an icon with the crop dialog, starter templates in the interface's language, and a live preview. No naming style option.

## Flow and states

```
Server menu ─ Create channel ──┐
            └ Create category ─┼─> Create channel dialog (type: Text channel | Category; name; topic for text)
Category "+" ──────────────────┘     (from "+": text only, "in Gaming")
                                      text channel ─> opens it; category ─> appears, focus back to the composer

Channel row menu ─ Edit channel ─────────────> channel settings
                 ├ Move up / Move down ──────> moved at once (pending order), read out
                 ├ Move to category… ────────> dialog: No category | each category ─> end of it
                 ├ Copy channel ID
                 └ Delete channel ───────────> typed confirmation (DESIGN.md §8)
Category menu ─── Rename category ───────────> one-field dialog
                 ├ Move up / Move down
                 ├ Copy category ID
                 └ Delete category ──────────> confirmation; its channels stay, without a category
Channel settings ─ Delete channel (danger, last in the navigation)

Pointer: press and drag a row or a category header (mouse: after 4 px; touch: hold, then drag)
         ─> apricot line where it lands, or a washed header to go into a category ─> drop
         Esc or a drop where it already was: nothing sent

Rail "+" ─> Create a server: icon (crop dialog), name, "Start with" (Friends, Community, Study group,
            Gaming, Blank), live preview ─> Create ─> server opens; icon sent next
```

| State | What shows |
|---|---|
| No `MANAGE_CHANNELS` | No drag, no move, rename or delete items; categories keep "Copy category ID", channels "Copy channel ID". Moving a channel to another category also needs `MANAGE_CHANNELS` in that channel (the instance checks it): without it, the row moves only within its section. |
| Move in flight | The new order shows at once (a pending order); other moves wait (drags do not start, move items are hidden) until the instance answers. The pending order gives way to the gateway's once they agree, or after 5 s. |
| Move refused | The old order comes back and a one-line alert sits above the channel list ("Couldn't move the channel: someone else changed the order meanwhile. Try again."), with a close button; from a menu, the error stays at the menu's foot. |
| Nothing moved | A drop where the item already was, or Esc, sends nothing. |
| Item gone during a drag | The drop does nothing. |
| Create a server, icon refused after the server exists | The dialog stays with "Rust Lyon was created, but its icon couldn't be saved: … Add it from Server settings." and only "Close". |
| Empty server name | "Create" disabled; the preview says "Your server". |
| Delete channel | Typed name, "Delete channel" enabled when it matches; the channel goes for everyone at once (CHANNEL_DELETE), focus to the composer. |
| Delete category | No typing: nothing is lost but the heading. |

## Design plan

**Palette (no new colour).** The sidebar keeps its ground and rows. Reordering speaks with three existing roles:
- `accent.indicator` (apricot) for the drop line: apricot is "you, now" in Dusk, and the line is exactly that, where your hand is putting the row.
- `state.selected` washes a category header the row would go into, with `text.heading`.
- `surface.floating` with `elevation.high` for the row under the pointer, the surface of menus: the row is lifted out of the list into the floating layer, and its old place stays at `component.channel-drag.source-opacity` so the list does not jump.
- Choices (channel type, templates, destination category) are native radios in `accent.indicator` (your choice, as the people picker's checkboxes) inside rows that take `state.selected` when checked, so the choice never relies on the dot alone.

**Type.** The existing scale: the preview card uses the sidebar's own styles (`type-heading-md` header, `type-category` categories, `type-channel-name` rows) so it is the real thing at its real size; choice rows use `type-channel-name` for the name and `type-body-sm` in `text.muted` for the one-line description.

**Shape and spacing.** Choice rows `radius.sm` (the channel row's radius), 1 px `border.divider`, at least 44 px; the preview card `component.navigator.radius` on `surface.navigator`, the navigator card in miniature. The wide "Create a server" dialog is `size.dialog.medium-width` (680 px): a form column and the 256 px preview (`component.server-preview.width`: the strip plus enough of the channel list to read a name). The two sit in a wrapping row, the form growing first, so the preview drops under the form when there is no room for both: no breakpoint, only the tokens' own widths.

**Motion.** One moment: the dragged row lifts (shadow and `component.channel-drag.lift-scale` over `motion.duration.fast`, settle curve), instant under reduced motion; everything else is direct manipulation (the row follows the finger) or instant (the list reflows on drop without animating, which also keeps reduced motion trivially honoured). Auto-scroll near the list's edges is driven by the pointer, not an animation.

**Layout.**

```
Sidebar, dragging #photos into Gaming          Sidebar, dragging between rows
  Text channels                                  Text channels
    # general                                      # general
    #·photos·(dimmed, its old place)            ●━━━━━━━━━━━━━━━━  apricot line
  ▸ Gaming      [state.selected wash]  <──┐       # links
    # clips                                │    ┌──────────────────┐
  ┌──────────────────┐                     │    │ # photos         │ floating, elevation.high
  │ # photos         │  under the pointer ─┘    └──────────────────┘
  └──────────────────┘

Create a server, wide (680 px)                                  phone (window − 32 px)
+--------------------------------------------------------------+ +---------------------------+
| Create a server                                              | | Create a server           |
| Give it a picture, a name and a start. You can change …      | | Give it a picture, …      |
| (RL)  Server name                       +-------------------+| | (RL) Server name          |
| Upload icon [Rust Lyon            ]     |(RL)▌ Rust Lyon    ||| |      [Rust Lyon        ]  |
|                                         |    | Text channels||| | Upload icon               |
| Start with                              |    | # general ▒▒ ||| | Start with                |
| [(•) Friends     general, photos, plans]|    | # photos     ||| | [(•) Friends …          ] |
| [( ) Community   welcome, announcements]|    | # plans      ||| | [( ) Community …        ] |
| [( ) Study group …                     ]|    |              ||| | …                         |
| [( ) Gaming      …                     ]|    |              ||| | Preview                   |
| [( ) Blank       general               ]|    +-------------------+| | +-----------------------+ |
| Have an invite already? Join a server                        | | |(RL)| Rust Lyon        | |
|                                     Cancel  [Create]         | | +-----------------------+ |
+--------------------------------------------------------------+ | Cancel [Create]           |
                                                                 +---------------------------+
Create channel (440 px)
+------------------------------------------+
| Create channel                           |
| in Gaming (from a category's "+")        |
| Channel type (not from "+")              |
| [(•) # Text channel  Messages, pictures, links]
| [( ) ≡ Category      A heading that groups channels]
| Channel name  [# General chat         ]  |
| Topic (optional) [                    ]  |
|                  Cancel [Create channel] |
+------------------------------------------+
```

Alignment: everything starts on the leading edge; the dialog titles keep their existing alignment (create a server stays centred as before, since it is the one welcome moment).

## Anti-slop review of the plan

- **Rejected: template cards with emoji or big illustrations** (Discord's "Create My Own / Gaming / School Club" tiles). Icons as illustration are a tell, and the five Phosphor icons we ship have no game pad or book. Each template is a radio row naming its channels ("general, photos, plans"), and the preview shows the structure itself: the content is the picture.
- **Rejected: a generic "preview" box of grey bars.** The preview is the navigator card in miniature, built from the same tokens and type as the real sidebar, rows and headers at their real size; the first channel shows selected and the icon sits in the strip with the apricot pill, exactly as the server will look once created.
- **Rejected: a drag handle (six dots) on every row.** It would show on every row for every manager, all the time, in a list that is read far more than rearranged. The row itself is the handle; the menu gives the single-pointer and keyboard path.
- **Rejected: animated reflow of rows while dragging** (rows sliding apart). It is the library default, costs layout work on every pointer move, and fights reduced motion. A 2 px apricot line says where the row lands; the list moves once, on drop.
- **Rejected: a blue drop line**, the default of every list library: Dusk's one lamp is apricot, and this is a "you, now" moment.
- **Rejected: typed confirmation for categories.** DESIGN.md §8 keeps typing for irreversible actions affecting many people; deleting a category loses nothing but its heading.
- **Rejected: "new-channel" placeholders and naming hints** (owner's decision): the field is empty and names stay as typed.
- **Changed after review**: the first plan put template names in a two-column grid of cards; a grid of equal cards is the SaaS kit and breaks at 320 px. Changed to a single column of rows with their channel list as the description, which also reads well with text at 200 %.

## Mapping

| Piece | Code | Tokens |
|---|---|---|
| Order rules: placements, moves, drop targets, pending order | `channel-order.ts` (tested) | — |
| Pointer drag (mouse after 4 px, touch after a hold), Esc, click suppression | `channel-drag.ts` (tested where pure) | `component.channel-drag.*`, `motion.delay.long-press` |
| Sidebar: rows and headers draggable, menus, pending order, alert | `ChannelSidebar.svelte` | `component.channel-drag.*` |
| Menus | `context-menus.ts` (`channelMenuActions`, `categoryMenuActions`, `serverMenuActions`) | — |
| Create channel or category | `CreateChannelDialog.svelte` | `component.choice.*` |
| Choice row (radio) | `ChoiceOption.svelte` (type, templates, destination) | `component.choice.*` |
| Move to category | `MoveToCategoryDialog.svelte` | — |
| Delete channel or category | `DeleteChannelDialog.svelte`, also from `ChannelSettings.svelte` | — |
| Rename category | `RenameCategoryDialog.svelte` | — |
| Create a server | `AddServerDialog.svelte`, `ServerPreview.svelte`, `server-templates.ts` (tested), `IconCropDialog.svelte` | `component.dialog.width-wide`, `component.server-preview.*` |
| Name kept as typed | `channel-settings.ts` (`channelNameAsSaved` trims only) | — |

## Follow-ups

- A channel inside a category the reader cannot see is listed with the uncategorised ones and keeps that category in every reorder (the instance accepts it). Moving it with "No category" keeps it there too, since the reader cannot see where it is; only a manager who sees the category can take it out.
- Drag between servers, multi-select, and dragging a whole category's channels with it beyond what the server already does (channels keep their category when it moves).
- Category settings (permissions synced with their channels), and the server's own English default ("Text channels", "general") when a client sends no template.
- Server reordering in the strip (DESIGN.md §5.2) reuses the same drag rules when it comes.
