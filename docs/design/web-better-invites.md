# Web client: better invites

DESIGN.md §5.3 ("Invite people"), §5.8 ("Invites"), §5.2 ("Join a server"), §6.57 and §8 on the web client, in the Dusk direction ([ADR-0022](../adr/0022-give-mellow-its-own-visual-language-on-a-familiar-layout.md)). ROADMAP item 11. The server change (readable codes, limits, normalised joining) and the web data layer are on `feat/better-invites`; this page records the design decisions and where they live, so the desktop catch-up keeps the same shape.

## Brief

- **Who**: a server's owner or any member allowed to invite, sending a code to friends over another app (a text message, a chat, read aloud at a table); and the friend who receives it and types or pastes it, often on a phone.
- **Job**: get a code that is easy to read, copy and type, and decide for how long and for how many people it works, without thinking about it when the default is fine.
- **Owner's wishes (2026-10-30)**: "a nicer invite interface, a nicer code in the style AE8C-78A9, and choose how long the link stays valid, how many people can join, etc." After the security audit, codes became three groups (`AE8C-78A9-K2QF`, 60 bits): two groups could be guessed from the many addresses of an IPv6 /48.
- **Default (decided here, Discord's)**: an invite lasts **7 days** with **no use limit**. A week covers "come join us this weekend" and stops forgotten codes from working forever; no limit avoids a code that silently stops after a few friends. Discord uses the same default, so people who know it are not surprised.

## Flow and states

```
Server menu / server icon menu ─ Invite people ─> dialog opens, an invite is created at once
   (7 days, no limit)  ─> code shown ─> Copy code ─> "Copied" (2 s), "Invite code copied" read out
                       └> Change expiry and uses (disclosure)
                            ─> Expire after [7 days ▾]  Maximum uses [No limit ▾]
                            ─> Create a new code ─> new code shown and read out; the old one keeps working

Server settings ─ Invites ─ Create invite ─> "Create an invite" (same two selects) ─> Create invite
   ─> dialog closes, list reloads with the new invite on top, "Invite AE8C-… created" read out

Rail + ─ Add a server ─ Join a server ─> type "ae8c 78a9 k2qf" or paste "AE8C-78A9-K2QF" ─> Join
```

| State | What shows |
|---|---|
| Creating the first invite | "Creating an invite…" inside the well, on the code's own line box so the well does not jump when the code arrives; "Copy code" disabled (still focusable). |
| Creating failed | The dialog's error line ("Couldn't create an invite: …") and "Try again" inside the well. |
| A new code refused (quota, rate limit) | The current code stays, with the error line; nothing is lost. |
| Copy refused by the browser | "Couldn't copy the code. Select it and copy it by hand." One click on the code selects all of it. |
| Reduced motion | "Copied" stays instead of flashing back after 2 s; no other motion was added. |
| Settings list, an invite expired while the page was open | "Expired" (the list is only re-read after a change). |
| Joining | Case, spaces and dashes are ignored and O, I, L read as 0, 1, 1 by the instance; the field does not change what is typed (older 10-character codes are case-sensitive, so the field does not force capitals either). |

## Design plan

**Palette (no new colour).** The code well is `surface.field`, the colour of a text field, sunk into the dialog's `surface.popover` with a 1 px `border.divider`: it reads as "the thing you take away" without becoming an input to fill. The code is `text.heading`; the dashes step back to `text.muted` so the eye reads three words, and the copied text still holds real hyphens. Apricot stays where it means "you, now": the one primary button, "Copy code". The limits sentence is `text.secondary`; the disclosure uses the dialog's existing link colour (`component.dialog.mode-link-text`), as "Join a server" does.

**Type.** One new role, `type.invite-code`: Figtree 800 at 28/36 px with 1 px tracking. Figtree, not the monospace stack, because Crockford's alphabet has no O, I, L or U (the reason DESIGN.md gave monospace to codes) and Figtree is bundled, so the code looks the same on every machine, while the platform monospace changes from Menlo to Consolas to DejaVu. Digits are tabular through the body's `font-variant-numeric`. On a phone the code shrinks to its well's width (`min(token, 100cqi / component.invite.code-width-in-em)`) rather than breaking a group across lines. The settings list keeps monospace for codes: it can still list older 10-character codes, where `0`/`O` and `l`/`I` matter.

**Shape and spacing.** The well takes `radius.md` (12 px), one step above fields and buttons (`radius.xs`/`radius.sm`), since it is a container; `space.md` above and below the code. The code, Copy and the sentence are one group (`component.invite.stack-gap`, 8 px); the disclosure is the next section (`component.dialog.section-gap`). Selects reuse the text field's tokens (44 px, `radius.xs`, `border.strong`), grow from `component.invite.field-basis` (160 px), side by side in a 440 px dialog, stacked on a phone.

**Motion.** None added. The dialog's own entrance stays; changing the code is instant and announced. One strong element (the code) is enough.

**Layout.**

```
Wide (440 px dialog)                             Phone (320 px window)
+--------------------------------------------+   +----------------------------+
| Invite friends to Rust Lyon                |   | Invite friends to Rust     |
| Send this code to a friend. They enter it  |   | Lyon                       |
| under Add a server, then Join a server.    |   | Send this code to a …      |
| +----------------------------------------+ |   | +------------------------+ |
| |          AE8C-78A9-K2QF                | |   | |  AE8C-78A9-K2QF        | |  (shrinks, one line)
| +----------------------------------------+ |   | +------------------------+ |
| [            Copy code                   ] |   | [       Copy code        ] |
| This invite expires in 7 days and has no   |   | This invite expires in 7 … |
| use limit.                                 |   |                            |
| > Change expiry and uses                   |   | v Change expiry and uses   |
|   Expire after        Maximum uses         |   | Expire after               |
|   [7 days      v]     [No limit    v]      |   | [7 days               v]   |
|   The current code keeps working until it  |   | Maximum uses               |
|   expires.                                 |   | [No limit             v]   |
|   [Create a new code]                      |   | The current code keeps …   |
|                                   [Close]  |   | [Create a new code]        |
+--------------------------------------------+   |                    [Close] |
                                                 +----------------------------+
Server settings > Invites row
 (A) AE8C-78A9-K2QF                                        [Revoke]
     Created by Ana
     23 / 100 uses · Expires in 3 days      (each part kept whole; breaks at the dot)
```

Alignment: the code is centred in its well (it is a single object to look at); everything else starts on the leading edge, like the other dialogs.

## Anti-slop review of the plan

- **Rejected: the code in a read-only text field with "Copy" beside it** (the previous dialog, and Discord's link field). It makes the code look like a form to fill in, and at 320 px the field truncates. The code is the dialog's hero, in a well, with the button under it.
- **Rejected: six or twelve boxed characters, one per cell** (the one-time-password look). It is the default treatment for any code, reads as a security prompt, and breaks at 320 px. Three groups with quiet dashes read like three words.
- **Rejected: platform monospace for the big code.** It differs on every system and was only needed for `0O`/`1lI`, which the new alphabet avoids.
- **Rejected: creating a new invite whenever a select changes.** Browsing the presets would leave live invites behind; the selects only change the choice, "Create a new code" applies it, as Discord's "Edit invite link" does with "Generate a new link".
- **Rejected: a nested "Invite settings" dialog** (Discord's). Two stacked modal dialogs are harder for keyboards and screen readers, and on a phone the second covers the code. A disclosure keeps the code on screen above the choices.
- **Rejected: preset chips (eight pills for the lifetime, seven for the uses).** At 320 px they wrap into four or five rows each; native selects give phones their own picker, keyboards and screen readers a real list box, and scale with text.
- **Rejected: middle-dot meta strings in the dialog** ("Expires in 7 days · no use limit"). One sentence says it plainly and translates well. The settings rows keep their dot, which the list already used for the uses and the expiry.
- **Changed after review**: the first build set the code at 24 px and let it wrap; at 320 px the third group fell to its own line after a dangling dash. The code now keeps one line and shrinks only when its well is narrower than the code, and grows to 28 px where there is room.
- **Removed**: an icon inside "Copy code" (the button family has no icon slot, and the word is enough).

## Mapping

| Piece | Code | Tokens |
|---|---|---|
| Presets, default, wording of durations, uses, expiry, spoken code | `web/src/components/invites.ts` (tested in `invites.test.ts`) | — |
| Invite people | `InvitePeopleDialog.svelte` | `type.invite-code`, `component.invite.*`, `component.dialog.*` |
| Limit selects (6.57) | `InviteLimitsFields.svelte` (in Invite people and Create an invite) | `component.text-field.*`, `component.invite.select-icon`, `component.invite.field-basis` |
| Settings list and Create an invite | `InviteSettings.svelte` | `component.server-settings.*` |
| Join a server | `AddServerDialog.svelte` (placeholder, helper, monospace, no auto-capitals) | `component.text-field.*` |
| Strings | `web/src/i18n/en.ts`, `fr.ts` (`invitePeople`, `inviteLimits`, `serverSettings.invites`, `addServer`) | — |

## Checked

Headless Chrome against a local instance built from this branch (`--features web-client`), in English and French, dark and light, at 1280 × 860 and 320 × 640: Invite people (default, limits open, a new code with 1 hour and 10 uses, Copy with the clipboard read back), Server settings → Invites (list and Create an invite with 30 minutes and 25 uses), Join a server (placeholder and helper), and a second account joining with the code typed as `g6dy hm2n 1rp6`.

Not checked: real phones (iOS and Android select pickers), screen readers (VoiceOver, NVDA, TalkBack) reading the spelled code and the announcements, text at 200 %, Windows high contrast.

## Follow-ups

- The previous code stays valid when a new one is created (members without `MANAGE_GUILD` cannot revoke); a manager can revoke it in Server settings → Invites. If the owner wants "Create a new code" to replace the old one, it needs a server rule allowing a creator to revoke their own invites.
- No Copy button on the settings rows (Discord has none either); the code can be selected there.
- Invite links (`https://instance/invite/AE8C-78A9-K2QF`) opening the join page with the code filled in, and invites on the sign-up page of invite-only instances.
- The desktop client keeps its never-expiring invites until it catches up with this page.
