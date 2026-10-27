# Web client: group conversations and the "New message" picker

DESIGN.md §5.6 and §5.6.2 on the web client, in the Dusk direction ([ADR-0022](../adr/0022-give-mellow-its-own-visual-language-on-a-familiar-layout.md)). ROADMAP "Next, in order" item 8, the owner's wish for Discord's group direct messages. The rules are [ADR-0026](../adr/0026-group-conversations.md); the protocol, the server and the web data layer (`startConversation`, `renameGroupConversation`, `addGroupParticipant`, `removeGroupParticipant`, `leaveGroupConversation`, the `DIRECT_MESSAGE_CHANNEL_UPDATE` and `_DELETE` events) came first, on `feat/group-conversations`. This page records the design decisions and where they live in the code, so the desktop catch-up keeps the same shape.

## Brief

- **Who**: the same small self-hosted communities as the Friends space, organising something among a few friends (a weekend, a game night) outside any server.
- **Job of the screens**: start a conversation with one friend or a group of several in one place; see at a glance which conversations are groups and how big they are; manage a group without leaving it: add friends, rename, leave, and for its owner remove someone.
- **Rules that shape the interface** (ADR-0026): only friends can be picked, by whoever adds them; 10 people at most, you included; the newcomer reads the whole history; the instance answers the same NOT_FOUND for "not a friend" and "a block either way", so the words must not tell them apart; only the owner removes others; the lowest id takes over when the owner leaves; a group stays open whatever the relationships inside it.

## Flow and states

```
Home ── "+" beside Direct messages ──> picker (New message)
  pick 1 ── Message ──> the one-to-one conversation opens (created or found), composer focused
  pick 2–9 ── [Group name] ── Create group ──> the group opens, composer focused
  no friends ── Add friend ──> Friends, Add friend tab, field focused
Group ── header (+u) or sheet "Add friends" ──> picker (Add friends to …) ── Add to group ──> "Ana joined the group."
      ── header (uu) / Ctrl+U ──> members side sheet
            Rename group ──> dialog ── Save name ──> "Group renamed to …"
            Leave group ──> confirmation (next owner, or deleted with you) ──> Friends, "You left the group “…”."
            (owner) Remove Bob ──> confirmation ──> "Bob was removed from the group.", focus on the next Remove
Gateway: removed while the group is open ──> Friends, "You're no longer in the group “…”."
```

| State | What shows |
|---|---|
| Picker, nothing picked | "You can add 9 more friends."; the action dimmed. |
| Picker, one picked | Action "Message"; no name field. |
| Picker, several picked | Action "Create group"; "Group name (optional)" with the names it would show as placeholder. |
| Picker, the group is full | "That's as many people as a group can hold."; checkboxes not picked are disabled and point to that sentence. |
| Picker, no friends | One sentence and "Add friend" (primary, focused). |
| Picker, all friends already in the group | "All your friends are already in this group."; only Cancel. |
| Picker, nothing matches | "No friends match “zed”." |
| A picked friend stops being one, or joins meanwhile | Gone from the chips and the list; the counter follows. |
| Create refused (NOT_FOUND) | "Couldn't create the group: one of these people can't join it. Check your friends list and try again." (never says "blocked"). |
| Add refused | "Couldn't add Ana to the group: they can't join this group. …" or "…: the group is full."; those already added are announced and the rest stay picked. |
| Group with a name / without | The name / "Ana and Bob", "Ana, Bob and Chloé", "Ana, Bob and 2 others". |
| Alone in a group | The `users` glyph on `neutral.fill` as its picture, "Just you" as its name, "1 member". |
| Someone you blocked in the group | Nothing locks; their runs of messages fold (DESIGN.md §5.4). |
| Owner leaving | "…, and Ana will own it." (the lowest id among the others). |
| Last person leaving | "You're the last one here: the group and its messages will be deleted for good." |
| The person being removed leaves meanwhile | The confirmation closes by itself. |
| Long or French labels on a phone | The chips wrap inside the field and the search goes under them; the side sheet covers the window. |
| Reduced motion | The side sheet and dialogs appear at once (the existing tokens); nothing else moves. |

## Design plan

- **Colour**: no new colour. Apricot keeps "you or now": the checkboxes of the friends you picked take `accent.indicator` (checked 3:1 against `surface.popover`), "Add friends" in the sheet takes `text.accent` like the server menu's Invite people, and the dialog's primary button is the screen's one apricot fill. "Leave group" is `text.danger` like Leave server. Chips use `state.selected`, the selection tone, not a new tint.
- **Type**: `type.member-name` and `type.body-sm` for picker rows and the group row's second line, `type.category` for the field labels and the "Members 5" header, `type.menu-item` for the sheet's actions, `type.body` for the counter.
- **Shape**: avatars stay circles (people); the group picture is two circles, never a square (squares are servers and channel icons). Chips `radius.xs` like role chips; picker rows `radius.sm`; the owner's Remove is a round action like the Friends rows'.
- **Spacing**: picker rows 48 px (two lines, above the 44 px target), chips 28 px (44 px under a coarse pointer), the list scrolls past 320 px so the name field and the buttons stay in view.
- **Motion**: none added. The dialog and the side sheet keep their existing transitions; the picker answers each pick with the chip and the counter, not an animation.
- **Layout**: the picker is a dialog (it starts something new from anywhere at Home); a group's management lives in the side sheet that already holds a server's members, so the place for "who is here and what can I do" is the same in servers and groups.
- **Signature**: the two leaning faces. Everything else is the vocabulary people already know from Friends and the server menu.

## Defaults rejected

- **A tinted square with a generic group glyph** (Discord's default group icon): a square reads as a server or a channel icon in Mellow; two of the actual people say "this is with Ana and Bob" before the name is read.
- **Presence dots on group rows**: one dot cannot stand for five people; the second line counts them instead, as Discord does.
- **A custom listbox with arrow keys and `aria-activedescendant`** for the picker: native checkboxes give Space, "checked" and forced colours for free; Enter in the search covers the fast path.
- **The search field and the chips as separate rows**: Discord's single field keeps the picked people where the eye already is, and Backspace takes them back.
- **A crown for the owner**: a mark alone fails "never by colour or symbol only", and Phosphor 2.1.1's set in the repository has none; the second line says "Owner".
- **Inline renaming in the chat header** (Discord's): a dialog is reachable by keyboard and screen readers without a hidden edit mode, and keeps the header a heading.
- **A toast when you are removed**: there is no toast component yet; the polite announcement and Friends opening say it, and a toast should come with notifications (ROADMAP item 13).
- **"Ana is no longer your friend" on a refused creation**: the instance answers the same for a block; naming the cause would reveal blocks (ADR-0025, ADR-0026).

## Mapping to the code

| Piece | File |
|---|---|
| Names ("Ana, Bob and 2 others"), counts, room left, search, next owner, who locks the composer (never in a group) | `web/src/components/group-conversations.ts` (tested) |
| Group picture | `web/src/components/GroupAvatar.svelte` |
| "New message" and "Add friends" picker | `web/src/components/PeoplePickerDialog.svelte` |
| Group side sheet: actions, members, Remove | `web/src/components/GroupMembersSheet.svelte` |
| Leave and Remove confirmations | `web/src/components/GroupConfirmDialog.svelte` |
| Rename | `web/src/components/RenameGroupDialog.svelte` |
| "+" beside Direct messages, group rows | `web/src/components/ChannelSidebar.svelte` (through `Navigator.svelte`) |
| Header avatar, member count, Add friends, member toggle, hero, composer label | `web/src/components/ChatPane.svelte` |
| Picker and sheet wiring, remembering the sheet per group, the announcement and focus after closing | `web/src/components/Shell.svelte` |
| Why the open group closed (`closedConversation`), even when the gateway's delete beats the answer | `web/src/lib/client.svelte.ts` (tested) |
| Groups in the quick switcher; people lead to their own conversation | `web/src/components/quick-switcher.ts` (tested), `QuickSwitcher.svelte` |
| Words | `groups`, `chat.composerToGroup`, `chat.groupStart` in `web/src/i18n/en.ts` and `fr.ts` |
| Tokens | `component.group-avatar.*`, `component.people-picker.*` in `design/tokens.json` |

## Checked

Headless Chrome (DevTools protocol) against a local instance built from the branch with the web client embedded, seeded through the REST API: ten people, eight friends of Paul's, a pending request, a named group of five owned by Paul, an unnamed group of four owned by Ana, a one-to-one conversation, and five people kept online through the gateway. English and French, dark and light, 1280 × 800 and 390 × 844 (touch emulated): Home's rows, the picker empty, searched and with two picked, a group's chat with its side sheet, the Remove, Leave and Rename dialogs, the Add friends picker, an unnamed group. Flows run end to end: Enter-picking two friends and naming the group created and opened it with the composer focused; rename, add, the owner's removal and leaving were each read out, and leaving landed on Friends with focus on its tab; Ana removing Paul through the API while the group was open brought Paul back to Friends with "Tu ne fais plus partie du groupe « Ana, Bob et Emma »." Not checked: a real screen reader (VoiceOver, NVDA), 200 % text zoom, Windows forced colours, touch on a real phone, a group of 10 filling up while the picker is open, and two browser tabs of the same account.
