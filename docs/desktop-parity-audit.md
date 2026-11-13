# Desktop parity audit (wave 10, steps 2 and 3)

2026-11-13. The web client (`web/`) is the reference; the GPUI desktop client (`crates/mellow-desktop`) must look and behave like it (ADR-0051, ROADMAP.md M3 "Next (owner, 2026-11-13…)").

## Method

- Local test instance (`target/debug/mellow-server`, `dev/seed.py` data), both clients signed in as alice, server "Atelier", dark theme, 1440 × 900.
- Web: Chrome through the DevTools MCP. Desktop: the `MELLOW_PROFILE=test` app in a headless gamescope, driven with `xdotool`.
- The same screen captured on both sides, in the same state, then compared against DESIGN.md and the tokens.
- Captures are in the session scratchpad, `shots/d-<screen>.png` (desktop) and `shots/w-<screen>.png` (web): shell, welcome, hover, msg-menu, channel-menu, server-menu, rail-menu, emoji, emoji-search, slash, members, profile, home, dm, quick, quick-typed, search, user-settings, us-profile, us-voice, us-appearance, us-notifications, server-settings, channel-settings, voice, call, signin.
- Headless Chrome reports `(hover: none)`, so Tailwind `hover:` styles never apply there. To capture the web hover bar, the rules inside `@media (hover: hover)` were copied into a page-level style. This is a property of the test browser, not a product bug.

Severity: **blocking** means a user cannot do or understand something the web allows. **Visible** means anyone comparing the two clients sees the difference. **Detail** means a measurable gap that is only noticed side by side.

## Summary

| Severity | Count |
|---|---|
| Blocking | 4 |
| Visible | 25 |
| Detail | 13 |

T5-4 is not counted: it follows from T1-2 and T1-3.

Five independent tracks, grouped so that each one touches its own files:

| Track | Scope | Main files |
|---|---|---|
| T1 Type, primitives, Melli | Font weights, switch and choice-group labels, hit sizes, text fields, the Melli component | `theme/fonts.rs`, `theme/mod.rs`, `components/{switch,choice,text_field,button,settings_sheet,range_field}.rs`, new `components/melli.rs` |
| T2 Shell | Rail, sidebar header and server menu, channel rows, chat header, Home and friends, member list | `shell/{server_rail,channel_sidebar,conversation_sheet,place_menus,member_sheet}.rs`, `home/{mod,friends_pane}.rs` |
| T3 Messages and composer | Markdown, code, list paddings, jump bar, composer, slash menu, emoji picker, message menu, search results | `messages/*`, `conversation/*`, `composer/*`, `emoji_picker.rs`, `message_search/*`, `mellow-client-core/src/shared_logic/context_menus_people.rs` |
| T4 Overlays | Tooltips, quick switcher | `components/tooltip.rs`, `components/menu_opening.rs`, `shell/quick_switcher.rs`, `shell/overlays.rs` |
| T5 Settings, voice, sign-in | Settings pages, voice panel, sign-in card and instance choice | `settings/**`, `shell/settings_screens.rs`, `shell/voice_panel.rs`, `sign_in_screen.rs`, `sign_in_form.rs` |

Order and dependencies:
- T1 goes first, or at least lands T1-2, T1-3 and T1-7 first. T5 depends on the switch and choice-group fix. T2 and T5 depend on the Melli component.
- T2 and T4 both touch tooltips on the rail. Tooltip placement and dismissal belong to T4. T2 only passes the side.
- The font-weight cause (T1-1) is the lead's.

---

## T1 — Type, primitives and Melli

| ID | Screen | Web | Desktop | Reference | Severity | Desktop file | Expected fix |
|---|---|---|---|---|---|---|---|
| T1-1 | Everywhere | 600 for author names, `**bold**` and labels. 700/800 for titles: the welcome hero, "Mon compte", the profile name, "Membres" | Everything is drawn at 400 | DESIGN §2.4, `type.*` weights | Visible (cross-cutting) | `theme/fonts.rs`, `theme/mod.rs` | The lead owns this: the variable Figtree weights are not applied. After the fix, recheck `type.message-author`, `type.display`, `type.heading-*`, `type.category`, `type.badge` and Markdown bold |
| T1-2 | Settings: Notifications, Appearance, Voice, About; also roles, invites, member management, the voice panel and the notification dialog | Each switch is a row: the label in `type.body-lg`, the description in `type.body-sm` muted, the switch at the trailing end, and a divider between rows (`NotificationSettings.svelte`) | Only the bare switch is drawn. Its label and description are only set as `aria_label` and `aria_description`. The Notifications page shows three unlabelled switches | DESIGN §5.9, `Switch.svelte` | **Blocking** | `components/switch.rs` (render) | Draw the label and the optional description, with the switch at the trailing end and the whole row clickable. The doc comment already says "the row also shows" the label. Add a divider between stacked rows, as on the web |
| T1-3 | Appearance (theme, language), Voice (microphone, output, input mode, noise suppression), other choice groups | Each group has a visible legend in `type.category` ("Images animées", "Mode d'entrée", "Suppression du bruit") | The legend is only set as `aria_label`. The two "Comme cet ordinateur" groups on Appearance cannot be told apart, and the Voice microphone and output lists have no heading | DESIGN §2.4 rules (form labels at 13 px semibold, `text.muted`) | **Blocking** | `components/choice.rs` (render) | Render the group label above the options (`type.category`, `text.muted`, `component.settings.section-label-gap`) |
| T1-4 | Settings nav, buttons, switches, sliders | Mouse sizes: settings nav rows are `component.settings-nav.item-height` (32 px, 34 px pitch). 44 px only under a coarse pointer | `SIZE_HIT_WEB_MIN` (44) is applied unconditionally: nav rows have a 46 px pitch, and the settings pages are visibly looser | DESIGN §7.4 (desktop is pointer-first, 32 px rows) | Visible | `components/settings_sheet.rs:269`, `button.rs`, `switch.rs`, `range_field.rs`, `text_field.rs`; also `settings/server/roles.rs`, `soundboard/popover.rs` | Use the desktop token sizes and drop the `.max(SIZE_HIT_WEB_MIN)` |
| T1-5 | Text fields (server name, channel name, sign-in) | Value in `type.body-lg` (16 px). Leading icon then 8 px, then the text | Value at about 14 px. The text starts 4–8 px further right; the `#` prefix has a wider gap | DESIGN §2.4 `type.body-lg` ("form values") | Visible | `components/text_field.rs` | Use `TYPE_BODY_LG` for the value and match the web's inline padding and icon gap |
| T1-6 | Profile side sheet buttons | 44 px tall | 40 px tall | `component.button.*` | Detail | `components/button.rs` | Use the button height token |
| T1-7 | Home rail button, sign-in card, friends empty state, call states | The Melli component (`Melli.svelte`), with its states | No Melli component exists on the desktop: the rail shows a chat-bubble glyph, and the sign-in card and empty states have no mascot | DESIGN §2.10, §5.1, §5.2 | Visible | new `components/melli.rs` (SVG from `web/src/components/Melli.svelte` / `design/`) | Port Melli: idle, connecting, connection-lost, still under reduced motion. T2 and T5 then use it |

## T2 — Shell: rail, sidebar, chat header, Home, members

| ID | Screen | Web | Desktop | Reference | Severity | Desktop file | Expected fix |
|---|---|---|---|---|---|---|---|
| T2-1 | Rail, Home | The Mellow mark (Melli) on the Home button | A chat bubble | DESIGN §5.2 ("Home (Mellow mark…)") | Visible | `shell/server_rail.rs` | Use the T1-7 component |
| T2-2 | Rail | "+ Ajouter un serveur" at the bottom (create a server, join with an invite, connect another instance) | Missing. A signed-in user cannot create or join a server, or reach another instance | DESIGN §5.1 "Adding another instance", §5.2 | **Blocking** | `shell/server_rail.rs` (+ dialog, see `web/src/components/AddServerDialog.svelte`) | Add the neutral `+` (`rail.add-icon`) and its menu. "Connect another instance" also answers the owner's "no visible way to pick another instance" |
| T2-3 | Sidebar header ("Atelier") | A button with a caret. A click opens the server menu: Inviter des gens (accent), Paramètres du serveur, Créer un salon, Créer une catégorie, Rendre muet…, Paramètres de notification…. Open, the caret becomes a close (×) | A plain heading: no caret and no hover state, and a click does nothing | DESIGN §5.3, `ChannelSidebar.svelte` | **Blocking** | `shell/channel_sidebar.rs:181` (heading), `shell/place_menus.rs` (`server_menu_entries`, `ServerMenuPlace`) | Make the header a menu button with a caret and open the server menu. Invite, create channel and create category are disabled through facts (`can_invite_people: false`, `can_create_channels: false`, documented shortcut): their dialogs come with this work |
| T2-4 | Channel list, category | "+" (Créer un salon dans …) at the trailing end of the category header | Missing | DESIGN §5.3 | Visible | `shell/channel_sidebar.rs` (shortcut at :250) | Add the button with the create-channel dialog |
| T2-5 | Channel list, selected row | A gear (Modifier le salon) on the selected or hovered row | Missing | DESIGN §5.3 | Visible | `shell/channel_sidebar.rs` | Add the trailing gear button |
| T2-6 | Chat header | From the leading end: "Aller à… Ctrl K" pill (`QuickSwitcherTrigger.svelte`), the search field, Paramètres du salon, Messages épinglés, Liste des membres. With the side sheet open, "Aller à…" folds into a compass icon | A magnifier (go-to) first, then a gear, the search field, pins and members. No "Aller à…" label and no shortcut hint | `QuickSwitcherTrigger.svelte`, DESIGN §5.10 | Visible | `shell/conversation_sheet.rs:391` | Draw the pill (`component.quick-switcher-trigger.*`, the tokens already exist in `tokens.rs`) before the search field, and the gear after it. Use the compass icon when folded |
| T2-7 | Rail and channel context menus | Server: Inviter des gens, Paramètres du serveur, notifications, Copier l'identifiant. Channel: Modifier, notifications, Monter, Descendre, Déplacer vers une catégorie…, Copier l'identifiant, Supprimer le salon | The server menu has no "Inviter des gens". The channel menu has no move or delete items | `context_menus_places.rs` facts | Visible | `shell/place_menus.rs:279, :361, :396` | Turn the facts on as each screen lands (same work as T2-3 and T2-4) |
| T2-8 | Member list, offline rows | Avatar at 50 %, no status shape | Avatar dimmed, plus a grey offline ring | DESIGN §5.7 ("offline rows show no shape") | Detail | `shell/member_sheet.rs` | Hide the status dot for offline members |
| T2-9 | Home, conversation rows | 50 px pitch, 12 px from avatar to name (`component.conversation-row.avatar-gap` = `space.sm`). The sidebar header sits 6 px lower | 44 px pitch, 8 px gap | `component.conversation-row.*` | Detail | `shell/channel_sidebar.rs` (`conversation_rows`) | Use the conversation-row tokens |
| T2-10 | Friends, empty state | Melli above, the sentence wrapped to a readable measure (about 450 px) and centred, then the "Ajouter un ami" button | No Melli. The sentence runs on one 620 px line | DESIGN §5.6.1, list states | Visible | `home/friends_pane.rs` | Add Melli (T1-7) and a max width on the text |
| T2-11 | Channel list, category header | Caret at x 86, label at x 96 | Caret at x 93, label at x 104 | `component.category.*` | Detail | `shell/channel_sidebar.rs:420` | Align the paddings with the web |
| T2-12 | Rail, server without artwork | A lighter squircle for the selected server (`state.selected`) | A darker fill | DESIGN §5.2 table | Detail | `shell/server_rail.rs` (`rail_item`) | Use `state.selected` for selected icons without artwork |

## T3 — Messages, composer, emoji picker, search

| ID | Screen | Web | Desktop | Reference | Severity | Desktop file | Expected fix |
|---|---|---|---|---|---|---|---|
| T3-1 | Markdown lists | Disc bullets at text size, `margin-inline-start: space.md` | Smaller dots, closer to the text | `MessageMarkdown.svelte` `.markdown-list` | Visible | `messages/markdown_view.rs` | Size the bullet as a disc of the message font, with an outside marker at `space.md` |
| T3-2 | Inline code and code blocks | Source Code Pro at `type.code` (14/18), `text.normal`, a subtle `component.message.code-background` chip | Grey text with wider letter spacing. The code-block text is dimmer than the web's. The friend code in "Mon compte" is grey as well | DESIGN §2.4 (Source Code Pro, `type.code`) | Visible | `messages/rich_text.rs:279`, `messages/code_block.rs`, `theme/fonts.rs` | Check that Source Code Pro is the face in use (spacing suggests a fallback) and draw code in `text.normal` |
| T3-3 | Message list | First group at y 89, 35 px between the last line and the composer | First group 15 px lower, 26 px between the last line and the composer | DESIGN §4.2 list paddings | Detail | `conversation/` (list container) | Align the top and bottom paddings with `MessageList.svelte` |
| T3-4 | Composer | Placeholder and text in `type.message` (16 px), text starting 12 px after the attach button | Placeholder about 14 px, starting 10 px further right | DESIGN §2.4 `type.message` ("composer text"), §5.5 | Visible | `composer/editor.rs`, `composer/mod.rs` | Give the input `TYPE_MESSAGE` and the web's inline padding |
| T3-5 | Composer focus | No ring around the composer while typing | A bright focus ring around the whole composer while typing | DESIGN §5.5 | Detail | `composer/mod.rs` | Follow the web: no ring on the composer's text area |
| T3-6 | Jump bar ("Tu consultes des messages plus anciens") | Fixed to the top of the composer, like a tab of the same card | A detached pill with a caret, 24 px above the composer | DESIGN §5.4 "Unread divider and jump bars" | Detail | `conversation/` (jump bar) | Attach it to the composer as on the web |
| T3-7 | Message context menu | Ajouter une réaction, then Répondre, Transférer, Copier le texte, Épingler, then Supprimer. The quick reactions are a header row only in the phone sheet. No "Copier l'identifiant du message" | Adds three rows "Réagir avec 👍/❤️/😂", each with a generic smiley icon, and "Copier l'identifiant du message" | DESIGN §5.4 Hover action bar; web `context-menus.ts:85` | Visible | `mellow-client-core/src/shared_logic/context_menus_people.rs:102`, `conversation/mod.rs:1748` | Decide (open question 1), then align the core list with the web's |
| T3-8 | Emoji picker (composer) | Opens above the composer with an 8 px gap, so the composer stays visible | Overlaps the composer's trailing end and covers its emoji button | DESIGN §6 emoji picker | Visible | `composer/emoji_button.rs`, `emoji_picker.rs` | Anchor the picker to the top of the composer |
| T3-9 | Emoji picker, People row 4 | 😐 (neutral face) in colour | A monochrome outline "☺" glyph in its place | §9 Twemoji COLR | Visible | `emoji_picker.rs` / emoji font fallback in `theme/fonts.rs` | Check that U+1F610 resolves to Twemoji. Other code points may fall back to a text font in the same way |
| T3-10 | Slash menu ("/") | 8 px above the composer | Touches the composer's edge | DESIGN §5.5 | Detail | `composer/slash_menu.rs` | Add the gap token |
| T3-11 | Search results | No button at rest. The match is bold on the highlight | A "Voir" button always shown. The match is highlighted but not bold | DESIGN §5.12 | Detail | `message_search/` | Show "Voir" on hover or focus only, and make the match bold (after T1-1) |

What matches already: the hover action bar (order, icons, size and position), reply lines, mentions, quotes, links, reactions, the date divider and the channel-start hero (except its weight).

## T4 — Overlays: tooltips, quick switcher

| ID | Screen | Web | Desktop | Reference | Severity | Desktop file | Expected fix |
|---|---|---|---|---|---|---|---|
| T4-1 | Tooltips everywhere | A tooltip hides when its button is pressed or when an overlay opens | The tooltip stays on screen after the click, on top of what opened. Seen with "Paramètres utilisateur" over the settings sheet, "Paramètres du salon", "Choisir un emoji" under the picker, "Atelier" on top of the rail menu, and "Messages privés" over the Home header | DESIGN §6 tooltip | Visible (bug) | `components/tooltip.rs` | Dismiss on mouse down, on focus loss and when a menu, popover or sheet opens |
| T4-2 | Rail tooltips | Appear at once on the trailing side of the icon | Appear below the icon and overlap the sidebar header | DESIGN §5.2 ("to the trailing side") | Visible | `components/tooltip.rs`, `shell/server_rail.rs` | Add a trailing placement and use it on the rail |
| T4-3 | Quick switcher (Ctrl+K) | Input in `type.heading-lg` (20/26, 700): placeholder "Où veux-tu aller ?" and typed text | About 15 px regular | DESIGN §2.4 (`type.heading-lg`: "quick switcher input") | Visible | `shell/quick_switcher.rs` | Give the input `TYPE_HEADING_LG` |

The rest of the quick switcher matches: sections, rows, trailing hints and footer.

## T5 — Settings, voice, sign-in

| ID | Screen | Web | Desktop | Reference | Severity | Desktop file | Expected fix |
|---|---|---|---|---|---|---|---|
| T5-1 | Settings sheet frame | The sheet card 8 px from the window edges. The nav column starts at x 271 | 16 px from the top. The nav column starts at x 265 | `size.sheet.inset` (8) | Detail | `components/settings_sheet.rs` | Use `size.sheet.inset` |
| T5-2 | Settings page text | Intros and descriptions in `type.body` (14/20). Section labels in `type.category` | Descriptions in 16 px (Profil: "Ton initiale s'affiche…") | DESIGN §2.4 `type.body` (descriptions) | Visible | `settings/user/parts.rs` (`body_text`) and the pages | Use `TYPE_BODY` for descriptions and keep `body-lg` for values |
| T5-3 | User settings › Profil | Two columns: the form, and an "Aperçu" profile card (banner, avatar and name). "+ Ajouter un lien" is a leading-aligned ghost button. "Enregistrer" sits at the leading end | One full-width column, no preview, "Ajouter un lien" as centred text without an icon, "Enregistrer" at the trailing end | DESIGN §5.9, `ProfileSettings.svelte` | Visible | `settings/user/profile.rs` | Add the preview card and match the button placements |
| T5-4 | User settings › Voix | Labelled groups (devices, "Mode d'entrée", "Suppression du bruit"). "Réglage automatique" is a row with a description and the switch at the trailing end | Unlabelled groups and a bare switch | as T1-2 and T1-3 | (follows T1-2 and T1-3) | `settings/user/voice.rs` | Recheck once T1 lands. Add the "Suppression du bruit" sub-label |
| T5-5 | User settings, app section | "Accessibilité" (Images animées: Animer au survol / Toujours animer) | "Apparence" (theme, language, keep running in the background), with no animated-pictures setting | ROADMAP step 6 gap | Detail | `settings/user/appearance.rs` | Known gap. Name the section as the web does once the setting exists |
| T5-6 | Server settings › Vue d'ensemble | The icon editor (initials, "Envoyer une icône"), the name field, an "Aperçu" of the sidebar header, and "Supprimer le serveur" (danger) in the nav | The name field alone. No "Supprimer le serveur" | DESIGN §5.8 | Visible | `settings/server/overview.rs` (documented shortcut), `shell/settings_screens.rs` | Add the icon editor (with the crop dialog), the preview and the danger nav entry |
| T5-7 | Channel settings › Vue d'ensemble | "Icône du salon" with "Envoyer une icône", then name and topic. "Supprimer le salon" (danger) in the nav | Name and topic only. No delete entry | DESIGN §5.8 | Visible | `settings/channel/overview.rs` (documented shortcut), `settings/channel/mod.rs` | As T5-6 |
| T5-8 | Voice panel (user card dock) | Row 1: status, sound and hang-up. Row 2: camera with its caret, "Partager l'écran", "1080p" | The camera sits in row 1 beside the sound button, with no caret | DESIGN §5.x voice panel; ROADMAP step 8 gap | Detail | `shell/voice_panel.rs` | Move the camera into the second row, as on the web. The caret is a known gap |
| T5-9 | Sign-in | Melli stands on the card's top edge. The password field has a "Show password" eye. Titles in `type.heading-xl` 700 | No Melli and no eye (documented shortcut) | DESIGN §5.1 | Visible | `sign_in_screen.rs`, `sign_in_form.rs` | Add Melli (T1-7) and the reveal toggle |
| T5-10 | Sign-in, instance choice | Not applicable: the web is served by its instance | The instance appears only in the title ("Se connecter à 127.0.0.1:8080"). "Utiliser une autre instance" is a small link at the bottom of the card. Once signed in, there is no way back to it except signing out (T2-2) | ROADMAP M3 "make choosing one's server obvious" | Visible | `sign_in_screen.rs` | Under the title, add an instance row (instance icon or initials, address, and a "Changer" secondary button) in place of the bottom link. The rail "+" gets "Se connecter à une autre instance" (T2-2) |

## Functional bugs seen in passing

1. The server header does nothing on click and has no hover state (T2-3).
2. Tooltips stay on screen after their button is used, on top of the opened overlay (T4-1).
3. The 😐 emoji is drawn as a monochrome "☺" in the picker (T3-9).
4. Switch and choice-group labels exist only for screen readers, so sighted users cannot tell what a switch does (T1-2 and T1-3).
5. Tab and Shift+Tab move focus correctly from the composer (composer, then the emoji button; back to the attach button). The reported "Tab does not always move focus" was not reproduced here. It is still to be checked in the settings sheet, the side sheet and the dialogs.
6. Voice handover works: joining the same channel from the web moves the call and shows "Voix déplacée — Tu as rejoint la voix depuis un autre onglet ou appareil" on the desktop.

## Not captured

- The desktop call view: by the time the desktop showed the call, the voice had moved to the web (handover above). `w-call.png` exists, with no desktop counterpart.
- The desktop "instance address" first step and the web sign-in in the dark theme: both need signing out, which this audit must not do. `w-signin.png` is in the light theme.
- The user status menu, the pinned messages panel, the GIF picker, the forward dialog and the reactions dialog.
- Everything at narrow widths (816 and 1064 breakpoints) and in the light theme.

## Open questions

1. **Message menu contents (T3-7).** Recommended: follow the web on desktop pointers. Keep "Ajouter une réaction" only, and show the quick reactions as one row of emoji keys at the top (the phone sheet's header row) rather than three text rows. Hide "Copier l'identifiant du message" until a developer-mode setting exists on both clients. Changing `message_menu_actions` in client-core changes both the order and the tests.
2. **Hit sizes on desktop (T1-4).** Recommended: follow DESIGN §7.4 (mouse sizes, 32 px rows) and keep 44 px for a later touchscreen-laptop decision (§9).
3. **Instance choice (T5-10).** Recommended: an instance row with "Changer" under the sign-in title, plus "Se connecter à une autre instance" in the rail "+" menu, as DESIGN §5.1 describes.
