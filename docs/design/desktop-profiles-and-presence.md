# Desktop: profiles and presence

M3 step 4 for the GPUI client (ADR-0051), at parity with the web's `MemberList.svelte` profile, `ProfileCard.svelte`, `ProfileDetails.svelte`, `user-profile-loader.svelte.ts`, `UserCard.svelte`, `CustomStatusDialog.svelte` and `custom-status.ts`. No new token, string or icon: everything below reuses `component.profile-popover.*`, `component.member-row.*`, `component.side-sheet.*` and the web's catalogue.

## The profile in the member sheet

```
 side sheet (240, or the window below the side sheet breakpoint)
 ┌──────────────────────────┐
 │ ‹  Profile            ✕  │  header 48: Back (caret left, focused on open), title, Close
 │▓▓▓▓▓▓ banner 120 ▓▓▓▓▓▓▓▓│  picture, cover; without one a 60 strip in the avatar tint
 │ (◯◯)                     │  avatar 80, ring 6 cut from the sheet, one text gap under the middle line
 │ Ana                      │  heading-lg, name text
 │ ABOUT ME                 │  category, section label text; inline Markdown through MessageMarkdown
 │ Reads Rust on Sundays    │
 │ LINKS                    │  rows 28: icon on its light plate (or the link glyph), label (host)
 │ [▣] Blog (example.org)   │  opens in the browser, http(s) only, tooltip with the address
 │ [ Send message ]         │  primary, full width; "Opening…" while the conversation opens
 └──────────────────────────┘
```

- Choosing a member row (click or Enter) replaces the list with the profile; Back returns to the list with focus on that row. The person is read from the server's member list on every frame, so a new name or avatar shows at once; another server's sheet shows its own list.
- "About me" and links are read when the profile opens (`UserProfileLoader` in client-core): an answer for a profile no longer shown is dropped, someone the reader shares nothing with shows no details, any other failure says "Couldn't load the About me and links."
- "Send message" opens or creates the one-to-one conversation, closes the member overlay and puts focus in the composer, as the quick switcher does. Your own profile and a blocked person's show no action.
- Member rows are buttons now: hover wash `member-row.background-hover`, name `member-row.name-text-hover`, inset focus ring on the row radius.

## Own presence

- The user card's status menu (online, idle, do not disturb, invisible) already existed; it gains "Set custom status" or "Edit custom status" between the statuses and "Copy user ID".
- The custom status dialog is `ModalDialog` with one `TextField` ("What are you up to?"), its helper counting characters left or too many (Unicode scalar values of the trimmed text, 128, `MAXIMUM_CUSTOM_STATUS_LENGTH` now in `mellow-protocol`), Save status enabled only for a changed text within the limit, Enter to save, Esc to cancel, and Clear status once there is one. Focus starts in the field and returns to the user card. A refusal names what to fix; any other failure says why.
- Status and custom status are sent with `PUT /users/@me/presence`, as the web does (there is no gateway frame for presence).

## Rejected defaults

- A floating popover anchored to whatever was clicked: the web shows profiles in the side sheet, and two different places for the same card would split the habit between clients.
- Role chips and "member since" in the card: the web card has neither; they would be desktop-only facts.
- Auto-idle after inactivity and status expiry choices: the web has neither, and presence is stored by the instance, so an automatic "idle" sent by one client would outlive a crash or quit and overwrite the status the person chose. Both need a protocol decision (a transient, per-session presence) first.
