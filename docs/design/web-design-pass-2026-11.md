# Mellow web client: full design pass (v0.45.0)

Status: audit and plan, 2026-11-05, copied into the repository with WP0 (DESIGN.md v0.53), which set the tokens, the code face and the spec below; the owner decisions N1, S2 and S3 were taken on 2026-11-05 as proposed (a neutral `+`, choice rows edged in `border.strong`, the ink switch). One deviation from D1: `button.disabled-text` is `text.muted`, not `text.disabled` (under 2:1 on the wash), so a disabled button's label stays readable. One addition to S3: in light the switch's off track is `border.strong` instead of `neutral.fill`, because the ink on track and the dusk off track with the same white knob were only 2.76:1 apart there; they are now 4.56:1 apart (7.55:1 in dark), guarded by a contrast pair. The audit itself changed nothing in the repository. Every finding below names its place in the code at commit `78a8714` (v0.45.0, branch `chore/progress-docs-after-audit`), what is wrong against DESIGN.md (Dusk, ADR-0022) and the tokens, the fix, and a size: **S** (under half a day), **M** (one to two days), **L** (more than two days). Findings are then grouped into work packages, each one pull request, ordered by what the owner will notice first.

## 1. How this was checked

- A throwaway instance (the `mellow-server` debug build of 2026-11-05 01:50, with polls, GIFs and link previews) ran on `127.0.0.1:18190` with Vite on `http://localhost:5173` (`MELLOW_DEV_INSTANCE=http://127.0.0.1:18190`). It was seeded through the REST API with five French users, a server "Atelier" with a category, three text channels and a voice channel, and a realistic conversation: Markdown (bold, italic, list, quote, inline code, spoiler, mention), a TOML code block, a JPEG, an MP4, a text file, a reply, reactions, a link to rust-lang.org (previewed), links to GitHub, YouTube and French Wikipedia (not previewed by this build in time), a poll with votes, an edited message and a forward.
- Playwright (`playwright-core` 1.64 from `../Alcove`) drove Google Chrome in French: dark at 1440 × 900, light at 1440 × 900, dark at 390 × 844 with touch, light at 320 × 640 with touch. It opened the emoji picker, the "/" and "@" lists, the message menu, the quick switcher, search with results and with its filter list, the pins panel, the server menu, server settings (overview, roles), channel settings, user settings (account, voice), the create-server dialog, the poll and forward dialogs, the picture viewer, a voice join (refused: the UDP port was taken, which showed the voice problem card), the members side sheet, Home and Friends, and the voice gallery (`web/dev/voice.html#call=grid`, `#call=speaker`, `#stream=live`).
- 70 screenshots were taken (`<theme>-<width>-<screen>.png`; `m-*.png` are phone strips). They are **not in the repository** (they lived in the audit's scratch directory); the names cited below say which screen and size showed each finding, so it can be shot again the same way.
- The components were grepped for hard-coded values. The result is good news: no hex colour, no `rgba()`, no Tailwind default type size, radius, shadow or duration in any `.svelte` file; the only arbitrary values are `calc()` expressions built from tokens and flex shorthands. **The token discipline is not the problem.** What the owner will notice is hierarchy, phone fit, a few states that are still missing, a few Discord tells left in the tokens, and one typeface.

Not rendered: the profile popover (the script did not reach it), the GIF picker (no Klipy key), the status menu and appearance page in detail, a real voice call, text at 200 %, Firefox and Safari, a screen reader. These are listed again in section 5.

## 2. What the owner will notice first

1. **Code looks like a typewriter.** Every code block and inline code renders in Courier on macOS: `type.code` falls back to the generic `monospace` because Source Code Pro is "not bundled yet" (DESIGN.md 2.4). It is the ugliest thing on screen next to Figtree.
2. **A bright blue ring around the composer whenever it is focused**, which is almost always. The ring is meant for keyboard focus only (DESIGN.md 7.2), but a textarea matches `:focus-visible` on a mouse click too.
3. **Phones: the channel name disappears.** At 320 px the header keeps the menu button, Aller à, search, settings, pins and members (six 44 px keys), so "#general" becomes "gene…". The placeholder "Écrire dans #general" wraps on two lines and grows the composer, the jump bar wraps on three lines, and the notification permission bar takes 100 px of a 640 px screen.
4. **No day dividers.** A conversation that spans several days reads as one block; DESIGN.md 4.5 still lists them as missing on the web.
5. **"/ Liste numérotée" writes a list the messages cannot show**: `markdown.ts` has no ordered lists, no subtext (`-#`) and no nested lists, while the "/" menu offers them.
6. **The hover bar on your own message has ten buttons** (three reactions, add reaction, edit, reply, forward, copy, delete, more): 330 px of icons. DESIGN.md 5.4 says Edit replaces Reply on your own messages and Delete lives in More.
7. **Discord tells still in the tokens**: green "+" to add a server and green switches with a check and a cross are Discord's exact controls; disabled primary buttons are apricot at reduced opacity, which reads as a muddy brown button ("Créer", "Transférer").
8. **Apricot is spent too freely in settings and Friends**: an apricot "Envoyer une icône", "Changer le mot de passe", apricot-bordered radio cards and apricot sliders on one page; on Friends both the "Ajouter un ami" tab and the empty-state button are apricot fills for the same action.
9. **Connection states are thin**: only "connecting" and "reconnecting" exist (`ConnectionBanner.svelte`); going offline showed nothing within four seconds, and there is no "Back online", no "Still trying… Retry now", no "session ended".
10. **Link previews are plain cards**, and a white `theme-color` (rust-lang.org) draws a glaring white pill in the dark theme. The provider embeds are designed separately in `rich-embeds-design.md` (a separate design note, not in the repository yet).

## 3. Findings by area

Each row: **where** (file:line at v0.45.0), **problem**, **fix**, **size**. "DESIGN" means DESIGN.md.

### 3.1 Typography and code

| # | Where | Problem | Fix | Size |
|---|---|---|---|---|
| T1 | `design/tokens.json:2271` (`font-family.mono`), `web/src/lib/tokens.css:1306,1423` | Stack is `"Source Code Pro", "Noto Sans Mono", monospace`; neither face is bundled, so Chrome on macOS shows Courier (screenshots `dark-1440-chat-up3`, `light-1440-chat-up3`). Thin slab serifs next to Figtree, poor `0O` and `1lI` distinction, the opposite of DESIGN 2.4's intent. | Bundle Source Code Pro (variable, OFL 1.1, upright only) in `design/fonts`, add its `@font-face` in `web/src/app.css` next to Figtree with `font-display: swap` and no preload (it downloads only when a code element renders). Until then, or as the tail of the stack: `ui-monospace, "SF Mono", Menlo, "Cascadia Mono", Consolas, "DejaVu Sans Mono", monospace`. | S |
| T2 | `web/src/lib/tokens.css:1787` (`component.message.code-border` light = `surface.rail` `#F5F1F6`) | In the light theme a code block (`surface.code` `#F8F4F8`) on the white sheet has an edge of `#F5F1F6`: the block has no visible boundary (`light-1440-chat-up3`). The dark value (`#110B11` on `#221B23`) works. | Alias `component.message.code-border` to `border.divider` in light (`#E3DDE4`), keep the dark value. Decorative, so no 3:1 needed, but it must be seen. | S |
| T3 | Messages, French text (`dark-390-chat-up1`) | "Trois points de mon côté :" wraps with the colon alone on the next line at 390 px. User text, but the UI is French-first. | Optional: when the interface language is French, render a plain space before `: ; ! ? »` and after `«` in message text as U+202F (display only, the stored text is untouched). Skip if the owner prefers raw text. | S |

### 3.2 Shell, chat header and search field

| # | Where | Problem | Fix | Size |
|---|---|---|---|---|
| H1 | `web/src/components/ChatPane.svelte:540-640` | On phones the header holds the drawer button, the channel name, "Aller à…" (compass), search, channel settings (managers), pins and members. At 320 px the name has about 40 px ("gene…", `m-light320a`). | In the drawer layout keep: drawer button, name, search, members. Move "Aller à…", pins and channel settings into one "Plus d'options" (`dots-three`) key opening the existing `ContextMenu` (items with their icons and shortcuts). The name gets at least half the header (`max-w-1/2` already exists on desktop). | M |
| H2 | `web/src/components/MessageSearchField.svelte:58` (placeholder `placeholderInServer(guild.name)`) | "Rechercher dans Atelier" truncates to "Rechercher dans At" in the header at rest (`dark-1440-home`), "Rechercher dans les" at Home. A sentence cut mid-word reads broken. | Placeholder "Rechercher" / "Search" at rest; the scope goes in the accessible name ("Rechercher dans Atelier") and appears in the placeholder only once the field expands on focus (where it fits, `dark-1440-search-suggest`). | S |
| H3 | `ChatPane.svelte:586-590` (topic) with members open (`dark-1440-members`) | With the member sheet docked, the topic is cut to "Discussions générales de l'atelier, une idée …" while "Aller à…" and the search field keep their full width. | Let the topic shrink last only to a minimum, and collapse "Aller à…" to its icon key (with its tooltip and `Ctrl K`) below a header width computed from tokens, before the topic is cut. | S |
| H4 | `dark-390-chat-up3` (focus ring on the drawer button after the drawer closes by touch) | Focus returns to the menu button after a tap and shows the blue ring, which is noise on touch. | Return focus there only when the drawer was opened from the keyboard; after a pointer or touch open, return it without a visible ring (focus with `focusVisible: false`, supported in Chrome and Firefox). | S |

### 3.3 Navigator, rail and channel list

| # | Where | Problem | Fix | Size |
|---|---|---|---|---|
| N1 | `web/src/components/ServerRail.svelte:224` with `color.positive.fill` (DESIGN 5.2 "then `+` (add) in green") | The green add-server button is Discord's. With Dusk the rail is otherwise Mellow's own. | Owner decision. Proposal: the "+" on `surface.chat` like an initials icon, glyph in `text.secondary`, and on hover/focus the squircle on `state.selected` with `text.heading` (the same treatment as servers without artwork); "Ajouter un serveur" stays its name and tooltip. Green stays for presence and success. | S |
| N2 | `ChannelSidebar.svelte:596-602` (server menu) with `ContextMenu.svelte:169,177` | The server menu takes the header's width (about 215 px at the minimum sidebar width), so "Paramètres de notification…" wraps on two lines (`dark-1440-server-menu`). | Width `max(anchor width, component.context-menu.min-width)`, and `w-max` up to `component.context-menu.max-width` when an item is longer, aligned to the header's leading edge. | S |
| N3 | Server menu, first item "Inviter des gens" in `text.accent` (`dark-1440-server-menu`) | Matches DESIGN ("Apricot text: Invite people") and Discord. Fine; listed only because it is the one apricot item in a neutral menu, which is deliberate. | None. | – |

### 3.4 Message list and rows

| # | Where | Problem | Fix | Size |
|---|---|---|---|---|
| L1 | `web/src/components/message-rows.ts`, `MessageList.svelte:531-560` (rows) | No day dividers (DESIGN 4.5 "Not in the web client yet"; spec in 5.4: a `border.divider` rule with a centred date in `type.message-timestamp`, `text.muted`). A day boundary also must break grouping (5.4 "Structure and grouping"). | Add a `day-divider` row kind in `message-rows.ts` between messages whose local calendar day differs (pure, tested with fixed time zones), render it as a `role="separator"` with `aria-label` "5 novembre 2026" / "5 November 2026", break the group there, and let the "Nouveau" divider share the row when both fall on the same boundary (Discord draws one line with both labels). | M |
| L2 | `MessageList.svelte:634-643` (bottom jump bar), `i18n/fr.ts:599-600` | On phones "Tu consultes des messages plus anciens" plus "Revenir au présent" wraps on two lines at 390 px and three at 320 px (`m-light320a`), covering the last message. | Below the drawer breakpoint, show only the action ("Revenir au présent", and "· 3 nouveaux" when new messages came) on one line, centred, with a `caret-down` glyph; the sentence stays as the accessible description. Desktop unchanged. | S |
| L3 | `ChatPane.svelte:652-660` (top unread bar) | Same wrapping risk for "12 nouveaux messages depuis 14:32" + "Marquer comme lu" at 320 px (not reached in the run). | Same rule: below the drawer breakpoint, shorten to "12 nouveaux" + "Marquer comme lu". | S |
| L4 | `MessageItem.svelte:526-580` (hover bar) | Your own message shows Edit **and** Reply, Forward, Copy, **Delete** and More besides four reaction keys: ten 32 px buttons (`dark-1440-chat-up4`). DESIGN 5.4: "Reply (others' messages) or Edit (your own)", Delete in More. Copy also shows on a message without text (Bob's file-only message). | Own message: reactions, Edit, Forward, Copy (only with text), More. Others: reactions, Reply, Forward, Copy, More. Delete stays in More and on Shift+hover (Discord shows Delete in the bar only while Shift is held; keep that as the power-user path, with `Shift`+click deleting at once as DESIGN already says). | S |
| L5 | `MessageItem.svelte` hover bar position (`dark-1440-chat-up3`) | The bar sits half over the previous message's last line. Discord does the same; acceptable. | None. | – |

### 3.5 Message content

| # | Where | Problem | Fix | Size |
|---|---|---|---|---|
| C1 | `web/src/components/markdown.ts`, `MessageMarkdown.svelte` | No numbered lists, subtext `-#` or nested lists, while the "/" menu offers "Liste numérotée" (DESIGN 4.5 lists them as "Not yet"). Choosing it writes `1. ` which then renders as plain text. | Parse `1.`/`1)` lists (keep the starting number, `<ol start>`), `-#` subtext (`type.message-subtext`, `text.muted`), and one level of nesting by indentation (two spaces or a tab), as Discord does. Tree only, never HTML; tests for each. | M |
| C2 | `MessageAttachments.svelte:93-97, 166-177` (video band) | At 320 px the band shows "lip.mp4282,4 Ko": the file name's tail (`shrink-0`) overflows into the size, the head is cut to nothing (`m-light320a`, fourth frame). | Give the tail `min-w-0` with its own ellipsis when the head is gone, and below a band width of `component.attachment.video-band-compact-width` hide the size (it stays in the download link's accessible name and the viewer's band). | S |
| C3 | `PollCard.svelte:168-173` (results column, `component.poll.count-min-width` 56 px) | At 320 px an answer wraps on six lines ("Chacun / ramène son / plat, on / partage en / salle de / réunion") because the mark, the 56 px column and paddings leave about 100 px of text (`m-light320a`, third frame). | Below a card width token, show only the percentage in the column (the vote count stays in the voters button's name and in the row's accessible name), which gives back about 30 px; and drop the column's inline padding to `space.2xs`. | S |
| C4 | `MessageLinkPreviews.svelte:93-97`, `link-previews.ts:85` (`linkPreviewBarColor`) | A white `theme-color` (rust-lang.org) draws a bright white pill on the dark card; a near-white one vanishes in light (`dark-1440-chat-up1`). Documented as "Not done" in `docs/design/web-link-previews.md`. | Drop the page's colour when it reaches under 1.5:1 against the card's background (read once from the token at runtime through `components/tokens.ts`), falling back to `bar-fallback`. Belongs with the embeds series (E1 in `rich-embeds-design.md`). | S |
| C5 | `MessageLinkPreviews.svelte:110` (`line-clamp-3`) | The description's three-line limit is a literal, not a token. | Token `component.link-preview.description-lines` (3), and `wikipedia-extract-lines` (6) for the Wikipedia card. | S |
| C6 | Picture viewer band (`dark-1440-picture-viewer`) | The file uploaded as `photo.jpg` is named `photo.avif` (the instance's re-encoding, ADR-0033, shows through). People recognise their file by its name. | Outside `web/`: keep the original base name with the new extension only in the download name, and show the original name in the band and cards. Flag to the server owner; the web change is nil if the protocol carries the original name. | S (server) |
| C7 | `MessageSearchPanel.svelte:456` | A forwarded message in the search results shows its copied text under the forwarder's name without "Transféré" or the origin (`dark-1440-search`), so it reads as if Ana wrote Chloé's words. | Render the forward chrome (quote bar, "Transféré", origin line) in results as in the list, reusing the list's forward snippet. | S |
| C8 | Spoiler (`dark-1440-chat-up3`) | Fine: a grey bar of the hidden text's width. | None. | – |

### 3.6 Composer and its menus

| # | Where | Problem | Fix | Size |
|---|---|---|---|---|
| P1 | `web/src/components/Composer.svelte:621` (`has-[textarea:focus-visible]:focus-ring`) | A textarea matches `:focus-visible` on every focus, so the 2 px sky ring surrounds the composer whenever it is focused, which is almost always (`dark-1440-slash-menu`, `dark-1440-reply-bar`, `m-dark390a`). DESIGN 7.2: "shown on keyboard focus only". It is the loudest element on screen. | Show the composer's ring only when focus arrived by keyboard: track the last input modality once in the shell (`keydown` of Tab/F6/arrows sets `data-input-modality="keyboard"` on `<html>`, `pointerdown` sets `pointer`) and style the composer with `[data-input-modality=keyboard] &:has(textarea:focus)`. Other text fields keep their ring on focus (they are entered on purpose and have no other boundary). | S |
| P2 | `Composer.svelte:686-687` (placeholder) | "Écrire dans #general" wraps on two lines at 320 px and the empty composer grows to 80 px (`m-light320a`). | One line with an ellipsis (`placeholder:truncate` on the textarea, or `::placeholder { white-space: nowrap; text-overflow: ellipsis; overflow: hidden }`), the full text staying the accessible name. | S |
| P3 | Emoji picker (`dark-1440-emoji-picker`) | The picker's bottom edge overlaps the composer's top by about 10 px. | Anchor it `component.emoji-picker.gap` above the composer's top edge. | S |
| P4 | "/" menu (`dark-1440-slash-menu`) | The underline row's Markdown chip `__` renders as a solid bar in Courier; fixed by T1. | T1. | – |

### 3.7 Members and profile

| # | Where | Problem | Fix | Size |
|---|---|---|---|---|
| M1 | `MemberList.svelte` (`dark-1440-members`, `m-dark390b`) | Clean. Offline avatars are dimmed; names stay readable. Group counts are tabular and separate, as DESIGN 10 asks. | None. | – |
| M2 | Profile popover | Not rendered in this run. | Check in WP8's visual pass. | – |

### 3.8 Settings (user, server, channel)

| # | Where | Problem | Fix | Size |
|---|---|---|---|---|
| S1 | `IconEditor.svelte:160` ("Envoyer une icône" primary), `AccountSettings.svelte:301` ("Changer le mot de passe" primary), `RoleSettings.svelte:273` ("Créer un rôle" primary) | Each settings page opens with an apricot button for an occasional action, competing with the unsaved-changes bar's "Enregistrer", the one action that should glow (`dark-1440-server-settings`, `dark-1440-user-settings`). DESIGN 1.2: "at most one apricot fill". | Secondary (`neutral.fill`) for upload icon, change password and create role; apricot stays for "Enregistrer les modifications" in `UnsavedChangesBar` and for dialog confirmations. | S |
| S2 | `ChoiceOption.svelte` (6.55 radio rows; `dark-1440-user-settings-voice`, `dark-1440-add-server`) | The chosen row gets a full apricot border plus an apricot dot; with three sliders filled apricot and a primary button the voice page holds five apricot marks. | Owner decision. Proposal: chosen row = `border.strong` edge, `state.selected` wash and `text.heading` label; only the dot stays apricot (`accent.indicator`). Sliders keep their apricot fill (a level is "you"). Needs DESIGN 6.55 updated. | S |
| S3 | `Switch.svelte` with `component.switch.track-on` = `positive.fill` (`design/tokens.json:5690`) | Green track with a check, grey with a cross: Discord's switch, pixel for pixel (`dark-1440-server-settings-roles`, `dark-1440-create-poll`). | Owner decision. Proposal "ink switch": on = `text.heading` track (`#F8F4F8` dark / `#221B23` light) with a `surface.chat` thumb carrying the check in `text.heading`; off = `neutral.fill` track, thumb with the cross. Contrast: track-on against `surface.chat` 15.4 / 16.8. Calm, monochrome, unmistakably not Discord, and keeps apricot for "you". | S |
| S4 | Settings layout (`dark-1440-server-settings`, `dark-1440-channel-settings`) | Pages with two fields leave 70 % of the sheet empty; the round "Échap" close is Discord's. Familiar (DESIGN 1.1), so keep; but the overview pages feel unfinished. | Keep the structure. Give the overview pages a live preview where it helps (server: the navigator header with icon and name, as the create dialog already has with `ServerPreview.svelte`; channel: the channel row and chat header). | M |
| S5 | `RoleSettings.svelte` ("Ctrl+Maj+Haut et Ctrl+Maj+Bas déplacent aussi le rôle choisi." under the list, `dark-1440-server-settings-roles`) | A keyboard hint permanently under a one-role list, in 12 px muted text. | Show it only when the list holds at least two movable roles, and as the list's `aria-description` always. | S |

### 3.9 Dialogs and buttons

| # | Where | Problem | Fix | Size |
|---|---|---|---|---|
| D1 | `Button.svelte:43` (`isDisabled && "… opacity-(--mw-component-button-disabled-opacity)"`) | A disabled primary is apricot at reduced opacity over the popover: a muddy brown button that still reads as clickable ("Créer", "Transférer", `dark-1440-add-server`, `dark-1440-forward`). | Disabled buttons of every variant: `component.button.disabled-background` (`state.selected`) and `disabled-text` (`text.disabled`, exempt and always paired with a reason, as DESIGN 2.3 says), no opacity. | S |
| D2 | `AddServerDialog.svelte:237-251` | The ghost "Envoyer une icône" under the 80 px icon starts 16 px to the right of the icon's edge (its padding), and the title is centred over a left-aligned form (`dark-1440-add-server`). | Pull ghost buttons that stand under a field or picture by their inline padding (as DESIGN does for "Clear status"), and left-align the dialog title and lead with the form, as the poll and forward dialogs already do. | S |
| D3 | `CreatePollDialog.svelte`, `ForwardMessageDialog.svelte` | Good hierarchy, counters and helpers. | None. | – |

### 3.10 Voice and call

| # | Where | Problem | Fix | Size |
|---|---|---|---|---|
| V1 | `VoicePanel.svelte` problem card (`dark-1440-voice-join`) | Clear copy and a close button; the title wraps in the 240 px navigator, fine. | None. | – |
| V2 | `CallView.svelte` (`dark-1440-gallery-call`) | Solid: tiles, speaking ring in apricot, control dock. The odd fifth tile centred alone is right. | None. | – |

### 3.11 Search and pins

| # | Where | Problem | Fix | Size |
|---|---|---|---|---|
| R1 | Search suggestions (`dark-1440-search-suggest`) | Descriptions are right-aligned at the far edge, 150 px from their key: the eye zigzags. | Description right after the key, `text.secondary`, truncating first, as in the "/" menu rows. | S |
| R2 | Pins empty state (`dark-1440-pins`) | Good: glyph, title, one sentence. | None. | – |
| R3 | C7 above. | | | |

### 3.12 Home and Friends

| # | Where | Problem | Fix | Size |
|---|---|---|---|---|
| F1 | `FriendsPane.svelte:262-263` (apricot "Ajouter un ami" tab) and `:339` (primary in the empty state) | Two apricot fills for the same action on one screen (`dark-1440-home-friends`). | While the empty state shows its primary button, the tab is a plain tab (`text.accent` label, no fill), as Discord's tab is quiet when the page itself invites the action. | S |
| F2 | `ConversationList.svelte:68` (Home navigator, `dark-1440-home-friends`) | "Aucune conversation pour l'instant…" is plain text where DESIGN 5 asks for "a short title, one sentence, one action". | Title "Pas encore de conversation", the sentence, and "Nouveau message" (secondary) opening the people picker. | S |

### 3.13 States across the app

| # | Where | Problem | Fix | Size |
|---|---|---|---|---|
| X1 | `ConnectionBanner.svelte:10-16` | Only "connecting" and "reconnecting". Missing (DESIGN 5.2, 4.5): device offline ("Tu es hors ligne. Les messages enregistrés restent lisibles."), back online for 2 s, "Toujours en reconnexion. Tes messages partiront à ton retour." with "Réessayer maintenant" after 30 s, session ended ("Ta session sur {instance} est terminée. Reconnecte-toi pour continuer." with "Se connecter"). Going offline in Chrome showed nothing within 4 s (`dark-1440-offline`). | Listen to `online`/`offline` in the component for the device state; the 30 s and session states need `client.svelte.ts` to expose `reconnectingSince` and a `session-ended` status (a small lib change, coordinate). Keep the bar's place and tokens. | M |
| X2 | `NotificationPermissionBar.svelte:50-60` | On phones the bar takes about 100 px (two lines of text, then the button on its own line) above the header on every visit until dismissed (`m-dark390a`). | Below the drawer breakpoint: one 48 px row, bell, "Notifications désactivées", "Activer" (primary, compact), close. The full sentence stays in the bar's accessible name. | S |
| X3 | Sign-in (`SignInScreen.svelte:93-128`, `required` fields without `novalidate` on the form at `:67`) | Submitting empty fields shows Chrome's native bubble "Veuillez renseigner ce champ." (`dark-1440-signin-empty-submit`), foreign to Mellow's voice and to DESIGN 5.1. | `novalidate` on the form, and Mellow's field errors under each field ("Entre ton nom d'utilisateur ou ton e-mail.", "Entre ton mot de passe."), focus on the first, announced. | S |
| X4 | Sign-in hero (`dark-1440-signin`) | The card is a plain centred form on black; Melli reads small and its shadow ellipse sits under the card's edge. DESIGN 5.1 makes this the one screen where Melli is the hero. | Keep the card. Melli at its full `auth-card.melli-size` visible above the edge (the cloud itself, not its box, at 96 px), the shadow dropped on the card, and Melli's `connecting` pose while the form submits. No gradient or glow on the ground. | S |

### 3.14 Phone layout, 320 to 430 px (summary)

H1, H4, L2, L3, P2, C2, C3, X2 above, plus:

| # | Where | Problem | Fix | Size |
|---|---|---|---|---|
| Q1 | Message menu under a coarse pointer (`m-dark390b`, `m-light320b`) | A right-click (emulated long press) opens the small desktop menu at the point, over the hover bar which stays visible. DESIGN 4.5 turns pickers into bottom sheets on phones; the message menu is the most used of them. | Under `(pointer: coarse)` and the drawer layout, open the message menu as a bottom sheet (`Drawer.svelte` from the bottom, 48 px rows, the quick reactions as a first row of large emoji), and hide the hover bar while it is open. | M |
| Q2 | Code block copy key at 320 px (`m-light320a`, last frame) | The 44 px copy key covers the end of a scrolled line. Acceptable with horizontal scroll; noted. | None now. | – |

### 3.15 Light theme (summary)

T2 (code edge), C4 (white pill), and the hover wash `#FBF9FC` on white being almost invisible (`light-1440-chat-up1`). That one is deliberate (calm) and the focused message also shows its hover bar, so it stays. The known muted-on-selected pair (4.36:1, see the project memory) did not appear on any rendered screen.

## 4. Work packages

One PR each, in this order. **WP0 lands first**: it is the only package that edits `design/tokens.json`, `design/icons`, `design/fonts` and DESIGN.md, so the others never conflict on them and can run in parallel worktrees afterwards. Each later package touches only the components listed, adds its strings to its own section of `web/src/i18n/{fr,en}.ts` (separate hunks merge cleanly), updates its own note in `docs/design/`, and runs `npm run check`, `npm test`, `npm run build` with zero warnings. Visual check: Playwright screenshots at 1440 dark, 1440 light, 390 dark touch and 320 light touch, French, before and after, in the PR.

### WP0. Tokens, font and spec for the pass (S–M)
- Files: `design/tokens.json`, generated `web/src/lib/tokens.css` (`cargo xtask tokens`), `design/fonts/SourceCodePro*.ttf` + licence, `web/src/app.css` (`@font-face` only), DESIGN.md (2.3, 2.4, 5.2, 5.4, 6.55, 7.2, changelog v0.53), `design/icons` (`caret-down` if missing, `dots-three` exists).
- Content: T1 (font and stack), T2 (light code border), C5 (`description-lines`, `wikipedia-extract-lines`), D1 (`button.disabled-background`, `disabled-text`), C2 (`attachment.video-band-compact-width`), C3 (`poll.compact-width`), and the owner's choices for N1, S2, S3 (token values only; components follow automatically where they read tokens).
- Acceptance: `cargo xtask tokens --check` passes, every new colour pair is declared in `contrast-pairs` and passes in both themes, a code block renders in Source Code Pro in Chrome (computed font), DESIGN.md records each decision.

### WP1. Composer focus and placeholder (S) — highest visibility
- Files: `web/src/components/Composer.svelte`, `web/src/components/Shell.svelte` (modality tracker, a few lines), `web/src/components/input-modality.ts` + test.
- Content: P1, P2, P3.
- Acceptance: clicking the composer shows no ring; Tab or F6 into it shows the 2 px ring; at 320 px the empty composer is 56 px tall with a one-line placeholder; the emoji picker leaves a gap above the composer.

### WP2. Chat header and search on phones (M)
- Files: `ChatPane.svelte` (header and top unread bar only), `MessageSearchField.svelte`, `MessageSearchPanel.svelte`.
- Content: H1, H2, H3, H4, L3, R1, C7.
- Acceptance: at 320 px with a manager account the header shows the drawer key, at least "#general" in full, search, members and "Plus d'options"; every moved action stays reachable by keyboard and screen reader with its shortcut; the search placeholder never cuts a word; a forwarded message in results shows "Transféré" and its origin.
- Shipped (branch `fix/web-phone-header-and-search`): below the drawer breakpoint the header is drawer key, name (all the room left), search, members and "Plus d'options" (`ContextMenu` anchored at its end: "Aller à…" with `aria-keyshortcuts="Control+K"`, "Messages épinglés", "Ajouter des amis au groupe", "Paramètres du salon"); the pins panel opens from it and gives focus back to it. At 320 px "#general" is whole (it was "#g"). The search field reads "Rechercher" at rest and its scope once focused (the sheet's field keeps the scope; long scopes end in an ellipsis). `trigger-collapse-width` became 976 px (the chat's minimum width plus the trigger at rest and the focused field), so with members docked at 1440 px "Aller à…" is a compass and the topic is whole, and with results docked the topic shows again instead of vanishing. Drawer focus returns without the ring after a tap or click (`focus({ focusVisible: false })`, checked in Chrome), with it after Tab and Enter. The phone unread bar reads "10 nouveaux" on one line, the sentence kept for screen readers. Filter descriptions follow their key in `text.secondary`. Search slips render a forward's quote bar, "Transféré" and origin line ("#design · Aujourd'hui à 16:13"). Deviation from H1: the moved items carry their shortcut as `aria-keyshortcuts`, not as a visible hint, because `ContextMenu` items have no hint slot and WP4 owns that component.

### WP3. Day dividers and the bottom jump bar (M)
- Files: `message-rows.ts` + tests, `MessageList.svelte`.
- Content: L1, L2.
- Acceptance: messages on two days show one divider with the date (the full date, as DESIGN 5.4 and Discord show it: French "5 novembre 2026", English "November 5, 2026"), the group breaks there, the unread divider and the day divider share one row when they meet, the jump bar is one line at 320 px; deterministic tests with a fixed time zone; no layout shift when an older page loads above.

### WP4. Hover bar and menus (M)
- Files: `MessageItem.svelte` (hover bar only), `ContextMenu.svelte`, `ChannelSidebar.svelte` (server menu instantiation), `Drawer.svelte` (bottom variant if needed), `message-actions.ts`.
- Content: L4, N2, Q1.
- Acceptance: own message bar = reactions, Edit, Forward, Copy (if text), More; others' = reactions, Reply, Forward, Copy, More; Shift shows Delete; the server menu never wraps an item at the minimum sidebar width; on a touch phone the message menu is a bottom sheet with 48 px rows and focus returns to the message.

### WP5. Markdown gaps (M)
- Files: `markdown.ts` + tests, `MessageMarkdown.svelte`.
- Content: C1.
- Acceptance: `1. a` / `2. b` renders an ordered list starting at its number, `-# petit` renders subtext, an indented `- b` under `- a` nests once, nothing renders as HTML, all existing markdown tests pass.

### WP6. Attachments and polls on narrow screens (S)
- Files: `MessageAttachments.svelte`, `PollCard.svelte`.
- Content: C2, C3.
- Acceptance: at 320 px the video band never overlaps name and size; a four-word poll answer fits on at most three lines; screen readers still hear vote counts.

### WP7. Connection, notification and sign-in states (M)
- Files: `ConnectionBanner.svelte`, `NotificationPermissionBar.svelte`, `SignInScreen.svelte`, a small addition in `web/src/lib/client.svelte.ts` (coordinate with whoever owns the client model).
- Content: X1, X2, X3, X4.
- Acceptance: Chrome's offline toggle shows the offline bar within one second and "De retour en ligne" for 2 s after; after 30 s of reconnecting, "Réessayer maintenant" appears and works; an expired session shows the sign-in prompt; on a 390 px phone the notification bar is one 48 px row; empty sign-in fields show Mellow's errors, never the browser bubble.

### WP8. Apricot discipline and the last Discord tells (S–M, owner decisions first)
- Files: `Button.svelte`, `Switch.svelte`, `ServerRail.svelte`, `ChoiceOption.svelte`, `IconEditor.svelte`, `AccountSettings.svelte`, `RoleSettings.svelte`, `FriendsPane.svelte`, `ConversationList.svelte`, `AddServerDialog.svelte`, `ServerOverviewSettings.svelte`, `ChannelOverviewSettings.svelte`.
- Content: D1, D2, N1, S1, S2, S3, S4, S5, F1, F2, and the profile popover check (M2).
- Acceptance: no screen shows more than one apricot fill besides Home, mentions and the drop overlay (check every settings page and Friends); disabled buttons never look brown; the add-server key and switches follow the owner's choice; Home's empty conversation list has a title, a sentence and an action.

### WP-E. Provider embeds (L, a series)
- Designed in `rich-embeds-design.md` (a separate design note, not in the repository yet) (slices E0 to E5). E0 (ADR, protocol, server fetchers, CSP) is server work; E1 to E5 are web packages touching `MessageLinkPreviews.svelte`, `link-previews.ts` and new `components/embeds/*`. They do not touch any file of WP1 to WP8 except C4/C5 (moved there). Start E1 after the in-flight `fix/youtube-links-play-inline` worktree has landed.

Order by owner impact: WP0, WP1, WP2, WP3, WP7, WP6, WP4, WP8, WP5, then WP-E in parallel from WP0 on. WP1 to WP6 share no component file; WP7 and WP8 share none with them; WP2 and WP4 both read the hover-bar keyboard model but edit different files.

## 5. Not verified, and risks

- Not rendered: profile popover, GIF picker, status menu, appearance settings, the "Aller à" results with many hits, a real call with audio, the stream view in a real share, text zoomed to 200 %, Firefox, Safari, iOS, a screen reader. WP8's visual pass should include the popover and status menu.
- Link previews for GitHub, YouTube and Wikipedia did not arrive in this run (the instance fetched rust-lang.org only within the session); their current rendering is taken from the code and from `docs/design/web-link-previews.md`.
- The offline finding (X1) relies on Playwright's `setOffline`, which may not close an open WebSocket at once; the code confirms that no device-offline state exists in `ConnectionBanner.svelte`.
- N1, S2 and S3 change the look of controls the owner has seen for weeks; they were decided on 2026-11-05 as proposed and recorded in DESIGN.md v0.53.
- C6 is server behaviour (file names after re-encoding).
