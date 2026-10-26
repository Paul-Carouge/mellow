# Web client: the Friends space and blocked messages

DESIGN.md §5.6.1 (Friends), §5.4 (folded messages from someone you blocked) and §5.7 (friends and blocks in the profile) on the web client, in the Dusk direction ([ADR-0022](../adr/0022-give-mellow-its-own-visual-language-on-a-familiar-layout.md)). ROADMAP item 7, the owner's request: "un espace pour voir les amis au dessus des messages privés et pouvoir les gérer". The rules are [ADR-0025](../adr/0025-friends-and-blocking.md); the protocol, the server and the web data layer (`relationshipByUserId`, `sendFriendRequest`, `acceptFriendRequest`, `blockUser`, `removeRelationship`) came first, on `feat/friends-and-blocking`. This page records the design decisions and where they live in the code, so the desktop catch-up keeps the same shape.

## Brief

- **Who**: members of small self-hosted communities who already manage friends on Discord, on a laptop or a phone; some of their friends share no server with them.
- **Job of the screen**: see who is around and start a conversation; answer the requests waiting; add someone by username; keep someone away.
- **Rules that shape the interface** (ADR-0025): a block is never shown to the blocked person; a missing account, yourself and a block all answer the same `NOT_FOUND`, so the form must say the same words for all three; guild messages from someone you blocked are not hidden by the server, the client folds them.

## Flow and states

```
Home (rail) or Friends row ──> Friends space, last tab (Online first)
  Online / All ── Message ──> conversation opens, composer focused
              └─ More ──> menu: Remove friend | Block ──> confirmation ──> row leaves, result read out
  Pending ── Accept ──> friend (row moves to Online/All) ; Ignore / Cancel ──> row leaves
  Blocked ── Unblock ──> row leaves
  Add friend ── Send friend request ──> sent | accepted theirs | already sent | already friends
                                    └─> "Hm, that didn't work…" (missing, yourself, block) | rate limited | limit reached
Server chat ── run of messages by someone blocked ──> "2 blocked messages  Show" ──> Show/Hide for the visit
Member profile ── Add friend | Accept friend request | (Friend request sent) | Block ──> confirmation ; blocked: Unblock
```

| State | What shows |
|---|---|
| Before the first `Ready` | The sheet's "Loading your servers and conversations…" (the space needs the relationships of `Ready`). |
| No friends at all | Online and All: idle Melli, "No friends yet. Add someone by their username and they'll show up here.", Add friend, and Add a server when you are in none (the first-run invitation Home used to show). |
| Friends, none online | Online: Melli `sleeping`, "None of your friends are around right now." |
| Nothing pending, nobody blocked | Pending and Blocked: idle Melli and one sentence. |
| A row waiting for the instance | Its actions dim (`aria-disabled`), the others stay usable. |
| A refusal on a row | Above the list, in an alert, with the usual reason. |
| Add friend answered | Under the field, check or warning icon, in a polite live region; the field empties after a success. |
| Many pending requests | The red badge on the Friends row and on the Pending tab (`MentionBadge`, "99+" past its cap). |
| Long or French labels on a phone | The tabs wrap onto a second row; nothing hides off the edge. |
| Reduced motion | The panel appears without its rise; Melli shows its still pose. |

## Design plan

- **Colour**: no new colour. Apricot keeps "you or now": Add friend is the space's single apricot fill until it is open, then its text turns apricot and the form's Send button takes the fill. Presence keeps the status shapes of v0.19. Accept turns `text.positive` on hover and Ignore, Cancel turn `text.danger`, like the message bar's delete; both are checked at 3:1 on `surface.input`. Rows hover with the sheet's own wash (`state.hover-message`), where muted text keeps 4.5:1 in light (the plain hover wash did not).
- **Type**: `type.heading-md` for the title, `type.button` for tabs, `type.member-name` and `type.body-sm` for rows, `type.category` for group headers with their separate count, `type.heading-lg` and `type.body` for Add friend, `type.body-lg` for empty sentences.
- **Shape**: rows `radius.sm`; tabs `radius.xs` pills; the row actions are circles, as on Discord, so they read as buttons on a row that is itself not clickable; the More menu is the context menu surface.
- **Spacing**: 60 px rows (Discord's 62 on the 4 px grid), 36 px actions with `space.xs` between, 44 px for both under a coarse pointer; the list keeps `space.md` from the sheet's edges like messages.
- **Motion**: one answer to one action: changing tab gives the panel the channel switch's rise (`mw-rise`, `motion.duration.base`, enter curve). Nothing else moves; Melli is the only loop, as everywhere.
- **Layout**: the space is a sheet like a conversation, so the navigator, the drawer and the user card stay where they are; tabs live in the header on wide screens (where a conversation shows its topic) and on their own rows on phones.
- **Signature**: Melli in the empty tabs, napping when nobody is around; everything else stays quiet.

## Defaults rejected

- **A separate "Friends" page with its own navigation** (or a modal): the space is Home's page in the sheet, as Discord does, so a friend is one click from a conversation and the drawer and back behaviour stay the same.
- **Hiding a fifth tab off the edge on phones** (a scrolling tab strip): the tabs wrap; a hidden Add friend is the one people need first.
- **One error for every refusal**: rate limit and the account's own cap say what to do; only the three cases ADR-0025 makes identical share a sentence.
- **Discord's jokes in empty states** ("No one's around to play with Wumpus"): DESIGN.md §8 keeps system copy plain; Melli carries the warmth instead.
- **Folding every blocked message alone**: one row per run of one author, as Discord does, so a burst stays one line.
- **Showing blocked messages' text in reply previews**: they read "Blocked message", or the fold would leak through replies.
- **An "Active now" column** (Discord's right rail): there is no activity data to fill it; the space keeps the full sheet.
- **A search field over the lists**: not asked for; at most 1 000 friends per account, and browser search works meanwhile.

## Mapping to the code

| Piece | File |
|---|---|
| Home without a conversation is Friends (`isFriendsSpaceOpen`; `selectGuild(null, null)` no longer falls back to the newest conversation) | `web/src/lib/client.svelte.ts` (tested) |
| The space: header, tabs, rows, menu, empty states, announcements | `web/src/components/FriendsPane.svelte` |
| Tab contents and what a request's answer means | `web/src/components/friends.ts` (tested) |
| Add friend | `web/src/components/AddFriendForm.svelte` |
| Remove friend and Block confirmations | `web/src/components/RelationshipConfirmDialog.svelte` |
| Friends row and its badge | `web/src/components/ChannelSidebar.svelte` |
| Space or chat in the sheet, focus after the drawer | `web/src/components/Shell.svelte` |
| Folding runs of blocked messages | `collapseBlockedRuns` in `web/src/components/message-rows.ts` (tested), rendered by `MessageList.svelte` |
| Profile actions | `web/src/components/MemberList.svelte` |
| Words | `friends`, `sidebar.friends*`, `messages.blocked*`, `members.addFriend` and neighbours in `web/src/i18n/en.ts` and `fr.ts` |
| Tokens | `component.friends.*`, `component.message.blocked-*` in `design/tokens.json` |
| Icons | `chat-circle`, `prohibit`, `user-minus` (Phosphor 2.1.1 Fill) in `crates/mellow-desktop/icons/` |

## Checked

Headless Chrome against a local instance built from the branch, seeded through the REST API with nine people: friends who share no server (Bob, Chloé, David), friends online in do not disturb and idle with custom statuses, two requests received, one sent, and a block whose author wrote three messages in two runs in `#general`. English and French, dark and light, 1280 px and 390 px; the More menu and its keyboard (arrows, Esc back to More), the remove dialog (Cancel focused), a revealed run, the profile of someone you asked and of someone you blocked, the tabs with arrow keys, accepting a request (announced "You're now friends with Emma.", focus moved to the next row's Accept), and the empty tabs as an account with no friends. Not checked: a real screen reader (VoiceOver, NVDA), 200 % text zoom, Windows forced colours, touch on a real phone, and the folded rows with long histories loading older pages.
