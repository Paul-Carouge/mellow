# Mellow design document

Status: draft v0.22 (2026-10-27), visual direction proposed in ADR-0022. Owner: design. Applies to the Slint desktop client, the web client, the Android client (Jetpack Compose, Material 3 Expressive) and the admin web UI embedded in the server.

This document is the reference for every visual and interaction decision in Mellow. Developers should be able to build any MVP screen from it without inventing a colour, a size or a sentence. When something is missing, add it here first, then implement it. Every value named here exists in `design/tokens.json`.

## Changelog

- **2026-10-27, v0.22 — The web client on every screen size, and installable as an app:** every web screen was checked from 320 to 1920 px wide, in phone landscape and with text at 200 %; the breakpoints of 4.2 stay as they are. The page now runs under notches, the installed app's status bar and the home indicator (`viewport-fit=cover`), and whatever fills the window keeps its content clear of them with the safe-area insets: the shell, the navigator drawer, the full-window members sheet, the phone settings and the bottom sheets (emoji picker, pins). The soft keyboard shrinks the layout (`interactive-widget=resizes-content`), so the composer stays above it. In the drawer the channel list gives up width on the narrowest phones instead of being cut (4.5). Menus opened from a message or a friend open above their button when the window has no room below (6.33). Every Friends tab, the Friends row, conversation rows and the Add reaction chip reach 44 px under a coarse pointer (7.4). File cards cut long names in the middle at any width, keeping the extension; the unread and jump bars wrap their sentence on the narrowest phones instead of cutting it; the quick switcher keeps the channel name when the server's name is long. The web client is installable from Chrome, Edge and Android Chrome: a manifest and its icons (192 and 512 px, a full-bleed maskable icon, an Apple touch icon) and the `theme-color` of each scheme are generated from the tokens by `cargo xtask tokens` (2.10, 4.5); there is no service worker. Details in [docs/design/web-responsive-and-installable.md](docs/design/web-responsive-and-installable.md).
- **2026-10-27, v0.21 — Group conversations and "New message" on the web (ADR-0026):** a `+` beside "Direct messages" at Home opens the **people picker** (5.6.2): your friends with a search that ignores case and accents, the ones picked as chips inside the field (Backspace takes the last back, Enter picks the first match), "You can add 9 more friends.", and native checkboxes in `accent.indicator` (apricot is "you": your choice). One friend gives "Message" (opens or finds your conversation), several "Create group" with an optional "Group name" whose placeholder is the names it will otherwise show. A group's picture is two of its people leaning on each other, the front face cut out of the surface (`component.group-avatar.*`, 6.8); its row and chat header show its name, or "Ana, Bob and 2 others", and the member count instead of presence. The chat header adds "Add friends to the group" and the member toggle (`Ctrl+U`); the side sheet of a group lists its actions as the server menu words them (Add friends in apricot, Rename group, Leave group in red) above everyone in it, you included, the owner said in words ("Owner"), and for the owner a Remove button per person. Leave and Remove use the destructive dialog of 8; Leave says who owns the group next, or that it goes with its last person. Leaving or being removed returns to Friends and is read out. A group stays open with someone you blocked in it: their messages fold as in servers. Groups appear in the quick switcher; a person there leads to their own conversation, never a group. New tokens `component.group-avatar.*` and `component.people-picker.*`, contrast pairs for `accent.indicator` on `surface.popover`, `text.accent` on `surface.navigator`, `text.muted` and `text.heading` on `state.hover`, `text.on-neutral` on `neutral.fill`. Details in [docs/design/web-group-conversations.md](docs/design/web-group-conversations.md).
- **2026-10-26, v0.20 — Friends and blocking on the web (ADR-0025):** Home opens on the **Friends space** when no conversation is open, as Discord's Home does (5.6.1). A Friends row tops the conversations with a red badge counting the requests received. The space sits in the sheet: the title, then tabs Online, All, Pending (with the same red badge), Blocked and Add friend, the last being the screen's one apricot fill until it is open. Friend rows carry round actions (Message, More with Remove friend and Block); Pending rows Accept or Ignore, or Cancel for requests sent; Blocked rows Unblock. Remove friend and Block confirm in the destructive dialog of 8; Block says the other person is never told. Add friend is one username field with Send friend request inside it, answering under it in words; a missing account, yourself and a block read the same ("Hm, that didn't work. Check the username and try again."). Melli keeps the empty tabs company (sleeping when no friend is around). In chats, each run of messages by someone you blocked folds into one row "2 blocked messages · Show" (5.4); a reply to them reads "Blocked message" and they are never read out. The member profile gains Add friend, Accept friend request, Block and Unblock (5.7). New tokens `component.friends.*`, `component.message.blocked-*`, contrast pairs for the positive and danger icons on `surface.input`, `text.accent` on `surface.chat` and `text.secondary` and `text.heading` on `state.hover-message`; Phosphor `chat-circle`, `prohibit` and `user-minus` (Fill). Details in [docs/design/web-friends.md](docs/design/web-friends.md).
- **2026-10-26, v0.19 — Presence statuses and the user card's status menu on the web:** the four status shapes of 2.3 are drawn as true cut-outs on a 20-unit grid (6.9: a dot online, a crescent idle, a disc with a rounded bar for do not disturb, a ring offline and invisible), on every avatar that shows presence: member rows, conversation rows and the user card. Member rows show the custom status as their second line and say it in their accessible name (5.7). The user card's avatar and name become one button that opens the status menu above the card (5.3): the four statuses with their shape, a one-line hint for Do not disturb and Invisible and a check on yours; Set or Edit custom status, which opens a one-field dialog (a characters-left hint counted as the instance counts, Save status, Cancel, Clear status); and Copy user ID, confirmed on the card's own second line. New tokens `component.status.size-menu` (12 px), `component.user-panel.status-text-hover` and `component.context-menu.item-hint-text-focus` (`text.secondary`: muted text is 4.36:1 on the hover wash in light), and contrast pairs for the status colours on `surface.floating` and `surface.sidebar-footer`. Details in [docs/design/web-presence-and-status-menu.md](docs/design/web-presence-and-status-menu.md).
- **2026-10-22, v0.18 — Channel settings and icons on the web:** channel settings (5.8) open from a gear on the channel row (shown on hover, focus and on the selected row) and in the chat header, in the server settings' sheet: Overview (icon, name, topic) and Permissions (the overwrite list, `@everyone` always in it, and a Deny / Inherit / Allow control per channel permission, 6.6, `component.channel-settings.*`). The control follows Discord's order (Deny on the leading side, Allow on the trailing side) and marks Inherit with a dash (`minus`, Phosphor Bold) since Phosphor 2.1.1 has no plain slash. Server and channel icons (6.54, `component.icon-picker.*`): a square crop dialog opening on the centred square, with drag, wheel, pinch, arrow keys and a zoom slider, and the result at its real size in the strip or a channel row; the picture is sent as chosen, so animated GIFs stay animated. Server icons replace the initials in the strip (still, animated on hover and focus, never under reduced motion, 5.2); channel icons are rounded squares (`component.channel-row.image-radius`: circles are people) in place of `#` in the row and the chat header, the lock of a read-only channel moving after its name (5.3).
- **2026-10-22, v0.17 — Reactions and pins on the web:** reaction chips under messages (`component.reaction.*`: yours take the apricot wash and edge, since apricot is "you"), three quick reactions and Add reaction in the hover bar, an emoji picker (6.52, `component.emoji-picker.*`), a More menu with Pin/Unpin, Reactions and Remove all reactions (5.4), a pin beside pinned messages, and the pinned messages panel under the chat header's pin button (6.53, `component.pins.*`). Section 9's question 9 is settled for reactions, the picker and pins.
- **2026-10-22, v0.16 — Dusk after the owner's first review (ADR-0022, proposed):** the layout no longer copies Discord's: a **navigator** card holds the server strip, the channel list and the user card; the member list becomes a **side sheet** opened on demand (`Ctrl+U`), docked when the window is wide; the chat header gets an "Aller à… Ctrl K" button (4.2, 5.3, 5.7, 5.10). The user card and the composer's base row share `size.dock.height` (56 px) and `size.dock.margin`, so their edges line up; the typing indicator moves above the composer (4.2, 5.5). The user card gets microphone and headset toggles, hidden until voice ships (5.3). The app icon and favicon put Melli on a dusk gradient tile with an apricot glow (`color.mascot.tile-*`, 2.10). French becomes a first-class language: tutoiement, French typography, French counterparts for the copy of sections 7 and 8, and a localisation plan in ADR-0022. The dark ground deepens (`ref.dusk.160`) and `color.surface.navigator` is added.
- **2026-10-22, v0.15 — Dusk, Mellow's own visual language (proposed, [ADR-0022](docs/adr/0022-give-mellow-its-own-visual-language-on-a-familiar-layout.md)):** the layout, behaviour and shortcuts stay Discord's; the look becomes Mellow's own. Principles rewritten (1): familiar where it matters; one ground, one sheet, one lamp; motion answers. Plum-tinted dusk neutrals (`ref.dusk`, named by OKLCH lightness) replace Discord's greys (`ref.grey` removed): rail, sidebar and member list share one ground, the conversation is a lit sheet inset from the window edges (2.2, 2.3, 4.2). Apricot means "you" or "now" only: the selected-server pill (`component.rail.pill-color-selected`, `color.accent.indicator`), the unread divider and an apricot "New" pill; hovered servers and focused menu items use the neutral selected tone. Sentence-case labels at 13 px instead of uppercase, tighter display type, 24 px message lines (2.4). Concentric radius scale 4/8/12/16/24 with `radius.xl` (2.6). Motion rewritten as state transitions with a key transitions table, `motion.duration.settle`, `motion.easing.settle` and Android springs (2.8). Member groups show their count as a separate number instead of "Moderators — 2" (5.7). The user panel becomes a card floating on the ground (5.3); the sign-in card gives Melli room (5.1). Token values change in place, so both clients compile and show the new colours, radii and type at once; the structural changes are phase 2 (ADR-0022).
- **2026-10-21, v0.14 — Melli without its tile, and on the desktop:** the owner wants the character alone, with no cream tile behind it (2.10). The cloud gets an ink outline (`color.mascot.outline`) so it stays whole on light surfaces and docks; marks drawn around it use `color.mascot.mark` (peach on dark, ink on light, at least 3:1 on the rail) and switch to ink on apricot; the ground shadow is themed. Home has no fill at rest and turns apricot on hover, focus and selection (5.2). The favicon and the app icons are the character alone on a transparent background. `color.mascot.tile` and `component.rail.home-background-rest` are gone. The desktop client shows Melli, animated, in its Home button with the same states, and gets a window icon and app icons for macOS and Linux generated from the tokens (2.10).
- **2026-10-21, v0.13 — Melli, the Mellow logo and mascot:** the owner's round cloud becomes the logo and the mascot "Melli" (2.10): named SVG parts, eight states (idle, connecting, connection lost, notification, incoming call, in call, update available, sleeping), each with a still pose for reduced motion. The web Home button shows Melli on its peach tile (apricot on hover, focus and selection) and maps connection and unread direct messages to states (5.2); the favicon is the still idle Melli, generated from the tokens by `cargo xtask tokens`. `ref.melli.*`, `color.mascot.*`, `component.melli.*` and `component.rail.home-background-rest` tokens; contrast pairs for `color.mascot.ink`. A development-only gallery shows every state (`npm run dev`, then `/dev/melli.html`).
- **2026-10-21, v0.12 — avatars and banners on the web (ADR-0021):** avatars show the user's picture wherever they appear, with the initial on the accent fill while it loads, when there is none or when it fails (6.8); animated pictures show their still first frame and play while the row holding them is hovered or focused, always in the profile card, never under reduced motion. The profile popout shows the banner picture in a 120 px strip (5.7). User settings gain "Profile" (5.9): avatar and banner editors beside a live preview of the profile card. `component.profile-popover.banner-image-height` token.
- **2026-10-21, v0.11 — file attachments (ADR-0020):** the web client attaches files (5.5): attach button, drag and drop with an apricot "Drop to attach to #general" overlay, paste; each file uploads at once into a tray inside the composer box (thumbnail or file icon, middle-truncated name, "Uploading…" then the size, or the error in `text.danger` with a red outline), up to 10 per message, Send waits for running uploads. Messages show images inline and other files as download cards (5.4); the desktop client lists "name · size" rows until it handles files. `component.attachment.*`, `component.drop-overlay.*` and `component.composer.attach-button-size` / `tray-*` tokens; `text.link` and `text.accent` on `surface.sidebar` contrast pairs; Phosphor `file`, `file-arrow-up` and `download-simple` (Fill).
- **2026-10-21, v0.10 — My account on desktop and web:** user settings open on "My account" (5.9): Discord's account card with display name, username and email, Edit buttons opening forms in place (one at a time), and a password section with "Change password"; Esc closes the open form before the screen; the web client gets the full-screen user settings (gear in the user panel, "Sign out" moved into the settings navigation, a list page then a section page on phones) and an About page with the source code link; `component.settings.account-*`, `error-text` and `notice-text` tokens, a `text.normal` on `surface.sidebar` contrast pair; Phosphor `check` and `gear-six` on the web.
- **2026-10-21, v0.9 — web client (ADR-0019):** the browser client is a port of the desktop shell (4.5): the same panes, tokens, copy and dialogs, laid out by the 4.2 breakpoints, with the rail and sidebar in a drawer and the member list as an overlay in narrow windows; 44 px targets under a coarse pointer and a send button on phones; native `<dialog>` for dialogs and drawers, the popover API for the server menu and the profile popover; `component.profile-popover.*`, `component.user-panel.button-icon-size` and `component.composer.reply-bar-icon-size` tokens; Phosphor `list`, `at` (Bold) and `paper-plane-right` (Fill) icons.
- **2026-10-21, v0.8 — one-to-one direct messages on desktop:** Home lists conversations in the sidebar (5.6), the rail's Home item carries the unread pill, clicking a member opens a reduced profile popout with "Message" (5.7), and conversations use `@name` in the chat header, hero, composer and errors (5.4, 5.5); `component.conversation-row.*` and `component.profile-popover.*` tokens, Phosphor `at` icon (Bold).
- **2026-10-21, v0.7 — server membership dialogs on desktop:** the rail's `+` opens "Create a server" / "Join a server" (one dialog, two pages switched by a link, as on Discord); the server menu gets "Invite people" (accent text), "Create channel" and "Leave server" (danger, separated), each shown only when allowed (5.3); invite, create-channel and leave-server dialogs (8); a shared `ModalDialog` frame for 6.35 (title, optional sentence, inline error with icon, Cancel plus one action, focus trapped, Esc and scrim close except while busy); `TextField` leading icon (`#`) and copy-only variant (6.3); `color.text.accent`, `component.context-menu.accent-text` and `separator-height`, `component.dialog.subtitle-*`, `error-*`, `mode-link-*`, `copied-label-duration` and `motion.delay.copied-label` tokens; Phosphor `user-plus` and `sign-out` (Fill) icons.
- **2026-10-20, v0.6 — user settings and app updates on desktop:** the user panel's settings button opens the user settings screen (5.3, 5.9) instead of a one-item menu; "Sign out" moves into the settings navigation; About shows the app version and drives the self-update, with a green dot on the settings button while an update waits; Primary and disabled variants of the desktop `Button` (6.1); `component.settings.*`, `component.user-panel.update-badge*` and `component.button.disabled-opacity` tokens; Phosphor `x` (Bold) icon for the close button.
- **2026-10-20, v0.5 — channel permissions on desktop:** read-only channel rows (5.3), Delete and Reply follow the user's permissions in the hover action bar (5.4), and the read-only composer (5.5); `component.composer.locked-text` token.
- **2026-10-19, v0.4 — message actions on desktop:** edit, delete and reply presentation (5.4, 5.5); `component.dialog.*`, `component.message.editor-*`, `component.composer.reply-bar-*` and `color.neutral.fill-hover` tokens.
- **2026-10-19, v0.3 — font and icons chosen by the user:** Figtree (bundled, OFL) replaces Noto Sans as the UI and message font, Noto Sans stays as the fallback for other scripts; the icon set is Phosphor (MIT), Fill weight, recoloured from tokens. Sections 2.4 and 2.9 are authoritative where older text below still says Noto Sans.
- **2026-10-19, v0.2 — direction changed: faithful Discord interface, Mellow brand.** The user asked for an interface that is "really like Discord" and chose a faithful clone with Mellow's own brand ([ADR-0012](docs/adr/0012-clone-discord-interface-with-own-brand.md)). The previous direction (warm graphite surfaces, two planes, Atkinson Hyperlegible and Fraunces, square server icons, apricot mention badges, a calmer layout) is superseded. Mellow now copies Discord's layout, pane dimensions, density, interaction patterns, keyboard shortcuts and layered cool-grey dark theme, and keeps its own accent (apricot), logo, open-licence font (Noto Sans) and accessibility guarantees. The Slint spike results of [ADR-0010](docs/adr/0010-desktop-ui-with-slint.md) (inline-only rich text, selection within one message only) are reflected in the message list and copy patterns (sections 5.4 and 7.3).
- **2026-10-19, v0.1 — first draft** (warm, "anti-Discord" identity).

Contents:

1. [Design principles](#1-design-principles)
2. [Visual identity](#2-visual-identity)
3. [Token architecture](#3-token-architecture)
4. [Layout and information architecture](#4-layout-and-information-architecture)
5. [Key screens and states](#5-key-screens-and-states)
6. [Components inventory](#6-components-inventory)
7. [Accessibility](#7-accessibility)
8. [UX copy: voice and tone](#8-ux-copy-voice-and-tone)
9. [Open questions and items to validate](#9-open-questions-and-items-to-validate)
10. [Defaults we rejected](#10-defaults-we-rejected)

Vocabulary used in this document and in the UI:

| Term in the UI | Term in code | Meaning |
|---|---|---|
| Instance | `instance` | One Mellow installation, reachable at a domain such as `chat.example.org`. Accounts live on an instance. |
| Server | `guild` | A community inside an instance, with channels, roles and members. We keep the word users already know from Discord. |
| Channel | `channel` | A text channel inside a server, optionally inside a category. |
| Conversation | `dm` / `group_dm` | A direct message with one person (1:1) or a small group. The tab and list are called "Messages", as on Discord. |

---

## 1. Design principles

A design or a pull request that breaks a principle needs an explicit justification in its description. Since v0.15 the principles follow [ADR-0022](docs/adr/0022-give-mellow-its-own-visual-language-on-a-familiar-layout.md) (accepted 2026-10-22): Discord's concepts and shortcuts, Mellow's own layout (the Navigator) and visual language, called **Dusk**.

### 1.1 Familiar where it matters

Someone who uses Discord every day must be able to use Mellow without learning anything: the same panes in the same order, the same right-click menus, the same shortcuts, the same message grouping, the same unread and mention conventions. What may differ is how it looks and moves: colour, type, shape, density and motion. When in doubt about *where* something is or *what* it does, do what Discord does; when in doubt about *how it looks*, follow this document.

- Test: servers, channels, conversations, members and the user panel are where a Discord user looks for them (servers and channels on the leading side, the user panel bottom-leading, members on the trailing side), and every shortcut in section 7.2 works.
- Test: a Discord user, given five tasks (switch server, find an unread channel, reply to a message, mark a channel as read, open a member's profile), completes them without help on the first try.

### 1.2 One ground, a few cards, one lamp

The interface is a quiet room: a dusk ground with a few cards on it. The navigator (servers, channels, you) and the side sheet (members, profiles) sit one step above the ground; the conversation is the lit sheet, the brightest card (white in the light theme). Regions are separated by tone and space, not by lines and shadows. Apricot is the lamp: it means **"you" or "now"** and nothing else (the primary action, Home, the selected server, the unread divider, mentions, jump bars). Everything else is dusk neutrals.

- Test: a screen shows at most one apricot fill besides Home and mentions; no border separates two cards; no decorative shadow sits on a resting surface.
- Test: no token, asset or screenshot uses Discord's blurple (`#5865F2`) or any accent with a hue between 215° and 255° in OKLCH; no Discord logo, wordmark, mascot or gg sans/Whitney font file is in the repository.

### 1.3 Motion answers

Every animation answers something: a pointer, a key, a message arriving, a state changing. It shows what changed and where it went, ends within 340 ms, and never loops, except Melli (2.10) and the typing indicator while someone types. Nothing animates while the app is idle.

- Test: each animation in the product is listed in the key transitions table (2.8) with its trigger, duration and reduced-motion variant.
- Test: with the window visible and nobody typing, the desktop client draws no frame except Melli's.

### 1.4 Accessible always

Every text/background pair meets WCAG 2.2 AA, every non-text indicator meets 3:1, information is never carried by colour alone, everything reachable by pointer is reachable by keyboard and screen reader, and every animation has a reduced-motion variant.

- Test: the token generator checks every pair declared in `design/tokens.json` (`$extensions.org.mellow.contrast-pairs`) in both themes and fails the build below the threshold.
- Test: every hover action also exists in the context menu (`Shift+F10` or the Menu key), as a keyboard shortcut on the focused item, and as an accessibility action.

### 1.5 Say what is happening

Self-hosting means several places, connections that drop and servers that are sometimes down. Mellow always tells you which instance you are talking on, whether you are connected, and whether your message was delivered. Pending, failed and queued messages are labelled in words.

- Test: every screen in section 5 defines loading, empty, error, offline and reconnecting states; no state is a lone spinner or a blank area.
- Test: when the user is signed in to more than one instance, the composer placeholder and every conversation header name the instance.

### 1.6 Honest about platform limits

When the toolkit cannot reproduce a behaviour, Mellow offers an explicit equivalent instead of a broken imitation. Slint cannot select text across messages, so every message has "Copy text" and a per-message "Select text" mode (section 5.4).

- Test: each known gap is listed in section 9 with its replacement pattern, and the replacement is reachable by mouse, keyboard and screen reader.

---

## 2. Visual identity

### 2.1 Personality: Dusk

Mellow is named for the mellow hour: the light is low and warm, the day's noise has gone, and people talk. Melli is a cloud at that hour, lit by a low apricot sun against a plum-grey sky. The interface borrows that light: dusk neutrals for the room, one lit sheet for the conversation, apricot only where something is for you or happening now.

Three words guide every decision: **calm, warm, alive**.

- **Calm**: a few cards on one ground, no lines between them, sentence-case labels, generous line height. The conversation is the brightest thing on screen.
- **Warm**: apricot is the only saturated warm hue, and the neutrals lean plum rather than blue, so a screenshot of Mellow is never mistaken for Discord.
- **Alive**: things respond. Rows warm up under the pointer, dialogs land with a small settle, your own message joins the list, Melli reacts to the connection and to direct messages. Nothing moves on its own.

The signature is Melli and the sheet. Each screen spends its boldness in one place (Melli on sign-in and empty states, the sheet in the shell); everything around stays quiet.

### 2.2 Palette

The reference palette (`ref.*` in `design/tokens.json`) has seven families. Components never use them directly; they use the semantic tokens in 2.3.

| Family | Role | Anchors |
|---|---|---|
| Dusk | Plum-tinted neutrals for surfaces and text (OKLCH hue 322, chroma 0.004–0.024). Steps are named by OKLCH lightness × 1000, so `ref.dusk.235` is lightness 0.235 | `#150F16` (dark ground) to `#FBF9FC` (light message hover); full list in `ref.dusk` |
| Apricot | The lamp: primary actions, Home, the selected-server pill, the unread divider, mentions, jump bars | `#F2A46E` (dark fill), `#E8894B` (light fill), `#A84F1A` (light indicator), `#2B1608` (text on apricot) |
| Sky | Links and the keyboard focus ring | `#33B8FF` (dark), `#005FCC` (light) |
| Green | Online presence, positive text, add-server button | `#23A55A`, `#2DC770` (dark text), `#1A7F45` (light) |
| Amber | Idle presence, warning notices | `#F0B232`, `#A06E00` (light idle) |
| Red | Danger, do-not-disturb, mention badges | `#DA373C` (fills and badges), `#FA777C` (dark text), `#C4262B` (light text), `#F45A5E` (dark presence) |
| Melli | The mascot's own colours (2.10) | `#FFFAF3` (cloud), `#3F332D` (ink), `#F4E2D2` (peach marks) |

**Why plum-tinted neutrals.** Discord's greys sit at hue 268 with almost no chroma, a cool blue-grey. Mellow's sit at hue 322, a plum dusk, with just enough chroma to read as warm-violet rather than neutral. Apricot (hue 55) sits opposite on the wheel, so the accent glows against them the way a low sun does against an evening sky. Two other families were tried and rejected (ADR-0022): warm charcoal, which with a peach accent looks like several current AI and productivity products, and navy, which is too close to Discord's family.

**Why apricot keeps its role.** It is Mellow's brand from the first draft, the complement of blurple (the largest possible hue distance from Discord), and dark ink on apricot reaches 8.44:1. It never appears on a presence indicator. Apricot and amber are neighbours, so neither carries meaning alone: idle always has its crescent shape, warnings always have an icon and a sentence, and the mention highlight always has its leading bar.

### 2.3 Semantic colour tokens

Dark is the default theme. Light is offered and follows the system setting when the user picks "Match system". Values below are the resolved hex values; the token file stores them as aliases of `ref.*`.

#### Surfaces

Four levels: the **ground** (the window behind everything), the **cards** on it (the navigator and the side sheet), the **sheet** (the conversation, settings content, the user card), and **floating** layers (popovers, dialogs, menus), each a step lighter in dark; in light the cards and the sheet are white and floating layers get a shadow.

| Token | Dark | Light | Used for |
|---|---|---|---|
| `color.surface.rail` | `#110B11` | `#F5F1F6` | Ground: the window behind the cards; the server rail in a classic layout |
| `color.surface.sidebar` | `#110B11` | `#F5F1F6` | Ground: settings navigation, reply bar, file cards |
| `color.surface.navigator` | `#1B151C` | `#FFFFFF` | The navigator card and the side sheet |
| `color.surface.sidebar-footer` | `#221B23` | `#FFFFFF` | The user card and connection dock inside the navigator |
| `color.surface.chat` | `#221B23` | `#FFFFFF` | The sheet: message pane, chat header, settings content, rail icons without artwork |
| `color.surface.input` | `#312A32` | `#F5F1F6` | Composer, search fields |
| `color.surface.field` | `#1A141A` | `#F8F4F8` | Form text fields (always with `border.strong`) |
| `color.surface.popover` | `#2B242C` | `#FFFFFF` | Profile popover, autocomplete, quick switcher, dialogs |
| `color.surface.floating` | `#2E272F` | `#FFFFFF` | Context menus, tooltips |
| `color.surface.code` | `#1A141A` | `#F8F4F8` | Code blocks and inline code |
| `color.border.divider` | `#3C353E` | `#E3DDE4` | Decorative dividers inside a surface, hover bar and menu outlines |
| `color.border.strong` | `#8C828D` | `#8C828D` | Form field and checkbox outlines (3:1) |
| `color.border.quote` | `#554C56` | `#CBC5CC` | Quote bar and reply spine (decorative) |
| `color.scrim` | `#080409` at 72 % | `#1B131C` at 55 % | Behind dialogs and drawers |

#### Interaction states

Solid tone steps, not translucent layers, so every state can be contrast-checked. On the ground, a hovered row lifts to the sheet's tone: the pointer "lights" it.

| Token | Dark | Light | Used for |
|---|---|---|---|
| `color.state.hover` | `#221B23` | `#ECE6ED` | Hover on channel, member, conversation and settings rows; icon button hover |
| `color.state.selected` | `#362E37` | `#DFD8DF` | Selected channel, conversation or settings page; pressed rows; hovered server without artwork; focused menu item |
| `color.state.hover-message` | `#282129` | `#FBF9FC` | Hover on a message row |

Selection is never conveyed by the background alone: selected rows also switch to `text.heading`, and the rail uses the pill.

#### Text

| Token | Dark | Light | Used for |
|---|---|---|---|
| `color.text.heading` | `#F8F4F8` | `#221B23` | Headers, author names without role colour, unread and selected channels |
| `color.text.normal` | `#D6D0D7` | `#392F3A` | Message text, menu items, hovered channels |
| `color.text.secondary` | `#C5BEC6` | `#594E5B` | Member names without role colour, interactive icons |
| `color.text.muted` | `#A99FAA` | `#695F6B` | Read channels, timestamps, group labels, placeholders, activity lines, chat header topic |
| `color.text.channel-muted` | `#988E9A` | `#726874` | Channels the user muted |
| `color.text.disabled` | `#635A64` | `#A99FAA` | Disabled controls only (exempt, always paired with a reason) |
| `color.text.link` | `#33B8FF` | `#005FCC` | Links |
| `color.text.danger` | `#FA777C` | `#C4262B` | Error text, danger menu items, "Not sent" |
| `color.text.positive` | `#2DC770` | `#1A7F45` | Success text such as "Available" |
| `color.text.accent` | `#F2A46E` | `#A84F1A` | Apricot text: "Invite people", file glyphs |
| `color.text.on-accent` | `#2B1608` | `#2B1608` | Text and icons on apricot |
| `color.text.on-danger`, `color.text.on-neutral` | `#FFFFFF` | `#FFFFFF` | Text on red, dusk and green fills |

Dark normal text stays below pure white (11.1:1 on the sheet) to avoid halation on dark surfaces; headings take the brightest step.

#### Accent, fills and mention

| Token | Dark | Light | Used for |
|---|---|---|---|
| `color.accent.fill` | `#F2A46E` | `#E8894B` | Primary buttons, Home when hovered or selected, jump bars, the "New" pill, mention chip hover, drop overlay |
| `color.accent.fill-hover` | `#E8925A` | `#E07B39` | Accent fill on hover |
| `color.accent.fill-pressed` | `#D47F47` | `#CC6A2C` | Accent fill when pressed |
| `color.accent.indicator` | `#F2A46E` | `#A84F1A` | Apricot marks that must reach 3:1 on any surface: the selected-server pill, the unread divider, the mention bar |
| `color.neutral.fill` / `-hover` | `#554C56` / `#635A64` | `#695F6B` / `#594E5B` | Secondary buttons |
| `color.danger.fill` / `-hover` | `#DA373C` / `#A12829` | same | Destructive buttons, mention badges, danger menu item hover |
| `color.positive.fill` | `#23A55A` | `#1A7F45` | Add-server icon and its hover fill, success notice |
| `color.warning.fill` / `on-fill` | `#F0B232` / `#1A141A` | same | Warning notice bar |
| `color.mention.background` | `#37292A` | `#FDF4EF` | Message that mentions you (apricot at 10 % on the dark sheet, 9 % on white, flattened) |
| `color.mention.bar` | `#F2A46E` | `#A84F1A` | 2 px leading bar on a mentioning message (`accent.indicator`) |
| `color.mention.chip-background` | `#60443A` | `#FAE7DB` | `@user`, `@role`, `#channel` chips in messages |
| `color.mention.chip-text` | `#FFE4D1` | `#7A3A0E` | Chip text |
| `color.unread.line` | `#F2A46E` | `#A84F1A` | Unread divider rule (`accent.indicator`) |
| `color.unread.pill` | `#F8F4F8` | `#221B23` | Rail and channel-list unread and hover pills |
| `color.focus` | `#33B8FF` | `#005FCC` | Keyboard focus ring, 2 px wide with a 2 px gap |

The unread divider is apricot, not Discord's red: new messages are "now", and red stays for danger and for the mention count, the one red convention people rely on. The focus ring is sky blue, not apricot, so that "focused" and "selected" or "primary" are never confused.

#### Presence

Status colours keep the conventions users already read, with shapes.

| Token | Dark | Light | Shape (never colour alone) |
|---|---|---|---|
| `color.status.online` | `#23A55A` | `#1A7F45` | Filled circle |
| `color.status.idle` | `#F0B232` | `#A06E00` | Crescent moon |
| `color.status.dnd` | `#F45A5E` | `#DA373C` | Circle with a horizontal bar cut out |
| `color.status.offline` | `#988E9A` | `#726874` | Hollow ring |

The indicator sits in a 3 px cut-out ring of the surface it is drawn on (including hover and selected row colours), so it separates from the avatar image. The inner shapes are holes, not drawn in a second colour, so the same ring shows through them: on a 20-unit grid, the crescent is the disc minus a 7.5-radius circle centred at (5, 5); the bar is 15 × 5 with round ends, centred; the ring keeps a 5-radius hole. Invisible is drawn as offline, the way others see it; only the words differ ("Invisible" on your own card). Mobile ("on phone") presence is out of scope for the MVP.

#### Verified contrast ratios (WCAG 2.2 AA)

Ratios are computed with the WCAG 2.x relative-luminance formula by `cargo xtask tokens`, which fails below the minimum. Body text needs 4.5:1; non-text indicators need 3:1.

| Pair | Dark | Light | Minimum |
|---|---|---|---|
| `text.heading` on `surface.chat` | 15.44 | 16.81 | 4.5 |
| `text.heading` on `surface.sidebar` | 17.86 | 15.05 | 4.5 |
| `text.heading` on `state.selected` | 12.03 | 12.03 | 4.5 |
| `text.heading` on `surface.sidebar-footer` | 15.44 | 16.81 | 4.5 |
| `text.heading` on `surface.popover` | 13.86 | 16.81 | 4.5 |
| `text.normal` on `surface.chat` | 11.10 | 12.79 | 4.5 |
| `text.normal` on `surface.sidebar` | 12.84 | 11.45 | 4.5 |
| `text.normal` on `state.hover-message` | 10.35 | 12.22 | 4.5 |
| `text.normal` on `mention.background` | 9.16 | 11.79 | 4.5 |
| `text.normal` on `state.hover` | 11.10 | 10.42 | 4.5 |
| `text.normal` on `surface.input` | 9.19 | 11.45 | 4.5 |
| `text.normal` on `surface.floating` | 9.58 | 12.79 | 4.5 |
| `text.normal` on `surface.popover` | 9.96 | 12.79 | 4.5 |
| `text.normal` on `surface.code` | 11.96 | 11.75 | 4.5 |
| `text.normal` on `surface.field` | 11.96 | 11.75 | 4.5 |
| `text.secondary` on `surface.chat` | 9.25 | 7.88 | 4.5 |
| `text.secondary` on `surface.sidebar` | 10.70 | 7.05 | 4.5 |
| `text.secondary` on `surface.sidebar-footer` | 9.25 | 7.88 | 4.5 |
| `text.secondary` on `surface.input` | 7.66 | 7.05 | 4.5 |
| `text.muted` on `surface.chat` | 6.59 | 6.09 | 4.5 |
| `text.muted` on `surface.sidebar` | 7.62 | 5.45 | 4.5 |
| `text.muted` on `surface.sidebar-footer` | 6.59 | 6.09 | 4.5 |
| `text.muted` on `surface.input` | 5.46 | 5.45 | 4.5 |
| `text.muted` on `surface.popover` | 5.91 | 6.09 | 4.5 |
| `text.muted` on `surface.floating` | 5.68 | 6.09 | 4.5 |
| `text.muted` on `surface.field` | 7.10 | 5.59 | 4.5 |
| `text.muted` on `state.hover-message` | 6.14 | 5.81 | 4.5 |
| `text.muted` on `mention.background` | 5.44 | 5.61 | 4.5 |
| `text.channel-muted` on `surface.sidebar` | 6.18 | 4.76 | 4.5 |
| `text.link` on `surface.chat` | 7.56 | 5.98 | 4.5 |
| `text.link` on `mention.background` | 6.24 | 5.52 | 4.5 |
| `text.link` on `state.hover-message` | 7.05 | 5.72 | 4.5 |
| `text.link` on `surface.sidebar` | 8.74 | 5.36 | 4.5 |
| `text.accent` on `surface.sidebar` | 9.55 | 4.95 | 3.0 |
| `text.danger` on `surface.chat` | 6.40 | 5.74 | 4.5 |
| `text.danger` on `surface.sidebar` | 7.40 | 5.14 | 4.5 |
| `text.danger` on `surface.floating` | 5.52 | 5.74 | 4.5 |
| `text.danger` on `surface.popover` | 5.75 | 5.74 | 4.5 |
| `text.accent` on `surface.floating` | 7.12 | 5.53 | 4.5 |
| `text.positive` on `surface.chat` | 7.62 | 5.04 | 4.5 |
| `text.positive` on `surface.sidebar` | 8.81 | 4.51 | 4.5 |
| `text.on-accent` on `accent.fill` | 8.44 | 6.64 | 4.5 |
| `text.on-accent` on `accent.fill-hover` | 7.12 | 5.78 | 4.5 |
| `text.on-accent` on `accent.fill-pressed` | 5.67 | 4.64 | 4.5 |
| `text.on-danger` on `danger.fill` | 4.57 | 4.57 | 4.5 |
| `text.on-danger` on `danger.fill-hover` | 7.37 | 7.37 | 4.5 |
| `text.on-neutral` on `neutral.fill` | 8.22 | 6.09 | 4.5 |
| `warning.on-fill` on `warning.fill` | 9.59 | 9.59 | 4.5 |
| `text.on-neutral` on `positive.fill` | 3.18 | 5.04 | 3.0 |
| `mention.chip-text` on `mention.chip-background` | 7.24 | 7.18 | 4.5 |
| `mention.bar` on `mention.background` | 6.81 | 5.10 | 3.0 |
| `mention.bar` on `surface.chat` | 8.25 | 5.53 | 3.0 |
| `unread.pill` on `surface.rail` | 17.86 | 15.05 | 3.0 |
| `unread.pill` on `surface.sidebar` | 17.86 | 15.05 | 3.0 |
| `unread.line` on `surface.chat` | 8.25 | 5.53 | 3.0 |
| `focus` on `surface.chat` | 7.56 | 5.98 | 3.0 |
| `focus` on `surface.sidebar` | 8.74 | 5.36 | 3.0 |
| `focus` on `surface.rail` | 8.74 | 5.36 | 3.0 |
| `focus` on `surface.input` | 6.26 | 5.36 | 3.0 |
| `border.strong` on `surface.chat` | 4.56 | 3.69 | 3.0 |
| `border.strong` on `surface.sidebar` | 5.27 | 3.30 | 3.0 |
| `positive.fill` on `surface.chat` | 5.28 | 5.04 | 3.0 |
| `status.online` on `surface.sidebar` | 6.11 | 4.51 | 3.0 |
| `status.online` on `state.selected` | 4.12 | 3.61 | 3.0 |
| `status.idle` on `surface.sidebar` | 10.30 | 3.98 | 3.0 |
| `status.idle` on `state.selected` | 6.94 | 3.18 | 3.0 |
| `status.dnd` on `surface.sidebar` | 5.99 | 4.09 | 3.0 |
| `status.dnd` on `state.selected` | 4.04 | 3.27 | 3.0 |
| `status.offline` on `surface.sidebar` | 6.18 | 4.76 | 3.0 |
| `status.offline` on `state.selected` | 4.16 | 3.81 | 3.0 |
| `status.offline` on `surface.sidebar-footer` | 5.34 | 5.32 | 3.0 |
| `status.offline` on `surface.popover` | 4.79 | 5.32 | 3.0 |
| `text.secondary` on `surface.field` | 9.97 | 7.24 | 3.0 |
| `text.on-neutral` on `neutral.fill-hover` | 6.61 | 7.88 | 4.5 |
| `text.danger` on `state.hover` | 6.40 | 4.68 | 3.0 |
| `positive.fill` on `surface.sidebar-footer` | 5.28 | 5.04 | 3.0 |
| `text.heading` on `state.hover` | 15.44 | 13.70 | 4.5 |
| `text.muted` on `state.hover` | 6.59 | 4.96 | 4.5 |
| `text.heading` on `surface.floating` | 13.33 | 16.81 | 4.5 |
| `positive.fill` on `text.on-neutral` | 3.18 | 5.04 | 3.0 |
| `neutral.fill` on `text.on-neutral` | 8.22 | 6.09 | 3.0 |
| `text.secondary` on `state.selected` | 7.21 | 5.64 | 4.5 |
| `accent.fill-pressed` on `surface.chat` | 5.55 | 3.71 | 3.0 |
| `mascot.mark` on `component.rail.background` | 15.42 | 10.91 | 3.0 |
| `mascot.ink` on `mascot.body` | 11.74 | 11.74 | 3.0 |
| `mascot.ink` on `mascot.body-overcast` | 8.91 | 8.91 | 3.0 |
| `mascot.ink` on `accent.fill` | 5.98 | 4.71 | 3.0 |
| `mascot.ink` on `accent.fill-pressed` | 4.02 | 3.29 | 3.0 |
| `accent.indicator` on `surface.rail` | 9.55 | 4.95 | 3.0 |
| `text.muted` on `surface.rail` | 7.62 | 5.45 | 4.5 |
| `text.heading` on `surface.navigator` | 16.48 | 16.81 | 4.5 |
| `text.normal` on `surface.navigator` | 11.84 | 12.79 | 4.5 |
| `text.secondary` on `surface.navigator` | 9.87 | 7.88 | 4.5 |
| `text.muted` on `surface.navigator` | 7.03 | 6.09 | 4.5 |
| `text.channel-muted` on `surface.navigator` | 5.70 | 5.32 | 4.5 |
| `text.danger` on `surface.navigator` | 6.83 | 5.74 | 4.5 |
| `unread.pill` on `surface.navigator` | 16.48 | 16.81 | 3.0 |
| `accent.indicator` on `surface.navigator` | 8.81 | 5.53 | 3.0 |
| `focus` on `surface.navigator` | 8.06 | 5.98 | 3.0 |
| `status.online` on `surface.navigator` | 5.64 | 5.04 | 3.0 |
| `status.idle` on `surface.navigator` | 9.50 | 4.44 | 3.0 |
| `status.dnd` on `surface.navigator` | 5.53 | 4.57 | 3.0 |
| `status.offline` on `surface.navigator` | 5.70 | 5.32 | 3.0 |
| `text.danger` on `surface.sidebar-footer` | 6.40 | 5.74 | 3.0 |
| `mascot.body` on `mascot.tile-top` | 10.65 | 10.65 | 3.0 |

`color.border.divider` and `color.border.quote` are intentionally below 3:1. They are decorative and are never the only thing that shows where a control is or what a block is (quotes are also indented).

The composer field (`surface.input` on `surface.chat`) has no 3:1 boundary. It is identified by its placeholder ("Message #general", 5.46:1), its attach button and its fixed position, which satisfies WCAG 1.4.11 because the boundary is not needed to identify the control. Form text fields, which have no such context, get a 1 px `border.strong` outline.

#### User-chosen colours (role colours, default avatars)

Role colours are chosen by server admins and shown on author and member names. Mellow keeps the hue but adjusts lightness in OKLCH until the colour reaches 4.5:1 against the surface it is drawn on, separately for each theme. This adjustment lives in the shared Rust core so both clients render the same result. The role editor shows a live preview on both themes and says "Adjusted for readability" when the displayed colour differs from the picked one.

Default avatars (no uploaded picture) show the first grapheme of the display name on one of eight tints derived from the palette, picked deterministically from the user ID: FNV-1a (32 bits) over the UTF-8 bytes of the id in decimal, modulo 8, gives `color.avatar.tint-1` to `tint-8` (index 0 is tint 1), with the initial in `color.avatar.initial`. A role colour keeps its hue and its OKLCH lightness moves in 0.005 steps (up on a dark surface, down on a light one) until it reaches 4.5:1 on `surface.chat` (web: `role-colors.ts`, desktop: `person_style.rs`). Each tint/initial pair is validated at 4.5:1 by the token generator.

### 2.4 Typography

One family, bundled with the apps, so the product looks the same everywhere and works offline.

| Family | Licence | Role | Why |
|---|---|---|---|
| Figtree (variable, `wght`, upright and italic) | SIL OFL 1.1 | Everything: UI, messages, display | Chosen by the owner (v0.3): a geometric, friendly sans with round bowls that echo Melli's puffs, very legible at 15–16 px, 63 KB per style, bundled in the desktop binary and served by the web client. It carries both the quiet UI sizes and, at 700–800 with tight tracking, the display moments; no second display face is needed (ADR-0022). |
| Noto Sans (system or later bundle) | SIL OFL 1.1 | Fallback for scripts Figtree lacks (Greek, Cyrillic, others) | Humanist proportions close to Figtree's x-height; script subsets to decide (section 9). |
| Source Code Pro (system or later bundle) | SIL OFL 1.1 | Inline code, code blocks, invite codes | Clear distinction of `0O`, `1lI`. Not bundled yet; the stack falls back to the platform monospace. |
| Twemoji (COLR font build) | CC-BY 4.0 (graphics) | Emoji | See section 9. |

Rules:

- Weights used: 400 (reading), 500 (names and controls), 600 (labels, author names), 700 and 800 (titles and display only).
- **Sentence case everywhere.** Category headers, member-group headers, settings section labels and form labels are sentence case at 13 px semibold in `text.muted`, never uppercase and never tracked out. Discord's uppercase labels are one of the things Dusk drops: they shout, they are harder to read, and they are a template tell.
- Display sizes (`type.display`, `heading-xl`, `heading-lg`) use negative tracking (−2 %, −1.5 %, −1 %) so large Figtree stays compact and confident.
- Messages use a 24 px line (1.5 at 16 px): calmer to read than Discord's 22 px, and the 4 px grid stays intact.
- No text is smaller than 12 px at 100 % zoom.
- Numbers that line up (member counts, badges, file sizes, timestamps in the gutter) use tabular figures.

#### Type scale tokens

Sizes are logical pixels on desktop and the admin web, and `sp` on Android. Line height is given in px here and stored as a multiplier in the token file.

| Token | Weight | Size / line height | Tracking | Used for | Compose `Typography` slot |
|---|---|---|---|---|---|
| `type.display` | 800 | 32 / 36 | −0.64 px | Channel-start hero ("Welcome to #general"), sign-in title | `displaySmall` (emphasized) |
| `type.heading-xl` | 700 | 26 / 32 | −0.39 px | Card and page titles of first-run flows | `headlineSmall` |
| `type.heading-lg` | 700 | 20 / 26 | −0.2 px | Dialog titles, settings page titles, quick switcher input | `titleLarge` |
| `type.heading-md` | 600 | 16 / 20 | 0 | Server name in sidebar header, channel name in chat header, popover display name | `titleMedium` |
| `type.body-lg` | 400 | 16 / 20 | 0 | Settings rows, form values | `bodyLarge` (outside messages) |
| `type.body` | 400 | 14 / 20 | 0 | Descriptions, helper text, dialog body, channel topic | `bodyMedium` |
| `type.body-sm` | 400 | 12 / 16 | 0 | Activity lines, user card status, field helper text | `bodySmall` |
| `type.message` | 400 | 16 / 24 | 0 | Message text, composer text | extended `MellowTheme.typography.message` |
| `type.message-author` | 600 | 16 / 24 | 0 | Author name on the first message of a group | extended |
| `type.message-timestamp` | 500 | 12 / 24 | 0 | Timestamp after the author name, hover timestamp in the gutter, "(edited)" | extended |
| `type.message-h1` / `-h2` / `-h3` | 700 | 24/30, 20/26, 16/24 | 0 | Markdown headings inside messages | extended |
| `type.message-subtext` | 400 | 12 / 16 | 0 | Markdown subtext (`-# text`) | extended |
| `type.code` | 400 (mono) | 14 / 18 | 0 | Inline code, code blocks | extended |
| `type.channel-name` | 500 | 15 / 20 | 0 | Channel and conversation names in the sidebar | extended |
| `type.member-name` | 500 | 15 / 20 | 0 | Names in the member list | extended |
| `type.category` | 600 | 13 / 16 | 0 | Category headers, member groups, settings section labels, form labels (sentence case) | `labelLarge` |
| `type.user-panel-name` | 600 | 14 / 18 | 0 | Display name on the user card | `titleSmall` |
| `type.button` | 500 | 14 / 16 | 0 | Button labels | `labelLarge` |
| `type.menu-item` | 500 | 14 / 18 | 0 | Context menu items | extended |
| `type.tooltip` | 600 | 14 / 16 | 0 | Tooltips | extended |
| `type.badge` | 700 | 12 / 16 | 0 | Mention badges, "New" pill | `labelSmall` |
| `type.typing` | 500 | 14 / 24 | 0 | Typing indicator | extended |

Scaling:

- **Zoom level** (desktop, 50–200 %, `Ctrl+=`/`Ctrl+-`/`Ctrl+0`) scales every dimension and type token.
- **Chat font size** (12–24 px, default 16) scales the `type.message*` and `type.code` tokens only; the message gutter and avatar stay put.
- Android follows the system font scale (including non-linear scaling on Android 14+) and the chat font size setting.

### 2.5 Spacing

A 4 px grid. Odd values inherited from Discord's components (2 px row gaps, 6 px icon gaps, 6 px menu padding) live in component tokens, not in the scale.

| Token | Value | Typical use |
|---|---|---|
| `space.3xs` | 2 | Message vertical padding, chip padding |
| `space.2xs` | 4 | Badge padding, menu separator margin |
| `space.xs` | 8 | Row margins and padding, the user card's margin on the ground |
| `space.sm` | 12 | Tooltip horizontal padding |
| `space.md` | 16 | Header padding, avatar inset, composer side margins |
| `space.lg` | 24 | Composer bottom band (typing indicator), dialog padding |
| `space.xl` | 32 | Sign-in card padding, settings section spacing |
| `space.2xl` | 48 | Message trailing padding (room for the hover bar) |
| `space.3xl` | 64 | Admin dashboard top spacing on wide screens |

`size.sheet.inset` (8) is the gap between the cards (navigator, sheet, side sheet) and between them and the window edges on wide layouts (0 below 816 px, where the sheet is full-bleed). `size.dock.height` (56) and `size.dock.margin` (8) are the height of the composer's base row and of the user card and their gap to the bottom of their card, so the two line up (4.2). The space between message groups is 20 by default (`component.message.group-gap`; setting: 0, 4, 8, 16, 20, 24). Pane dimensions are in the `size.*` tokens and in the table of section 4.2.

### 2.6 Radius and shape

Radii are assigned by hierarchy, not one value everywhere, and nested corners are **concentric**: an inner radius equals its container's radius minus the padding between them (a 12 px menu with 6 px padding has 6 px items). Things that float are rounder than things that sit.

| Token | Value | Use |
|---|---|---|
| `radius.xs` | 4 | Mention chips, inline code, small tags |
| `radius.sm` | 8 | Rows (channel, member, conversation, settings), buttons, tooltips, the hover bar |
| `radius.md` | 12 | Popovers, context menus, attachments, jump bars, the user card, settings cards |
| `radius.lg` | 16 | The sheet, dialogs, the composer, the hovered or selected server squircle (at 48 px) |
| `radius.xl` | 24 | Sign-in card, Android bottom sheets |
| `radius.full` | 9999 | Avatars, badges, the "New" pill, status dots, resting server icons |

Signature shape: **server icons are circles at rest (Melli's puffs) and settle into a squircle (`radius.lg`) on hover, focus and selection**, the interaction Discord users know, timed with `motion.easing.settle` so it lands rather than snaps. Avatars are always circles. The navigator, the sheet and the side sheet all have `radius.lg` corners; the user card inside the navigator has `radius.md`, the composer inside the sheet `radius.lg` (both 8 px from their card's edge).

### 2.7 Elevation

Depth comes from tone. Shadows exist only on floating layers, and in the light theme, where white floating layers on a white sheet need one.

| Token | Dark | Light | Use |
|---|---|---|---|
| `elevation.low` | 0 1 0 black 20 % | 0 1 0 ink 8 % | Kept for the admin web; the clients' headers no longer draw a line |
| `elevation.medium` | 0 4 4 black 16 % | 0 4 4 black 10 % | Message hover bar, popovers, jump bars |
| `elevation.high` | 0 8 16 black 24 % | 0 8 16 black 16 % | Context menus, tooltips, dialogs, quick switcher |

Values are `x y blur colour`. Slint has no shadow tokens, so floating layers on desktop get a 1 px `border.divider` outline instead; Compose maps them to `shadowElevation`, CSS to `box-shadow`.

### 2.8 Motion

Motion is how Mellow feels alive without being busy. Every animation is a **state transition**: it starts because something happened, it shows what changed, and it ends. Nothing loops except Melli (2.10) and the typing dots while someone types, so the app costs nothing while idle.

#### Tokens

| Token | Value | Use |
|---|---|---|
| `motion.duration.instant` | 0 ms | Reduced-motion replacement for spatial motion |
| `motion.duration.fast` | 120 ms | Hover and pressed colour changes, focus ring, popovers and menus, dialog close |
| `motion.duration.base` | 200 ms | Rail pill height, category collapse, tooltips, channel switch, a message arriving |
| `motion.duration.slow` | 280 ms | Drawers and panels sliding, Android panel settle |
| `motion.duration.settle` | 340 ms | Things that land: dialog open, the rail squircle, your own message joining the list |
| `motion.duration.highlight` | 2000 ms | Fade-out of the highlight on a message you jumped to |
| `motion.delay.tooltip` / `tooltip-rail` | 500 ms / 0 ms | Tooltip delay; rail tooltips appear immediately |
| `motion.delay.skeleton` | 300 ms | Before skeletons appear, to avoid flicker |
| `motion.delay.pending-label` | 2000 ms | Before a pending message shows "Sending…" |
| `motion.easing.standard` | cubic-bezier(0.2, 0, 0, 1) | Colour and opacity |
| `motion.easing.enter` / `exit` | (0, 0, 0, 1) / (0.3, 0, 1, 1) | Appearing / leaving elements |
| `motion.easing.settle` | cubic-bezier(0.34, 1.3, 0.64, 1) | About 5 % overshoot, Melli's bounce; only with `duration.settle` |
| `motion.spring.settle-damping-ratio` / `settle-stiffness` | 0.72 / 500 | Android: `spring()` for the same moments as `settle` |

Slint does not receive easing tokens (the generator skips `cubicBezier`); Slint components write `cubic-bezier(0.34, 1.3, 0.64, 1)` (or `ease-out-back`) next to the token's duration, citing the token in a comment.

#### Key transitions

| Moment | Trigger | What moves | Duration and curve | Reduced motion |
|---|---|---|---|---|
| Row hover | Pointer enters a channel, member, conversation or settings row | Background ground → `state.hover`, text to `text.normal` | `fast`, standard | Same (colour only) |
| Server hover and select | Pointer, focus or selection on a rail icon | Circle → squircle; pill grows to 20 (hover) or 40 (selected, apricot) | Morph `settle` with the settle curve; pill `base`, standard | Instant shape and pill; colour kept |
| Channel switch | A channel or conversation is opened | Header title cross-fades; the new message list fades in from 0 and rises `channel-switch.offset` (8 px) | `channel-switch.duration` (`base`), enter | Instant swap |
| Message arrives | A message from someone else is added while the list is at the bottom | The new row fades in and rises `message.enter-offset` (8 px) | `message.enter-duration` (`base`), enter | Appears in place |
| Your message | You press Enter | Your message joins the list as pending (muted text), rising 8 px with the settle curve; when the server confirms, its text eases to `text.normal` | `settle`, then `fast` for the colour | Appears in place; colour change kept |
| Dialog open | A dialog is opened | Scrim fades in; the card grows from `dialog.enter-scale` (0.96) and fades in | Scrim `base`, card `dialog.enter-duration` (`settle`), settle curve | Scrim and card appear at once |
| Dialog close | Esc, Cancel, scrim, or the action completes | Card and scrim fade out | `dialog.exit-duration` (`fast`), exit | Instant |
| Menu and popover | Context menu, server menu, profile popover | Fade in and slide 4 px from the anchor side | `fast`, enter | Fade only |
| Unread divider, jump bar | Opening a channel with unread messages; scrolling up | Jump bar slides 8 px from its edge and fades | `base`, enter | Appears at once |
| Melli | Connection and direct-message state | See 2.10 | Per-state loops | Still poses |

What never animates: list items on first load or history load (they appear in place), skeletons (static), scrolling (except "Jump to present", which scrolls smoothly unless motion is reduced), the composer while typing, badges and counts.

#### Cost rules

- Web: animate only `opacity` and `transform` (plus colour); no layout properties, no filters, no `backdrop-filter`. Transitions end; no `animation-iteration-count: infinite` outside Melli and the typing dots.
- Desktop (Slint): an `animate` block runs only while its property changes, so transitions cost frames only while they play. New animations go on the few elements that change (the new row, the dialog card, the rail item), never on a whole list. Melli's clock stays the only continuous one (about 7 % of a core while visible, 2.10).
- Android: `MotionScheme.expressive()` springs for the settle moments only; `standard()` everywhere else.

#### Reduced motion

- Web and the admin web: `@media (prefers-reduced-motion: reduce)` sets every `motion.duration.*` to 0 ms (generated); delays keep their value.
- Desktop: reads the OS preference where available (macOS "Reduce motion", Windows "Show animations in Windows", GNOME `enable-animations`) and offers an in-app override: "Match system", "Reduce motion", "Allow motion". Slint 1.18 does not expose the OS preference, so the desktop stores its own choice instead: Appearance → Motion, "Allow motion" or "Reduce motion" (`MotionPreference` in `ui/preferences.slint`, saved per device by `preferences.rs`). Switch to "Match system" once Slint exposes the setting.
- Android follows "Remove animations" and the same in-app override.
- When reduced motion is on: spatial motion (squircle, pill growth, rises, scales, panel slides, smooth scrolling) is instant; colour changes are kept where the platform allows; "jump to message" scrolls instantly and keeps the highlight as a static background for `motion.duration.highlight`; animated media never autoplays; the typing indicator shows static dots; Melli shows its still pose.

### 2.9 Iconography

One icon set everywhere: **Phosphor Icons** (MIT), **Fill** weight, which matches the solid, compact feel of Discord's icons without copying their artwork. Sizes: 20 px in rows and the hover bar, 24 px in headers, the composer, the user panel and on Android. Icons are exported as SVG for Slint and the admin web UI, and converted to `ImageVector` for Compose. Emoji are never used as icons.

The **Home button** shows Melli, the Mellow logo (2.10): the cloud fills about 28 px of the 48 px rail icon, straight on the rail at rest and on the apricot fill when hovered, focused or selected.

### 2.10 Melli, the logo and mascot

Melli (Melli / Mellow) is a round cloud with two pill eyes, drawn by the owner. Inside the interface it is shown alone (the Home button, sign-in, empty states); as an icon (favicon, window and app icons) it sits on its dusk tile (below). It is the logo and the only character in the interface. It floats, blinks and slowly turns its puffs; everything else it does is built from the same soft, round, bouncy motion. Melli is Mellow's own artwork and shares nothing with Discord's logo or mascot.

#### Parts

The drawing lives in a 256 × 256 viewBox (`web/src/components/Melli.svelte`). One part is one element or group with a stable class, so a state only shows, hides or restyles parts:

| Part | What it is | Notes |
|---|---|---|
| `shadow` | Ellipse under the cloud | Translucent and themed, so it works on the rail and on apricot |
| `motion` | Group holding the cloud and the face | The state's loop (float, hop, rock, bob, bounce, breathe) pivots on the cloud's base (128, 200) |
| `puff` > `cloud-outline`, `cloud-body` | Seven circles: a core of radius 62 and six puffs of radius 30; the outline is the same circles 6 units larger, drawn behind | Turns once per `puff-spin-duration` around (128, 124); pauses when the connection is lost. Only the cloud's outer edge shows the outline |
| `face` | Eyes, cheeks, mouth | Moves as one (looks up while connecting) |
| `eyes` > `eyes-blink` | The two pill eyes, plus `glint`s | `eyes` holds the pose (droop, wide), `eyes-blink` the blink, so both combine |
| `eyes-happy`, `eyes-closed` | Arcs replacing the pill eyes | Happy "^ ^"; sleeping "‿ ‿" |
| `blush`, `mouth` | Cheeks, a small talking mouth | Hidden at rest |
| Marks | `dots`, `drops`, `sparkle`, `rings`, `waves`, `arrow`, `zzz` | Drawn in `color.mascot.mark`, like comic marks, on the surface Melli sits on; hidden at rest. `drops` sit behind the cloud so they fall out of it |

#### Palette

| Token | Value | Role |
|---|---|---|
| `color.mascot.shadow` | Dark: black at 35 %. Light: `#A46338` at 15 % | Ground shadow, visible on the rail and on apricot |
| `color.mascot.body` | `#FFFAF3` | The cloud and the eye glints |
| `color.mascot.body-overcast` | `#E2DBD7` | The cloud while the connection is lost |
| `color.mascot.ink` | `#3F332D` | Eyes and mouth; marks on apricot |
| `color.mascot.outline` | `#3F332D` | The cloud's edge: keeps it whole on light surfaces and light docks (on dark ones the cloud itself stands out) |
| `color.mascot.mark` | Dark: `#F4E2D2`. Light: `#3F332D` | Marks drawn around Melli, on the rail |
| `color.mascot.blush` | `#F5B3A1` | Cheeks (decorative) |
| `color.mascot.tile-top` / `tile-bottom` | `#4A3450` / `#140C15` | The icon tile's vertical dusk gradient |
| `color.mascot.tile-glow` | `#F2A46E` at 55 % | A low apricot glow behind the cloud on the icon tile: the cloud at sunset |

Melli is brand artwork: the cloud, its face and its outline are the same in both themes. Melli has no tile, so what it draws outside the cloud depends on the surface: marks use `color.mascot.mark`, at least 3:1 on the rail in both themes, and a host that puts Melli on apricot (Home when hovered, focused or selected) switches them to ink, which reaches at least 3:1 on both apricot fills, the cloud and the overcast cloud (contrast pairs in `design/tokens.json`). The cloud stands out on the dark rail by itself (cream on near black) and on light surfaces by its ink outline. Blush is the only colour that carries no meaning.

#### States

| State | When | Motion | Still pose (reduced motion) |
|---|---|---|---|
| `idle` | Nothing to report; the logo and favicon | Float 8 units and back, shadow shrinks; blink; puffs turn | The logo |
| `connecting` | The client is connecting | Float; looks up at three dots that rise and brighten in turn | Looking up, three dots |
| `connection-lost` | The connection dropped and the client reconnects | Cloud turns overcast, eyes half-closed, puffs stop, slow sag; three drops fall out of the cloud | Overcast, heavy eyes, three drops under the cloud |
| `notification` | Unread direct messages (later: mentions) | Squash, hop 22 units with a stretch, land; happy eyes, blush, two sparkles twinkling at the top of the hop | Happy eyes, blush, sparkles, lifted 6 units |
| `incoming-call` | Someone is calling (not wired yet) | Rocks ±9° in a burst, then rests, like a ringing phone; wide eyes; ring arcs on both sides flash with each burst | Wide eyes, ring arcs, tilted 10° |
| `in-call` | In a call (not wired yet) | Calm 4-unit bob; mouth talks; two sound waves pulse on the right | Open mouth, sound waves |
| `update-available` | A new version is ready (not wired yet) | Double bounce; glints in the eyes, blush; an arrow rising above the head | Glints, blush, arrow |
| `sleeping` | Away or idle presence (not wired yet) | Eyes closed, slow breath (squash 4 %), two Zs drifting up in turn | Closed eyes, two Zs |

Wherever Melli shows a state, the state is also said in words: Home keeps its accessible name ("Direct messages, unread") and its unread pill, and the connection notice above the chat says what the connection is doing. Melli itself is `aria-hidden`.

#### Motion rules

- Timings are tokens (`component.melli.*`): one loop duration per state, plus `state-change-duration` (360 ms, `motion.easing.standard`) for poses, colours and marks easing between states. Keyframe shapes and amplitudes are part of the drawing, like the paths, and stay in the component.
- Every loop starts and ends at rest; a state change restarts the loop from rest, a jump of up to 22 units when leaving the notification hop; poses (eyes, face, colours, marks) ease over `state-change-duration`. Pausing the puffs (`animation-play-state`) keeps their angle.
- Melli is always animated, idle included, in the rail too (the owner's choice, 2026-10-21): an always-visible infinite SVG loop repaints every frame, a cost accepted for a living mascot; reduced motion still stops it.
- Under `prefers-reduced-motion: reduce` (or the gallery's "Still poses"), Melli stops every animation and transition and shows the still pose of its state. A still pose must show the state on its own: add a static offset or tilt when the motion was the only sign.
- Marks stay within 126 units of the centre (128, 128), so the Home button's circle (radius 128) never clips them.
- The favicon and the app icons are the still idle Melli (outline, cloud and eyes, no shadow) centred on a rounded tile (radius 57 of 256, close to Apple's corner) filled with the dusk gradient and the apricot glow, so the icon reads on light and dark tabs, docks and taskbars alike (ADR-0022). The tile fills the PNGs; macOS insets it into its 824/1024 grid. `cargo xtask tokens` writes `web/public/favicon.svg` from the mascot tokens and rasterises it into every app icon; icons never animate. The installed web app (4.5) gets the same tile at 192 and 512 px, and a **maskable** variant with square corners: the launcher (Android) cuts its own shape, and Melli, which reaches 29 % of the width from the centre, stays inside the 40 % safe zone of any mask; the Apple touch icon is the maskable tile at 180 px, since iOS rounds it itself and would fill transparent corners with black. A textured variant (a few faint stars on the gradient) is reserved for large marketing artwork, because texture turns to noise at 16 and 32 px.

#### Adding a state

1. Add its name to `MelliState` in `web/src/components/melli.ts` and its line to the gallery (`web/src/dev/MelliGallery.svelte`).
2. If it needs a new part, draw it in ink inside the 120-unit circle, hidden by default (`.mark` or `opacity: 0`).
3. Add `[data-state="…"]` rules: which parts show, the pose, and a loop with a `component.melli.*-duration` token. Loops start and end at rest.
4. Give it a still pose under `.is-still` if the pose alone does not show the state.
5. Add the row to the table above, check it in the gallery at 192 px and 48 px on the rail and on apricot, in both themes, animated and still.

#### Desktop (Slint)

The desktop client shows the same Melli in its Home button (`crates/mellow-desktop/ui/components/melli.slint`), a port of the web drawing part for part:

- **Parts.** One named element per part, in the same 256-unit viewBox: `shadow`, `drops`, `motion` > `puff` (`cloud-outline`, `cloud-body`), `face` (`blush`, `eyes` > `eyes-blink`, `eyes-happy`, `eyes-closed`, `mouth`) and the marks `dots`, `sparkle`, `rings`, `waves`, `arrow`, `zzz`. Circles are rounded `Rectangle`s, everything else a `Path` with the web's commands.
- **States.** `MelliState` (the same eight names). Every state difference sits in one block of properties at the top of the component: the loop values (offset, scale, rotation of `motion`, the shadow) and the poses (which parts show, the eye squash, the overcast colour, the face looking up). Poses, colours and marks ease over `MellowComponent.melli-state-change-duration` with `motion.easing.standard`.
- **Motion.** Every loop is a pure function of `animation-tick()` (no timer): the keyframes of `Melli.svelte` are written as sums of eased ramps over the state's phase, with the durations of `MellowComponent.melli-*`. A state change restarts its loop from rest; the puffs keep their angle while the connection is lost. Clocks stay `duration`s and `MelliAnimation.cycle` takes them modulo the loop before dividing, so the motion stays smooth however long the app runs. Melli is always animated while the window can be seen, focused or not, as the owner chose; its clock stops while the window is occluded or minimized (`melli_animation.rs`), so a hidden window does not redraw. Measured on macOS at 60 Hz (`MELLOW_SPIKE_IDLE_BENCHMARK=1`, release build): about 7 % of one core and 60 frames per second of about 1 ms while visible, against about 0.3 % with a still Melli; nothing while minimized. Under the desktop's reduced-motion choice (2.8), Melli shows the rest pose of its state and stops redrawing.
- **Rail.** `RailItem` with `is-home` draws Melli instead of initials: no fill at rest, apricot squircle on hover, focus and selection, where it passes `MellowColor.mascot-ink` as Melli's `mark-color`. Rust sets `home-melli-state` with `live_rows::home_melli_state`, the web's rule (reconnecting, connecting, unread direct message, idle); the connection notice comes from the same connection status.
- **Icons.** `cargo xtask tokens` rasterises the favicon into `release/icons/` (`mellow-16.png` to `mellow-512.png`, full bleed) and writes `Mellow.icns` (`ic07` to `ic14`, the artwork inside Apple's 824/1024 grid); `--check` compares pixels within a small tolerance. The window icon is `mellow-256.png`, embedded in the binary. The signed binary also embeds `Mellow.icns` and `mellow-256.png` and, at every start and off the UI thread, writes them into `Mellow.app/Contents/Resources` or the user's hicolor icon theme next to the menu entry `install.sh` wrote, when they differ; installs from before get `CFBundleIconFile` or `Icon=mellow` added. It never writes through a link or into anything but a regular file, and replaces files atomically. Nothing unsigned is downloaded, and in-app updates refresh the icon. If the macOS app is ever signed with an Apple Developer ID and notarized, `Mellow.icns` must ship inside the signed bundle and this runtime writer must go on macOS: writing into a signed bundle breaks its signature. The window's app id is `mellow`, like the menu entry, so Linux docks match them. Windows has no installer yet, so no `.ico`.

---

## 3. Token architecture

### 3.1 Single source of truth

All tokens live in one file: `design/tokens.json`, in the W3C Design Tokens Community Group format (`$value`, `$type`, `$description`), using string values for colours (`#RRGGBB` or `#RRGGBBAA`), dimensions (`"72px"`) and durations (`"200ms"`). A generator (a `cargo xtask tokens` command in the workspace) produces the platform files. Nobody edits generated files by hand.

```
design/
  tokens.json              # source of truth (palette, semantic, type, size, motion, components)
  fonts/                   # bundled font files (OFL texts included)
  icons/                   # Phosphor SVG subset actually used
generated (committed, checked in CI):
  crates/mellow-desktop/ui/tokens.slint
  android/core/designsystem/src/main/kotlin/.../theme/MellowTokens.kt
  crates/mellow-server/assets/admin/tokens.css
```

The generated paths are indicative and follow ARCHITECTURE.md. Today `crates/mellow-desktop/ui/tokens.slint` is a hand-copied spike file with the v0.1 warm values; it must be regenerated from this file before M1 UI work.

### 3.2 Three tiers

1. **Reference tokens** (`ref.*`): raw values with no meaning, for example `ref.apricot.300 = #F2A46E`, `ref.dusk.235 = #221B23`. Dusk steps are named by OKLCH lightness × 1000 (`ref.dusk.180` is the dark ground, `ref.dusk.985` the light message hover). Flattened tints (`ref.apricot.wash-dark`, `chip-dark`, …) are also reference values, so semantic tokens never hold raw hex. Only semantic tokens may reference this tier.
2. **Semantic tokens** (`color.*`, `font.*`, `type.*`, `space.*`, `radius.*`, `size.*`, `elevation.*`, `motion.*`): the tokens documented in section 2 and in the dimension table of 4.2.
3. **Component tokens** (`component.<name>.*`): aliases of semantic tokens for the components that define the shell: `rail`, `sidebar`, `category`, `channel-row`, `user-panel`, `chat-header`, `message`, `action-bar`, `mention`, `unread-divider`, `jump-bar`, `composer`, `member-list`, `member-row`, `status`, `badge`, `tooltip`, `context-menu`, `button`, `focus-ring`. Components read this tier first and the semantic tier for anything not listed.

### 3.3 Themes and naming

- **Themes are values, not names.** A colour token's `$value` is the dark value (dark is the default theme); its light value is in `$extensions["org.mellow.theme"].light`. Elevation tokens use the same extension. Aliases resolve within the active theme, so component tokens need no light value of their own.
- Source names are dot-separated, lowercase, kebab-case within a segment: `color.text.on-accent`, `component.channel-row.text-selected`.
- Names describe the role, never the hue: `color.danger.fill`, not `color.red`.
- Labels are sentence case (2.4). The `org.mellow.text-transform` typography extension stays available, but no token uses it since v0.15; strings are never baked in capitals.

| Platform | Transformation | Example for `color.text.on-accent` | Example for `component.channel-row.height` |
|---|---|---|---|
| Slint | Global per top-level category or component, property name is the rest in kebab-case | `MellowColor.text-on-accent` | `MellowChannelRow.height` |
| Compose | Material slot when one exists (3.4), otherwise a camelCase property in `MellowTheme` | `MaterialTheme.colorScheme.onPrimary` | `MellowTheme.components.channelRow.height` |
| CSS | Custom property prefixed with `--mw-`, dots become dashes | `--mw-color-text-on-accent` | `--mw-component-channel-row-height` |

### 3.4 Platform mapping

#### Slint (desktop)

The generator writes one `.slint` file with the globals `MellowColor`, `MellowType`, `MellowSpace`, `MellowSize`, `MellowRadius`, `MellowElevation`, `MellowMotion`, plus one global per component (`MellowRail`, `MellowChannelRow`, `MellowMessage`, …).

- `MellowColor` has an `in-out property <bool> dark` set by the Rust side from the user's theme setting and the OS appearance. Every colour property is generated as `dark ? <dark value> : <light value>`.
- `MellowType` and `MellowSize` multiply every length by `in-out property <float> zoom` (0.5–2.0); `type.message*` and `type.code` are also multiplied by `chat-font-scale` (chat font size / 16).
- `MellowMotion` durations are generated as `reduce-motion ? 0ms : <value>`.
- We build Mellow components on Slint primitives so the Mellow look is identical on every OS; std-widgets are used only for behaviour we cannot reasonably rebuild (for example scrolling), styled with our tokens. Our colour global is `MellowColor` because std-widgets already export `Palette`.

#### Compose (Android, Material 3 Expressive)

The generator writes `MellowTokens.kt` containing the light and dark `ColorScheme`, the `Typography`, the `Shapes`, and an extended `MellowColors`/`MellowSpacing`/`MellowSizes`/`MellowComponents` object provided through a `CompositionLocal`. `MellowTheme { }` wraps `MaterialExpressiveTheme` (or `MaterialTheme` if the expressive theme is not available in the pinned version) with `motionScheme = MotionScheme.standard()`; the settle moments of 2.8 use `spring(motion.spring.settle-damping-ratio, motion.spring.settle-stiffness)`. Dynamic colour is off (section 9): the dusk neutrals are the brand.

| Material role | Mellow token |
|---|---|
| `primary` / `onPrimary` | `accent.fill` / `text.on-accent` |
| `primaryContainer` / `onPrimaryContainer` | `mention.chip-background` / `mention.chip-text` |
| `secondary` / `onSecondary` | `neutral.fill` / `text.on-neutral` |
| `secondaryContainer` / `onSecondaryContainer` (navigation indicator) | `state.selected` / `text.heading` |
| `tertiary` / `onTertiary` | `text.link` / `surface.chat` |
| `error` / `onError`, `errorContainer` / `onErrorContainer` | `danger.fill` / `text.on-danger` |
| `background`, `surfaceDim` / `onBackground` | `surface.sidebar` (the ground) / `text.normal` |
| `surface` / `onSurface` | `surface.chat` (the sheet) / `text.normal` |
| `onSurfaceVariant` | `text.muted` |
| `surfaceContainerLowest` | `surface.field` |
| `surfaceContainerLow` | `surface.sidebar-footer` |
| `surfaceContainer` | `surface.popover` |
| `surfaceContainerHigh` | `surface.input` |
| `surfaceContainerHighest`, `surfaceBright` | `state.selected` |
| `outline` / `outlineVariant` | `border.strong` / `border.divider` |
| `inverseSurface` / `inverseOnSurface` / `inversePrimary` | the other theme's `surface.chat` / `text.normal` / `accent.fill` |
| `scrim` | `scrim` |

Menus, tooltips and the message hover bar do not rely on Material defaults; they read `MellowTheme.components.*`. `Shapes`: `extraSmall = radius.xs`, `small = radius.sm`, `medium = radius.md`, `large = radius.lg`, `extraLarge = radius.xl`. Screens never contain raw `dp` values.

#### Admin web (CSS custom properties)

- `:root` holds the dark values, `@media (prefers-color-scheme: light)` and `[data-theme="light"]` hold the light values, `[data-theme="dark"]` forces dark.
- Every token becomes `--mw-<path>`; spacing and type sizes are emitted in `rem` (16 px base) so browser zoom works.
- Motion durations get `@media (prefers-reduced-motion: reduce)` overrides.
- Fonts are embedded as WOFF2 files served by the server binary, with `font-display: swap`.

### 3.5 Generator guarantees (acceptance criteria, not code)

- `cargo xtask tokens --check` fails if a generated file is stale. CI runs it.
- The generator re-computes every pair in `$extensions["org.mellow.contrast-pairs"]` in both themes and fails if any falls below its `minimum`.
- The generator fails if an alias does not resolve, if an alias cycle exists, or if a `color.*` token lacks a light value.
- The generator fails if any colour used as an accent has an OKLCH hue between 215° and 255° (principle 1.2).

---

## 4. Layout and information architecture

### 4.1 Mental model

```
Instance (chat.example.org)            ← account lives here
 ├─ Home: conversations (Messages)
 └─ Servers
     └─ Server "Rust Lyon"
         ├─ Category "General"
         │   ├─ #welcome
         │   └─ #general
         └─ Category "Projects"
             └─ #mellow
```

One client can be signed in to several instances. Discord has no equivalent, so Mellow adds the smallest possible thing: in the rail, each instance's servers follow a rail separator, and the first icon after it is the instance icon (a server-sized icon whose tooltip and accessible name give the domain). With one instance, nothing is added. Home merges conversations from all instances and labels each with its instance when more than one is connected.

### 4.2 Desktop (Slint)

Mellow keeps Discord's concepts and where people look for them, but not its silhouette (ADR-0022). The window is a dusk ground holding up to three cards, each inset by `size.sheet.inset`: the **navigator** (server strip, channel list, user card), the **sheet** (the conversation) and, on demand, the **side sheet** (members, a member's profile; later threads and pins). Every dimension below is a token.

#### Dimension table

| Element | Dimension | Token(s) |
|---|---|---|
| Window minimum / default | 480 × 560 / 1280 × 800 (or 80 % of the screen if smaller) | `size.window.*` |
| Navigator | Card with radius 16 on `surface.navigator`: server strip (72) beside the channel list (240), user card across both at the bottom | `component.navigator.*` |
| Server strip width | 72 | `size.rail.width`, `component.navigator.strip-width` |
| Rail icon | 48 × 48; circle at rest (radius 24), squircle radius 16 on hover, focus, selection | `size.rail.icon`, `component.rail.icon-radius-*` |
| Rail item gap / top padding | 8 / 12 | `size.rail.gap`, `size.rail.padding-top` |
| Rail pill | 8 wide (4 visible at the rail edge); 8 high unread, 20 hover (`unread.pill`), 40 selected (`accent.indicator`, apricot) | `component.rail.pill-*` |
| Rail separator | 32 × 2, centred | `size.rail.separator-*` |
| Mention badge | 16 high, min 16 wide, 3 px cut-out ring, bottom trailing corner of the icon | `component.badge.*` |
| Channel list width | 240 (resizable 240–360) | `size.sidebar.*` |
| Sidebar header (server name) | 48 high, 16 horizontal padding, no line below (the ground continues) | `size.sidebar.header-height` |
| Category header | 24 high, 16 top margin, 8 padding, 12 px chevron, sentence case | `component.category.*` |
| Channel row | 32 high, 2 gap (34 pitch), 8 margin and 8 padding, radius 8, 20 px icon, 6 px icon gap, 15 px name | `component.channel-row.*` |
| Channel unread pill | 8 × 8 (4 visible) at the sidebar's leading edge | `component.channel-row.unread-pill-*` |
| User card | `size.dock.height` (56) high, `size.dock.margin` (8) from the navigator's bottom and sides, radius 12, `surface.sidebar-footer`, 8 padding, 36 px avatar, three 32 px buttons (microphone, headset, settings) | `size.dock.*`, `component.user-panel.*` |
| Sheet | The conversation card: inset 8 on every side; radius 16; full-bleed and square below 816 | `size.sheet.inset`, `component.sheet.*` |
| Chat header | 48 high, 16 padding, 24 px icons, inside the sheet, no line below | `size.chat.header-height` |
| Chat pane minimum width | 480 | `size.chat.min-width` |
| Message avatar | 40, inset 16 from the pane edge; text gutter 72; trailing padding 48 | `size.message.*`, `component.message.*` |
| Message row | 2 px top and bottom padding, 24 px line height | `component.message.padding-block`, `type.message` |
| Space between message groups | 20 by default (setting: 0, 4, 8, 16, 20, 24) | `component.message.group-gap` |
| Reply reference | 16 px avatar, 2 px spine from the avatar centre | `component.message.reply-*` |
| Hover action bar | 32 high, 32 px buttons, 20 px icons, offset 16 above the row top and 16 from the trailing edge | `component.action-bar.*` |
| Mention bar | 2 wide | `component.mention.bar-width` |
| Composer | Base row `size.dock.height` (56) high, `size.dock.margin` (8) from the sheet's bottom, grows upward to 50 % of the sheet; radius 16; 16 margin each side; the typing indicator's 24 px row sits above it | `size.dock.*`, `component.composer.*` |
| Jump bars | 32 high, radius 12 | `component.jump-bar.*` |
| Side sheet | 240 wide (resizable 240–320), radius 16, `surface.navigator`; docked beside the sheet from 1064 px, over it below | `component.side-sheet.*`, `size.member-list.*` |
| Member group header | 40 high, label at the bottom, sentence case, count as a separate tabular number at the trailing end | `component.member-list.group-height` |
| Member row | 42 high, 2 gap, 8 margin and padding, 32 px avatar, radius 8, 15 px name | `component.member-row.*` |
| Status dot | 10 on 32 px avatars, 16 on 80 px avatars, 3 px ring | `component.status.*` |
| Profile popover | 300 wide | `size.popover.profile-width` |
| Quick switcher / small dialog | 570 / 440 wide | `size.dialog.*` |
| Tooltip | max 190 wide, 8 × 12 padding, 5 px arrow | `component.tooltip.*` |
| Context menu | 188–320 wide, radius 12, 6 padding, 32 px items with radius 6 (concentric) | `component.context-menu.*` |

#### Breakpoints

The navigator needs 8 + 312 + 8 px beside a sheet of at least 480 px plus its trailing 8: 816 px. A docked side sheet adds 240 + 8: 1064 px. Both are computed from the tokens.

| Window width | Layout |
|---|---|
| ≥ 1064 | Navigator + sheet; the side sheet docks beside the sheet when opened (member button or `Ctrl+U`, remembered per server; closed by default) |
| 816–1063 | Navigator + sheet; the side sheet slides over the sheet's trailing side, with a scrim |
| 480–815 | The sheet alone, full-bleed; the navigator opens as a drawer from the leading edge (header button, or `F6` into the navigation regions); the side sheet covers the sheet |

#### Layout (≥ 1064, side sheet open, dark)

```
 8      72          240         8                flexible (≥ 480)                  8      240       8
┌──────────────────────────────────────────────────────────────────────────────────────────────────┐
│ ╭─────────────────────────────╮ ╭─────────────────────────────────────────────╮ ╭──────────────╮ │
│ │ (M)  │ Rust Lyon         ⌄  │ │ # général │ Parle de tout…  [Aller à… Ctrl K] ☰│ │ Membres    ✕ │ │
│ │ ──── │                      │ │                                             │ │              │ │
│ │▌(RL) │ ⌄ Général         +  │ │ (A) Ana  Aujourd'hui à 14:02                │ │ Modération 2 │ │
│ │ (LC) │   # bienvenue        │ │     Quelqu'un a testé Slint sous Wayland ?  │ │ (A) Ana      │ │
│ │ (SF)②│ ▌ # général          │ │ ──────────────────────────────── (Nouveau)  │ │ (L) Léo  ◐   │ │
│ │  +   │   # hors-sujet       │ │▌(Z) Zoé  Aujourd'hui à 14:33     [↩ ⧉ ⋯]    │ │              │ │
│ │      │ ⌄ Projets            │ │▌    @Paul tu peux relire la PR… ?           │ │ En ligne   4 │ │
│ │      │ ▪ # mellow        ①  │ │  ••• Léo écrit…                             │ │ (P) Paul     │ │
│ │ ╭─────────────────────────╮ │ │ ╭─────────────────────────────────────────╮ │ │              │ │ 56
│ │ │(P) Paul Carouge  🎙 🎧 ⚙ │ │ │ │ ⊕  Écrire dans #général                 │ │ │              │ │
│ │ ╰─────────────────────────╯ │ │ ╰─────────────────────────────────────────╯ │ │              │ │
│ ╰─────────────────────────────╯ ╰─────────────────────────────────────────────╯ ╰──────────────╯ │ 8 + 8
└──────────────────────────────────────────────────────────────────────────────────────────────────┘
  navigator (surface.navigator)        the sheet (surface.chat)                     side sheet
```

Legend:

- `(M)` is Home with Melli; the separator below it is the strip separator.
- In the server strip, `▌` is the tall selected pill (40, apricot) and the unread pill is 8 (`unread.pill`). `②` is a red mention badge.
- In the channel list, `▌` is the selected row (`state.selected`, heading text), `▪` the channel unread pill, `①` a mention badge. Category labels are sentence case.
- `(Nouveau)` is the apricot "New" pill at the trailing end of the apricot unread divider; `▌` in the chat marks the 2 px apricot mention bar.
- `[↩ ⧉ ⋯]` is the hover action bar (Reply, Copy text, More; Edit replaces Reply on your own messages, see 5.4).
- The header holds the channel icon and name (`type.heading-md`), a 1 × 24 divider, the topic (`type.body`, `text.muted`, single line), the "Aller à… Ctrl K" button that opens the quick switcher (5.10), and the member button that opens or closes the side sheet.
- The user card and the composer's base row line up: same height (`size.dock.height`), same gap to the bottom of their card (`size.dock.margin`), cards with the same inset. The typing indicator sits above the composer.

**When the composer grows** (several lines, a reply bar, an attachment tray), it grows upward from its base row: the base row keeps `size.dock.height` and stays aligned with the user card; only the composer's top edge moves, and the message list above it shrinks. The user card never grows with it. On desktop the base row is the field's last line with the attach button; on the web it is the same, laid out with `align-items: end`.

Below 816 px the sheet is full-bleed (no inset, no radius) and the navigator opens as a drawer: the same card, full height, over a scrim. The side sheet then covers the sheet from the trailing edge.

#### Connection dock

Discord shows voice connection status in a panel directly above the user panel. Mellow uses the same slot (a card like the user card, `surface.sidebar-footer`, directly above it inside the navigator) for connection and sync status (section 5.2). When voice arrives, it becomes the voice status panel without a layout change.

#### Density

Cozy (default): avatar, author and timestamp on the first message of a group; following messages aligned on the 72 px gutter, with their time shown in the gutter on hover and focus. Compact: one line per message, timestamp in a 56 px column starting at 16 px, author inline, wrapped lines indented to the 72 px gutter, avatars hidden unless "Show avatars in compact mode" is on. Both are set in Appearance settings, as on Discord.

### 4.3 Android (Compose, adaptive)

Android follows Discord's mobile navigation model, built with Material 3 Expressive components and Mellow tokens.

Top-level tabs (a Material `NavigationBar`, shown while the channel panel is open and hidden in chat, as on Discord mobile):

| Tab | Content |
|---|---|
| Servers | Server rail and channel list, with the chat panel on top |
| Messages | Conversations from all connected instances, with a "New message" extended FAB |
| Notifications | Recent mentions and replies across instances |
| You | Profile, status, settings, instances |

#### Compact width (< 600 dp, phones): three sliding panels

```
Channel panel open             Chat (centre)                 Members open
┌────┬───────────────┬──┐     ┌───────────────────────────┐ ┌──┬───────────────────┐
│(M) │ Rust Lyon   ⌄ │ C│     │ ☰  # general         [M]  │ │ C│ MODERATORS — 2    │
│────│ ⌄ GENERAL     │ h│     │                           │ │ h│ (A) Ana           │
│▌RL │   # welcome   │ a│     │ (A) Ana  14:02            │ │ a│ ONLINE — 12       │
│ MB │ ▌ # general   │ t│     │     Has anyone tried…     │ │ t│ (P) Paul          │
│ XY │   # mellow ③  │  │     │ ──────────────── [NEW] ── │ │  │                   │
│ +  │               │  │     │▌(L) Léo  14:33            │ │  │                   │
│    │ (P) Paul   ⚙  │  │     │ ┌───────────────────────┐ │ │  │                   │
├────┴───────────────┴──┤     │ │ ⊕ Message #general  ➤ │ │ │  │                   │
│Servers Msgs Notifs You │     │ └───────────────────────┘ │ │  │                   │
└────────────────────────┘     └───────────────────────────┘ └──┴───────────────────┘
   56 dp of chat stays visible                                  56 dp of chat stays visible
```

("Msgs" and "Notifs" abbreviate the Messages and Notifications tabs in the sketch only.)

- The three panels are one horizontal surface with three anchors (channel panel, chat, members), built with `Modifier.anchoredDraggable` and the theme's spring. A horizontal drag anywhere in the chat moves it: drag right to reveal the rail and channel list, drag left to reveal the member list. When a side panel is open, `size.mobile.panel-peek` (56 dp) of the chat stays visible and tapping it returns to the chat.
- Tapping a channel slides the chat back to the centre and hides the tab bar so the composer has room.
- Every gesture has a button: the top app bar has "Channels" (leading) and "Members" (trailing) icon buttons, and TalkBack users get the same actions as custom accessibility actions on the chat pane.
- Back (including predictive back): from chat, back reveals the channel panel; from the member panel, back returns to the chat; from the channel panel, back leaves the app or returns to the previous tab. This matches Discord mobile and keeps the system back gesture coherent with the drag direction.
- Rows are 48 dp (`size.mobile.row-height`) with a 40 dp visible highlight, so every target meets 48 dp while keeping Discord's look. The rail keeps 72 dp width and 48 dp icons.
- The composer sits above the keyboard with correct IME insets; the app is edge-to-edge with system bar insets handled. On phones the send button is always visible.
- Message actions: long-press opens a `ModalBottomSheet` with Reply, Edit (own), Copy text, Mark unread, Copy message link, Delete. The same actions are TalkBack custom actions. Swipe-to-reply is not used, because horizontal drags belong to the panels.
- Expressive moment, one per screen: the selected server icon morphs from circle to squircle with `MaterialShapes`/`Morph` and the expressive spring, the Android expression of Discord's signature interaction. `LoadingIndicator` shows attachment upload progress on the send button.

#### Medium width (600–839 dp, foldables in portrait, small tablets)

- The rail and channel list are persistent on the leading side, the chat fills the rest; the tab bar sits at the bottom of the leading column.
- The member list opens from the trailing edge with the same drag and button as on phones.

#### Expanded and larger (≥ 840 dp, tablets, foldables in landscape, ChromeOS)

- Rail, channel list and chat are persistent; from 1200 dp the member list is a supporting pane shown by default and togglable.
- Book posture on foldables: the pane split follows the hinge so no content sits on the fold.

#### Push notifications

- One notification channel group per instance, each with channels "Direct messages", "Mentions and replies", "Other messages" (off by default) and "Connection problems" (low importance).
- `MessagingStyle` with the sender's avatar, conversation title `#general · Rust Lyon` or the conversation name, inline reply and "Mark as read" actions.
- Lock screen: content hidden by default ("New message from Ana") until the user allows previews in settings, a privacy choice for a self-hosted product.
- The first time a user signs in, Mellow explains notifications before showing the Android 13+ permission prompt. If permission is denied, You → Notifications shows "Notifications are off for Mellow. You won't hear about mentions or messages." with "Open Android settings".

### 4.5 Web client (Svelte)

The web client ([ADR-0019](docs/adr/0019-web-client-served-by-the-server-binary.md), `web/`) is the desktop shell in a browser, for people who join from a link or a phone until the Android app ships. It reuses everything above: the dimension table, the tokens (as `--mw-*` custom properties and Tailwind classes generated from `design/tokens.json`), the Phosphor icons of the desktop client and the copy of section 8. It has no instance address field: it talks to the instance that served it and names it ("Sign in to chat.example.org.").

| Window width | Layout |
|---|---|
| ≥ 1064 | As on desktop: navigator, sheet, and the side sheet docked when opened |
| 816–1063 | Navigator and sheet; the side sheet opens over the sheet from the trailing edge, with a scrim |
| < 816 (phones included) | The sheet alone; an "Open servers and channels" button (`list` icon) at the leading end of the chat header opens the navigator as a drawer from the leading edge, leaving `size.mobile.panel-peek` of the sheet visible; choosing a channel closes it and focuses the composer. Where the window cannot hold the strip, the full channel list and the peek (below 368 px), the channel list gives up width and truncates, never its trailing controls |

```
Phone (< 816)                     Drawer open
┌─────────────────────────────┐   ┌──────┬──────────────┬────┐
│ ☰  # general           [M]  │   │ (M)  │ Rust Lyon  ⌄ │░░░░│
│                             │   │ ──── │ ⌄ GENERAL    │░░░░│
│ (#) Welcome to #general     │   │▌(RL) │ ▌# general   │░░░░│
│ (A) Ana  Today at 14:02     │   │  +   │              │░░░░│
│     Has anyone tried…       │   │      │ (P) Paul  ⇥  │░░░░│
│ ┌─────────────────────────┐ │   └──────┴──────────────┴────┘
│ │ Message #general      ➤ │ │     drawer = rail + sidebar, scrim
│ └─────────────────────────┘ │
│ Ana is typing…              │
└─────────────────────────────┘
```

- Breakpoints are computed from the pane width tokens, as on desktop, so they follow the tokens. They hold from 320 px phones to 1920 px screens, in phone landscape (740 to 844 × 360 to 390: the navigator layout from 816 px) and with text at 200 %; no Tailwind screen breakpoint (`sm`, `md`…) is used, since none matches the tokens.
- Window edges: the page runs under notches, rounded corners, the installed app's status bar and the home indicator (`viewport-fit=cover`), so the ground colours them; the shell, the navigator drawer, the full-window members sheet, the phone settings and the bottom sheets pad themselves by `env(safe-area-inset-*)` (zero on screens without such areas).
- Soft keyboard: `interactive-widget=resizes-content` makes the keyboard shrink the layout in Chrome and Firefox on Android, so the shell (`100dvh`) shrinks, the message list stays on the latest message and the composer sits right above the keyboard. Dialogs and the quick switcher are sized in `dvh` for the same reason.
- Menus opened from a button (a message's More, a friend's More) open under it, or above it when the window has no room below (`menu-placement.ts`); pickers and panels already keep within the window, and become bottom sheets on phones.
- Installable app: the server serves `manifest.webmanifest` (name and short name "Mellow", `id`, `start_url` and `scope` `/`, `standalone`, the ground `color.surface.rail` of the dark theme as theme and background colour, icons of 2.10), and `index.html` carries one `theme-color` per colour scheme, so an installed window's title bar is the ground it frames. All of it is generated by `cargo xtask tokens`. There is no service worker: Chrome installs without one, nothing personal can be cached, and an installed window always loads the client the server currently serves. No install banner either: Chrome, Edge and Android Chrome offer installation in their own interface.
- Under a coarse pointer (`@media (pointer: coarse)`), rows, header and hover-bar buttons, menu items and inline links grow to `size.hit.web-min` (44 px), and the composer shows a send button (`paper-plane-right`), as on Android; with a mouse the composer keeps Discord's Enter-to-send without a button.
- Dialogs and drawers are native modal `<dialog>` elements: the page behind is inert, Esc and the scrim close them (dialogs not while busy), focus returns to the control that opened them. The server menu and the profile popover use the popover API (light dismiss, Esc), with focus returning to the header or the member.
- The message list has one Tab stop: `Up`/`Down`/`Home`/`End` move between messages, and the focused message shows its hover bar, so every action is reachable by keyboard and by tap. New messages from others are announced through a polite live region; action errors through an alert.
- Message text is rendered as Discord's Markdown (5.4) by a parser of its own (`markdown.ts`, a typed tree rendered element by element, never as HTML): bold, italic, underline, strikethrough, inline code, fenced code blocks (language kept, no highlighting), `>` and `>>>` quotes, `||spoilers||` (a button, "Spoiler, select to reveal", that gives way to the text and moves focus to it; `component.message.spoiler-*`), headings `#` to `###`, `-`/`*` lists, masked links `[text](url)` (destination in the tooltip, no confirmation dialog yet) and bare URLs, backslash escapes. Only `http:` and `https:` become links, opened in a new tab with `rel="noopener noreferrer"`. Mentions are never read inside code. Not yet: subtext `-#`, numbered lists, "Copy code", nested lists.
- User settings (5.9) open from the user panel's gear button ("User settings"), as on desktop; "Sign out" is the last item of their navigation.
- Attachments (5.4, 5.5): images open in a new tab instead of the viewer; an image that fails to load becomes its download card instead of "Couldn't load this image."; no upload progress percentage (the tray says "Uploading…"), no "Add a description" or "Mark as spoiler" yet.
- Server settings follow 5.8 ([design/web-server-settings.md](docs/design/web-server-settings.md)); channel settings are not there yet.
- Not in the web client yet: day dividers, the connection dock's "Back online" and session-expired states. Mentions are found by the server with the client's rule (`Message.mention_ids`) and counted in `Ready.channel_unreads` for the red badges; the client still reads names itself to draw chips, and to highlight messages written before the server found mentions ([design/web-dusk-navigator.md](docs/design/web-dusk-navigator.md)).

### 4.4 Admin web UI (server-rendered HTML + htmx)

Purpose: instance administrators check health and manage users, servers, invites and settings. Usage is occasional, often from a laptop, sometimes from a phone. The layout reuses Discord's settings screen, so admins recognise it: a navigation column on `surface.sidebar` and content on `surface.chat`, with the same palette, font and sentence-case section labels.

```
Wide (≥ 960 px)                                   Narrow (< 960 px)
┌──────────────┬───────────────────────────────┐  ┌──────────────────────────┐
│ (M) chat.ex… │ Overview                      │  │ (M) chat.ex.org   Menu ⌄ │
│ INSTANCE     │                               │  │ Overview                 │
│ Overview   ▌ │ ┌────────┐┌────────┐┌────────┐│  │ ┌──────────────────────┐ │
│ Users        │ │Healthy ││Users   ││Storage ││  │ │ Healthy              │ │
│ Servers      │ │v0.4.1  ││1,204   ││12/50 GB││  │ └──────────────────────┘ │
│ Invites      │ └────────┘└────────┘└────────┘│  │ ┌──────────────────────┐ │
│ MAINTENANCE  │ RECENT SIGN-UPS               │  │ │ Users 1,204          │ │
│ Storage      │ ┌───────────────────────────┐ │  │ └──────────────────────┘ │
│ Backups      │ │ table                     │ │  │ ...                      │
│ Updates      │ └───────────────────────────┘ │  │                          │
│ Settings     │                               │  │                          │
│ Paul · Sign out                              │  │                          │
└──────────────┴───────────────────────────────┘  └──────────────────────────┘
```

- Navigation items look like Discord settings items (`type.body-lg`, `radius.sm`, `state.hover`/`state.selected`) but are 44 px high, the web target minimum. Section labels use `type.category`.
- Content is capped at 740 px for forms (Discord's settings width) and 1120 px for tables. Under 960 px the navigation becomes a `<details>` disclosure menu in the header.
- Every page works without JavaScript for reading (plain links and forms); htmx enhances searching, pagination, inline actions and the health refresh.
- The health summary refreshes every 15 s with `hx-trigger="every 15s"` only while the page is visible.
- Destructive actions use a native `<dialog>` with the confirmation pattern in section 8, and a server-side confirmation page when JavaScript is off. Primary buttons are apricot, destructive buttons red, as everywhere else.

---

## 5. Key screens and states

Common state rules for every screen:

- **Loading**: static skeletons shaped like the real content, shown after `motion.delay.skeleton` (shorter loads show nothing, to avoid flicker). Never a lone centred spinner.
- **Empty**: a short title, one sentence that explains, one action.
- **Error**: what happened, why if known, what to do, and a retry action. Errors keep any content already loaded visible.
- **Offline**: cached content stays readable; actions that need the network are disabled with a reason; messages typed offline are queued.
- **Reconnecting**: shown in the connection dock (desktop) or a slim banner under the top app bar (Android), never as a modal.

### 5.1 Instance connect, sign in, register

Flow: Welcome → Instance address → Instance card → Sign in or Create account → (first server or invite) → Home.

All steps use one centred card (480 px wide, `surface.chat`, `radius.xl`, `space.xl` padding, `elevation.high` in light) on the ground (`surface.rail`). This is the one screen where Melli is the hero: Melli sits above the title at `component.auth-card.melli-size` (96 px), animated in its current state (`connecting` while the instance is checked, `connection-lost` when it cannot be reached, otherwise `idle`), half outside the card's top edge so the card and the cloud read as one object. Titles use `type.heading-xl`, field labels `type.category` (sentence case). The card opens with the dialog transition (2.8); nothing else moves.

**Welcome (first run)**: title "Chat that you can host yourself." One sentence: "Connect to a Mellow instance to sign in or create an account." Primary button "Connect to an instance". Link button "I have an invite link".

**Instance address**: one field "Instance address", placeholder `chat.example.org`. We accept full URLs and strip the scheme and path. Primary button "Continue".

| State | Presentation |
|---|---|
| Checking | Button shows a progress indicator and the label "Checking chat.example.org…"; field stays editable; "Cancel" appears |
| Not reachable | Field error: "We couldn't reach chat.example.org. Check the address, or try again if your connection is down." |
| Not a Mellow instance | "chat.example.org is online but doesn't look like a Mellow instance. Check the address with the person who invited you." |
| Incompatible version | "chat.example.org runs Mellow 0.3. This app needs 0.5 or later. Ask the instance admin to update." |
| Certificate problem | "The security certificate of chat.example.org isn't valid, so we didn't connect. The instance admin needs to fix it." No bypass in the MVP (section 9). |
| Offline | Field disabled, info banner: "You're offline. Connect to the internet to add an instance." |

**Instance card**: icon (circle, as a server), name, description, member count if public, registration mode ("Open to everyone", "Invite only", "Registrations closed"), link to rules. Actions: "Sign in" (primary when the account likely exists), "Create account".

**Sign in**: "Username or email", "Password" (show/hide toggle with accessible label "Show password"), primary "Sign in", link "Forgot your password?" (only when the instance has email configured).

| State | Copy |
|---|---|
| Wrong credentials | "That username and password don't match. Check them and try again." |
| Rate limited | "Too many attempts. You can try again in 5 minutes." (time from the server) |
| Account suspended | "This account is suspended on chat.example.org. Contact the instance admin if you think this is a mistake." |

**Create account**: username (live availability check after 400 ms idle: "Available" in `text.positive` with an icon, or "Taken. Try another one."), display name (optional, "Shown to others. You can change it later."), email (if required), password with its requirements listed up front ("At least 12 characters"), invite code (prefilled from an invite link, shown only when invite-only), and a checkbox "I agree to the rules of chat.example.org" when the instance has rules. Primary "Create account".

**Adding another instance**: the `+` at the bottom of the rail opens a menu: "Create a server", "Join with an invite", "Connect another instance". The connect flow is the same, in a dialog on desktop and full screen on Android.

### 5.2 Server strip (the rail)

The rail is now the **server strip** inside the navigator card (4.2): same items, sizes, pills and badges, on `surface.navigator` instead of a column of its own.

Contents, top to bottom: Home (Mellow mark; mention badge aggregating conversations from all instances), unread conversations as avatars (as on Discord, shown only while unread), rail separator, the servers of the first instance, then for each further instance a separator, its instance icon and its servers, then `+` (add) in green.

| Element state | Presentation |
|---|---|
| Rest | Server artwork in a 48 px circle; servers without artwork show initials in `text.normal` on `surface.chat` |
| Hover / focus | Settle into the squircle (radius 16, `motion.duration.settle`, settle curve); icons without artwork switch to `state.selected` with `text.heading`; pill grows to 20; tooltip with the server name (and instance when ≥ 2) appears immediately, to the trailing side |
| Unread messages | Pill 8 high |
| Mentions | Red badge with the count (`danger.fill`, `text.on-danger`, 3 px ring of `surface.rail`) at the bottom trailing corner; "99+" beyond 99; also unread pill |
| Selected | Squircle, apricot pill 40 high (`component.rail.pill-color-selected`); icons without artwork stay on `state.selected` |
| Home selected | Squircle on `accent.fill` behind Melli, apricot pill 40 |
| Home states | At rest Home is Melli alone on the rail, with no fill; hover, focus and selection fill it apricot and switch Melli's marks to ink. Melli shows `connection-lost` while reconnecting, `connecting` while connecting, `notification` while a direct message is unread, otherwise `idle` (2.10). The pill and the accessible name ("Direct messages, unread") still carry unread on their own; the connection notice says what the connection is doing |
| Muted server | No unread pill; mention badges still show, because direct mentions are never muted |
| Instance offline | Its icons are shown at 60 % opacity with a tooltip "chat.example.org is unreachable. Showing saved messages." and the instance icon gets an offline status dot |
| Loading (first sync) | Static circle skeletons for the expected number of servers |
| Empty (no servers) | Only Home and `+`; Home shows the empty state in 5.6 |

Reordering: drag and drop with the pointer, `Ctrl+Shift+Up/Down` with the keyboard, "Move up"/"Move down" in the context menu. Server folders are out of scope for the MVP.

Server context menu (right-click, `Shift+F10`): Mark as read, Invite people, Mute server, Notification settings, Server settings, Copy server link, Leave server (danger, last, separated).

**Connection dock** (desktop, above the user panel) and slim banner (Android):

| State | Copy |
|---|---|
| Connected | Hidden |
| Reconnecting | "Reconnecting to chat.example.org…" with a small progress indicator; after 30 s: "Still trying to reconnect. Messages you send will wait until you're back." with "Retry now" |
| Offline (device) | "You're offline. Showing saved messages." |
| Back online | "Back online" for 2 s, then hidden; missed messages are fetched and the unread divider marks them |
| Session expired | "Your session on chat.example.org has ended. Sign in again to keep chatting." with "Sign in" |

### 5.3 Channel sidebar

Header (48 px): server name (`type.heading-md`, `text.heading`) with a chevron opening the server menu (Invite people, Server settings, Create channel, Notification settings, Mark as read, Leave server). Then categories and channels, then the connection dock and the user panel.

| Row | Presentation |
|---|---|
| Read channel | `#` icon and name in `text.muted` |
| Hover | `state.hover` background, `text.normal` |
| Unread channel | Name and icon in `text.heading`, unread pill at the sidebar edge |
| Channel with mentions | As unread, plus a red count badge at the trailing end |
| Selected | `state.selected` background, `text.heading` |
| Muted | `text.channel-muted`, no unread pill or heading colour; mention badges still show |
| Locked (cannot send) | Lock icon (Phosphor `lock`, Fill) replaces `#`, with the same size and colour as `#` in every row state; the accessible name gets ", read-only" (for example "announcements, read-only, unread") |
| Category | `type.category` (sentence case, as the server admin typed it), chevron, collapsible by click or `Left`/`Right`; collapsed categories still show unread and selected channels, as on Discord |

Channel context menu: Mark as read, Mute channel, Notification settings, Copy channel link, Edit channel (if allowed), Delete channel (danger, if allowed).

States:

- Loading: 6–8 static row skeletons with varied widths.
- Empty server (no channels, user can manage): "No channels yet" + "Create a channel". User cannot manage: "No channels you can see yet. Ask a moderator for access."
- Error: inline banner "Couldn't load channels for Rust Lyon." with "Try again", while keeping cached channels.

User card (`size.dock.height`, `surface.sidebar-footer`, radius 12, across the whole navigator at its bottom, aligned with the composer, 4.2): avatar 36 with status dot, display name (`type.user-panel-name`), a second line with the custom status or `@username` (and `@instance` when ≥ 2 instances) in `text.muted`, then three 32 px buttons: microphone, headset, settings. Clicking the avatar or name opens the status menu (Online, Idle, Do not disturb, Invisible, custom status).

Status menu (web, v0.19): the avatar and the two lines are one button (`aria-haspopup="menu"`, `aria-expanded`; its accessible name is "Ana, Idle, Reviewing PRs. Set your status"), at least 44 px tall, washed in `component.user-panel.button-background-hover` while hovered or open; the status ring follows that wash and the second line turns `component.user-panel.status-text-hover`. The menu is the ContextMenu of 6.33 opened above the card, as wide as the card, lifting in by `space.2xs` over `motion.duration.fast` (at once under reduced motion):
- Online, Idle, Do not disturb ("You won't get notifications" / « Tu ne recevras pas de notifications »), Invisible ("You'll look offline" / « Tu apparaîtras hors ligne »): `menuitemradio`, each with its 12 px shape (`component.status.size-menu`) leading and a check trailing on yours. Picking one keeps the menu open, its status items dimmed (`aria-disabled`), until the instance agrees, then closes it; picking yours closes it.
- A separator, then "Set a custom status" or "Edit custom status" (smiley icon), then a separator and "Copy user ID" (copy icon).
- Focus goes to your current status when the menu opens; arrows wrap, Home and End jump, Esc and a click outside close it, and focus returns to the button.
- Errors ("Couldn't change your status: too many attempts…", "Couldn't copy your user ID. Allow clipboard access and try again.") show at the foot of the menu with the warning icon, in a live region; the menu stays usable.
- Copy user ID closes the menu and the card's second line shows a check icon and "User ID copied" for `motion.delay.copied-label`, also announced politely, where focus lands rather than in a toast.

Custom status dialog (Dialog, 6.35): titled "Set a custom status" or "Edit custom status"; one field "Custom status" with the placeholder "What are you up to?" and, as its helper, "120 characters left" or "3 characters too many" (Unicode characters of the trimmed text, as the instance counts, up to 128); "Clear status" (ghost button, its label aligned with the field) when one exists; Cancel and "Save status", which stays disabled until the text changed and fits. Enter saves, Esc cancels; while saving the field is read-only and the buttons wait. A refusal for hidden characters says "remove line breaks and invisible characters, then try again"; other errors use the usual "Couldn't save your custom status: …". The status shape is kept; your own card shows your custom status even while invisible, others see none then. The settings button opens the user settings screen (5.9); closing it returns focus to the button (to the composer when the navigator is a drawer). While an app update is available or ready to install, the settings button carries a 10 px dot in `component.user-panel.update-badge` (`positive.fill`, 3 px cut-out ring in the card colour, top trailing corner, like presence dots) and its accessible label becomes "User settings, update available" (« Paramètres utilisateur, mise à jour disponible »); the dot is a shape on the button, so it does not rely on colour alone.

**Microphone and headset** (Discord's convention, ADR-0022):

| Control | Rest | Muted | Accessible name (stays the same) | Tooltip at rest / muted |
|---|---|---|---|---|
| Microphone (`microphone`) | `text.secondary` icon | The icon with a 2 px diagonal strike-through cut out of the card colour, both in `component.user-panel.toggle-muted-icon` (`text.danger`) | "Mute microphone" / « Couper le micro », with `aria-pressed` (desktop: `accessible-checked`) | "Mute microphone" / "Unmute microphone" · « Couper le micro » / « Réactiver le micro » |
| Headset (`headphones`) | `text.secondary` icon | Same treatment | "Deafen" / « Couper le son », with `aria-pressed` | "Deafen" / "Undeafen" · « Couper le son » / « Réactiver le son » |

- Deafening also mutes the microphone (it shows muted); undeafening restores the microphone to what it was before, as on Discord. Shortcuts: `Ctrl+Shift+M` (microphone) and `Ctrl+Shift+D` (headset), as on Discord.
- The strike-through and the colour change together, so the state never relies on colour alone; `text.danger` reaches 6.40:1 (dark) and 5.74:1 (light) on the card.
- **Until voice ships** (ROADMAP: after the web client, with its own ADR), both buttons are hidden in released builds: a toggle with no effect would mislead. The card keeps its height and layout so nothing moves when they appear. The mockup shows them toggling.

### 5.4 Message list

#### Structure and grouping

- Messages from the same author are grouped when they follow each other within 7 minutes, with no day divider, no unread divider and no reply in between (Discord's rule).
- Cozy group start: avatar 40 at 16 px, author name (`type.message-author`, role colour adjusted for readability, otherwise `text.heading`), role icon if any, timestamp (`type.message-timestamp`, `text.muted`: "Today at 14:02", "Yesterday at 09:15", or a locale-formatted date for older messages; full date and time in the tooltip and accessible name). Group starts are separated by `component.message.group-gap`.
- Continuation messages align on the 72 px gutter; their time ("14:03") appears in the gutter on hover and focus.
- Day dividers: a `border.divider` rule with a centred date ("13 October 2026") in `type.message-timestamp`, `text.muted`.

#### Message body rendering (Slint constraint)

Per ADR-0010, Slint's `StyledText` renders inline formatting only. The Rust side splits each message body into blocks, and each block is rendered by its own element:

| Block | Rendering |
|---|---|
| Paragraph | `StyledText`: bold, italic, strikethrough, inline code (`surface.code`, `radius.xs`), links (`text.link`), masked links `[text](url)` (the URL is shown in a confirmation dialog before opening, as on Discord), emoji, mention chips |
| Heading (`#`, `##`, `###`) | `type.message-h1` / `-h2` / `-h3` |
| Subtext (`-# text`) | `type.message-subtext`, `text.muted` |
| Quote (`> text`) | 4 px `border.quote` bar, text indented by 12 |
| Code block | `surface.code` with a 1 px `surface.rail` border, `radius.sm`, `type.code`, horizontal scroll inside the block only, "Copy code" button on hover and focus (Mellow addition, because text cannot be selected across blocks) |
| List (`-`, `1.`) | Indented list items, each a paragraph |
| Spoiler (`\|\|text\|\|`) | Rendered as a block-level hidden span ("Spoiler, select to reveal") until revealed; inline spoilers inside a sentence are a known gap (section 9) |

Raw HTML is escaped before parsing so senders cannot restyle text (covered by a unit test). Discord's `__underline__` is not supported by Slint's Markdown parser and renders as bold; this is listed in section 9.

#### Selecting and copying text

Discord lets people drag-select across several messages; Slint cannot (ADR-0010). Mellow replaces it with explicit, per-message actions:

- **Copy text** is in the hover action bar (an extra icon next to Reply), in the context menu, and on `Ctrl+C` when a message is focused. It copies the message's source text, as Discord's "Copy Text" does.
- **Select text** (context menu, or `F2` on a focused message) turns that one message into a read-only text field showing its plain text, so part of it can be selected and copied with the pointer or `Shift+Arrow`. `Esc` or moving focus away returns to the formatted view. The message shows a hint under it: "Selecting text · Esc to finish".
- **Copy code** on code blocks and **Copy link** on links cover the most common partial copies.
- Selection across messages is not offered; there is no fake multi-message selection.

#### Unread divider and jump bars

- When opening a channel with unread messages, the list scrolls to the first unread message and shows the divider: a 1 px `unread.line` rule (apricot) with an apricot "New" pill (`type.badge`, `text.on-accent`, fully rounded) at its trailing end. Discord's divider is red; Mellow's is apricot because new messages are "now", and red stays for danger and mention counts (ADR-0022). The accessible name is "New messages since 14:32".
- A bar pinned to the top of the sheet, in `accent.fill` with `text.on-accent`, `radius.md`, sliding in with the jump-bar transition (2.8): "12 new messages since 14:32" and "Mark as read" (`Esc`). It disappears when the divider has been seen and the user reaches the bottom.
- When the user scrolls up, a bar appears directly above the composer, in `accent.fill`: "You're viewing older messages" and "Jump to present" (or "Jump to present · 3 new" when new messages arrived). `End` (list focused) or `Esc` (composer focused) jumps. The list never auto-scrolls while the user reads older messages.

#### Replies

- A reply shows a one-line reference above the message: a 2 px `border.quote` spine curving from the avatar column, the replied author's 16 px avatar and name, and a single-line excerpt in `text.secondary`. Clicking it or pressing `Enter` on it jumps to the original and highlights it with `mention.background` fading over `motion.duration.highlight`.
- If the original was deleted: "Original message was deleted" in `text.muted`, not clickable.
- If the original is not loaded yet, jumping loads the surrounding history first (with skeletons) and keeps "Jump to present" available. Jumping to a message whose row height is only estimated by `ListView` must land exactly on the message (ADR-0010 note).

#### Mentions

- A message that mentions you (directly, via a role you have, or `@everyone`) gets `mention.background` and a 2 px `mention.bar` on the leading edge. The bar ensures the state is not carried by the background alone; the accessible name starts with "Mentions you".
- Mention chips inside text (`@Ana`, `@Moderators`, `#general`) use `mention.chip-background` / `mention.chip-text` with `radius.xs`; on hover and focus they switch to `accent.fill` / `text.on-accent`. They are focusable and open the profile, role description or channel.

#### Hover action bar

On hover or focus, a bar appears at the trailing top of the message (`component.action-bar.*`: `surface.chat`, 1 px `border.divider`, `elevation.medium`). Buttons, each 32 px with a tooltip: three quick reactions (the user's latest picks, else 👍 ❤️ 😂; hidden under a coarse pointer) and Add reaction, with "Add reactions"; Reply (others' messages) or Edit (your own), Copy text, More. More opens the message context menu: Reply, Edit (own), Copy text, Select text, Copy message link, Mark unread, Pin or Unpin message ("Manage messages", or anyone in a conversation; after a confirmation with the message's preview), Reactions (who reacted, by emoji), Remove all reactions (moderators, danger, confirmed), Delete (own, or with "Manage messages", danger, last). The web client has the Pin, Reactions and Remove all reactions items so far, beside its bar buttons. Forwarding is not in the MVP (section 9). Right-click on a message opens the same menu.

**Reactions** (`component.reaction.*`): chips under the message's text and files, `reaction.row-gap` apart: the emoji and its count, 28 px tall (44 px under a coarse pointer), `surface.input` with an edge only on hover; the user's own take `mention.background` with an `accent.indicator` edge and `mention.chip-text` count, and are `aria-pressed`. A click adds or takes back the user's reaction at once and undoes it if the instance refuses. Hovering or focusing a chip shows, after `motion.delay.tooltip`, "You, Ana and 3 others reacted with 👍" (« Toi, Ana et 3 autres avez réagi avec 👍 »), asked from the instance only then. A smiley chip after them opens the picker (on hover or focus, always under a coarse pointer). Without "Add reactions" no new emoji can be added, but existing ones can be joined; without "Read message history" nothing reacts. Moderators remove someone's reaction from the Reactions dialog.

**Pins**: a pinned message shows a 14 px pin (`reaction.pinned-icon`) after its time, or in the gutter of a continuation, with "Pinned" for screen readers. The chat header's pin button opens the pinned messages panel (6.53).

Desktop, until the More menu exists: your own messages show Edit, Reply, Copy text, Delete and More, in that order; others' messages show Reply, Copy text and More, plus Delete before More when you have "Manage messages" there. Reply and Edit are hidden in channels where you cannot send messages, since the server refuses both there. The Delete icon turns `text.danger` on hover, as on Discord. Delete opens the "Delete this message?" dialog (section 8: `component.dialog.*`, window-wide scrim, the message's author and text as a preview, focus on "Cancel", `Tab` cycles between the two buttons, `Esc` or a click on the scrim cancels); `Shift`+click deletes at once, as on Discord.

#### Edited, deleted, pending, failed

| State | Presentation |
|---|---|
| Edited | "(edited)" after the text, `type.message-timestamp` at 12 px, `text.muted`; tooltip and accessible description give the edit time. Desktop: the body is a Rust-built `StyledText`, which can neither carry a differently styled suffix nor keep its wrapping inside a horizontal layout, so "(edited)" sits on its own line under the text rather than at the end of the last line; the row's accessible name ends with "(edited)" |
| Deleted by author | Removed from the list for everyone |
| Deleted by a moderator | Removed; the moderation log keeps the record |
| Pending | Text in `text.muted`; after `motion.delay.pending-label` a "Sending…" label appears |
| Queued offline | `text.muted`, label "Will send when you're back online", with "Cancel" |
| Failed | `text.danger` icon and "Not sent." followed by "Retry" and "Delete"; the text stays intact so nothing is lost |

#### Attachments

- Images: inline, aspect ratio preserved, max 400 × 300 (cozy) or 320 × 240 (compact), `radius.md`, a dominant-colour block while loading. Click or `Enter` opens the viewer (zoom, next/previous, "Save image", "Copy link", `Esc` to close). Alt text, when present, is the accessible name; otherwise "Image, no description, file name.jpg".
- Animated images: first frame shown; play on hover or focus (or always, per accessibility setting).
- Other files: a `surface.sidebar` card with `radius.md`, file-type icon, file name (middle-truncated, full name in tooltip and accessible name), size, and a "Download" icon button labelled "Download file name.pdf".
- Loading failed: "Couldn't load this image." with "Try again"; the card keeps its size. Removed by the server: "This file is no longer available."
- Previewable images are the four types the instance serves inline (PNG, JPEG, GIF, WebP, checked by their bytes, ADR-0020); everything else, SVG included, is a file card.
- Web client (`component.attachment.*`): attachments sit under the text, `attachment.gap` apart, in a list named "2 attachments" (7.3). Images are links that open the file in a new tab, with the file name as alt text; until loaded they hold a 400 × 300 `image-placeholder` block (the protocol has no pixel size yet), then shrink to the image, and the list stays pinned to the latest message. File cards (max 432 px wide, `surface.sidebar`, 1 px `border.divider`, `radius.md`): apricot file glyph (`file-icon`, decorative), the name middle-truncated as the download link (`text.link`, full name in its tooltip and accessible name "Download report.pdf"), the size ("2.4 MB", binary units with Discord's labels) in `text.muted`, and a download icon duplicating the link for the pointer only. A message still sending shows its files as cards without links, its name in `message.pending-text`.
- Desktop, until it handles files: one compact row per attachment under the text, a 16 px file glyph and "name · size" in `attachment.row-text`; the row's accessible name ends with "2 attachments".

#### List states

| State | Presentation |
|---|---|
| Initial load | 5 static message skeletons (avatar circle, name bar, 1–3 text bars) |
| Loading older history | 3 skeletons at the top; scroll position stays stable |
| Beginning of channel | Discord's channel-start hero: a 68 px circle with the channel icon, "Welcome to #general" (`type.display`), "This is the start of the #general channel." and the topic, then "Edit channel" (if allowed) |
| Empty channel | Same hero; composer placeholder "Message #general" |
| Error (initial) | Replaces the skeletons: "Couldn't load messages in #general. The instance didn't respond." with "Try again" |
| Error (older history) | Inline at the top: "Couldn't load older messages." with "Try again"; newer messages stay visible |
| Offline | Cached messages shown; info pill at the top: "Offline. Showing saved messages." |
| No access (permissions changed) | "You no longer have access to #general." with "Go to #welcome" (first accessible channel) |
| Messages from someone you blocked (web, ADR-0025) | Each run of consecutive messages by one blocked author folds into one row in the message column: a `prohibit` icon in the avatar column, "2 blocked messages" in `component.message.blocked-text` and a "Show" button (`blocked-action-text`, `aria-expanded`); shown, the messages follow the row and the button reads "Hide", for the visit (remembered by message, so an older page extending the run keeps it shown and the reading position). The row takes the list's one Tab stop like a message, where Enter or Space toggles it, and its button stays out of the Tab order (7.2). The "New" divider sits above the folded row when it holds the first unread message, and above the message itself once shown, never twice. Such messages are never announced; a reply to one, and its pinned copy in the pins panel, read "Blocked message", and Jump from the pins panel shows the run first. The member list and the profile keep working. |

### 5.5 Composer

Layout: a `surface.input` field with `radius.lg` whose base row is `size.dock.height` (56) high, `size.dock.margin` (8) above the sheet's bottom and aligned with the user card (4.2), 16 px side margins, with the typing indicator's 24 px row above it (not below, as on Discord, so the composer and the user card share a bottom edge). Attach button (`⊕`, 24 px) on the leading side; emoji button on the trailing side when the emoji picker ships (section 9). Desktop has no send button by default, as on Discord (`Enter` sends); the accessibility setting "Show send button" adds one. Android always shows the send button.

- Placeholder: "Message #general", "Message @Ana" in conversations, or "Message #general on chat.example.org" when ≥ 2 instances are connected.
- Markdown is typed as plain text (the composer field cannot render inline styles, section 9) and rendered in messages as in 5.4.
- Shortcuts: `Ctrl+B` and `Ctrl+I` wrap the selection in bold and italic markers. `Ctrl+E` is reserved for the emoji picker, as on Discord.
- Autocomplete popovers (`surface.popover`) for `@`, `#` and `:` (emoji shortcodes), above the field, navigable with arrows, `Tab`/`Enter` to insert, `Esc` to close.
- Sending: `Enter` sends, `Shift+Enter` inserts a new line (desktop). Android: Enter inserts a new line and the send button sends; a setting swaps this for hardware keyboards.
- Edit mode: `Up` in an empty composer edits your last message inline. The message shows "Esc to cancel · Enter to save" with the two words as link buttons, as on Discord. Saving an empty message asks to delete it. Desktop: the field (`component.message.editor-*`, `surface.input`, `radius.md`) takes focus with the caret at the end; `Shift+Enter` inserts a new line; saving unchanged text just closes the editor; focus returns to the composer.
- Reply mode: a bar attached above the field (`surface.sidebar`): "Replying to Ana", a "@ On/Off" toggle for pinging the author (on by default, accessible label "Mention Ana"), and a close button (`Esc`). Desktop (`component.composer.reply-bar-*`): the field's top corners square off under the bar, the composer takes focus when a reply starts, and the close button is a filled x-circle labelled "Cancel reply"; the "@ On/Off" toggle comes with mention notifications.
- Action errors: when an edit or delete fails, one line in `text.danger` sits above the composer (or the reply bar) and is announced assertively.
- Attachments: attach button, drag and drop anywhere on the chat pane (overlay "Drop to attach to #general"), or paste. An attachment tray above the field shows thumbnails with "Add a description" (alt text), "Mark as spoiler" and "Remove". Upload progress is shown per file. Upload errors: "photo.heic is 48 MB. chat.example.org accepts files up to 25 MB."
- Attachments in the web client (first version, ADR-0020: upload first, then send):
  - The attach button ("Attach files", `plus-circle`, `composer.attach-button-size`) sits at the leading end of the field, only where the user has "Attach files" (and can send). It opens the browser's file picker (several files at once).
  - Dragging files over the chat pane shows the drop overlay: the window scrim, then an apricot card (`drop-overlay.*`, `accent.fill`, `text.on-accent`) with a dashed inner outline marking the drop zone, the `file-arrow-up` glyph, "Drop to attach to #general" (or "@Ana") and "You can add a message before sending." It is the screen's one loud moment, as Discord's blurple drop card is. It never takes focus and has no controls, so it cannot trap the keyboard; the attach button does the same job. Without "Attach files", the pointer shows that dropping is refused and no overlay appears. A file dropped elsewhere in the window is refused rather than opened by the browser, which would leave Mellow.
  - Pasting files into the field attaches them; pasted text stays text.
  - Each file uploads at once into the tray, inside the composer box above the field, separated by `composer.tray-divider` and scrolling sideways when full. Items (`composer.tray-item-*`, 160 px, `surface.sidebar`): a 4:3 preview (the image, contained, or the apricot file glyph), the name middle-truncated, then "Uploading…" (preview at half opacity) or the size once uploaded. A failed upload keeps its item with a 1 px `text.danger` outline, a warning icon and the reason; it is not sent. Each item has a Remove button (trash, the hover-bar button) labelled "Remove photo.png"; removing returns focus to the field. Upload results are announced politely ("photo.png uploaded.", or the error).
  - At most 10 files per message; extra files are not added and "You can attach up to 10 files to one message." appears in the action error line.
  - Send works with text, files, or both. While a file is still uploading, Enter does not send and the error line says "Your files are still uploading. Send again once they're done."; the phone send button is dimmed. Sending clears the tray; failed items are dropped with it.
  - Each channel keeps its own tray, like its draft.
- Length: a counter appears at 90 % of the instance limit ("3,620 / 4,000"); beyond the limit it turns `text.danger`, sending is blocked and the field's accessible description says "Message is 120 characters too long."
- Typing indicator: in its 24 px row above the composer (above the reply bar when there is one), `type.typing`, `text.secondary`, with three dots: "Ana is typing…", "Ana and Léo are typing…", "Ana, Léo and 2 others are typing…", "Several people are typing…" (beyond 5); in French « Ana écrit… », « Ana et Léo écrivent… », « Ana, Léo et 2 autres écrivent… », « Plusieurs personnes écrivent… ». The row keeps its height when empty, so the composer never jumps. It is not announced by screen readers.
- Read-only channel (no "Send messages"): the field is replaced, as on Discord, by "You do not have permission to send messages in this channel." in `component.composer.locked-text` (`text.muted`, 4.80 dark / 5.51 light on `surface.input`) on the same `surface.input` field, same size and position, without the attach button. It is plain text for screen readers, never focused; nothing that would focus the composer (a reply starting, "Jump to present", "Try again", closing a dialog) moves focus there. The reply bar never shows, and `Up` to edit is unavailable since it lives in the composer. At narrow widths the sentence wraps and the field grows to fit it.
- Offline: the composer stays usable; sent messages are queued (5.4).
- Draft: unsent text and reply state are kept per channel across channel switches and app restarts.

### 5.6 Conversations (Messages)

- Home shows conversations in the channel-sidebar column, as on Discord: a "Find or start a conversation" button at the top (opens the quick switcher), then a "Direct messages" category header with a `+` button ("New message"), then conversations sorted by last activity, each a 42 px row with avatar and status dot (group: stacked avatars), name, and the instance label when ≥ 2 instances are connected. Unread conversations use `text.heading` and also appear as avatars at the top of the rail.
- New message: the people picker of 5.6.2 (web, ADR-0026). It lists your friends only (a group is made of friends of whoever adds them), up to 9 others for a group. Conversations cannot mix instances; once several instances are connected, the picker explains: "People from different instances can't be in the same conversation yet."
- Group conversation (web, 5.6.2): its own name, or its people's names until renamed; the group avatar and the member count in its row and the chat header; "Add friends to the group" and the member toggle in the header; "Add friends", "Rename group" and "Leave group" in the members side sheet.
- Empty: "No conversations yet. Start one with someone from your servers." with "New message".
- Desktop, until "New message" and the quick switcher exist: the header reads "Direct messages" as plain text (no menu), the list starts with the "Direct messages" category header (no `+`), rows share the channel rows' colours, unread pill and selection, and the empty state has no button, since conversations start from a member's profile popout. In a conversation the chat header shows the `at` icon and the name, the hero shows the name as its title and "This is the beginning of your direct message history with @Ana.", errors read "Couldn't load messages with @Ana.", and the member list and its toggle are hidden.
- Someone you blocked (web, ADR-0025): the conversation stays readable and the composer is locked like a read-only channel's, saying "You blocked Ana. Unblock them to send messages." (« Tu as bloqué Ana. Débloque cette personne pour lui écrire. ») with a secondary "Unblock" button; once unblocked, the composer comes back with focus. When the other person blocked you, nothing changes on your side: sending fails with the usual refusal, so the block is never revealed.
- Error and offline: as in 5.4.

### 5.6.1 Friends (web, ADR-0025)

Home with no conversation open is the Friends space, in the sheet. It is reached from the Home button and from the **Friends** row at the top of the conversation list (`users` icon in the avatar column, `type.channel-name`, the red badge of 2.3 counting requests received; accessible name "Friends, 2 friend requests"). The tab chosen is kept while the tab of the browser lives.

```
wide (sheet)                                                      phone (full bleed)
┌───────────────────────────────────────────────────────────┐   ┌──────────────────────────┐
│ [u] Friends | [Online] All  Pending (2)  Blocked  {Add}   │   │ = [u] Amis               │
│                                                           │   │ [En ligne] Tous  Att.(2) │
│ Online                                                3   │   │ Bloqués {Ajouter un ami} │
│ (A*) Ana                                      (m)  (...)  │   │ En ligne              3  │
│      Online                                               │   │ (A*) Ana      (m) (...)  │
│ (B-) Bob                                      (m)  (...)  │   │      En ligne            │
│      In a meeting until 15:00                             │   └──────────────────────────┘
└───────────────────────────────────────────────────────────┘
[u] users icon   (2) red badge   {…} apricot Add friend tab   (m) Message   (...) More
```

- **Header**: `users` and "Friends" (`type.heading-md`) as the chat header's name, a divider, then the tabs (`role="tablist"`, arrows, Home and End move and choose). On phones the tabs wrap onto their own rows under the title instead of hiding off the edge. Tabs are 32 px pills (`component.friends.tab-*`, 44 px under a coarse pointer); the selected one takes `state.selected`. **Add friend** is filled apricot (`add-tab-*`) until it is open, then becomes apricot text, so the Send button below is the screen's only apricot fill (1.2).
- **Rows** (`component.friends.row-*`, 60 px): avatar 32 with the status shape for friends online (offline friends dimmed, as in 5.7), the name in `text.heading`, then a detail line in `text.muted`: the custom status or the status name for friends, "Incoming friend request", "Outgoing friend request" or "Blocked". Round 36 px actions on `surface.input` (44 px under a coarse pointer), each named with the person ("Message Ana", "Accept Ana's request"): Message and More for friends; Accept (turns green on hover) and Ignore (red) for requests received; Cancel for requests sent; a secondary "Unblock" button for blocks. Groups use the member list's headers: "Online 3", "All friends 4", "Received 2" then "Sent 1", "Blocked 1".
- **More** opens a menu under the button (6.33 keyboard): Remove friend (`user-minus`) and Block (`prohibit`), both danger. Each opens the destructive dialog of 8: "Remove Ana from your friends?" / "Ana won't be in your friends anymore. To be friends again, one of you sends a new request." / "Remove friend"; "Block Ana?" / "Ana won't be told. You won't be able to message each other or send friend requests, their messages in servers will be hidden, and they'll leave your friends." / "Block Ana". Cancel has focus.
- **Results** are read out by a polite live region ("You're now friends with Emma.", "Request to Hugo cancelled.", "Gabin was unblocked."); when the focused row leaves, by your action, a confirmation or someone else's (a request cancelled, a friend going offline under Online), focus moves to the row now in its place, else the selected tab; focus you moved elsewhere stays. Rows wait for the instance independently. A removal the instance no longer finds (already done elsewhere) counts as done. The More menu and a Remove or Block confirmation close by themselves if that person stops being a friend meanwhile. Arrow keys move along the tabs without leaving them; only a click, Enter or Space on Add friend moves focus into its field. A refusal shows above the list in `text.danger` with `warning-circle`, in an alert: "Couldn't accept Emma's request: too many attempts. Wait a moment and try again."
- **Add friend**: "Add friend" (`type.heading-lg`), "You can add friends with their username on chat.example.org.", then one bordered field with "Send friend request" (primary) inside it, as on Discord; under `component.friends.username-min-width` the button wraps under the name and spans the field. The answer appears under the field with a check in `text.positive` or `warning-circle` in `text.danger`, in a polite live region: "Friend request sent to Ana.", "Ana had asked you too: you're now friends.", "You already asked Ana. The request is waiting for their answer.", "You're already friends with Ana.". A missing account, yourself and someone who blocked you (or whom you blocked) all read "Hm, that didn't work. Check the username and try again."; RATE_LIMITED reads "You've sent a lot of requests. Wait a moment, then try again."; LIMIT_REACHED "You have as many friends and sent requests as an account can. Cancel a request or remove a friend to add someone new."
- **Empty tabs**: Melli at `component.friends.empty-melli-size` above one sentence (`type.body-lg`, `text.secondary`). No friends at all (Online and All): idle Melli, "No friends yet. Add someone by their username and they'll show up here.", "Add friend" (primary) and, when you are in no server, "Add a server". Online with friends all offline: Melli `sleeping`, "None of your friends are around right now." Pending: "No pending requests. Requests you send or receive wait here." Blocked: "You haven't blocked anyone."
- **Motion**: changing tab gives the panel the channel switch's small rise (`mw-rise`, `motion.duration.base`); nothing else moves except Melli. Reduced motion: at once, and Melli still.
- **Loading and offline**: the space appears with the first `Ready`, so it is never half loaded; before it, the sheet says "Loading your servers and conversations…". While reconnecting, the lists keep the last known state and actions answer with the network reason.
- French: « Amis », « En ligne », « Tous », « En attente », « Bloqués », « Ajouter un ami », « Envoyer la demande d’ami », « Retirer l’ami », « Bloquer », « Débloquer », « Hum, ça n’a pas marché. Vérifie le nom d’utilisateur et réessaie. ».

### 5.6.2 Group conversations and the people picker (web, ADR-0026)

A group holds up to 10 people, you included. Anyone in it adds their own friends, renames it and leaves; only its owner removes others; when the owner leaves, the person with the lowest id owns it. Everyone writes, whatever their relationships: someone you blocked stays, and their messages fold as in servers (5.4).

```
navigator row                       chat header (wide)
┌──────────────────────────────┐    ┌────────────────────────────────────────────────────────────┐
│ (A(B) Week-end à Lyon        │    │ (A(B) Week-end à Lyon | 5 members   [Go to… Ctrl K] (p)(+u)(uu) │
│       5 members          (3) │    └────────────────────────────────────────────────────────────┘
└──────────────────────────────┘
(A(B) two faces, the front one cut out of the surface   (+u) Add friends to the group   (uu) members

people picker (dialog)                              members side sheet
┌──────────────────────────────────────┐            ┌──────────────────────────────┐
│ New message                          │            │ Members                    x │
│ Pick one friend to message them, or  │            │ Add friends            [+u]  │  apricot
│ several to start a group.            │            │ Rename group            [/]  │
│ Search your friends                  │            │ Leave group            [->]  │  red
│ ┌──────────────────────────────────┐ │            │ Members                    5 │
│ │ [(C) Chloé x] [(A) Ana x] A frie…│ │            │ (A*) Ana                [-u] │  owner only
│ └──────────────────────────────────┘ │            │ (P*) Paul                    │
│ You can add 7 more friends.          │            │      Owner                   │
│ (A) Ana   ana                   [x]  │            └──────────────────────────────┘
│ (B) Bob   bob                   [ ]  │
│ Group name (optional)                │
│ [ Chloé and Ana                    ] │
│                  Cancel [Create group]│
└──────────────────────────────────────┘
```

- **Entry points**: the `+` beside "Direct messages" in the navigator at Home ("New message", 44 px under a coarse pointer); in a group, "Add friends to the group" in the chat header (`user-plus`) and "Add friends" in its side sheet. Groups are also found by name, or by anyone in them, in the quick switcher (5.10), labelled "Group".
- **Picker**: one dialog (6.35) for both jobs. Title "New message" with "Pick one friend to message them, or several to start a group.", or "Add friends to Week-end à Lyon" with "They'll see the whole conversation, older messages included." (the newcomer reads the history, ADR-0026). A labelled search field ("Search your friends") holds the chips of the friends picked (`component.people-picker.chip-*`, each a button "Remove Ana"); it finds display names and usernames, ignoring case and accents; Enter after typing picks the first match and clears the search, Enter with nothing typed does the dialog's action, Backspace in an empty search takes back the last chip. Under it, in a polite live region: "You can add 9 more friends." (counting down; "That's as many people as a group can hold." at 0; checkboxes not picked are disabled then). The list (48 px rows, avatar, name, username, a native checkbox in `accent.indicator`) scrolls past 320 px. With 2 or more picked, "Group name (optional)" appears with the names it would show as its placeholder and "Leave it empty to show everyone's names." The action reads "Message" for one, "Create group" for several, "Add to group" in a group ("Opening…", "Creating…", "Adding…" while busy); it stays dimmed until someone is picked.
- **Picker states**: no friends at all: "You don't have friends to message yet. Add a friend by their username first." with "Add friend", which opens Friends on its Add friend tab; every friend already in the group: "All your friends are already in this group."; no match: "No friends match “zed”."; someone who stops being a friend meanwhile leaves the list and the chips by themselves.
- **Refusals**: creating answers NOT_FOUND when someone picked is no longer a friend or a block stands either way; the sentence never tells which: "Couldn't create the group: one of these people can't join it. Check your friends list and try again." Adding: "Couldn't add Ana to the group: they can't join this group. Check your friends list and try again.", or "…: the group is full." Adding several runs one request per person in the order picked; on a refusal the ones added are announced ("Ana and Bob joined the group.") and the rest stay picked.
- **Rows and header**: the group avatar (`component.group-avatar.*`): the first two other people at `face-scale` of the avatar, the second in front at the bottom trailing corner with a `ring-width` cut-out in the surface's colour; one face when one other is left; the `users` glyph on `neutral.fill` when you are alone. No presence on a group. Second line "4 members" (`member-count-text`, `type.body-sm`); accessible name "Week-end à Lyon, group, 5 members", then the unread state and mentions as for any conversation. Unnamed groups read "Ana and Bob", "Ana, Bob and Chloé", then "Ana, Bob and 2 others". The chat header shows the group avatar at 32 px, the name, and in the topic's place "5 members" (wide windows only). The hero uses the `users` glyph: "This is the beginning of your group conversation with Ana, Bob and 2 others."; the composer reads "Message Week-end à Lyon".
- **Side sheet**: the same card, place and behaviour as the member list (5.7: docked from 1064 px, an overlay below, `Ctrl+U`, Esc, remembered per group). Actions first, worded like the server menu: "Add friends" (`user-plus`, `text.accent`), "Rename group" (`pencil-simple`), "Leave group" (`sign-out`, `text.danger`). Then "Members 5" and everyone, you included, by name: presence on avatars as in 5.7, the owner's second line "Owner" (said in words, never by a mark alone), else the custom status. Accessible name "Ana, online, owner, custom status: …". For the owner, each other row ends with a round "Remove Ana from the group" (`user-minus`, turns red on hover).
- **Rename**: a dialog "Rename group", field "Group name" filled with the current name, the people's names as placeholder, "Leave it empty to show everyone's names.", "Save name". Result read out: "Group renamed to Board games." or "The group shows everyone's names again."
- **Leave and remove**: the destructive dialog of 8, Cancel focused. "Leave Week-end à Lyon?" / "You won't see this group or its messages anymore. Someone in it can add you back later."; for the owner "…, and Ana will own it. …"; for the last person "You're the last one here: the group and its messages will be deleted for good." / "Leave group". "Remove Bob from the group?" / "Bob won't see this group or its messages anymore. Someone in it can add them back later." / "Remove Bob"; "Bob was removed from the group." is read out and focus moves to the next row's Remove button.
- **Closing under you**: leaving, or being removed by the owner while the group is open, returns to Friends; the shell's polite live region says "You left the group “Week-end à Lyon”." or "You're no longer in the group “Week-end à Lyon”.", and focus that fell with the conversation goes to Friends' selected tab. Removed from a group that is not open: it leaves the list quietly.
- French: « Nouveau message », « Choisis un ami pour lui écrire, ou plusieurs pour créer un groupe. », « Chercher parmi tes amis », « Tu peux encore ajouter 7 amis. », « Nom du groupe (facultatif) », « Écrire », « Créer le groupe », « Ajouter au groupe », « Ana, Bob et 2 autres », « 5 membres », « Propriétaire », « Ajouter des amis », « Renommer le groupe », « Quitter le groupe », « Retirer Bob du groupe ? », « Tu ne fais plus partie du groupe « Week-end à Lyon ». ».

### 5.7 Member list and profile (the side sheet)

The member list lives in the **side sheet** (`component.side-sheet.*`): a card on `surface.navigator` with radius 16, opened by the member button in the chat header or `Ctrl+U`, closed by the same button, its own close button (« Fermer ») or `Esc` when focus is inside it. It is closed by default and remembered per server. From 1064 px it docks beside the sheet and the sheet narrows; below, it slides over the sheet's trailing side with a scrim (focus moves into it and returns to the member button on close). Choosing a member shows their profile in the same side sheet, with a back button (« Retour aux membres »); the profile popout of the classic layout is not used. Later, threads open in the side sheet too; pinned messages open in their own panel under the header's pin button (6.53), so the member list stays where it is. It slides in from the trailing edge over `motion.duration.slow` with the enter curve; reduced motion makes it appear at once.


- Groups: hoisted roles in role order, then "Online", then "Offline", in `type.category` (sentence case), `text.muted`, 40 px headers, with the member count as a separate tabular number at the trailing end ("Moderators" … "2"), not joined with a dash. Accessible name: "Moderators, 2 members".
- Row (42 px): avatar 32 with status dot, display name (`type.member-name`, role colour adjusted, otherwise `text.secondary`; `text.normal` on hover), activity or custom status on a second line in `type.body-sm`, `text.muted`.
- Offline members: avatar at 50 % opacity, text at full contrast (Discord dims the whole row, which fails AA for the name).
- Accessible name: "Ana, online, Moderator, custom status: Reviewing PRs". Web (v0.19): the custom status line is truncated with an ellipsis; offline rows show no shape (their dimmed avatar says it), online rows show online, idle or do not disturb.
- Large servers: offline members are not listed above 1,000 members ("Offline members are hidden in large servers.").
- Loading: row skeletons. Error: "Couldn't load members." with "Try again".
- Desktop, first version: click, `Enter` or `Space` on a member row opens a reduced popout to the leading side of the member list, top-aligned with the row (`component.profile-popover.*`): banner strip, avatar 80, display name, and "Message" (primary, full width, focused on open; accessible label "Message Ana") for anyone but yourself. "Message" opens or creates the conversation and moves focus to its composer. `Esc` or a click outside closes it, and the row shows `state.selected` while it is open.
- Avatars and banners (ADR-0021, web first): the popout's strip is the banner picture at `component.profile-popover.banner-image-height` (120 px, Discord's 5:2 banner across the 300 px card, cropped to fill), or the plain `banner-height` strip without one. Banner and avatar play animated here, as on Discord, except under reduced motion. The popout follows `USER_UPDATE`, so a picture changed meanwhile shows at once. The card itself (`ProfileCard` on the web) is reused as the settings preview (5.9).
- Friends and blocks in the profile (web, ADR-0025): under "Send message", full-width secondary buttons stacked (the side sheet is too narrow for two French labels side by side): "Add friend" with no relationship, "Accept friend request" for a request received, a dimmed "Friend request sent" for one sent, nothing more for a friend; then "Block", which opens the confirmation of 5.6.1. For someone you blocked, "Send message" gives way to "Unblock". The button pressed is replaced when the relationship changes, so focus moves on: to Block after Add friend or Accept, to Send message after Unblock, to Unblock after a confirmed Block, else to the back button. Results are read out politely; refusals use the profile's error line.
- Profile popover (click, `Enter`, or context menu → "Profile"): 300 px, `surface.popover`, banner strip in the user's accent or a default grey, avatar 80, display name (`type.heading-md`), `@username@instance`, about text, roles as chips, member since, actions "Message" (primary), "Mention", and for moderators a "Moderation" menu (Kick, Ban, Manage roles).

### 5.8 Server settings

Discord's full-screen settings layout: navigation on `surface.sidebar` (right-aligned column on wide windows), content on `surface.chat` capped at 740 px, a close button with the label "Esc" at the trailing top. Sections: Overview, Roles, Members, Channels, Invites, Bans, Moderation log, then "Delete server" (owner only, `text.danger`, last).

**Unsaved changes**: a bar slides up at the bottom of the content (`surface.floating`, `elevation.high`): "Careful — you have unsaved changes." with "Reset" and "Save changes". Leaving with unsaved changes shakes the bar (instant under reduced motion) and turns it red-bordered, as on Discord, and the bar's text becomes "Save or reset your changes before leaving." The dialog alternative is used on Android: "Leave without saving? Your changes to Roles will be lost." with "Keep editing" and "Leave without saving".

**Roles editor**:

- Left: role list in hierarchy order, each with colour dot and member count. Reorder by drag and drop, `Ctrl+Shift+Up/Down`, or "Move up"/"Move down". Roles above your highest role are shown but locked, with the reason in the tooltip and accessible description: "You can only edit roles below your highest role."
- Right: tabs "Display", "Permissions", "Members".
- Display: name, colour picker (with the readability preview on both themes), "Show separately in the member list", "Allow anyone to mention this role".
- Permissions: grouped (General, Membership, Text channels, Moderation, Advanced). Each permission is a switch row with a plain-language description, for example "Ban members — Remove people and stop them from rejoining." Sensitive permissions (Administrator, Ban members, Manage roles) carry a warning icon and the word "Sensitive".
- Channel overrides (in channel settings): each permission is a three-state control "Deny / Inherit / Allow" (Discord's order) with icons (cross, dash, check) and text labels, never colours alone: the chosen option is filled (`state.selected`, label in `text.heading`) and only its glyph takes its colour (`text.danger`, `text.heading`, `text.positive`, each at least 3:1 on the fill).

**Channel settings, web (2026-10-22)**: Discord's "Edit channel", in the server settings' sheet, for text channels: Overview (icon, name and topic; the name is lowercased with dashes, as the instance stores it) with `MANAGE_CHANNELS` in the channel, and Permissions with `MANAGE_ROLES` in the channel. Permissions lists the roles with an overwrite (highest first), `@everyone` (always), then members; "Add a role or member" adds one at once, all Inherit; "Remove" deletes its overwrite at once; state changes wait for the unsaved-changes bar. Roles at or above the reader's highest are shown locked with the reason, and options the reader does not hold in the channel are locked with one sentence saying so. Opened from a gear on the channel row and in the chat header. Details: [design/web-channel-settings.md](docs/design/web-channel-settings.md).
- Empty (only @everyone): "Roles let you group people and give them permissions." with "Create a role".

**Desktop, first slice (2026-10-21)**: the server header turns into a menu button (chevron) only for members who can open settings; Roles is the only section. `@everyone` hides the colour picker and "Delete role", and its name field is read-only ("@everyone can't be renamed."). A locked role shows a lock and "You can only edit roles below your highest role." above the tabs. With only `@everyone`, the empty-state text and "Create a role" sit above the list, and `@everyone` stays editable below. Colours come from presets only, as radio buttons marked by a check and a ring.

**Web (2026-10-22)**: every section but Channels, with the user settings' layout (5.9). Lists are rows (avatar, name, a muted second line, one action) rather than tables, so they read the same on phones; roles are given from a member's "Manage" dialog as switches, not with chip × buttons (too small a target); the moderation log reads as sentences in the reader's language. Details: [design/web-server-settings.md](docs/design/web-server-settings.md).

**Invites**: table of active invites (code in mono, created by, uses / maximum, expires). "Create invite" dialog: "Expire after" (30 minutes, 1 hour, 1 day, 7 days, Never), "Maximum uses" (No limit, 1, 5, 10, 25, 100), result with the full link and "Copy" (confirmation "Copied"). Revoking asks for confirmation (section 8). Empty: "No active invites." with "Create invite".

**Bans**: searchable list (avatar, name, reason, banned by, date). Action "Revoke ban". Empty: "No one is banned from Rust Lyon."

**Kick and ban dialogs**: see section 8.

### 5.9 User settings

Same full-screen layout as server settings. Sections: My account (per instance), Profile, Appearance, Accessibility, Notifications, Instances, Keybinds (desktop), Language, About.

- My account: display name, username, email, password, active sessions with "Sign out of this session", and "Delete account" in a separate danger zone.
- Profile: avatar upload with crop (circle preview), about text with counter.
- Appearance: Theme (Match system, Dark, Light), Message display (Cozy, Compact), Chat font size (12–24 px), Space between message groups (0, 4, 8, 16, 24 px), Zoom level (50–200 %), "Show avatars in compact mode", with a live message preview, as on Discord.
- Accessibility: Reduce motion (Match system, Reduce motion, Allow motion), Play animated images (On hover or focus, Always, Never), Always underline links, Show send button, "Announce new messages in the open channel" (screen readers), Role colours (Show on names, Show next to names, Don't show).
- Notifications: per instance, defaults for new servers ("Only @mentions" by default), sounds, lock-screen previews (Android).
- Instances: list with status (Connected, Reconnecting, Signed out), "Sign out", "Connect another instance".
- About: version, open-source licences (including Twemoji attribution), link to the source code (AGPL-3.0 requires offering it), instance versions.

Desktop, first iteration (`UserSettingsScreen` in `user_settings.slint`, `component.settings.*`): the screen covers the whole window and hides the shell, so `Tab` stays inside it and screen readers do not reach the panes behind. The navigation column (218 px, 60 px top padding, 32 px items with a 2 px gap, `type.channel-name`) sits right-aligned in a `surface.sidebar` area that grows with the window; the content area grows too, and the page is capped at 740 px with 60 px top and 40 px side padding (16 px below 792 px). The close button (36 px circle, 2 px `text.secondary` border, 18 px `x` icon) with "Esc" under it sits 24 px after the page (8 px below 792 px). Navigation: section label "App settings" (`type.category`, sentence case), "About" (selected until My account arrived, below), a separator, then "Sign out" (danger, last), which signs out at once and closes the screen, as the old menu item did. "Sign out" fills `danger.fill` with `text.on-danger` on hover, like danger menu items, because `text.danger` on `state.hover` is 4.34:1 in the light theme; keyboard focus shows only the ring.

Keyboard and screen readers: focus moves to the selected section ("My account") when the screen opens; `Esc` closes it from anywhere inside (after closing an open My account form). The navigation is a navigation landmark labelled "User settings" holding one tab list per group ("User settings", "App settings"); the page is a tab panel labelled with the section name; the close button is labelled "Close settings" with the description "Esc". Update status changes are announced through a polite live region that exists from the start.

About page: the page title "About" (`type.heading-lg`), then "Mellow 0.1.0" (`type.heading-md`, just "Mellow" while the version is unknown), then the "Updates" section (label in `type.category`, sentence in `type.body-lg`, one Primary button 16 px under it):

| Update status | Sentence | Button |
|---|---|---|
| Unknown (no check yet) | — | "Check for updates" |
| Up to date | "Mellow is up to date." | "Check for updates" |
| Checking | "Checking for updates…" | "Check for updates", disabled |
| Available | "Mellow 0.2.0 is available." | "Update Mellow" |
| Downloading | "Downloading Mellow 0.2.0…" | "Update Mellow", disabled |
| Ready to restart | "Mellow 0.2.0 is ready. Restart to finish updating." | "Restart Mellow" |
| Failed | The error from the app, after a `warning-circle` icon in `text.danger`, for example "Couldn't reach the update server. Check your connection and try again." | "Try again" (checks again) |
| Unsupported | The reason from the app, for example "Mellow can't replace itself in this folder. Download the new version from github.com/Paul-Carouge/mellow/releases." | None |

The button stays in place while busy, so keyboard focus is not lost; when the status becomes unsupported and the button disappears, focus moves to the screen itself so `Esc` still works. Still to come on this page: licences and the source code link (both required above).

**My account, first iteration (2026-10-21, desktop and web)**: the default section, under the navigation label "User settings"; About moves under "App settings", with a separator between the groups and before "Sign out". The navigation is otherwise unchanged.

```
My account                                         (x)
┌──────────────────────────────────────────────┐   Esc
│ DISPLAY NAME                          [Edit] │
│ Paul Carouge                                 │
│ ✓ Display name saved.                        │
│ USERNAME                              [Edit] │
│ @paul                                        │
│ EMAIL                                        │
│ paul@example.org                             │
└──────────────────────────────────────────────┘
PASSWORD
[Change password]

Editing one field (the row turns into its form):
┌──────────────────────────────────────────────┐
│ USERNAME                                     │
│ [paul                                      ] │
│ 2 to 32 characters: lowercase letters, …     │
│ CURRENT PASSWORD                             │
│ [••••••••                               (o)] │
│ (!) Couldn't change the username: …          │
│                          [Cancel] [Save]     │
└──────────────────────────────────────────────┘
```

- The card is `component.settings.account-card-*`: `surface.sidebar` on the `surface.chat` page, `radius.md`, 16 px padding, 24 px between rows. Each row is a sentence-case label (`type.category`, `text.muted`) over its value (`type.body-lg`, `text.normal`), with a Secondary "Edit" button at the trailing end, as on Discord. The username shows as `@paul`. The email is read-only (no Edit button): changing it needs a verification flow that does not exist yet.
- "Edit" turns the row into its form in place, prefilled: Display name (one field, helper "Up to 32 characters. This is the name people see in chat.", Save); Username (the username, helper "2 to 32 characters: lowercase letters, digits, dots and underscores.", then "Current password", as Discord asks for it; Save). "Change password" (Primary) turns into a card with "Current password", "New password" (helper "At least 10 characters.") and "Confirm new password" (helper "Doesn't match the new password." while it differs), then Cancel and "Change password". Password fields have the show/hide toggle; on the web they use `autocomplete="current-password"` / `"new-password"`.
- One form is open at a time: opening another discards the first. Save stays disabled until the form can succeed by the client's checks (display name not empty and changed; username changed and a password typed; new password of at least 10 characters, confirmation equal); the helpers say why. `Enter` in a field saves when Save is enabled.
- Busy: fields turn read-only (dimmed on desktop), Cancel and every Edit button are disabled, and Save reads "Saving…" ("Changing password…"); disabled buttons stay focusable.
- Error: under the form that raised it, after a `warning-circle` icon, in `component.settings.error-text` (`text.danger`), announced assertively (desktop live region, web `role="alert"`). An error left from an earlier attempt never shows under another form.
- Success: the form closes and forgets its fields (passwords are never kept), focus returns to the row's Edit button (or "Change password"), and the result sentence appears under that row after a `check` icon in `component.settings.notice-text` (`text.positive`), announced politely: "Display name saved.", "Username saved.", "Password changed. Your other devices are signed out." (desktop shows the text Rust provides).
- `Esc` closes the open form (not while busy) and returns focus to its Edit button; with no form open it closes the settings. Switching sections discards an open form.
- Desktop: the page scrolls (an open password form is taller than short windows); the tab panel is labelled "My account". Contract: `account-display-name`, `account-username` (without `@`), `account-email`, `is-account-busy`, `account-error`, `account-notice`, `account-saved-count` (Rust increments it after each success) and the callbacks `save-display-name`, `save-username(username, current password)`, `change-password(current, new)`.
- Web: a full-screen native modal `<dialog>` labelled "User settings" over the shell, opened from the user panel's gear (`gear-six`) button; focus starts on the selected section and returns to the gear on close. Navigation items are buttons with `aria-current="page"`, 32 px (44 px under a coarse pointer); the close button is a 44 px target holding the 36 px circle. Below 792 px the navigation is its own page ("User settings" title, close button), and choosing a section opens it with a "Back to user settings" button; focus goes to the section, then back to its item. About on the web: "Mellow web client", then "Source code": "Mellow is free software under the AGPL-3.0 licence. You can read, change and share its code." and the link "Source code on GitHub" (new tab).

**Profile, first iteration (2026-10-21, web; ADR-0021)**: under "User settings", after My account. No crop step: the instance resizes the picture and the avatar is cropped to its circle when shown (a crop dialog stays in the plan above).

```
Profile                                                  (x)
Pictures can be PNG, JPEG, GIF or WebP, up to 8 MB.      Esc
Animated GIFs stay animated.

AVATAR                               PREVIEW
[Change avatar] [Remove avatar]      ┌──────────────────────┐
✓ Avatar saved.                      │ banner picture 120   │
                                     │ (avatar 80)          │
PROFILE BANNER                       │ Paul Carouge         │
A plain strip shows until you add    │ @paul                │
a picture.                           └──────────────────────┘
[Change banner]
```

- Two columns that start at the card's width (`component.profile-popover.width`) with `component.settings.section-gap` between them, so the preview wraps under the editors on narrow pages. The preview is the profile card with the popover's 1 px border, the name as plain text (the page keeps its headings), always animated except under reduced motion.
- Each editor: label (`type.category`), the empty sentence when there is no picture ("Your initial shows until you add a picture.", "A plain strip shows until you add a picture."), then "Change avatar" (Primary, opens the file picker, accepting `image/png`, `image/jpeg`, `image/gif`, `image/webp`, described by the formats sentence under the title) and "Remove avatar" (Secondary, only when there is one). Same for the banner.
- Checked before sending: over 8 MB, "Couldn't change the avatar: the picture is larger than 8 MB. Choose a smaller one."; another type, "…: choose a PNG, JPEG, GIF or WebP picture."
- Busy (one change at a time, as the instance rate-limits them): every button disables but stays focusable, and the pressed one reads "Uploading avatar…" or "Removing avatar…"; the instance compresses before answering (a second or two for a large GIF), so this is a labelled wait, not a percentage: the browser cannot measure the processing.
- Error: under its editor after `warning-circle`, `role="alert"`: an unreadable picture or one over the limits, "Couldn't change the avatar: the picture couldn't be read, or it's over 8192 pixels a side or 600 frames. Choose another one."; otherwise the shared reasons (busy instance, too many changes, network).
- Success: the preview, the user panel and every message and member row update from the answer (and other people's clients from `USER_UPDATE`); "Avatar saved." / "Avatar removed." / "Banner saved." / "Banner removed." after a `check` icon, announced politely. After a removal focus moves to "Change …", since "Remove …" is gone.

### 5.10 Quick switcher

`Ctrl+K`, the "Aller à… Ctrl K" button in the chat header ("Go to… Ctrl K"), or "Find or start a conversation" opens the quick switcher, Mellow's fastest way around (« Où veux-tu aller ? »): a 570 px dialog on `surface.popover`, `elevation.high`, with a large input ("Where would you like to go?", `type.heading-lg`) and results grouped by type (conversations, channels, servers), each with an icon, the server or instance name in `text.muted` and an unread or mention indicator. Prefixes filter as on Discord: `@` people, `#` text channels, `*` servers. Arrows move, `Enter` opens, `Esc` closes. Empty: "No results for 'anx'. Check the spelling or search another server." The result list is a listbox; the count is announced politely ("5 results").

### 5.11 Admin web dashboard

- Overview: health card (status in words: "Healthy", "Degraded: database is slow", "Down: storage unavailable"), version and update notice, uptime, users (total, active in the last 7 days), servers, storage used and limit with a plain bar, push gateway status, active connections.
- Users: searchable table (username, display name, email if any, created, last seen, status). Actions: "Suspend", "Reset password link", "Delete account" (typed confirmation).
- Servers: table (name, owner, members, created). Actions: "View", "Transfer ownership", "Delete server".
- Invites and registration: registration mode, instance-wide invite codes.
- Storage and backups: usage by type, upload size limit, last backup time and result, "Run backup now".
- Updates: current and latest version, release notes link, update status.
- Settings: instance name, description, icon, rules text, email, push gateway.
- States: htmx swaps use static table-row skeletons; empty tables say what would appear and how ("No users yet. Share an invite to let people sign up."); errors render inline above the table with "Try again"; a lost connection shows a warning notice bar (`warning.fill`) "Can't reach the server. Data on this page may be out of date."

---

## 6. Components inventory

All components use tokens only (component tier first), support light and dark themes, both message densities where relevant, zoom and text scaling to 200 %, keyboard focus and screen readers. Desktop components live in `crates/mellow-desktop`; Android components live in `:core:designsystem`; admin web components are HTML partials with CSS classes.

| # | Component | Variants | States | Notes |
|---|---|---|---|---|
| 6.1 | Button | Primary (`accent.fill`), Secondary (`neutral.fill`), Danger (`danger.fill`), Link (text only, `text.link`) · sizes 40 (default), 32 (small), 44 (large and web), 48 on Android | Default, hover, pressed, focus, disabled, loading (label kept, progress inline) | `radius.sm`, `type.button`. One primary button per view. Labels are verbs ("Save changes"). Desktop: disabled and loading buttons dim their fill and label to `button.disabled-opacity` (the focus ring keeps full contrast) and stay focusable, so focus is not lost while an action runs. |
| 6.2 | IconButton | Standard, Toggle, Danger | Same as Button, plus selected | `text.secondary` icon, `state.hover` background; always a tooltip and an accessible label; target ≥ `size.hit.*`. |
| 6.3 | TextField | Single line, Multiline, Password (show/hide), Search (with clear) | Default, hover, focus, filled, error (message below, `text.danger` icon and text), disabled, read-only | `surface.field` with 1 px `border.strong`, `radius.sm`; sentence-case label above, never placeholder-only. |
| 6.4 | Select | Single select | Closed, open, focus, error, disabled | Options use the ContextMenu visuals. |
| 6.5 | Checkbox, Switch, Radio group | — | Off, on, mixed (checkbox), focus, disabled | Switch on: `positive.fill` track with a check glyph, as on Discord; off: `neutral.fill` with a cross glyph, so state never relies on colour. |
| 6.6 | SegmentedControl | 2–4 options; tri-state permission (Deny / Inherit / Allow) | Selected, focus, disabled | Icon + text in each segment. Web: native radios in a `radiogroup` named by the permission, 32 px high (44 px under a coarse pointer), `component.channel-settings.override-*`. |
| 6.7 | Slider | Continuous, Stepped (chat font size 12–24, zoom 50–200 %) | Focus, dragging, disabled | Arrow keys step; value announced. |
| 6.8 | Avatar | Sizes 16, 24, 32, 40, 80 · image, initials fallback, group (stacked; web: `GroupAvatar.svelte`, 5.6.2) | Loading (tint block), with status, animated (play on hover/focus) | Always a circle. Web: the initial shows until the picture has loaded and again if it fails; an animated picture loads its still URL (`/profile-images/{id}/still`) and switches to the animated one while the enclosing row (message, member or conversation row; otherwise the avatar) is hovered or holds focus, and always in the profile card; never under `prefers-reduced-motion: reduce`. Decorative (`alt=""`): the name is always next to it. |
| 6.9 | StatusIndicator | Online, idle, dnd, offline, invisible (drawn as offline) | — | Shape + colour; cut-out ring of the surface below; inner shapes are holes (2.3). Sizes `component.status.size-small` (avatars), `size-large`, `size-menu` (status menu). Web: `StatusIndicator.svelte`, decorative, its host names the status. |
| 6.10 | Skeleton | Text line, avatar, message, row, server icon, table row | Static | `state.hover` fill in light, `state.selected` in dark (`state.hover` is nearly invisible on `surface.chat`); hidden from screen readers; container announces "Loading messages". |
| 6.11 | ServerIcon | Server, instance, Home, unread conversation, Add (`+`) | Rest (circle), hover/focus (squircle), unread pill, mention badge, selected, offline, dragging | `component.rail.*`; morph with `MaterialShapes` on Android. |
| 6.12 | RailPill | Unread (8), hover (20), selected (40) | — | `unread.pill`; height animates over `motion.duration.base`. |
| 6.13 | ChannelRow | Text channel (with `#` or its icon), category header | Read, hover, unread, mentions, selected, muted, locked, focus, dragging | `component.channel-row.*`, `component.category.*`. A channel icon replaces `#` as a 20 px rounded square; a read-only channel with an icon shows its lock after the name. Members who can edit the channel get a gear at the trailing end, shown on hover, on focus within the row and on the selected row ("Edit channel general"). |
| 6.14 | ConversationRow | 1:1, group | Read, unread, selected, offline instance | Instance label when ≥ 2 instances. |
| 6.15 | UserPanel (user card) | With or without voice controls | Default, status menu open, status saving, menu error, user ID copied, offline, microphone muted, deafened, update available | `size.dock.*`, `component.user-panel.*`; aligned with the composer's base row (4.2); voice toggles hidden until voice ships (5.3). Web: `UserCard.svelte` with the status menu and `CustomStatusDialog.svelte` (5.3). |
| 6.15a | Navigator | Card (wide), drawer (narrow) | Default, drawer open | `component.navigator.*`: server strip, channel list, user card. |
| 6.15b | SideSheet | Docked, overlay | Closed, members, profile | `component.side-sheet.*`; `Ctrl+U`; focus moves in and back when it overlays. |
| 6.16 | ChatHeader | Channel, conversation, group | Topic collapsed / expanded | `component.chat-header.*`. |
| 6.17 | MessageRow | Group start (cozy/compact), continuation, system message (joins, pins) | Default, hover, focus, mentioned, highlighted (jump target), selecting text, pending, queued, failed, editing | Accessible name includes the author even on continuations. |
| 6.18 | MessageBlock | Paragraph, heading, subtext, quote, code block, list, spoiler | Code block: hover shows "Copy code" | Rendered per block (5.4). |
| 6.19 | MessageActionBar | Own message, other's message | Button hover, focus | `component.action-bar.*`; mirrors the context menu. |
| 6.20 | ReplyReference | Normal, original deleted, original not loaded | Hover, focus | Jumps to the original. |
| 6.21 | MentionChip | User, role, channel, `@everyone` | Hover, focus | `component.mention.chip-*`. |
| 6.22 | AttachmentImage | Static, animated, spoiler ("Spoiler" overlay) | Loading, loaded, error, removed | Alt text as accessible name. Web: `MessageAttachments.svelte`, `component.attachment.image-*`. |
| 6.23 | AttachmentFile | Card (web), compact row (desktop, until it handles files) | Default, pending (no link), downloading (progress), downloaded, error | Middle-truncated file name. Web: `component.attachment.file-*`; desktop rows: `attachment.row-*`. |
| 6.24 | Divider | Day, unread ("New" pill) | — | `component.unread-divider.*`. |
| 6.25 | JumpBar | New messages (top), Older messages / Jump to present (bottom) | Hover, focus | `component.jump-bar.*`. |
| 6.26 | Composer | Channel, conversation, edit inline, read-only | Empty, typing, with attachments, uploading, too long, offline, disabled | Includes reply bar and attachment tray. |
| 6.27 | AttachmentTray item | Image, file | Uploading, uploaded, error, spoiler on | "Add a description" for alt text. Web: `AttachmentTray.svelte`, `component.composer.tray-*`; Remove uses the hover-bar button. |
| 6.27a | DropOverlay | Channel, conversation | Shown while files are dragged over the chat pane | `component.drop-overlay.*`; hidden from assistive technologies, never focused. |
| 6.28 | TypingIndicator | 1, 2, 3–5, many | Static under reduced motion | Above the composer; keeps its row when empty. Not announced to screen readers. |
| 6.29 | Autocomplete popover | Users, channels, emoji | Results, no results ("No one called anx in Rust Lyon"), loading | Listbox semantics. |
| 6.30 | QuickSwitcher | — | Results, no results, loading | 5.10. |
| 6.31 | MemberRow | With activity, without | Hover, focus, offline (avatar 50 %) | `component.member-row.*`. |
| 6.32 | ProfilePopover | Self, other, other with moderation | Loading, error | `surface.popover`, `elevation.high`. |
| 6.33 | ContextMenu | With icons, with shortcut hints, with danger items (last, separated), submenus | Item hover (`state.selected` with `text.heading`; apricot is kept for 'you' and 'now'), danger hover (`danger.fill`); keyboard focus shows only the focus ring, because the first item takes focus when the menu opens and a menu opened with the mouse must not show a filled danger item; disabled with reason | `component.context-menu.*`; arrow keys, type-ahead, `Esc`. Slint has no shadow tokens, so the menu gets a 1 px `context-menu.border` outline instead of `elevation.high`; the keyboard focus ring is drawn `focus-ring.offset` outside the item so it stays visible on the danger fill (`ContextMenuItem` in `primitives.slint`). |
| 6.34 | Tooltip | Default, rail (instant, trailing side) | Shown after `motion.delay.tooltip` hover or immediately on focus | `component.tooltip.*`; never the only copy of essential information. |
| 6.35 | Dialog | Standard, destructive, typed confirmation | Default, submitting, error inline | One `surface.popover` card, `radius.lg`, no separate footer band; opens with the dialog transition (2.8); focus trapped; `Esc` closes (except while submitting); focus returns to the trigger. Bottom sheet on Android compact. |
| 6.36 | Toast / Snackbar | Info, success, with action ("Undo") | Auto-dismiss after 5 s (paused on hover/focus), dismissible | Polite live region; never for errors that need action. |
| 6.37 | NoticeBar | Info (`accent.fill`), warning (`warning.fill`), danger (`danger.fill`), success (`positive.fill`) | With / without action, dismissible or persistent | Full-width strip at the top of the window, as Discord's notices. |
| 6.38 | ConnectionDock | Hidden, reconnecting, offline, back online, session expired | — | Future voice slot. |
| 6.39 | EmptyState | Channel-start hero, compact | With one primary action | One title, one sentence, one action. |
| 6.40 | Badge | Count (mention), "New" pill | — | `component.badge.*`; "99+" cap; accessible text "3 mentions". |
| 6.41 | RoleChip | With colour dot | Removable | Colour dot plus name, never colour alone. |
| 6.42 | PermissionRow | Switch (role), tri-state (channel override) | Sensitive marker, locked | Plain-language description. |
| 6.43 | SettingsNav | Desktop column, Android list, admin web, web client (column; list page on phones) | Selected, focus, danger item | Discord settings look; 44 px items on the admin web. Desktop and web client: `component.settings.nav-*`, 32 px items (44 px under a coarse pointer on the web); groups with a sentence-case label, separated by a divider; the danger item fills `danger.fill` on hover (see 5.9). |
| 6.44 | UnsavedChangesBar | — | Visible, attention (leave attempted), saving, error | `elevation.high`. |
| 6.45 | Tabs | Standard | Selected, focus | Arrow keys move between tabs. |
| 6.46 | PaneResizeHandle | Vertical | Hover, dragging, focus | Keyboard resize. |
| 6.47 | ChatPanels (Android) | Channel panel, chat, members | Settling, dragging | `anchoredDraggable`, `size.mobile.panel-peek`. |
| 6.48 | AdminTable (web) | With search, pagination, row actions | Loading rows, empty, error | `<table>` with `<caption>` and `scope` headers. |
| 6.49 | AdminStatCard (web) | Health, number, storage bar | Normal, degraded, down | Status in words plus icon. |
| 6.51 | ReactionChip | Others', yours, add (smiley) | Hover, focus, pressed (yours), disabled (no history) | `component.reaction.*`; tooltip names who reacted, loaded on hover or focus. Web: `MessageReactions.svelte`. |
| 6.52 | EmojiPicker | Popover beside its button; bottom sheet on phones | Categories, recently used, search results, no results | `component.emoji-picker.*`; search first, one Tab stop for the emoji, arrows between them, Enter picks, Esc closes and focus goes back. About 200 built-in emoji in 8 categories with English and French search words; recently used kept in the browser. Web: `EmojiPicker.svelte`. |
| 6.54 | IconPicker | Server icon (circle at rest, animated allowed), channel icon (rounded square, always still) | Empty (initials or `#`), with icon, uploading, removing, error; crop dialog: opening, unreadable, cropping, uploading, upload error | `component.icon-picker.*`. The Overview shows the current icon at 80 px with "Upload icon" and "Remove icon", saved at once. Choosing a file opens "Choose the square": the picture on a stage, the kept square framed in white with the rest under `color.scrim`, starting on the centred square (what the instance keeps without a crop); drag, wheel and pinch move and zoom around the pointer, the stage takes arrow keys (Shift for larger steps) and + / −, a zoom slider and "Centre it" do the same; the result shows at its real size where it lives (two strip tiles, or a read and a selected channel row). The file is sent untouched with `crop_x`, `crop_y`, `crop_size` in picture pixels, or none for the centred square; the instance compresses. Animated pictures show their first frame in the dialog under reduced motion, and channel ones always. Web: `IconEditor.svelte`, `IconCropDialog.svelte`, `icon-crop.ts`. |
| 6.53 | PinsPanel | Popover under the header's pin button; bottom sheet on phones | Loading, empty, error with Try again, list | `component.pins.*`; newest pin first; Jump for messages loaded in the list, Unpin for those who may. Web: `PinnedMessagesPanel.svelte`. |
| 6.50 | Melli | The character alone; a host on apricot switches its marks to ink | The eight states of 2.10, each animated or still | Decorative (`aria-hidden`; `accessible-role: none` on the desktop): the host names what it stands for. Web: `Melli.svelte`, size from a length token. Desktop: `melli.slint`, sized by its host. |

Before adding a component, check this list. A new component is added here first, with variants and states.

---

## 7. Accessibility

Target: WCAG 2.2 level AA on all three surfaces, verified with real assistive technologies. Every colour pair is checked by the token generator (2.3); where a convention borrowed from Discord conflicts with AA, AA wins.

### 7.1 Contrast and colour

- Text pairs and non-text pairs in section 2.3 meet AA in both themes and are enforced by the token generator.
- Information is never carried by colour alone: status shapes, unread pills and heading colour, the mention bar, the "New" pill text, icons and words in permission and switch states, words in status messages.
- Role colours are adjusted for readability (2.3).

### 7.2 Keyboard (desktop)

`Ctrl` means `Cmd` on macOS. Shortcuts follow Discord's wherever Discord has one. They are listed in Settings → Keybinds and in a cheat sheet (`Ctrl+/`).

| Area | Keys | Action |
|---|---|---|
| Global | `Ctrl+K` | Quick switcher |
| Global | `Alt+Up` / `Alt+Down` | Previous / next channel or conversation |
| Global | `Alt+Shift+Up` / `Alt+Shift+Down` | Previous / next unread channel |
| Global | `Ctrl+Shift+Alt+Up` / `Down` | Previous / next channel with unread mentions |
| Global | `Ctrl+Alt+Up` / `Ctrl+Alt+Down` | Previous / next server |
| Global | `Esc` | Close the topmost popover, menu or dialog; otherwise mark the channel as read |
| Global | `Shift+Esc` | Mark the whole server as read |
| Global | `Shift+PageUp` | Jump to the oldest unread message |
| Global | `Ctrl+U` | Open or close the side sheet (members) |
| Global | `Ctrl+Shift+M` / `Ctrl+Shift+D` | Mute microphone / deafen (once voice ships) |
| Global | `Ctrl+Shift+T` | New message (conversation) |
| Global | `Ctrl+Shift+U` | Upload a file |
| Global | `Ctrl+E` | Emoji picker (reserved until it ships) |
| Global | `Ctrl+F` | Search (reserved, section 9) |
| Global | `Ctrl+,` | User settings |
| Global | `Ctrl+=` / `Ctrl+-` / `Ctrl+0` | Zoom in / out / reset |
| Global | `Ctrl+/` | Keyboard shortcut cheat sheet |
| Global | `F6` / `Shift+F6` | Move focus between regions: rail → channel sidebar → message list → composer → member list (and header); in narrow windows, entering the rail or sidebar opens the drawer |
| Rail, sidebar, member list | `Up` / `Down`, `Home` / `End` | Move focus (roving focus, one tab stop per list) |
| Rail, sidebar | `Enter` / `Space` | Open |
| Sidebar | `Left` / `Right` | Collapse / expand category |
| Rail, roles, channels (settings) | `Ctrl+Shift+Up` / `Down` | Move the focused item up or down |
| Any focusable item | `Shift+F10` or Menu key | Context menu |
| Message list | `Up` / `Down`, `PageUp` / `PageDown`, `End` | Previous / next message, scroll, jump to present |
| Focused message | `R` reply, `E` edit (own), `Ctrl+C` copy text, `F2` select text, `Alt+Enter` mark unread, `Backspace`/`Delete` delete (with confirmation), `Shift+Delete` delete own without confirmation, `Enter` open the actions menu | Message actions (Discord's message keybinds plus `F2`) |
| Focused message | `Tab` | Move into the message's links, chips, attachments and "Copy code" buttons |
| Composer | `Enter` / `Shift+Enter` | Send / new line |
| Composer | `Up` (empty) | Edit last message |
| Composer | `Ctrl+B` / `Ctrl+I` | Bold / italic |
| Dialogs | `Tab` cycle, `Esc` cancel, `Enter` primary action when focus is in a field | — |

Focus visibility: a 2 px `color.focus` ring with a 2 px gap, following the element's radius (including the squircle morph), shown on keyboard focus only, never removed. Focused elements are never obscured by sticky bars (WCAG 2.4.11): the message list scrolls the focused message clear of the composer and both jump bars.

### 7.3 Screen readers

Desktop (Slint with AccessKit: VoiceOver, NVDA/Narrator, Orca):

- Every interactive element sets `accessible-role`, `accessible-label` and, where useful, `accessible-description`, `accessible-checked` or `accessible-value`.
- Regions have names: "Servers", "Channels in Rust Lyon", "Messages in #general", "Message composer", "Members of Rust Lyon".
- Message accessible name pattern: "[Mentions you.] Ana, Moderator, today at 14:32: Has anyone tried Slint on Wayland? Edited. 2 attachments. Replying to Léo." In French: « [Te mentionne.] Ana, Modération, aujourd’hui à 14:32 : Quelqu’un a testé Slint sous Wayland ? Modifié. 2 pièces jointes. En réponse à Léo. » Continuation messages repeat the author.
- Every accessible name, description and announcement exists in English and French in the same catalogue as the visible copy (ADR-0022); region names in French: « Serveurs », « Salons de Rust Lyon », « Messages dans #général », « Zone de saisie », « Membres de Rust Lyon »; badges: « Rust Lyon, 3 mentions, non lu ».
- Text that cannot be selected with the pointer is still fully exposed as the message's accessible text; "Select text" mode exposes an editable-text role so screen-reader users can review it character by character.
- Labels are sentence case on screen and in the accessibility tree, so screen readers never spell out capitals.
- Badges are text: "Rust Lyon, 3 mentions, unread".
- New messages in the open channel are announced politely when the user is at the bottom, if the accessibility setting is on (the spike exposes a live region; real-AT verification pending, section 9).
- Decorative elements (skeleton bars, dividers, quote bars, reply spines, status cut-outs) are hidden from the tree.

Android (TalkBack, Switch Access, keyboards):

- Message rows merge descendants; custom accessibility actions expose Reply, Edit, Copy text, Mark unread, Delete.
- The chat pane exposes "Show channels" and "Show members" as custom actions; the top app bar buttons do the same, so the panels never require a swipe.
- `contentDescription` on meaningful icons and images, `null` on decorative ones; status is in the row's description ("online").
- Heading semantics on the channel name and settings section titles; pane titles for the three panels.
- Typing indicators are excluded from announcements; send results use polite live regions ("Message sent" only when sending was slow or failed before).

Admin web: semantic HTML first (`<nav>`, `<main>`, `<table>`, `<label>`), `aria-live="polite"` on htmx-swapped status regions, focus moved to the new content heading after a page-level htmx swap, skip link "Skip to content".

### 7.4 Targets

| Surface | Minimum target | Notes |
|---|---|---|
| Android | 48 × 48 dp (`minimumInteractiveComponentSize`, `size.mobile.row-height`) | The 40 dp highlight sits inside a 48 dp row. |
| Admin web | 44 × 44 CSS px | Includes navigation items and table row actions. |
| Web client | The desktop sizes with a mouse; 44 × 44 CSS px (`size.hit.web-min`) under a coarse pointer | Rows, tabs, header, hover-bar and menu buttons, emoji cells and inline links all grow; short tabs ("All") widen to 44 px too. |
| Desktop | 24 × 24 px absolute minimum (WCAG 2.5.8); Discord's own sizes give 32 px for rows, hover bar buttons and header icons, 48 px for rail icons, 24 px for category headers | Pointer-first. Touchscreen laptops are an open question (section 9). |

Drag-and-drop actions (reordering servers, roles, channels) and the Android panel swipes always have a non-drag alternative (WCAG 2.5.7).

### 7.5 Text scaling and reflow

- Desktop: Zoom level 50–200 % scales the whole UI; Chat font size 12–24 px scales messages. At 200 % zoom, panes keep their proportions and the breakpoints of 4.2 apply to the zoomed width, so the layout collapses to the drawer rather than clipping. Channel names truncate with a tooltip in the sidebar; the sidebar can be widened to 360 px.
- Android: system font scale up to 200 % (non-linear on Android 14+); layouts tested at `fontScale = 2f` in previews.
- Admin web: works at 400 % zoom (reflow at 320 px width, WCAG 1.4.10).
- Web client: every size token is in `rem`, so browser zoom and the browser's text size both scale the interface; at 200 % the breakpoints of 4.5 apply to the zoomed width, and every screen reflows at 320 px without scrolling sideways (long words, URLs and code blocks wrap or scroll inside their block).

### 7.6 Motion, time and media

- Reduced motion as in 2.8.
- No time limits on reading; toasts with actions stay while hovered or focused, and their actions remain available elsewhere (for example "Undo delete" is also in the message's menu for 10 s).
- Spoilers and animated media require a user action to reveal or play by default.

---

## 8. UX copy: voice and tone

Mellow speaks like a calm friend who also happens to run the server: plain, direct, never blaming. Every string exists in English and French (ADR-0022); neither is a word-for-word translation of the other. Where Discord has a well-known phrase for the same action ("Mark as read", "Jump to present", "You're viewing older messages", "Copy text", "Mark unread", "Server settings"), Mellow uses the same words so the interface feels familiar; it never copies Discord's marketing or jokes.

Rules:

- Sentence case everywhere, on screen as in source strings: buttons, titles, category headers, member groups, form labels and the "New" pill. Nothing is rendered in capitals.
- Active voice, second person ("You can't send messages here"), first person plural for the app only when it acts ("We couldn't reach…").
- Buttons name the action: "Save changes", "Ban Ana", "Create invite". Never "OK", "Submit", "Yes".
- Results echo the action: "Changes saved", "Copied", "Ana was banned".
- Errors say what happened, why if we know, and what to do. No "Oops", no "Something went wrong" without a next step, no error codes in the main sentence (codes go in a "Details" disclosure for admins).
- Empty states invite the next action in one sentence.
- No exclamation marks, including in places where Discord uses them ("Welcome to #general", not "Welcome to #general!"). No emoji in system copy.
- Use the user's words: "server", "channel", "Messages", "instance". Never "guild", "payload", "websocket", "token".
- Name the instance whenever it matters (errors, sign-in, multi-instance contexts), because self-hosted means different places with different rules.
- Numbers and dates use the user's locale; relative dates only for "Today" and "Yesterday".

French (Français):

- **Tutoiement** everywhere (« Tu ne peux pas envoyer de messages ici »), the register of a friend who runs the server and of the French Discord interface; the app says « on » or the passive voice when it acts, never « nous » for a single action that failed (« Impossible de joindre chat.example.org »).
- Sentence case, as in English. Buttons are infinitive verbs (« Enregistrer les modifications », « Bannir Ana », « Créer une invitation »); results are past participles (« Modifications enregistrées », « Copié »).
- French typography: a narrow no-break space (U+202F) before « ? », « ! » and « ; », a no-break space (U+00A0) before « : » and inside « guillemets », typographic apostrophes (’), the ellipsis character (…), « e-mail » not « email », thousands with a narrow no-break space (« 1 204 »), sizes in Ko, Mo, Go.
- Words users already know from Discord in French: « serveur », « salon », « Messages privés », « Marquer comme lu », « Revenir au présent », « Répondre », « Copier le texte », « Paramètres utilisateur ». « Instance » stays « instance ».
- No anglicisms where French has a common word: « diapos » not « slides », « relire » not « reviewer », « notes de version » not « changelog »; product names stay as they are.
- Avoid gendered agreement on the person addressed or named (« Ana ne fera plus partie du serveur » rather than « Ana sera retiré(e) »), without inclusive punctuation.

Examples:

| Situation | English | Français |
|---|---|---|
| Message failed | "Not sent. Retry · Delete" | « Non envoyé. Réessayer · Supprimer » |
| Upload too large | "photo.heic is 48 MB. chat.example.org accepts files up to 25 MB." Until the instance tells its limit, the web client says "photo.heic is 48 MB, more than chat.example.org accepts." | « photo.heic fait 48 Mo. chat.example.org accepte les fichiers jusqu’à 25 Mo. » |
| Too many files | "You can attach up to 10 files to one message." | « Tu peux joindre jusqu’à 10 fichiers par message. » |
| Sending while uploading | "Your files are still uploading. Send again once they're done." | « Tes fichiers sont encore en cours d’envoi. Renvoie le message quand ils sont prêts. » |
| Drop overlay | "Drop to attach to #general" · "You can add a message before sending." | « Dépose pour joindre à #général » · « Tu pourras ajouter un message avant l’envoi. » |
| Connection lost | "Reconnecting to chat.example.org… Messages you send will wait until you're back." | « Reconnexion à chat.example.org… Les messages que tu envoies partiront dès ton retour. » |
| Permission missing | "You can't send messages in #announcements." | « Tu ne peux pas envoyer de messages dans #annonces. » |
| Instance unreachable | "We couldn't reach chat.example.org. Check the address, or try again if your connection is down." | « Impossible de joindre chat.example.org. Vérifie l’adresse, ou réessaie si ta connexion est coupée. » |
| Selecting text | "Selecting text · Esc to finish" | « Sélection du texte · Échap pour terminer » |
| Empty server | "No channels yet. Create the first one to get the conversation going." · "Create a channel" | « Aucun salon pour l’instant. Crée le premier pour lancer la conversation. » · « Créer un salon » |
| Empty conversations | "No conversations yet. Start one with someone from your servers." · "New message" | « Aucune conversation pour l’instant. Lances-en une avec quelqu’un de tes serveurs. » · « Nouveau message » |
| Empty bans | "No one is banned from Rust Lyon." | « Personne n’est banni de Rust Lyon. » |
| Kick confirmation | Title "Kick Ana from Rust Lyon?" · Body "Ana will be removed from the server. They can rejoin with a new invite." · Buttons "Cancel" / "Kick Ana" | Titre « Expulser Ana de Rust Lyon ? » · Texte « Ana ne fera plus partie du serveur. Une nouvelle invitation lui permettra de revenir. » · Boutons « Annuler » / « Expulser Ana » |
| Ban confirmation | Title "Ban Ana from Rust Lyon?" · Body "Ana will be removed and won't be able to rejoin, even with an invite." · Field "Reason (optional, visible to moderators)" · Select "Delete their recent messages" (Don't delete, Last hour, Last 24 hours, Last 7 days) · Buttons "Cancel" / "Ban Ana" | Titre « Bannir Ana de Rust Lyon ? » · Texte « Ana ne fera plus partie du serveur et ne pourra pas revenir, même avec une invitation. » · Champ « Raison (facultatif, visible par la modération) » · Liste « Supprimer ses messages récents » (Ne rien supprimer, Dernière heure, Dernières 24 heures, 7 derniers jours) · Boutons « Annuler » / « Bannir Ana » |
| Delete channel | Title "Delete #old-announcements?" · Body "All 1,204 messages in this channel will be deleted for everyone. This can't be undone." · Field "Type old-announcements to confirm" · Buttons "Cancel" / "Delete channel" (enabled when the name matches) | Titre « Supprimer #anciennes-annonces ? » · Texte « Les 1 204 messages de ce salon seront supprimés pour tout le monde. C’est irréversible. » · Champ « Saisis anciennes-annonces pour confirmer » · Boutons « Annuler » / « Supprimer le salon » |
| Delete own message | Title "Delete this message?" · Body "It will be removed for everyone." · Buttons "Cancel" / "Delete message" · Hint "Tip: hold Shift while deleting to skip this next time." | Titre « Supprimer ce message ? » · Texte « Il sera supprimé pour tout le monde. » · Boutons « Annuler » / « Supprimer le message » · Astuce « Maintiens Maj en supprimant pour passer cette étape la prochaine fois. » |
| Add a server | Title "Create a server" · Body "Give your server a name. You can add channels and invite people next." · Field "Server name" · Link "Have an invite already? Join a server" · Buttons "Cancel" / "Create" ("Creating…" while busy) | Titre « Créer un serveur » · Texte « Donne un nom à ton serveur. Tu pourras ensuite ajouter des salons et inviter des gens. » · Champ « Nom du serveur » · Lien « Tu as déjà une invitation ? Rejoindre un serveur » · Boutons « Annuler » / « Créer » (« Création… ») |
| Join a server | Title "Join a server" · Body "Enter an invite to join a server someone already runs." · Field "Invite code", helper "Paste the invite code someone gave you." · Link "Don't have an invite? Create a server" · Buttons "Cancel" / "Join" ("Joining…") | Titre « Rejoindre un serveur » · Texte « Saisis une invitation pour rejoindre un serveur qui existe déjà. » · Champ « Code d’invitation », aide « Colle le code d’invitation qu’on t’a donné. » · Lien « Pas d’invitation ? Créer un serveur » · Boutons « Annuler » / « Rejoindre » (« Ajout du serveur… ») |
| Invite people | Title "Invite friends to Rust Lyon" · Field "Send this invite code to a friend" (read-only, "Creating an invite…" while empty) with "Copy" ("Copied" for 2 s) · Hint "This invite never expires." · Button "Close" | Titre « Inviter des gens sur Rust Lyon » · Champ « Envoie ce code d’invitation à quelqu’un » (« Création de l’invitation… » tant qu’il est vide) avec « Copier » (« Copié » pendant 2 s) · Note « Cette invitation n’expire jamais. » · Bouton « Fermer » |
| Create channel | Title "Create channel" · Field "Channel name" with a `#` prefix, placeholder "new-channel", helper "Names are lowercase, with dashes instead of spaces." · Buttons "Cancel" / "Create channel" | Titre « Créer un salon » · Champ « Nom du salon » avec le préfixe `#`, exemple « nouveau-salon », aide « Les noms sont en minuscules, avec des tirets à la place des espaces. » · Boutons « Annuler » / « Créer le salon » |
| Leave server | Title "Leave 'Rust Lyon'" · Body "Are you sure you want to leave Rust Lyon? You won't be able to rejoin unless you are re-invited." · Buttons "Cancel" / "Leave server" (Danger; Cancel has focus) | Titre « Quitter « Rust Lyon » » · Texte « Tu veux vraiment quitter Rust Lyon ? Tu ne pourras revenir qu’avec une nouvelle invitation. » · Boutons « Annuler » / « Quitter le serveur » (« Départ du serveur… ») |
| Revoke invite | Title "Revoke this invite?" · Body "The link will stop working. People who already joined stay in the server." · Buttons "Cancel" / "Revoke invite" | Titre « Révoquer cette invitation ? » · Texte « Le lien ne fonctionnera plus. Les personnes déjà arrivées restent sur le serveur. » · Boutons « Annuler » / « Révoquer l’invitation » |
| Leaving a masked link | Title "Leaving Mellow" · Body "This link goes to example.com/page. Only open links you trust." · Buttons "Cancel" / "Open link" | Titre « Tu quittes Mellow » · Texte « Ce lien mène à example.com/page. N’ouvre que les liens de confiance. » · Boutons « Annuler » / « Ouvrir le lien » |
| Session expired | "Your session on chat.example.org has ended. Sign in again to keep chatting." · "Sign in" | « Ta session sur chat.example.org a pris fin. Reconnecte-toi pour continuer à discuter. » · « Se connecter » |
| Notifications denied | "Notifications are off for Mellow. You won't hear about mentions or messages." · "Open Android settings" | « Les notifications de Mellow sont désactivées. Tu ne recevras aucune alerte pour les mentions ni les messages. » · « Ouvrir les réglages Android » |

Destructive confirmation pattern:

- The title is a question naming the object.
- The body states the consequence and whether it can be undone.
- The destructive button repeats the verb and the object, uses the Danger variant, and is never the default focus. Default focus goes to "Cancel".
- Typed confirmation only for irreversible actions affecting many people (deleting a channel, a server, an account, an instance user).

---

## 9. Open questions and items to validate

Brand and legal:

1. **Mellow logo**: chosen (Melli, 2.10), on the web and the desktop, with macOS and Linux app icons. Still to do: the Windows `.ico` (with a Windows installer), the Android adaptive icon and the admin header.
2. **Trade-dress review** before any public release: check accent hue, logo, fonts, default avatars and marketing screenshots against the brand rules of ADR-0022 (inherited from ADR-0012).
3. **Emoji set**: Twemoji (familiar to Discord users, CC-BY 4.0 with attribution) is preferred over Noto Color Emoji. Confirm that a maintained COLR font build of Twemoji renders in Slint on all three desktop platforms and on Android, and measure its size; otherwise fall back to Noto Color Emoji or the system emoji font.

Fidelity:

4. **Dimension check against Discord**: overlay screenshots of Mellow and the current Discord desktop client at 100 % zoom and verify every row of the table in 4.2 (especially the hover bar, compact mode, typing band and member rows, whose values were taken from Discord's CSS as publicly known in 2023–2024). Since ADR-0022 only the pane order, pane widths and interaction targets are kept; colours, radii and line heights are Mellow's own.
5. **Muted channels**: AA leaves only a small step between read (7.40:1) and muted (6.00:1) channels in dark. Test with Discord users whether muted channels are still recognisable; if not, add a muted icon after the name.
6. **Mention highlight in apricot instead of Discord's gold**: confirm with users that it still reads as "this mentions you" at a glance.

Product scope:

7. **"Server" vs "instance"**: validate with five users that "instance" is understood after the one-line explanation on the welcome screen; alternatives are "host" and "community".
8. **Message search**: post-MVP according to ARCHITECTURE.md (SQLite FTS5). The header reserves the slot and `Ctrl+F` is kept free; users expect it early.
9. **Forwarding, link previews and custom emoji** are not in the MVP; reactions, the emoji picker and pins shipped on the web (v0.17), without the `Ctrl+E` shortcut yet. Confirm scope and order of the rest.
10. **Group conversation size**: "Up to 9 people" (10 including you) is assumed; confirm the server limit.
11. **Cross-instance conversations**: excluded (no federation). Confirm the copy and that the picker hides people from other instances rather than disabling them.
12. **Default notification level** for new servers ("Only @mentions"): confirm with the product owner; Discord defaults to "All messages" for small servers.
13. **Self-signed certificates**: the MVP refuses invalid certificates without a bypass. Decide whether to add trust-on-first-use pinning later (needs a security review).

Platform and technical:

14. **Cross-message selection**: impossible with Slint's `StyledText` (ADR-0010). Mellow ships Copy text, Select text (`F2`), Copy code and Copy link instead. Revisit if Slint gains selectable styled text; consider "Copy messages" for a focused range if users ask.
15. **Inline spoilers and underline**: `StyledText` cannot hide a span inside a sentence or render `__underline__`. Decide whether to render spoiler paragraphs as whole hidden blocks (current spec) or to extend the block splitter.
16. **Composer formatting preview**: Slint's text input does not render inline styles, so the composer shows raw Markdown, as Discord did before its rich composer. Check later Slint versions.
17. **Slint live regions and screen readers**: no one has listened to the spike with VoiceOver, NVDA or Orca yet; this is required before the message list is considered done. If live regions do not work on a platform, "Announce new messages" is unavailable there.
18. **Variable fonts in Slint**: confirm that Figtree's `wght` axis renders in Slint; otherwise bundle static instances (400, 500, 600, 700, 800). Re-check the clipped accented capital ("É") seen in the spike once the real font is bundled.
19. **Zoom implementation in Slint**: confirm that multiplying length tokens by `zoom` covers every element (including std-widgets used for scrolling), or use the window scale factor if Slint exposes it.
20. **Jump to message with estimated row heights**: `ListView` estimates variable heights; landing exactly on a target message must be prototyped.
21. **Reduced-motion detection on Linux desktops** beyond GNOME: find reliable signals or rely on the in-app setting.
22. **Script coverage**: decide which Noto subsets to bundle versus relying on system fallback, and validate right-to-left message rendering.
23. **Android panel gestures**: validate the three-panel drag against the system back gesture and with Switch Access; verify `anchoredDraggable`, `MaterialShapes` morphing and `LoadingIndicator` in the pinned Compose Material 3 version, falling back to stable APIs if needed.
24. **Android dynamic colour**: off for brand identity. Consider an opt-in "Use wallpaper colours" setting after the MVP if users ask.
25. **Touchscreen desktops**: consider a "Touch" density that raises desktop targets to 44 px.

Dusk direction (ADR-0022, proposed), decisions for the owner:

30. **Plum-tinted neutrals**: the dusk family (hue 322) is the core of the new identity. Confirm it reads "warm evening" and not "purple" on real screens, especially low-quality laptop panels; the fallback is the same family at lower chroma.
31. **Apricot unread divider**: Discord users read a red line as "new". Validate that the apricot divider with its "New" pill is noticed as quickly.
32. **Hovered servers stay neutral**: Discord fills hovered servers without artwork with blurple; Mellow uses the selected tone so apricot means "you" and "now". Confirm the owner wants this restraint.
33. **Sentence-case labels**: dropping Discord's uppercase category and group labels is the most visible typographic change; it needs a Rust change on desktop (`live_rows.rs`, `shell_rows.rs`).
34. **Inset sheet**: costs 8 px around the chat on wide windows; confirm on 1280 × 800 and on 13-inch laptops.
35. **24 px message line**: about 9 % fewer messages per screen than Discord's 22 px; Compact density stays available for people who want more.
36. **Desktop reduced-motion setting**: shipped as a stored choice (2.8); "Match system" waits for Slint to expose the OS preference.
37. **Navigator and side sheet** (v0.16): test with Discord users that servers and channels in one card, and members on demand, are found without help; check that the side sheet closed by default does not hide who is online for people who rely on it (the setting is remembered per server).
38. **Typing indicator above the composer**: a change from Discord, made so the composer and the user card share a bottom edge; validate that it is still noticed.
39. **Microphone and headset hidden until voice**: the owner asked to see them; confirm that hiding them in releases until voice works is acceptable, or show them as a stored preference.
40. **Icon tile**: confirm the gradient tile at 16 and 32 px on Windows, GNOME and KDE taskbars and on light and dark browser tabs; the textured variant stays for marketing only.
41. **French copy review** by a native speaker before release, and the language setting's place (User settings → Language).

Visual validation still to do:

26. Render both themes on real screens (OLED phone, low-quality laptop IPS, sunlight) and check that the dusk layers stay distinguishable, especially the ground versus the sheet and the user card (`#150F16` versus `#221B23`).
27. Check apricot versus amber and red distinguishability under protanopia, deuteranopia and tritanopia simulation, even though none carries meaning alone.
28. Usability test the multi-instance rail with users who have two or more instances.
29. Test with real assistive technologies: VoiceOver (macOS), NVDA and Narrator (Windows), Orca (Linux), TalkBack and Switch Access (Android).

---

## 10. Defaults we rejected

For each, the reason it was rejected for Mellow:

| Default | Why we rejected it | What we do instead |
|---|---|---|
| Pixel-perfect Discord clone including blurple and gg sans | Uses Discord's brand identity (trade-dress risk), and gg sans cannot be licensed | Discord's concepts and behaviour with Mellow's own layout and look (ADR-0022) |
| Faithful Discord look with only the accent changed (ADR-0012) | A clone is judged against the original and loses; the owner wants Mellow to compete | Dusk: Mellow's colour, type, shape and motion in the Navigator layout |
| A new layout (single column, threads first) | Discord users would have to relearn where things are | Same panes in the same order and sizes |
| Discord's cool greys | They are Discord's identity, and several fail AA for text roles | Plum-tinted dusk neutrals, checked at AA by the generator |
| Warm charcoal neutrals with apricot | Looks like several current AI and productivity products | Plum dusk, the evening sky Melli floats in |
| Navy neutrals | Too close to Discord's blue-grey family | Plum dusk, opposite apricot on the wheel |
| Any blue or violet accent | Too close to blurple; a screenshot would look like Discord | Apricot, the complement of blurple |
| Four flush panes separated by lines and shadows | Busy, and the conversation does not stand out | One ground, one lit sheet, separation by tone and space |
| Apricot on every interactive state (hovered servers, focused menu items) | The accent stops meaning anything | Apricot only for "you" and "now"; neutral selected tone elsewhere |
| Uppercase, tracked-out labels | They shout, read worse and are a template tell | Sentence-case labels at 13 px semibold |
| "Moderators — 2" group headers | A dash joining label and number is noise | Label and a separate tabular count |
| Inter, the system UI font, or a second display face | Generic, or more weight to ship in both clients | Figtree only, tighter tracking at display sizes |
| One radius on everything | Flattens hierarchy | Concentric scale 4/8/12/16/24 by role |
| Square server icons (v0.1) | The circle-to-squircle morph is the most recognisable rail interaction | Circle at rest, settling into a squircle |
| Fake drag-selection across messages | Slint cannot do it; a half-working imitation loses text | Copy text, Select text (`F2`), Copy code, Copy link |
| Material baseline purple, standard Material navigation and dynamic colour on Android | Template look, and not how Discord mobile navigates | Three-panel model with Material 3 Expressive components and Mellow tokens |
| Expressive motion everywhere on Android | Restless, and costly | `MotionScheme.standard()`, springs only for the settle moments |
| Shimmering skeletons, per-item entrance animations on load, decorative loops | Restless and costly; nothing should draw while idle | Static skeletons; motion only for state transitions (2.8) |
| Glassmorphism, gradients, glow | Decoration without function, costly in Slint, hard to keep at AA | Flat tone steps; shadows only on floating layers |
| Hover-only message actions | Inaccessible to keyboard and touch users | Hover bar, context menu, message keybinds and accessibility actions |
| Silent connection problems | Users lose messages and trust | Connection dock, labelled pending, queued and failed messages |
| Text smaller than 12 px | Discord's 10 px "(edited)" and "NEW" are hard to read | 12 px floor |
| Discord's exact silhouette (72 px rail of circles, flat channel list, permanent member column) | The owner found that Dusk on that layout still reads as Discord | The navigator card, the sheet and the side sheet (4.2) |
| Palette-first navigation without a channel list | Two actions to switch channel, unread channels out of sight | The quick switcher stays one key or one click away ("Aller à… Ctrl K") |
| User panel shorter than the composer, misaligned | Two bars side by side at different heights look careless | `size.dock.*` shared by both |
| Melli on a transparent or white background as the app icon | Disappears on light tabs and docks; the owner disliked it | Melli on a dusk gradient tile |
| English-only interface | The owner and much of the first audience are French | English and French catalogues, tutoiement, French typography |
| A service worker with an offline shell for the installed web app | Chrome installs without one; a cached client can outlive the server it talks to, and caching risks keeping personal data on shared devices | No service worker: the installed window loads the client the server serves |
| An "Install the app" banner | Nags on every visit, and the browsers already offer installation | Nothing in the interface; the browser's own install entry |
| Tailwind's screen breakpoints (`sm`, `md`, `lg`) | Arbitrary widths unrelated to the panes | Breakpoints summed from the pane tokens (4.2) |
