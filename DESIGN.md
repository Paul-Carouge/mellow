# Mellow design document

Status: draft v0.6 (2026-10-20). Owner: design. Applies to the Slint desktop client, the Android client (Jetpack Compose, Material 3 Expressive) and the admin web UI embedded in the server.

This document is the reference for every visual and interaction decision in Mellow. Developers should be able to build any MVP screen from it without inventing a colour, a size or a sentence. When something is missing, add it here first, then implement it. Every value named here exists in `design/tokens.json`.

## Changelog

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

A design or a pull request that breaks a principle needs an explicit justification in its description.

### 1.1 Familiar first

Someone who uses Discord every day must be able to use Mellow without learning anything: the same panes in the same places at the same sizes, the same hover states, the same right-click menus, the same shortcuts and the same message grouping. When in doubt, do what Discord does.

- Test: every pane and control matches the dimension table in section 4.2 and the shortcut table in section 7.2.
- Test: a Discord user, given five tasks (switch server, find an unread channel, reply to a message, mark a channel as read, open a member's profile), completes them without help on the first try.

### 1.2 Mellow's brand wherever Discord's brand would be

The interface is Discord's; the identity is Mellow's. The rule is mechanical: **every place Discord uses blurple, Mellow uses apricot (`color.accent.fill`), and white text on blurple becomes dark ink on apricot (`color.text.on-accent`)**. The Home button shows the Mellow logo, the font is Figtree, the icons are Phosphor, the emoji are open-licence artwork.

- Test: no token, asset or screenshot uses Discord's blurple (`#5865F2`) or any accent with a hue between 215° and 255° in OKLCH; no Discord logo, wordmark, mascot or gg sans/Whitney font file is in the repository.
- Test: default avatars and empty states use Mellow's own artwork, never a mascot.

### 1.3 Accessible even where Discord is not

Mellow keeps Discord's look, but not its contrast failures. Every text/background pair meets WCAG 2.2 AA, every non-text indicator meets 3:1, information is never carried by colour alone, and everything reachable by pointer is reachable by keyboard and screen reader. Where a Discord grey fails, the token is adjusted by the smallest step that passes, and the deviation is listed in section 2.3.

- Test: the token generator checks every pair declared in `design/tokens.json` (`$extensions.org.mellow.contrast-pairs`) in both themes and fails the build below the threshold.
- Test: every hover action also exists in the context menu (`Shift+F10` or the Menu key), as a keyboard shortcut on the focused item, and as an accessibility action.

### 1.4 Say what is happening

Self-hosting means several places, connections that drop and servers that are sometimes down. Mellow always tells you which instance you are talking on, whether you are connected, and whether your message was delivered. Pending, failed and queued messages are labelled in words.

- Test: every screen in section 5 defines loading, empty, error, offline and reconnecting states; no state is a lone spinner or a blank area.
- Test: when the user is signed in to more than one instance, the composer placeholder and every conversation header name the instance.

### 1.5 Honest about platform limits

When the toolkit cannot reproduce a Discord behaviour, Mellow offers an explicit equivalent instead of a broken imitation. Slint cannot select text across messages, so every message has "Copy text" and a per-message "Select text" mode (section 5.4).

- Test: each known gap is listed in section 9 with its replacement pattern, and the replacement is reachable by mouse, keyboard and screen reader.

---

## 2. Visual identity

### 2.1 Personality

Mellow looks like the chat app people already live in, so the brand speaks in a few precise places rather than everywhere: the apricot accent on the actions that matter, the Mellow mark in the Home button, the warm mention highlight that says "this one is for you". Everything else is Discord's quiet, dense, cool-grey workspace.

Three words guide the brand moments: **familiar, warm, dependable**.

- **Familiar**: layout, density and behaviour are Discord's.
- **Warm**: apricot is the only warm hue on cool greys, so it reads instantly as "action" or "you". It sits opposite Discord's blurple on the colour wheel, so a screenshot of Mellow is never mistaken for Discord.
- **Dependable**: connection state, delivery state and the instance you are on are always visible.

### 2.2 Palette

The reference palette (`ref.*` in `design/tokens.json`) has six families. Components never use them directly; they use the semantic tokens in 2.3.

| Family | Role | Anchors |
|---|---|---|
| Grey | Discord's layered cool neutrals for surfaces and text, plus Mellow's AA adjustments | `#111214` (floating, dark) to `#FFFFFF` (chat, light); full list in `ref.grey` |
| Apricot | Mellow's brand accent: primary actions, selected Home, focused menu items, jump bars, mention highlight | `#F2A46E` (dark fill), `#E8894B` (light fill), `#2B1608` (text on apricot) |
| Sky | Links and the keyboard focus ring | `#33B8FF` (dark), `#005FCC` (light) |
| Green | Online presence, positive text, add-server button | `#23A55A`, `#2DC770` (dark text), `#1A7F45` (light) |
| Amber | Idle presence, warning notices | `#F0B232`, `#A06E00` (light idle) |
| Red | Danger, do-not-disturb, mention badges, unread divider | `#DA373C` (fills and badges), `#FA777C` (dark text), `#C4262B` (light text), `#F45A5E` (dark presence) |

**Why apricot and not another hue.** Apricot (hue about 55° in OKLCH) is the complement of blurple (about 270°), the largest possible hue distance from Discord's brand, so the trade-dress risk is low. It is the only warm, saturated colour on Discord's cool greys, so it carries the "primary action" role as strongly as blurple does. It keeps continuity with Mellow's first identity and its name. It does not collide with status colours: green (online), amber (idle) and red (do-not-disturb, badges) all keep their Discord meaning, and apricot never appears on a presence indicator. Dark ink on apricot reaches 8.44:1, better than white on blurple.

Apricot and amber are neighbours. Neither carries meaning alone: idle always has its crescent shape, warnings always have an icon and a sentence, and the mention highlight always has its leading bar.

### 2.3 Semantic colour tokens

Dark is the default theme. Light is offered and follows the system setting when the user picks "Match system". Values below are the resolved hex values; the token file stores them as aliases of `ref.*`.

#### Surfaces (the layered grey family)

Like Discord's dark theme, surfaces get lighter from the window edge toward the conversation: rail, then user panel, then sidebar, then chat.

| Token | Dark | Light | Used for | Discord equivalent |
|---|---|---|---|---|
| `color.surface.rail` | `#1E1F22` | `#E3E5E8` | Server rail | background-tertiary |
| `color.surface.sidebar-footer` | `#232428` | `#EBEDEF` | User panel, connection dock | background-secondary-alt |
| `color.surface.sidebar` | `#2B2D31` | `#F2F3F5` | Channel sidebar, member list, settings navigation, dialog footers | background-secondary |
| `color.surface.chat` | `#313338` | `#FFFFFF` | Message pane, chat header, settings content, dialogs, rail icons without artwork | background-primary |
| `color.surface.input` | `#383A40` | `#EBEDEF` | Composer, search fields | channel text area |
| `color.surface.field` | `#1E1F22` | `#EBEDEF` | Form text fields (always with `border.strong`) | input background |
| `color.surface.popover` | `#232428` | `#FFFFFF` | Profile popover, autocomplete, quick switcher | popout background |
| `color.surface.floating` | `#111214` | `#FFFFFF` | Context menus, tooltips | background-floating |
| `color.surface.code` | `#2B2D31` | `#F2F3F5` | Code blocks and inline code | — |
| `color.border.divider` | `#3F4147` | `#E3E5E8` | Decorative dividers, hover bar outline | — |
| `color.border.strong` | `#80848E` | `#80848E` | Form field and checkbox outlines (3:1) | Mellow addition |
| `color.border.quote` | `#4E5058` | `#D4D7DC` | Quote bar and reply spine (decorative) | — |
| `color.scrim` | black 70 % | black 60 % | Behind dialogs and drawers | — |

#### Interaction states

Discord uses solid shade changes, not translucent layers, and so does Mellow, so every state can be contrast-checked.

| Token | Dark | Light | Used for |
|---|---|---|---|
| `color.state.hover` | `#35373C` | `#DEE0E4` | Hover on channel, member, conversation and settings rows; icon button hover |
| `color.state.selected` | `#404249` | `#D4D7DC` | Selected channel, conversation or settings page; pressed rows |
| `color.state.hover-message` | `#2E3035` | `#F8F8F8` | Hover on a message row |

Selection is never conveyed by the background alone: selected rows also switch to `text.heading`, and the rail uses the pill.

#### Text

| Token | Dark | Light | Used for |
|---|---|---|---|
| `color.text.heading` | `#F2F3F5` | `#060607` | Headers, author names without role colour, unread and selected channels |
| `color.text.normal` | `#DBDEE1` | `#313338` | Message text, menu items, hovered channels |
| `color.text.secondary` | `#B5BAC1` | `#4E5058` | Member names without role colour, interactive icons |
| `color.text.muted` | `#A3A9B1` | `#5C5E66` | Read channels, timestamps, category labels, placeholders, activity lines, chat header topic |
| `color.text.channel-muted` | `#949BA4` | `#686A72` | Channels the user muted |
| `color.text.disabled` | `#4E5058` | `#80848E` | Disabled controls only (exempt, always paired with a reason) |
| `color.text.link` | `#33B8FF` | `#005FCC` | Links |
| `color.text.danger` | `#FA777C` | `#C4262B` | Error text, danger menu items, "Not sent" |
| `color.text.positive` | `#2DC770` | `#1A7F45` | Success text such as "Available" |
| `color.text.on-accent` | `#2B1608` | `#2B1608` | Text and icons on apricot |
| `color.text.on-danger`, `color.text.on-neutral` | `#FFFFFF` | `#FFFFFF` | Text on red, grey and green fills |

#### Accent, fills and mention

| Token | Dark | Light | Used for |
|---|---|---|---|
| `color.accent.fill` | `#F2A46E` | `#E8894B` | Primary buttons, selected or hovered Home, hovered rail icon without artwork, focused context menu item, jump bars, mention chip hover |
| `color.accent.fill-hover` | `#E8925A` | `#E07B39` | Accent fill on hover |
| `color.accent.fill-pressed` | `#D47F47` | `#CC6A2C` | Accent fill when pressed |
| `color.neutral.fill` | `#4E5058` | `#6D6F78` | Secondary buttons |
| `color.danger.fill` / `-hover` | `#DA373C` / `#A12829` | same | Destructive buttons, mention badges, "New" pill, danger menu item focus |
| `color.positive.fill` | `#23A55A` | `#1A7F45` | Add-server icon and its hover fill, success notice |
| `color.warning.fill` / `on-fill` | `#F0B232` / `#1E1F22` | same | Warning notice bar |
| `color.mention.background` | `#403C3C` | `#FDF4EE` | Message that mentions you (apricot at 8 % dark, 12 % light, flattened) |
| `color.mention.bar` | `#F2A46E` | `#A84F1A` | 2 px leading bar on a mentioning message |
| `color.mention.chip-background` | `#6B5548` | `#FBE6D6` | `@user`, `@role`, `#channel` chips in messages |
| `color.mention.chip-text` | `#FFE4D1` | `#7A3A0E` | Chip text |
| `color.unread.line` | `#F45A5E` | `#DA373C` | Unread divider rule |
| `color.unread.pill` | `#F2F3F5` | `#060607` | Rail and channel-list pills |
| `color.focus` | `#33B8FF` | `#005FCC` | Keyboard focus ring, 2 px wide with a 2 px gap |

The mention highlight follows Discord's pattern (tinted background plus a leading bar) but uses Mellow's apricot instead of Discord's gold, so "this is for you" is the brand's warmest moment. Mention chips stay distinguishable inside a highlighted message because their tint is almost four times stronger (chip on the highlighted background: 5.15:1 dark, 6.71:1 light).

The focus ring is sky blue, not apricot, so that "focused" and "selected" or "primary" are never confused.

#### Presence

Status colours follow Discord's conventions, with shapes, because these are generic conventions users already read.

| Token | Dark | Light | Shape (never colour alone) |
|---|---|---|---|
| `color.status.online` | `#23A55A` | `#1A7F45` | Filled circle |
| `color.status.idle` | `#F0B232` | `#A06E00` | Crescent moon |
| `color.status.dnd` | `#F45A5E` | `#DA373C` | Circle with a horizontal bar cut out |
| `color.status.offline` | `#8A8E98` | `#747881` | Hollow ring |

The indicator sits in a 3 px cut-out ring of the surface it is drawn on (including hover and selected row colours), so it separates from the avatar image. Mobile ("on phone") presence is out of scope for the MVP.

#### Verified contrast ratios (WCAG 2.2 AA)

Ratios were computed with the WCAG 2.x relative-luminance formula by the script that generated `design/tokens.json`; the same pairs are declared in the file so the token generator can re-check them. Body text needs 4.5:1; non-text indicators need 3:1.

| Pair | Dark | Light | Minimum |
|---|---|---|---|
| `text.heading` on `surface.chat` | 11.39 | 20.25 | 4.5 |
| `text.heading` on `surface.sidebar` | 12.42 | 18.24 | 4.5 |
| `text.heading` on `state.selected` | 9.03 | 14.04 | 4.5 |
| `text.heading` on `surface.sidebar-footer` | 13.96 | 17.26 | 4.5 |
| `text.normal` on `surface.chat` | 9.36 | 12.64 | 4.5 |
| `text.normal` on `state.hover-message` | 9.78 | 11.90 | 4.5 |
| `text.normal` on `mention.background` | 8.06 | 11.65 | 4.5 |
| `text.normal` on `state.hover` | 8.82 | 9.56 | 4.5 |
| `text.normal` on `surface.input` | 8.42 | 10.77 | 4.5 |
| `text.normal` on `surface.floating` | 13.88 | 12.64 | 4.5 |
| `text.normal` on `surface.popover` | 11.48 | 12.64 | 4.5 |
| `text.normal` on `surface.code` | 10.21 | 11.39 | 4.5 |
| `text.normal` on `surface.field` | 12.20 | 10.77 | 4.5 |
| `text.secondary` on `surface.chat` | 6.48 | 8.04 | 4.5 |
| `text.secondary` on `surface.sidebar` | 7.07 | 7.24 | 4.5 |
| `text.secondary` on `surface.sidebar-footer` | 7.94 | 6.85 | 4.5 |
| `text.secondary` on `surface.input` | 5.82 | 6.85 | 4.5 |
| `text.muted` on `surface.chat` | 5.34 | 6.47 | 4.5 |
| `text.muted` on `surface.sidebar` | 5.82 | 5.82 | 4.5 |
| `text.muted` on `surface.sidebar-footer` | 6.55 | 5.51 | 4.5 |
| `text.muted` on `surface.input` (placeholders) | 4.80 | 5.51 | 4.5 |
| `text.muted` on `surface.popover` | 6.55 | 6.47 | 4.5 |
| `text.muted` on `surface.floating` | 7.91 | 6.47 | 4.5 |
| `text.muted` on `surface.field` | 6.96 | 5.51 | 4.5 |
| `text.muted` on `state.hover-message` | 5.58 | 6.09 | 4.5 |
| `text.muted` on `mention.background` | 4.60 | 5.96 | 4.5 |
| `text.channel-muted` on `surface.sidebar` | 4.92 | 4.86 | 4.5 |
| `text.link` on `surface.chat` | 5.68 | 5.98 | 4.5 |
| `text.link` on `mention.background` | 4.89 | 5.52 | 4.5 |
| `text.link` on `state.hover-message` | 5.94 | 5.64 | 4.5 |
| `text.danger` on `surface.chat` | 4.81 | 5.74 | 4.5 |
| `text.danger` on `surface.sidebar` | 5.25 | 5.17 | 4.5 |
| `text.danger` on `surface.floating` | 7.14 | 5.74 | 4.5 |
| `text.danger` on `surface.popover` | 5.90 | 5.74 | 4.5 |
| `text.accent` on `surface.floating` ("Invite people") | 9.20 | 5.53 | 4.5 |
| `text.positive` on `surface.chat` | 5.73 | 5.04 | 4.5 |
| `text.positive` on `surface.sidebar` | 6.25 | 4.54 | 4.5 |
| `text.on-accent` on `accent.fill` | 8.44 | 6.64 | 4.5 |
| `text.on-accent` on `accent.fill-hover` | 7.12 | 5.78 | 4.5 |
| `text.on-accent` on `accent.fill-pressed` | 5.67 | 4.64 | 4.5 |
| `text.on-danger` on `danger.fill` (badges, "New" pill) | 4.57 | 4.57 | 4.5 |
| `text.on-danger` on `danger.fill-hover` | 7.37 | 7.37 | 4.5 |
| `text.on-neutral` on `neutral.fill` | 8.04 | 5.01 | 4.5 |
| `warning.on-fill` on `warning.fill` | 8.73 | 8.73 | 4.5 |
| `text.on-neutral` on `positive.fill` (add-server icon on hover) | 3.18 | 5.04 | 3.0 |
| `mention.chip-text` on `mention.chip-background` | 5.72 | 7.13 | 4.5 |
| `mention.bar` on `mention.background` | 5.34 | 5.09 | 3.0 |
| `mention.bar` on `surface.chat` | 6.21 | 5.53 | 3.0 |
| `unread.pill` on `surface.rail` | 14.84 | 16.05 | 3.0 |
| `unread.pill` on `surface.sidebar` | 12.42 | 18.24 | 3.0 |
| `unread.line` on `surface.chat` | 3.90 | 4.57 | 3.0 |
| `focus` on `surface.chat` | 5.68 | 5.98 | 3.0 |
| `focus` on `surface.sidebar` | 6.20 | 5.39 | 3.0 |
| `focus` on `surface.rail` | 7.41 | 4.74 | 3.0 |
| `focus` on `surface.input` | 5.11 | 5.10 | 3.0 |
| `border.strong` on `surface.chat` | 3.38 | 3.74 | 3.0 |
| `border.strong` on `surface.sidebar` | 3.68 | 3.37 | 3.0 |
| `positive.fill` on `surface.chat` (add-server icon) | 3.97 | 5.04 | 3.0 |
| `status.online` on `surface.sidebar` / `state.selected` | 4.34 / 3.15 | 4.54 / 3.49 | 3.0 |
| `status.idle` on `surface.sidebar` / `state.selected` | 7.30 / 5.31 | 4.00 / 3.08 | 3.0 |
| `status.dnd` on `surface.sidebar` / `state.selected` | 4.25 / 3.09 | 4.12 / 3.17 | 3.0 |
| `status.offline` on `surface.sidebar` / `state.selected` | 4.21 / 3.06 | 3.99 / 3.07 | 3.0 |
| `status.offline` on `surface.sidebar-footer` / `surface.popover` | 4.73 / 4.73 | 3.77 / 4.42 | 3.0 |

`color.border.divider` and `color.border.quote` are intentionally below 3:1. They are decorative and are never the only thing that shows where a control is or what a block is (quotes are also indented).

The composer field (`surface.input` on `surface.chat`, 1.15:1) has no 3:1 boundary, exactly like Discord. It is identified by its placeholder ("Message #general", 4.80:1), its attach button and its fixed position, which satisfies WCAG 1.4.11 because the boundary is not needed to identify the control. Form text fields, which have no such context, get a 1 px `border.strong` outline.

#### Deviations from Discord's greys and colours (AA adjustments)

Each deviation is the smallest change that makes the pair pass. Discord's values are given for reference; ratios are against the surface where Discord's value fails.

| Role | Discord value | Fails at | Mellow value | Now |
|---|---|---|---|---|
| Dark muted text (timestamps, placeholders, category labels) | `#949BA4` | 4.05 on input, 3.88 on mention highlight, 4.50 on chat (borderline) | `#A3A9B1` | 4.80, 4.60, 5.34 |
| Dark read-channel text | `#80848E` | 3.68 on sidebar | `#A3A9B1` (same as muted text) | 5.82 |
| Dark muted-channel text | `#4E5058` | 1.72 on sidebar | `#949BA4` | 4.92 |
| Dark composer placeholder | `#6D6F78` | 2.27 on input | `#A3A9B1` | 4.80 |
| Dark danger text | `#F23F43` | 3.35 on chat | `#FA777C` | 4.81 |
| Mention badge and "New" pill fill (white text) | `#F23F43` | 3.77 | `#DA373C` | 4.57 |
| Dark do-not-disturb dot | `#F23F43` | 2.66 on selected row | `#F45A5E` | 3.09 |
| Dark offline dot | `#80848E` | 2.68 on selected row | `#8A8E98` | 3.06 |
| Dark link | `#00A8FC` | 4.15 on mention highlight | `#33B8FF` | 4.89 |
| Offline members | Whole row at 30 % opacity | Text far below 4.5 | Avatar at 50 % opacity, text unchanged | ≥ 4.5 |
| Light online dot | `#23A55A` | 2.87 on sidebar | `#1A7F45` | 4.54 |
| Light idle dot | `#F0B232` | 1.70 on sidebar | `#A06E00` | 4.00 |
| Light offline dot | `#80848E` | 2.59 on selected row | `#747881` | 3.07 |
| Light danger text | `#DA373C` | 4.12 on sidebar | `#C4262B` | 5.17 |
| Light link | `#006CE7` | 4.40 on sidebar | `#005FCC` | 5.39 |

The muted-channel adjustment has a visible consequence: in Discord, muted channels are much dimmer than read channels; in Mellow the step is smaller (5.82 versus 4.92 in dark). Muted channels also never show the unread pill or the heading colour, and their accessible description says "muted", so the state does not rely on the shade alone.

#### User-chosen colours (role colours, default avatars)

Role colours are chosen by server admins and shown on author and member names. Discord shows them as-is, which often produces unreadable names. Mellow keeps the hue but adjusts lightness in OKLCH until the colour reaches 4.5:1 against the surface it is drawn on, separately for each theme. This adjustment lives in the shared Rust core so both clients render the same result. The role editor shows a live preview on both themes and says "Adjusted for readability" when the displayed colour differs from the picked one.

Default avatars (no uploaded picture) show the first grapheme of the display name on one of eight tints derived from the palette, picked deterministically from the user ID. Discord's mascot avatars are not reproduced. Each tint/initial pair is validated at 4.5:1 by the token generator.

### 2.4 Typography

All fonts are bundled with the apps and the admin web UI, so the product looks the same everywhere and works offline.

| Family | Licence | Role | Why |
|---|---|---|---|
| Figtree (variable, `wght`) | SIL OFL 1.1 | All UI and message text | Chosen by the user (v0.3): a geometric, friendly sans close to the feel of Discord's gg sans, very legible at 16 px, 63 KB per style, bundled in the desktop binary. |
| Noto Sans (variable, `wght`) | SIL OFL 1.1 | Fallback for scripts Figtree lacks (Greek, Cyrillic) | Discord's own font stack lists Noto Sans immediately after its proprietary gg sans, so it is what many Discord users already see for text gg sans does not cover; its humanist proportions, open apertures and x-height sit close to gg sans at Discord's sizes, so Discord's dimensions work unchanged. One family covers Latin, Greek and Cyrillic, and sibling Noto families cover other scripts with matching metrics. |
| Source Code Pro | SIL OFL 1.1 | Inline code, code blocks, invite codes, IDs in the admin UI | Also part of Discord's code font fallback stack; clear distinction of `0O`, `1lI`. |
| Twemoji (COLR font build) | CC-BY 4.0 (graphics) | Emoji | The emoji artwork Discord users recognise; open-licence with attribution in About. Noto Color Emoji is the fallback. See section 9. |
| Noto Sans CJK, Arabic, Hebrew, Devanagari (subsets to decide) | SIL OFL 1.1 | Script fallback | Harmonious metrics with Noto Sans. |

Rules:

- Weights used: 400, 500, 600, 700, 800. Emphasised Material 3 Expressive styles on Android add 100.
- Category headers, member-list group headers and form labels are rendered uppercase with 0.24 px tracking, as on Discord. The uppercase is a rendering transform (`$extensions.org.mellow.text-transform` on the token); source strings, translations and accessible names stay sentence case. Slint has no text transform, so on desktop the Rust layer passes an uppercased display string next to the sentence-case accessible name; Compose and CSS apply the transform at render time.
- No text is smaller than 12 px at 100 % zoom. Discord sets "(edited)" and its "NEW" pill at 10 px; Mellow uses 12 px.
- Numbers in tables (admin UI, member counts) use tabular figures.

#### Type scale tokens

Sizes are logical pixels on desktop and the admin web, and `sp` on Android. Line height is given in px here and stored as a multiplier in the token file.

| Token | Weight | Size / line height | Used for | Compose `Typography` slot |
|---|---|---|---|---|
| `type.display` | 800 | 32 / 40 | Channel-start hero ("Welcome to #general"), first-run title | `displaySmall` |
| `type.heading-xl` | 600 | 24 / 30 | Sign-in and connect card titles | `headlineSmall` |
| `type.heading-lg` | 600 | 20 / 24 | Dialog titles, settings page titles, quick switcher input | `titleLarge` |
| `type.heading-md` | 600 | 16 / 20 | Server name in sidebar header, channel name in chat header, popover display name | `titleMedium` |
| `type.body-lg` | 400 | 16 / 20 | Settings rows, form values | `bodyLarge` (outside messages) |
| `type.body` | 400 | 14 / 18 | Descriptions, helper text, dialog body, channel topic | `bodyMedium` |
| `type.body-sm` | 400 | 12 / 16 | Activity lines, user panel status, field helper text | `bodySmall` |
| `type.message` | 400 | 16 / 22 | Message text, composer text | extended `MellowTheme.typography.message` |
| `type.message-author` | 500 | 16 / 22 | Author name on the first message of a group | extended |
| `type.message-timestamp` | 500 | 12 / 22 | Timestamp after the author name, hover timestamp in the gutter, "(edited)" | extended |
| `type.message-h1` / `-h2` / `-h3` | 700 | 24/30, 20/26, 16/22 | Markdown headings inside messages | extended |
| `type.message-subtext` | 400 | 12 / 16 | Markdown subtext (`-# text`) | extended |
| `type.code` | 400 (mono) | 14 / 18 | Inline code, code blocks | extended |
| `type.channel-name` | 500 | 16 / 20 | Channel and conversation names in the sidebar | extended |
| `type.member-name` | 500 | 16 / 20 | Names in the member list | extended |
| `type.category` | 600, uppercase | 12 / 16 | Category headers, member groups, form labels | `labelMedium` |
| `type.user-panel-name` | 600 | 14 / 18 | Display name in the user panel | `titleSmall` |
| `type.button` | 500 | 14 / 16 | Button labels | `labelLarge` |
| `type.menu-item` | 500 | 14 / 18 | Context menu items | extended |
| `type.tooltip` | 600 | 14 / 16 | Tooltips | extended |
| `type.badge` | 700 | 12 / 16 | Mention badges, "New" pill | `labelSmall` |
| `type.typing` | 500 | 14 / 24 | Typing indicator | extended |

Scaling follows Discord's two settings:

- **Zoom level** (desktop, 50–200 %, `Ctrl+=`/`Ctrl+-`/`Ctrl+0`) scales every dimension and type token.
- **Chat font size** (12–24 px, default 16) scales the `type.message*` and `type.code` tokens only; the message gutter and avatar stay put, as on Discord.
- Android follows the system font scale (including non-linear scaling on Android 14+) and Discord-mobile's chat font size setting.

### 2.5 Spacing

A 4 px grid. Discord's odd values (2 px row gaps, 6 px icon gaps, 6 px menu padding) live in component tokens, not in the scale.

| Token | Value | Typical use |
|---|---|---|
| `space.3xs` | 2 | Message vertical padding, chip padding |
| `space.2xs` | 4 | Badge padding, menu separator margin |
| `space.xs` | 8 | Row margins and padding, sidebar list top padding |
| `space.sm` | 12 | Tooltip horizontal padding |
| `space.md` | 16 | Header padding, avatar inset, composer side margins, default message group gap |
| `space.lg` | 24 | Composer bottom band (typing indicator), dialog padding |
| `space.xl` | 32 | Auth card padding, settings section spacing |
| `space.2xl` | 48 | Message trailing padding (room for the hover bar) |
| `space.3xl` | 64 | Admin dashboard top spacing on wide screens |

Pane dimensions are in the `size.*` tokens and in the table of section 4.2.

### 2.6 Radius and shape

| Token | Value | Use |
|---|---|---|
| `radius.xs` | 3 | Buttons, text fields, mention chips, inline code |
| `radius.sm` | 4 | Channel and member rows, menus, tooltips, code blocks, hover bar, dialogs |
| `radius.md` | 8 | Composer field, attachments, jump bars, popovers |
| `radius.lg` | 16 | Hovered or selected server squircle (at 48 px), Android sheets |
| `radius.full` | 9999 | Avatars, badges, status dots, resting server icons |

Signature shape, copied from Discord: **server icons are circles at rest and morph into a squircle (`radius.lg`) on hover, focus and selection** (`component.rail.icon-radius-rest` 24 → `icon-radius-active` 16, `motion.duration.base`). Avatars are always circles.

### 2.7 Elevation

Depth comes from the layered greys; shadows only separate headers and floating layers.

| Token | Dark | Light | Use |
|---|---|---|---|
| `elevation.low` | 0 1 0 black 20 % | 0 1 0 ink 8 % | 1 px line under the sidebar header and chat header |
| `elevation.medium` | 0 4 4 black 16 % | 0 4 4 black 10 % | Message hover bar, popovers, jump bars |
| `elevation.high` | 0 8 16 black 24 % | 0 8 16 black 16 % | Context menus, tooltips, dialogs, quick switcher |

Values are `x y blur colour`. Slint maps them to `drop-shadow-*` (one layer), Compose to `shadowElevation`, CSS to `box-shadow`.

### 2.8 Motion

Discord's motion is quick and functional: colour changes on hover, the squircle morph, the pill growing, menus popping in. Mellow copies that and nothing more.

| Token | Value | Use |
|---|---|---|
| `motion.duration.instant` | 0 ms | Reduced-motion replacement for spatial motion |
| `motion.duration.fast` | 100 ms | Hover and pressed colour changes, focus ring |
| `motion.duration.base` | 200 ms | Squircle morph, rail pill height, category collapse, menus, tooltips |
| `motion.duration.slow` | 250 ms | Dialogs (scale 0.95 → 1 with fade), Android panel settle |
| `motion.duration.highlight` | 2000 ms | Fade-out of the highlight on a message you jumped to |
| `motion.delay.tooltip` / `tooltip-rail` | 500 ms / 0 ms | Tooltip delay; rail tooltips appear immediately, as on Discord |
| `motion.delay.skeleton` | 300 ms | Before skeletons appear, to avoid flicker |
| `motion.delay.pending-label` | 2000 ms | Before a pending message shows "Sending…" |
| `motion.easing.standard` | cubic-bezier(0.2, 0, 0, 1) | Most transitions |
| `motion.easing.enter` / `exit` | (0, 0, 0, 1) / (0.3, 0, 1, 1) | Appearing / leaving elements |

Rules:

- No per-item entrance animation in lists; new messages appear in place.
- Loading placeholders are static (no shimmer).
- Animated GIFs, avatars and emoji play on hover or focus by default (Discord's "Play animated emoji/GIFs when the app is focused" setting maps to the "Play animated images" setting in 5.9).
- Android uses `MotionScheme.standard()` and the expressive spring only for the server squircle morph and the chat panel settle.

Reduced motion:

- Desktop reads the OS preference where available (macOS "Reduce motion", Windows "Show animations in Windows", GNOME `enable-animations`) and offers an in-app override: "Match system", "Reduce motion", "Allow motion".
- Android follows "Remove animations" and the same in-app override. The admin web uses `@media (prefers-reduced-motion: reduce)`.
- When reduced motion is on: the squircle morph, pill growth, panel slides and smooth scrolling become `motion.duration.instant`; colour changes are kept; "jump to message" scrolls instantly and keeps the highlight as a static background for `motion.duration.highlight`; animated media never autoplays; the typing indicator shows static dots.

### 2.9 Iconography

One icon set everywhere: **Phosphor Icons** (MIT), **Fill** weight, which matches the solid, compact feel of Discord's icons without copying their artwork. Sizes: 20 px in rows and the hover bar, 24 px in headers, the composer, the user panel and on Android. Icons are exported as SVG for Slint and the admin web UI, and converted to `ImageVector` for Compose. Emoji are never used as icons.

The **Home button** shows the Mellow mark (28 px glyph inside the 48 px rail icon), in `text.normal` at rest and `text.on-accent` on the apricot fill when hovered or selected. The mark must not resemble Discord's logo or mascot (section 9).

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

1. **Reference tokens** (`ref.*`): raw values with no meaning, for example `ref.apricot.300 = #F2A46E`, `ref.grey.700 = #313338`. Grey steps are numbered by lightness (0 = white, 950 = near black). Flattened tints (`ref.apricot.wash-dark`, `chip-dark`, …) are also reference values, so semantic tokens never hold raw hex. Only semantic tokens may reference this tier.
2. **Semantic tokens** (`color.*`, `font.*`, `type.*`, `space.*`, `radius.*`, `size.*`, `elevation.*`, `motion.*`): the tokens documented in section 2 and in the dimension table of 4.2.
3. **Component tokens** (`component.<name>.*`): aliases of semantic tokens for the components that define the Discord look: `rail`, `sidebar`, `category`, `channel-row`, `user-panel`, `chat-header`, `message`, `action-bar`, `mention`, `unread-divider`, `jump-bar`, `composer`, `member-list`, `member-row`, `status`, `badge`, `tooltip`, `context-menu`, `button`, `focus-ring`. Components read this tier first and the semantic tier for anything not listed.

### 3.3 Themes and naming

- **Themes are values, not names.** A colour token's `$value` is the dark value (dark is the default theme); its light value is in `$extensions["org.mellow.theme"].light`. Elevation tokens use the same extension. Aliases resolve within the active theme, so component tokens need no light value of their own.
- Source names are dot-separated, lowercase, kebab-case within a segment: `color.text.on-accent`, `component.channel-row.text-selected`.
- Names describe the role, never the hue: `color.danger.fill`, not `color.red`.
- Uppercase rendering is a typography extension (`org.mellow.text-transform: uppercase`), never baked into strings.

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
- We build Mellow components on Slint primitives so the Discord look is identical on every OS; std-widgets are used only for behaviour we cannot reasonably rebuild (for example scrolling), styled with our tokens. Our colour global is `MellowColor` because std-widgets already export `Palette`.

#### Compose (Android, Material 3 Expressive)

The generator writes `MellowTokens.kt` containing the light and dark `ColorScheme`, the `Typography`, the `Shapes`, and an extended `MellowColors`/`MellowSpacing`/`MellowSizes`/`MellowComponents` object provided through a `CompositionLocal`. `MellowTheme { }` wraps `MaterialExpressiveTheme` (or `MaterialTheme` if the expressive theme is not available in the pinned version) with `motionScheme = MotionScheme.standard()`. Dynamic colour is off (section 9).

| Material role | Mellow token |
|---|---|
| `primary` / `onPrimary` | `accent.fill` / `text.on-accent` |
| `primaryContainer` / `onPrimaryContainer` | `mention.chip-background` / `mention.chip-text` |
| `secondary` / `onSecondary` | `neutral.fill` / `text.on-neutral` |
| `secondaryContainer` / `onSecondaryContainer` (navigation indicator) | `state.selected` / `text.heading` |
| `tertiary` / `onTertiary` | `text.link` / `surface.chat` |
| `error` / `onError`, `errorContainer` / `onErrorContainer` | `danger.fill` / `text.on-danger` |
| `background`, `surface` / `onBackground`, `onSurface` | `surface.chat` / `text.normal` |
| `onSurfaceVariant` | `text.muted` |
| `surfaceContainerLowest`, `surfaceDim` | `surface.rail` |
| `surfaceContainerLow` | `surface.sidebar-footer` |
| `surfaceContainer` | `surface.sidebar` |
| `surfaceContainerHigh` | `surface.input` |
| `surfaceContainerHighest`, `surfaceBright` | `state.selected` |
| `outline` / `outlineVariant` | `border.strong` / `border.divider` |
| `inverseSurface` / `inverseOnSurface` / `inversePrimary` | the other theme's `surface.chat` / `text.normal` / `accent.fill` |
| `scrim` | `scrim` |

Menus, tooltips and the message hover bar do not rely on Material defaults; they read `MellowTheme.components.*`. `Shapes`: `extraSmall = radius.xs`, `small = radius.sm`, `medium = radius.md`, `large = radius.lg`, `extraLarge = radius.lg`. Screens never contain raw `dp` values.

#### Admin web (CSS custom properties)

- `:root` holds the dark values, `@media (prefers-color-scheme: light)` and `[data-theme="light"]` hold the light values, `[data-theme="dark"]` forces dark.
- Every token becomes `--mw-<path>`; spacing and type sizes are emitted in `rem` (16 px base) so browser zoom works.
- Motion durations get `@media (prefers-reduced-motion: reduce)` overrides; uppercase tokens emit `text-transform: uppercase`.
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

The reference is Discord's desktop client in its dark theme, as most users know it. Every dimension below is a token.

#### Dimension table

| Element | Dimension | Token(s) |
|---|---|---|
| Window minimum / default | 480 × 560 / 1280 × 800 (or 80 % of the screen if smaller) | `size.window.*` |
| Server rail width | 72 | `size.rail.width` |
| Rail icon | 48 × 48; circle at rest (radius 24), squircle radius 16 on hover, focus, selection | `size.rail.icon`, `component.rail.icon-radius-*` |
| Rail item gap / top padding | 8 / 12 | `size.rail.gap`, `size.rail.padding-top` |
| Rail pill | 8 wide (4 visible at the rail edge); 8 high unread, 20 hover, 40 selected | `component.rail.pill-*` |
| Rail separator | 32 × 2, centred | `size.rail.separator-*` |
| Mention badge | 16 high, min 16 wide, 3 px cut-out ring, bottom trailing corner of the icon | `component.badge.*` |
| Channel sidebar width | 240 (resizable 240–360, Mellow addition) | `size.sidebar.*` |
| Sidebar header (server name) | 48 high, 16 horizontal padding, `elevation.low` below | `size.sidebar.header-height` |
| Category header | 24 high, 16 top margin, 8 padding, 12 px chevron | `component.category.*` |
| Channel row | 32 high, 2 gap (34 pitch), 8 margin and 8 padding, radius 4, 20 px icon, 6 px icon gap | `component.channel-row.*` |
| Channel unread pill | 8 × 8 (4 visible) at the sidebar's leading edge | `component.channel-row.unread-pill-*` |
| User panel | 52 high, 8 padding, 32 px avatar, 32 px buttons | `size.user-panel.height`, `component.user-panel.*` |
| Chat header | 48 high, 16 padding, 24 px icons, `elevation.low` below | `size.chat.header-height` |
| Chat pane minimum width | 480 | `size.chat.min-width` |
| Message avatar | 40, inset 16 from the pane edge; text gutter 72; trailing padding 48 | `size.message.*`, `component.message.*` |
| Message row | 2 px top and bottom padding, 22 px line height | `component.message.padding-block` |
| Space between message groups | 16 by default (setting: 0, 4, 8, 16, 24) | `component.message.group-gap` |
| Reply reference | 16 px avatar, 2 px spine from the avatar centre | `component.message.reply-*` |
| Hover action bar | 32 high, 32 px buttons, 20 px icons, offset 16 above the row top and 16 from the trailing edge | `component.action-bar.*` |
| Mention bar | 2 wide | `component.mention.bar-width` |
| Composer | min 44 high (grows to 50 % of the pane), radius 8, 16 margin each side, 24 bottom band for the typing indicator | `component.composer.*` |
| Jump bars | 32 high, radius 8 | `component.jump-bar.*` |
| Member list width | 240 (resizable 240–320, Mellow addition) | `size.member-list.*` |
| Member group header | 40 high, label at the bottom | `component.member-list.group-height` |
| Member row | 42 high, 2 gap, 8 margin and padding, 32 px avatar, radius 4 | `component.member-row.*` |
| Status dot | 10 on 32 px avatars, 16 on 80 px avatars, 3 px ring | `component.status.*` |
| Profile popover | 300 wide | `size.popover.profile-width` |
| Quick switcher / small dialog | 570 / 440 wide | `size.dialog.*` |
| Tooltip | max 190 wide, 8 × 12 padding, 5 px arrow | `component.tooltip.*` |
| Context menu | 188–320 wide, 6 padding, 32 px items | `component.context-menu.*` |

#### Breakpoints

The full layout needs 72 + 240 + 480 + 240 = 1032 px.

| Window width | Layout |
|---|---|
| ≥ 1032 | Rail + channel sidebar + chat + member list (member list shown by default, toggled with `Ctrl+U`, remembered per server) |
| 792–1031 | Rail + channel sidebar + chat; the member list opens as an overlay on the trailing edge |
| 480–791 | Chat only; rail and channel sidebar open together as a drawer from the leading edge (header button, or `F6` into the navigation regions). This is a Mellow addition for half-screen tiles and tiling window managers; Discord's minimum width is larger. |

#### Layout (≥ 1032, cozy, dark)

```
  72         240                         flexible (≥ 480)                          240
┌──────┬──────────────────┬───────────────────────────────────────────────┬──────────────────┐
│ (M)  │ Rust Lyon      ⌄ │ # general │ Talk about anything      ⌕ ⚲ ☰   │                  │ 48
│ ──── │──────────────────│───────────────────────────────────────────────│ MODERATORS — 2   │
│▌(RL) │ ⌄ GENERAL        │                                               │ (A) Ana          │
│ (MB)③│   # welcome      │ (A) Ana  Today at 14:02                       │ (L) Léo   ◐      │
│▪(XY) │ ▌ # general      │     Has anyone tried Slint on Wayland?        │                  │
│      │   # off-topic    │     It works for me with winit.               │ ONLINE — 12      │
│  +   │ ⌄ PROJECTS       │ ──────────────────────────────────── [NEW] ── │ (P) Paul         │
│      │▪  # mellow    ③  │▌(L) Léo  Today at 14:33          [↩ ⧉ ⋯]      │                  │
│      │                  │▌    @Paul can you review the PR?              │ OFFLINE — 40     │
│      │                  │ ┌───────────────────────────────────────────┐ │ (o) Zoé          │
│      │──────────────────│ │ ⊕  Message #general                       │ │                  │ 44
│      │ (P) Paul     ⚙   │ └───────────────────────────────────────────┘ │                  │
│      │     Online       │  Ana is typing…                               │                  │ 24
└──────┴──────────────────┴───────────────────────────────────────────────┴──────────────────┘
                user panel 52
```

Legend:

- `(M)` is Home with the Mellow mark; the separator below it is the rail separator.
- In the rail, `▌` is the tall selected pill (40) and `▪` the short unread pill (8). `③` is a red mention badge.
- In the sidebar, `▌` is the selected row (`state.selected`, heading text), `▪` the channel unread pill, `③` a mention badge.
- `[NEW]` is the red "New" pill at the trailing end of the unread divider.
- `[↩ ⧉ ⋯]` is the hover action bar (Reply, Copy text, More; Edit replaces Reply's neighbour on your own messages, see 5.4).
- `▌` in the chat marks the 2 px apricot mention bar on a message that mentions you.
- The header holds the channel icon and name (`type.heading-md`), a 1 × 24 divider, the topic (`type.body`, `text.muted`, single line, opens fully on click), then search (reserved slot, section 9), pinned messages (reserved slot) and the member list toggle.

#### Connection dock

Discord shows voice connection status in a panel directly above the user panel. Mellow uses the same slot (`surface.sidebar-footer`, same width) for connection and sync status (section 5.2). When voice arrives, it becomes the voice status panel without a layout change.

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

### 4.4 Admin web UI (server-rendered HTML + htmx)

Purpose: instance administrators check health and manage users, servers, invites and settings. Usage is occasional, often from a laptop, sometimes from a phone. The layout reuses Discord's settings screen, so admins recognise it: a navigation column on `surface.sidebar` and content on `surface.chat`, with the same palette, font and uppercase section labels.

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

All steps use Discord's authentication layout: a centred card (480 px wide, `surface.chat`, `radius.sm`, `space.xl` padding, `elevation.high`) on a `surface.rail` backdrop, without illustration. Titles use `type.heading-xl`, field labels `type.category`.

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

### 5.2 Server rail

Contents, top to bottom: Home (Mellow mark; mention badge aggregating conversations from all instances), unread conversations as avatars (as on Discord, shown only while unread), rail separator, the servers of the first instance, then for each further instance a separator, its instance icon and its servers, then `+` (add) in green.

| Element state | Presentation |
|---|---|
| Rest | Server artwork in a 48 px circle; servers without artwork show initials in `text.normal` on `surface.chat` |
| Hover / focus | Morph to squircle (radius 16); icons without artwork switch to `accent.fill` with `text.on-accent`; pill grows to 20; tooltip with the server name (and instance when ≥ 2) appears immediately, to the trailing side |
| Unread messages | Pill 8 high |
| Mentions | Red badge with the count (`danger.fill`, `text.on-danger`, 3 px ring of `surface.rail`) at the bottom trailing corner; "99+" beyond 99; also unread pill |
| Selected | Squircle, pill 40 high; icons without artwork stay on `accent.fill` |
| Home selected | Squircle on `accent.fill`, Mellow mark in `text.on-accent`, pill 40 |
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
| Category | `type.category` (uppercase), chevron, collapsible by click or `Left`/`Right`; collapsed categories still show unread and selected channels, as on Discord |

Channel context menu: Mark as read, Mute channel, Notification settings, Copy channel link, Edit channel (if allowed), Delete channel (danger, if allowed).

States:

- Loading: 6–8 static row skeletons with varied widths.
- Empty server (no channels, user can manage): "No channels yet" + "Create a channel". User cannot manage: "No channels you can see yet. Ask a moderator for access."
- Error: inline banner "Couldn't load channels for Rust Lyon." with "Try again", while keeping cached channels.

User panel (52 px, `surface.sidebar-footer`): avatar 32 with status dot, display name (`type.user-panel-name`), a second line with the custom status or `@username` (and `@instance` when ≥ 2 instances) in `text.muted`, and a settings button. Clicking the avatar or name opens the status menu (Online, Idle, Do not disturb, Invisible, custom status). The settings button opens the user settings screen (5.9); closing it returns focus to the button (to the composer when the sidebar is hidden below 792 px). While an app update is available or ready to install, the button carries a 10 px dot in `component.user-panel.update-badge` (`positive.fill`, 3 px cut-out ring in the panel colour, top trailing corner, like presence dots) and its accessible label becomes "User settings, update available"; the dot is a shape on the button, so it does not rely on colour alone.

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

- When opening a channel with unread messages, the list scrolls to the first unread message and shows the divider: a 1 px `unread.line` rule with a red "New" pill (rendered uppercase, `type.badge`) at its trailing end, as on Discord. The accessible name is "New messages since 14:32".
- A bar pinned to the top of the pane, in `accent.fill` with `text.on-accent` (Discord's blurple bar): "12 new messages since 14:32" and "Mark as read" (`Esc`). It disappears when the divider has been seen and the user reaches the bottom.
- When the user scrolls up, a bar appears directly above the composer, in `accent.fill`: "You're viewing older messages" and "Jump to present" (or "Jump to present · 3 new" when new messages arrived). `End` (list focused) or `Esc` (composer focused) jumps. The list never auto-scrolls while the user reads older messages.

#### Replies

- A reply shows a one-line reference above the message: a 2 px `border.quote` spine curving from the avatar column, the replied author's 16 px avatar and name, and a single-line excerpt in `text.secondary`. Clicking it or pressing `Enter` on it jumps to the original and highlights it with `mention.background` fading over `motion.duration.highlight`.
- If the original was deleted: "Original message was deleted" in `text.muted`, not clickable.
- If the original is not loaded yet, jumping loads the surrounding history first (with skeletons) and keeps "Jump to present" available. Jumping to a message whose row height is only estimated by `ListView` must land exactly on the message (ADR-0010 note).

#### Mentions

- A message that mentions you (directly, via a role you have, or `@everyone`) gets `mention.background` and a 2 px `mention.bar` on the leading edge. The bar ensures the state is not carried by the background alone; the accessible name starts with "Mentions you".
- Mention chips inside text (`@Ana`, `@Moderators`, `#general`) use `mention.chip-background` / `mention.chip-text` with `radius.xs`; on hover and focus they switch to `accent.fill` / `text.on-accent`. They are focusable and open the profile, role description or channel.

#### Hover action bar

On hover or focus, a bar appears at the trailing top of the message (`component.action-bar.*`: `surface.chat`, 1 px `border.divider`, `elevation.medium`). Buttons, each 32 px with a tooltip: Reply (others' messages) or Edit (your own), Copy text, More. More opens the message context menu: Reply, Edit (own), Copy text, Select text, Copy message link, Mark unread, Delete (own, or with "Manage messages", danger, last), plus moderation actions for moderators. Reactions, pins and forwarding are not in the MVP (section 9). Right-click on a message opens the same menu.

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

### 5.5 Composer

Layout: Discord's composer, a `surface.input` field with `radius.md`, min 44 px, 16 px side margins and a 24 px band below for the typing indicator. Attach button (`⊕`, 24 px) on the leading side; emoji button on the trailing side when the emoji picker ships (section 9). Desktop has no send button by default, as on Discord (`Enter` sends); the accessibility setting "Show send button" adds one. Android always shows the send button.

- Placeholder: "Message #general", "Message @Ana" in conversations, or "Message #general on chat.example.org" when ≥ 2 instances are connected.
- Markdown is typed as plain text (the composer field cannot render inline styles, section 9) and rendered in messages as in 5.4.
- Shortcuts: `Ctrl+B` and `Ctrl+I` wrap the selection in bold and italic markers. `Ctrl+E` is reserved for the emoji picker, as on Discord.
- Autocomplete popovers (`surface.popover`) for `@`, `#` and `:` (emoji shortcodes), above the field, navigable with arrows, `Tab`/`Enter` to insert, `Esc` to close.
- Sending: `Enter` sends, `Shift+Enter` inserts a new line (desktop). Android: Enter inserts a new line and the send button sends; a setting swaps this for hardware keyboards.
- Edit mode: `Up` in an empty composer edits your last message inline. The message shows "Esc to cancel · Enter to save" with the two words as link buttons, as on Discord. Saving an empty message asks to delete it. Desktop: the field (`component.message.editor-*`, `surface.input`, `radius.md`) takes focus with the caret at the end; `Shift+Enter` inserts a new line; saving unchanged text just closes the editor; focus returns to the composer.
- Reply mode: a bar attached above the field (`surface.sidebar`): "Replying to Ana", a "@ On/Off" toggle for pinging the author (on by default, accessible label "Mention Ana"), and a close button (`Esc`). Desktop (`component.composer.reply-bar-*`): the field's top corners square off under the bar, the composer takes focus when a reply starts, and the close button is a filled x-circle labelled "Cancel reply"; the "@ On/Off" toggle comes with mention notifications.
- Action errors: when an edit or delete fails, one line in `text.danger` sits above the composer (or the reply bar) and is announced assertively.
- Attachments: attach button, drag and drop anywhere on the chat pane (overlay "Drop to attach to #general"), or paste. An attachment tray above the field shows thumbnails with "Add a description" (alt text), "Mark as spoiler" and "Remove". Upload progress is shown per file. Upload errors: "photo.heic is 48 MB. chat.example.org accepts files up to 25 MB."
- Length: a counter appears at 90 % of the instance limit ("3,620 / 4,000"); beyond the limit it turns `text.danger`, sending is blocked and the field's accessible description says "Message is 120 characters too long."
- Typing indicator: in the 24 px band, `type.typing`, `text.secondary`, with three dots: "Ana is typing…", "Ana and Léo are typing…", "Ana, Léo and 2 others are typing…", "Several people are typing…" (beyond 5). It is not announced by screen readers.
- Read-only channel (no "Send messages"): the field is replaced, as on Discord, by "You do not have permission to send messages in this channel." in `component.composer.locked-text` (`text.muted`, 4.80 dark / 5.51 light on `surface.input`) on the same `surface.input` field, same size and position, without the attach button. It is plain text for screen readers, never focused; nothing that would focus the composer (a reply starting, "Jump to present", "Try again", closing a dialog) moves focus there. The reply bar never shows, and `Up` to edit is unavailable since it lives in the composer. At narrow widths the sentence wraps and the field grows to fit it.
- Offline: the composer stays usable; sent messages are queued (5.4).
- Draft: unsent text and reply state are kept per channel across channel switches and app restarts.

### 5.6 Conversations (Messages)

- Home shows conversations in the channel-sidebar column, as on Discord: a "Find or start a conversation" button at the top (opens the quick switcher), then a "Direct messages" category header with a `+` button ("New message"), then conversations sorted by last activity, each a 42 px row with avatar and status dot (group: stacked avatars), name, and the instance label when ≥ 2 instances are connected. Unread conversations use `text.heading` and also appear as avatars at the top of the rail.
- New message: a picker to choose one or more people (from shared servers and previous conversations on the same instance), with a visible limit "Up to 9 people" for groups. Conversations cannot mix instances; the picker explains: "People from different instances can't be in the same conversation yet."
- Group conversation header: name (auto-generated until renamed), member avatars, actions "Add people", "Rename", "Leave conversation".
- Empty: "No conversations yet. Start one with someone from your servers." with "New message".
- Desktop, until "New message" and the quick switcher exist: the header reads "Direct messages" as plain text (no menu), the list starts with the "Direct messages" category header (no `+`), rows share the channel rows' colours, unread pill and selection, and the empty state has no button, since conversations start from a member's profile popout. In a conversation the chat header shows the `at` icon and the name, the hero shows the name as its title and "This is the beginning of your direct message history with @Ana.", errors read "Couldn't load messages with @Ana.", and the member list and its toggle are hidden.
- Error and offline: as in 5.4.

### 5.7 Member list and profile

- Groups: hoisted roles in role order ("Moderators — 2"), then "Online — 12", then "Offline — 40", in `type.category` (uppercase), `text.muted`, 40 px headers.
- Row (42 px): avatar 32 with status dot, display name (`type.member-name`, role colour adjusted, otherwise `text.secondary`; `text.normal` on hover), activity or custom status on a second line in `type.body-sm`, `text.muted`.
- Offline members: avatar at 50 % opacity, text at full contrast (Discord dims the whole row; see 2.3).
- Accessible name: "Ana, online, Moderator, custom status: Reviewing PRs".
- Large servers: offline members are not listed above 1,000 members ("Offline members are hidden in large servers.").
- Loading: row skeletons. Error: "Couldn't load members." with "Try again".
- Desktop, first version: click, `Enter` or `Space` on a member row opens a reduced popout to the leading side of the member list, top-aligned with the row (`component.profile-popover.*`): banner strip, avatar 80, display name, and "Message" (primary, full width, focused on open; accessible label "Message Ana") for anyone but yourself. "Message" opens or creates the conversation and moves focus to its composer. `Esc` or a click outside closes it, and the row shows `state.selected` while it is open.
- Profile popover (click, `Enter`, or context menu → "Profile"): 300 px, `surface.popover`, banner strip in the user's accent or a default grey, avatar 80, display name (`type.heading-md`), `@username@instance`, about text, roles as chips, member since, actions "Message" (primary), "Mention", and for moderators a "Moderation" menu (Kick, Ban, Manage roles).

### 5.8 Server settings

Discord's full-screen settings layout: navigation on `surface.sidebar` (right-aligned column on wide windows), content on `surface.chat` capped at 740 px, a close button with the label "Esc" at the trailing top. Sections: Overview, Roles, Members, Channels, Invites, Bans, Moderation log, then "Delete server" (owner only, `text.danger`, last).

**Unsaved changes**: a bar slides up at the bottom of the content (`surface.floating`, `elevation.high`): "Careful — you have unsaved changes." with "Reset" and "Save changes". Leaving with unsaved changes shakes the bar (instant under reduced motion) and turns it red-bordered, as on Discord, and the bar's text becomes "Save or reset your changes before leaving." The dialog alternative is used on Android: "Leave without saving? Your changes to Roles will be lost." with "Keep editing" and "Leave without saving".

**Roles editor**:

- Left: role list in hierarchy order, each with colour dot and member count. Reorder by drag and drop, `Ctrl+Shift+Up/Down`, or "Move up"/"Move down". Roles above your highest role are shown but locked, with the reason in the tooltip and accessible description: "You can only edit roles below your highest role."
- Right: tabs "Display", "Permissions", "Members".
- Display: name, colour picker (with the readability preview on both themes), "Show separately in the member list", "Allow anyone to mention this role".
- Permissions: grouped (General, Membership, Text channels, Moderation, Advanced). Each permission is a switch row with a plain-language description, for example "Ban members — Remove people and stop them from rejoining." Sensitive permissions (Administrator, Ban members, Manage roles) carry a warning icon and the word "Sensitive".
- Channel overrides (in channel settings): each permission is a three-state control "Allow / Inherit / Deny" with icons (check, slash, cross) and text labels, never colours alone.
- Empty (only @everyone): "Roles let you group people and give them permissions." with "Create a role".

**Desktop, first slice (2026-10-21)**: the server header turns into a menu button (chevron) only for members who can open settings; Roles is the only section. `@everyone` hides the colour picker and "Delete role", and its name field is read-only ("@everyone can't be renamed."). A locked role shows a lock and "You can only edit roles below your highest role." above the tabs. With only `@everyone`, the empty-state text and "Create a role" sit above the list, and `@everyone` stays editable below. Colours come from presets only, as radio buttons marked by a check and a ring.

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

Desktop, first iteration (`UserSettingsScreen` in `user_settings.slint`, `component.settings.*`): the screen covers the whole window and hides the shell, so `Tab` stays inside it and screen readers do not reach the panes behind. The navigation column (218 px, 60 px top padding, 32 px items with a 2 px gap, `type.channel-name`) sits right-aligned in a `surface.sidebar` area that grows with the window; the content area grows too, and the page is capped at 740 px with 60 px top and 40 px side padding (16 px below 792 px). The close button (36 px circle, 2 px `text.secondary` border, 18 px `x` icon) with "Esc" under it sits 24 px after the page (8 px below 792 px). Navigation: section label "App settings" (rendered "APP SETTINGS" with `type.category`), "About" (selected), a separator, then "Sign out" (danger, last), which signs out at once and closes the screen, as the old menu item did. "Sign out" fills `danger.fill` with `text.on-danger` on hover, like danger menu items, because `text.danger` on `state.hover` is 4.34:1 in the light theme; keyboard focus shows only the ring.

Keyboard and screen readers: focus moves to "About" when the screen opens; `Esc` closes it from anywhere inside. The navigation is a navigation landmark labelled "User settings" holding a tab list ("App settings"); the page is a tab panel labelled "About"; the close button is labelled "Close settings" with the description "Esc". Update status changes are announced through a polite live region that exists from the start.

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

### 5.10 Quick switcher

`Ctrl+K` (or "Find or start a conversation") opens Discord's quick switcher: a 570 px dialog on `surface.popover`, `elevation.high`, with a large input ("Where would you like to go?", `type.heading-lg`) and results grouped by type (conversations, channels, servers), each with an icon, the server or instance name in `text.muted` and an unread or mention indicator. Prefixes filter as on Discord: `@` people, `#` text channels, `*` servers. Arrows move, `Enter` opens, `Esc` closes. Empty: "No results for 'anx'. Check the spelling or search another server." The result list is a listbox; the count is announced politely ("5 results").

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
| 6.1 | Button | Primary (`accent.fill`), Secondary (`neutral.fill`), Danger (`danger.fill`), Link (text only, `text.link`) · sizes 38 (default), 32 (small), 44 (large and web), 48 on Android | Default, hover, pressed, focus, disabled, loading (label kept, progress inline) | `radius.xs`, `type.button`. One primary button per view. Labels are verbs ("Save changes"). Desktop: disabled and loading buttons dim their fill and label to `button.disabled-opacity` (the focus ring keeps full contrast) and stay focusable, so focus is not lost while an action runs. |
| 6.2 | IconButton | Standard, Toggle, Danger | Same as Button, plus selected | `text.secondary` icon, `state.hover` background; always a tooltip and an accessible label; target ≥ `size.hit.*`. |
| 6.3 | TextField | Single line, Multiline, Password (show/hide), Search (with clear) | Default, hover, focus, filled, error (message below, `text.danger` icon and text), disabled, read-only | `surface.field` with 1 px `border.strong`, `radius.xs`; uppercase label above, never placeholder-only. |
| 6.4 | Select | Single select | Closed, open, focus, error, disabled | Options use the ContextMenu visuals. |
| 6.5 | Checkbox, Switch, Radio group | — | Off, on, mixed (checkbox), focus, disabled | Switch on: `positive.fill` track with a check glyph, as on Discord; off: `neutral.fill` with a cross glyph, so state never relies on colour. |
| 6.6 | SegmentedControl | 2–4 options; tri-state permission (Allow / Inherit / Deny) | Selected, focus, disabled | Icon + text in each segment. |
| 6.7 | Slider | Continuous, Stepped (chat font size 12–24, zoom 50–200 %) | Focus, dragging, disabled | Arrow keys step; value announced. |
| 6.8 | Avatar | Sizes 16, 24, 32, 40, 80 · image, initials fallback, group (stacked) | Loading (tint block), with status, animated (play on hover/focus) | Always a circle. |
| 6.9 | StatusIndicator | Online, idle, dnd, offline | — | Shape + colour; cut-out ring of the surface below. |
| 6.10 | Skeleton | Text line, avatar, message, row, server icon, table row | Static | `state.hover` fill in light, `state.selected` in dark (`state.hover` is nearly invisible on `surface.chat`); hidden from screen readers; container announces "Loading messages". |
| 6.11 | ServerIcon | Server, instance, Home, unread conversation, Add (`+`) | Rest (circle), hover/focus (squircle), unread pill, mention badge, selected, offline, dragging | `component.rail.*`; morph with `MaterialShapes` on Android. |
| 6.12 | RailPill | Unread (8), hover (20), selected (40) | — | `unread.pill`; height animates over `motion.duration.base`. |
| 6.13 | ChannelRow | Text channel, category header | Read, hover, unread, mentions, selected, muted, locked, focus, dragging | `component.channel-row.*`, `component.category.*`. |
| 6.14 | ConversationRow | 1:1, group | Read, unread, selected, offline instance | Instance label when ≥ 2 instances. |
| 6.15 | UserPanel | — | Default, status menu open, offline | `component.user-panel.*`. |
| 6.16 | ChatHeader | Channel, conversation, group | Topic collapsed / expanded | `component.chat-header.*`. |
| 6.17 | MessageRow | Group start (cozy/compact), continuation, system message (joins, pins) | Default, hover, focus, mentioned, highlighted (jump target), selecting text, pending, queued, failed, editing | Accessible name includes the author even on continuations. |
| 6.18 | MessageBlock | Paragraph, heading, subtext, quote, code block, list, spoiler | Code block: hover shows "Copy code" | Rendered per block (5.4). |
| 6.19 | MessageActionBar | Own message, other's message | Button hover, focus | `component.action-bar.*`; mirrors the context menu. |
| 6.20 | ReplyReference | Normal, original deleted, original not loaded | Hover, focus | Jumps to the original. |
| 6.21 | MentionChip | User, role, channel, `@everyone` | Hover, focus | `component.mention.chip-*`. |
| 6.22 | AttachmentImage | Static, animated, spoiler ("Spoiler" overlay) | Loading, loaded, error, removed | Alt text as accessible name. |
| 6.23 | AttachmentFile | — | Default, downloading (progress), downloaded, error | Middle-truncated file name. |
| 6.24 | Divider | Day, unread ("New" pill) | — | `component.unread-divider.*`. |
| 6.25 | JumpBar | New messages (top), Older messages / Jump to present (bottom) | Hover, focus | `component.jump-bar.*`. |
| 6.26 | Composer | Channel, conversation, edit inline, read-only | Empty, typing, with attachments, uploading, too long, offline, disabled | Includes reply bar and attachment tray. |
| 6.27 | AttachmentTray item | Image, file | Uploading, uploaded, error, spoiler on | "Add a description" for alt text. |
| 6.28 | TypingIndicator | 1, 2, 3–5, many | Static under reduced motion | Not announced to screen readers. |
| 6.29 | Autocomplete popover | Users, channels, emoji | Results, no results ("No one called anx in Rust Lyon"), loading | Listbox semantics. |
| 6.30 | QuickSwitcher | — | Results, no results, loading | 5.10. |
| 6.31 | MemberRow | With activity, without | Hover, focus, offline (avatar 50 %) | `component.member-row.*`. |
| 6.32 | ProfilePopover | Self, other, other with moderation | Loading, error | `surface.popover`, `elevation.high`. |
| 6.33 | ContextMenu | With icons, with shortcut hints, with danger items (last, separated), submenus | Item hover (`accent.fill`), danger hover (`danger.fill`); keyboard focus shows only the focus ring, because the first item takes focus when the menu opens and a menu opened with the mouse must not show a filled danger item; disabled with reason | `component.context-menu.*`; arrow keys, type-ahead, `Esc`. Slint has no shadow tokens, so the menu gets a 1 px `context-menu.border` outline instead of `elevation.high`; the keyboard focus ring is drawn `focus-ring.offset` outside the item so it stays visible on the danger fill (`ContextMenuItem` in `primitives.slint`). |
| 6.34 | Tooltip | Default, rail (instant, trailing side) | Shown after `motion.delay.tooltip` hover or immediately on focus | `component.tooltip.*`; never the only copy of essential information. |
| 6.35 | Dialog | Standard, destructive, typed confirmation | Default, submitting, error inline | `surface.chat` body, `surface.sidebar` footer with buttons; focus trapped; `Esc` closes (except while submitting); focus returns to the trigger. Bottom sheet on Android compact. |
| 6.36 | Toast / Snackbar | Info, success, with action ("Undo") | Auto-dismiss after 5 s (paused on hover/focus), dismissible | Polite live region; never for errors that need action. |
| 6.37 | NoticeBar | Info (`accent.fill`), warning (`warning.fill`), danger (`danger.fill`), success (`positive.fill`) | With / without action, dismissible or persistent | Full-width strip at the top of the window, as Discord's notices. |
| 6.38 | ConnectionDock | Hidden, reconnecting, offline, back online, session expired | — | Future voice slot. |
| 6.39 | EmptyState | Channel-start hero, compact | With one primary action | One title, one sentence, one action. |
| 6.40 | Badge | Count (mention), "New" pill | — | `component.badge.*`; "99+" cap; accessible text "3 mentions". |
| 6.41 | RoleChip | With colour dot | Removable | Colour dot plus name, never colour alone. |
| 6.42 | PermissionRow | Switch (role), tri-state (channel override) | Sensitive marker, locked | Plain-language description. |
| 6.43 | SettingsNav | Desktop column, Android list, admin web | Selected, focus, danger item | Discord settings look; 44 px items on web. Desktop: `component.settings.nav-*`, 32 px items; the danger item fills `danger.fill` on hover (see 5.9). |
| 6.44 | UnsavedChangesBar | — | Visible, attention (leave attempted), saving, error | `elevation.high`. |
| 6.45 | Tabs | Standard | Selected, focus | Arrow keys move between tabs. |
| 6.46 | PaneResizeHandle | Vertical | Hover, dragging, focus | Keyboard resize. |
| 6.47 | ChatPanels (Android) | Channel panel, chat, members | Settling, dragging | `anchoredDraggable`, `size.mobile.panel-peek`. |
| 6.48 | AdminTable (web) | With search, pagination, row actions | Loading rows, empty, error | `<table>` with `<caption>` and `scope` headers. |
| 6.49 | AdminStatCard (web) | Health, number, storage bar | Normal, degraded, down | Status in words plus icon. |

Before adding a component, check this list. A new component is added here first, with variants and states.

---

## 7. Accessibility

Target: WCAG 2.2 level AA on all three surfaces, verified with real assistive technologies. Where Discord's look and AA conflict, AA wins with the smallest visible change (section 2.3).

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
| Global | `Ctrl+U` | Toggle member list |
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
- Message accessible name pattern: "[Mentions you.] Ana, Moderator, today at 14:32: Has anyone tried Slint on Wayland? Edited. 2 attachments. Replying to Léo." Continuation messages repeat the author.
- Text that cannot be selected with the pointer is still fully exposed as the message's accessible text; "Select text" mode exposes an editable-text role so screen-reader users can review it character by character.
- Uppercase labels expose their sentence-case source string, so screen readers do not spell them out.
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
| Desktop | 24 × 24 px absolute minimum (WCAG 2.5.8); Discord's own sizes give 32 px for rows, hover bar buttons and header icons, 48 px for rail icons, 24 px for category headers | Pointer-first. Touchscreen laptops are an open question (section 9). |

Drag-and-drop actions (reordering servers, roles, channels) and the Android panel swipes always have a non-drag alternative (WCAG 2.5.7).

### 7.5 Text scaling and reflow

- Desktop: Zoom level 50–200 % scales the whole UI; Chat font size 12–24 px scales messages. At 200 % zoom, panes keep their proportions and the breakpoints of 4.2 apply to the zoomed width, so the layout collapses to the drawer rather than clipping. Channel names truncate with a tooltip in the sidebar; the sidebar can be widened to 360 px.
- Android: system font scale up to 200 % (non-linear on Android 14+); layouts tested at `fontScale = 2f` in previews.
- Admin web: works at 400 % zoom (reflow at 320 px width, WCAG 1.4.10).

### 7.6 Motion, time and media

- Reduced motion as in 2.8.
- No time limits on reading; toasts with actions stay while hovered or focused, and their actions remain available elsewhere (for example "Undo delete" is also in the message's menu for 10 s).
- Spoilers and animated media require a user action to reveal or play by default.

---

## 8. UX copy: voice and tone

Mellow speaks like a calm friend who also happens to run the server: plain, direct, never blaming. Where Discord has a well-known phrase for the same action ("Mark as read", "Jump to present", "You're viewing older messages", "Copy text", "Mark unread", "Server settings"), Mellow uses the same words so the interface feels familiar; it never copies Discord's marketing or jokes.

Rules:

- Sentence case in source strings everywhere, including buttons and titles. Category headers, member groups, form labels and the "New" pill are rendered uppercase by their type token, never written in capitals.
- Active voice, second person ("You can't send messages here"), first person plural for the app only when it acts ("We couldn't reach…").
- Buttons name the action: "Save changes", "Ban Ana", "Create invite". Never "OK", "Submit", "Yes".
- Results echo the action: "Changes saved", "Copied", "Ana was banned".
- Errors say what happened, why if we know, and what to do. No "Oops", no "Something went wrong" without a next step, no error codes in the main sentence (codes go in a "Details" disclosure for admins).
- Empty states invite the next action in one sentence.
- No exclamation marks, including in places where Discord uses them ("Welcome to #general", not "Welcome to #general!"). No emoji in system copy.
- Use the user's words: "server", "channel", "Messages", "instance". Never "guild", "payload", "websocket", "token".
- Name the instance whenever it matters (errors, sign-in, multi-instance contexts), because self-hosted means different places with different rules.
- Numbers and dates use the user's locale; relative dates only for "Today" and "Yesterday".

Examples:

| Situation | Copy |
|---|---|
| Message failed | "Not sent. Retry · Delete" |
| Upload too large | "photo.heic is 48 MB. chat.example.org accepts files up to 25 MB." |
| Connection lost | "Reconnecting to chat.example.org… Messages you send will wait until you're back." |
| Permission missing | "You can't send messages in #announcements." |
| Instance unreachable | "We couldn't reach chat.example.org. Check the address, or try again if your connection is down." |
| Selecting text | "Selecting text · Esc to finish" |
| Empty server | "No channels yet. Create the first one to get the conversation going." · "Create a channel" |
| Empty conversations | "No conversations yet. Start one with someone from your servers." · "New message" |
| Empty bans | "No one is banned from Rust Lyon." |
| Kick confirmation | Title "Kick Ana from Rust Lyon?" · Body "Ana will be removed from the server. They can rejoin with a new invite." · Buttons "Cancel" / "Kick Ana" |
| Ban confirmation | Title "Ban Ana from Rust Lyon?" · Body "Ana will be removed and won't be able to rejoin, even with an invite." · Field "Reason (optional, visible to moderators)" · Select "Delete their recent messages" (Don't delete, Last hour, Last 24 hours, Last 7 days) · Buttons "Cancel" / "Ban Ana" |
| Delete channel | Title "Delete #old-announcements?" · Body "All 1,204 messages in this channel will be deleted for everyone. This can't be undone." · Field "Type old-announcements to confirm" · Buttons "Cancel" / "Delete channel" (enabled when the name matches) |
| Delete own message | Title "Delete this message?" · Body "It will be removed for everyone." · Buttons "Cancel" / "Delete message" · Hint "Tip: hold Shift while deleting to skip this next time." |
| Add a server | Title "Create a server" · Body "Give your server a name. You can add channels and invite people next." · Field "Server name" · Link "Have an invite already? Join a server" · Buttons "Cancel" / "Create" ("Creating…" while busy) |
| Join a server | Title "Join a server" · Body "Enter an invite to join a server someone already runs." · Field "Invite code", helper "Paste the invite code someone gave you." · Link "Don't have an invite? Create a server" · Buttons "Cancel" / "Join" ("Joining…") |
| Invite people | Title "Invite friends to Rust Lyon" · Field "Send this invite code to a friend" (read-only, "Creating an invite…" while empty) with "Copy" ("Copied" for 2 s) · Hint "This invite never expires." · Button "Close" |
| Create channel | Title "Create channel" · Field "Channel name" with a `#` prefix, placeholder "new-channel", helper "Names are lowercase, with dashes instead of spaces." · Buttons "Cancel" / "Create channel" |
| Leave server | Title "Leave 'Rust Lyon'" · Body "Are you sure you want to leave Rust Lyon? You won't be able to rejoin unless you are re-invited." · Buttons "Cancel" / "Leave server" (Danger; Cancel has focus) |
| Revoke invite | Title "Revoke this invite?" · Body "The link will stop working. People who already joined stay in the server." · Buttons "Cancel" / "Revoke invite" |
| Leaving a masked link | Title "Leaving Mellow" · Body "This link goes to example.com/page. Only open links you trust." · Buttons "Cancel" / "Open link" |
| Session expired | "Your session on chat.example.org has ended. Sign in again to keep chatting." · "Sign in" |
| Notifications denied | "Notifications are off for Mellow. You won't hear about mentions or messages." · "Open Android settings" |

Destructive confirmation pattern:

- The title is a question naming the object.
- The body states the consequence and whether it can be undone.
- The destructive button repeats the verb and the object, uses the Danger variant, and is never the default focus. Default focus goes to "Cancel".
- Typed confirmation only for irreversible actions affecting many people (deleting a channel, a server, an account, an instance user).

---

## 9. Open questions and items to validate

Brand and legal:

1. **Mellow logo** for the Home button, app icon and admin header: to be designed. It must not resemble Discord's logo or mascot (no game-controller or face silhouette) and must read at 28 px on both `surface.chat` and `accent.fill`.
2. **Trade-dress review** before any public release: check accent hue, logo, fonts, default avatars and marketing screenshots against ADR-0012's brand rules.
3. **Emoji set**: Twemoji (familiar to Discord users, CC-BY 4.0 with attribution) is preferred over Noto Color Emoji. Confirm that a maintained COLR font build of Twemoji renders in Slint on all three desktop platforms and on Android, and measure its size; otherwise fall back to Noto Color Emoji or the system emoji font.

Fidelity:

4. **Dimension check against Discord**: overlay screenshots of Mellow and the current Discord desktop client at 100 % zoom and verify every row of the table in 4.2 (especially the hover bar, compact mode, typing band and member rows, whose values were taken from Discord's CSS as publicly known in 2023–2024). Discord's interface can change; ADR-0012 says Mellow does not chase every redesign.
5. **Muted channels**: AA leaves only a small step between read (5.82:1) and muted (4.92:1) channels in dark. Test with Discord users whether muted channels are still recognisable; if not, add a muted icon after the name.
6. **Mention highlight in apricot instead of Discord's gold**: confirm with users that it still reads as "this mentions you" at a glance.

Product scope:

7. **"Server" vs "instance"**: validate with five users that "instance" is understood after the one-line explanation on the welcome screen; alternatives are "host" and "community".
8. **Message search**: post-MVP according to ARCHITECTURE.md (SQLite FTS5). The header reserves the slot and `Ctrl+F` is kept free; users expect it early.
9. **Reactions, emoji picker, pins, forwarding, link previews and custom emoji** are not in the MVP; their slots (hover bar, `Ctrl+E`, header pin button) are reserved. Confirm scope and order.
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

Visual validation still to do:

26. Render both themes on real screens (OLED phone, low-quality laptop IPS, sunlight) and check that the grey layers stay distinguishable, especially rail versus user panel (`#1E1F22` versus `#232428`).
27. Check apricot versus amber and red distinguishability under protanopia, deuteranopia and tritanopia simulation, even though none carries meaning alone.
28. Usability test the multi-instance rail with users who have two or more instances.
29. Test with real assistive technologies: VoiceOver (macOS), NVDA and Narrator (Windows), Orca (Linux), TalkBack and Switch Access (Android).

---

## 10. Defaults we rejected

For each, the reason it was rejected for Mellow:

| Default | Why we rejected it | What we do instead |
|---|---|---|
| Pixel-perfect Discord clone including blurple and gg sans | Uses Discord's brand identity (trade-dress risk), and gg sans cannot be licensed | Discord's interface with apricot, the Mellow mark and Noto Sans (ADR-0012) |
| Mellow's own calmer layout and warm surfaces (v0.1) | The user wants Discord users to feel at home instantly; a different layout means relearning | Discord's layout, dimensions and layered cool greys |
| Any blue or violet accent | Too close to blurple; a screenshot would look like Discord | Apricot, the complement of blurple |
| Inter or the system UI font | Generic, and further from Discord's look than the font Discord itself falls back to | Noto Sans, from Discord's own font stack |
| Atkinson Hyperlegible (v0.1) | Its distinctive letterforms and wider set change the density and look Discord users expect | Noto Sans at Discord's sizes; Source Code Pro for codes and IDs |
| Fraunces display serif (v0.1) | Discord has no serif moments; it would break familiarity | Noto Sans 800 for the few display moments |
| Discord's greys as-is for every text role | Several fail WCAG AA (muted text, read and muted channels, placeholders, red text) | Minimal adjustments listed in 2.3 |
| Apricot mention badges (v0.1) | Discord users read a red count as "someone pinged you"; changing it costs familiarity | Red badges with white text at 4.57:1; apricot keeps primary actions and the mention highlight |
| Square server icons (v0.1) | Discord's circle-to-squircle morph is its most recognisable interaction | Circle at rest, squircle on hover and selection |
| Fake drag-selection across messages | Slint cannot do it; a half-working imitation loses text | Copy text, Select text (`F2`), Copy code, Copy link |
| Material baseline purple, standard Material navigation and dynamic colour on Android | Template look, and not how Discord mobile navigates | Discord's three-panel model with Material 3 Expressive components and Mellow tokens |
| Expressive motion everywhere on Android | Discord's motion is quick and functional | `MotionScheme.standard()`, expressive only for the squircle morph and panel settle |
| Shimmering skeletons and per-item entrance animations | Restless and costly | Static skeletons, no list animations |
| Glassmorphism, gradients, glow | Decoration without function, and not Discord | Flat layered greys, shadows only on headers and floating layers |
| Hover-only message actions | Inaccessible to keyboard and touch users | Hover bar, context menu, message keybinds and accessibility actions |
| Silent connection problems | Users lose messages and trust | Connection dock, labelled pending, queued and failed messages |
| Text smaller than 12 px | Discord's 10 px "(edited)" and "NEW" are hard to read | 12 px floor |
