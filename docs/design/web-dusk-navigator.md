# Web client: the Dusk navigator layout

Phase 2 of [ADR-0022](../adr/0022-give-mellow-its-own-visual-language-on-a-familiar-layout.md) for the web client. The spec is the accepted mockup, `design/mockups/dusk-direction.html` ("Navigateur" layout); this page records how the web client maps it, so later work keeps the two identical.

## Layout

```
wide (>= 1064 px, members docked)                      phone (< 816 px)
+--------------------------------------------------+  +----------------------+
| ground: 16 px above, 8 px leading/bottom          |  | = #general       [m] |
| +----------+ +-------------------------+ +-----+  |  |                      |
| |strip|list| | # general | topic   [m] | |Memb.|  |  |  messages            |
| |     |    | |                         | |     |  |  |                      |
| |     |    | |  messages               | |     |  |  | typing row           |
| |     |    | |  typing row             | |     |  |  | [+  Message      >]  |
| |[ user card ]| [+  Message #general ] | |     |  |  +----------------------+
| +----------+ +-------------------------+ +-----+  |  navigator = drawer,
+--------------------------------------------------+  members = full window
```

- **Navigator card** (`Navigator.svelte`): a grid of `component.navigator.strip-width` and `component.sidebar-width`, with `UserCard` spanning both columns at the foot.
- **Sheet** (`ChatPane.svelte`): radius `component.sheet.radius`; square and full-bleed below 816 px, where messages keep `space.md` trailing padding instead of `component.message.padding-trailing`.
- **Side sheet** (`MemberList.svelte`): members, then a member's profile with "Send message". Closed by default, remembered per server for the tab's life, toggled by the header button or `Ctrl+U`, closed by Esc. Docked from 1064 px; between 816 and 1064 px it floats over the sheet in a modal `Drawer` (top `3 × inset`, trailing and bottom `2 × inset`, as in the mockup); below 816 px it covers the window.
- **Breakpoints** come from the tokens in `Shell.svelte`: inset + strip + list + inset + `size.chat.min-width` + inset = 816 px, plus side sheet + inset = 1064 px.
- **Ground spacing**: the mockup keeps an empty row for its "Palette" top bar and an empty column for the side sheet, so its cards sit 16 px from the top and, while the side sheet is closed, 16 px from the trailing edge. The web client reproduces those values on purpose.

## Alignment

The user card and the composer's base row are both `size.dock.height` tall and `size.dock.margin` above the bottom of their card. The composer's attach and send buttons are centred on that row, the text area's block padding is `(dock height − message line) / 2`, and extra content (reply bar, attachments, more lines) grows the composer upward. The typing row (`component.composer.typing-height`) sits above the composer.

## Motion

Every entrance answers an action and uses the `motion.duration.*` tokens, which reduced motion sets to 0 ms (component duration tokens such as `dialog.enter-duration` are not, so they are not used for entrances): the drawer and side sheet slide in (`slow`, `enter`, `enter-offset` tokens), scrims fade (`base`, `standard`), the server squircle and dialogs settle (`settle`), the profile rises, the server menu drops in, the reply bar fades, the unread bar slides down. The typing dots loop only while someone types and stop under reduced motion.

## Tokens added for this layout

`component.message.reply-height`, `component.message.reply-spine-radius`, `component.composer.typing-dot-{size,gap,lift,duration,stagger}`, `component.user-panel.toggle-strike-{length,cut-width}`, `component.channel-row.unread-pill-offset`, `component.navigator.enter-offset`, `component.side-sheet.enter-offset`. Changed to match the mockup: `color.surface.sidebar-footer` (light: one step below the white navigator), `component.user-panel.button-background-hover` and `component.composer.reply-bar-background` (`state.selected`), `component.auth-card.background` (`surface.navigator`).

Later additions: `color.avatar.tint-1…8` and `color.avatar.initial` (default avatars; the mockup's blue and green darkened in OKLCH to reach 4.5:1 with white, all eight pairs checked by the generator), `component.quick-switcher.{width,input-height,option-height}`, `component.category.add-icon-size`.

## Content features

- **Default avatars**: the initial in white on `color.avatar.tint-N`, N from an FNV-1a hash of the user id (`avatarTintToken`); a profile without a banner picture uses the same tint for its strip.
- **Role colours**: the member's highest role with a colour tints their name in messages (on `surface.chat`) and in the side sheet (on `side-sheet.background`), its OKLCH lightness moved until 4.5:1 in the current theme (`role-colors.ts`).
- **Mentions**: "@username" or a one-word display name of someone the client knows becomes a chip; a message mentioning you gets the mention tint and the 2 px apricot bar, and "Mentions you." for screen readers. The server decides who a message mentions (`Message.mention_ids`, same rule as `mentions.ts`); the chips are still matched client-side, and an older server without `channel_unreads` in `Ready` falls back to the client-side match for the highlight too.
- **Unread state and mention badges**: `Ready.channel_unreads` gives every viewable channel its newest message and unread mention count, so a channel no session opened shows unread (heading text, the 8 px pill) and its count. `MellowClient` keeps both apart from the guild snapshots, adds live mentions outside the channel being read (in a conversation: every message from someone else), follows edits and deletions of cached messages, and resets the count when the channel is read. The red badge (`MentionBadge.svelte`, `component.badge.*`, `type.badge`, tabular figures, never animated) sits at the trailing end of channel and conversation rows (the gear takes its place on hover and focus) and on the bottom trailing corner of each server and of Home, with the sum of their channels, cut out of the navigator by `badge.ring-width` and drawn above the channel pills it can touch; "99+" from `MAXIMUM_COUNTED_UNREAD_MENTIONS`. The badge is hidden from screen readers: the row or the icon says "general, unread, 3 mentions" ("général, non lu, 3 mentions").
- **Unread divider and jump bar**: opening a channel keeps the previous read position for the visit (`MellowClient.unreadMarkers`); the "New" divider sits above the first message from someone else after it (scrolled into view when above it), and the bar says "N new messages since HH:MM" with "Mark as read" (also Esc in the composer). The channel is still marked read at once; leaving it ends the visit.
- **Quick switcher**: `Ctrl+K` or "Go to… Ctrl K" (hidden below 816 px, like the mockup); channels, people and servers, `#`, `@` and `*` prefixes, combobox and listbox.
- **Category "+"**: for people who can manage channels; "Create channel" then says "in General" and sends `parent_id`.

## Not in the web client yet

- Read positions and mention counts moved by the user's other sessions: the gateway sends no read-state event, so they follow at the next `Ready`.
- Member groups by hoisted role ("Moderation 2"): roles have no hoist flag in the protocol.
- Idle and do-not-disturb dots, and activity text: presence only says online or offline.
- Microphone and headset toggles are present but shown in development builds only (`import.meta.env.DEV`) until voice ships.
