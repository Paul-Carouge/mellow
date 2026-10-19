# Mellow design document

Status: draft v0.1 (2026-10-19). Owner: design. Applies to the Slint desktop client, the Android client (Jetpack Compose, Material 3 Expressive) and the admin web UI embedded in the server.

This document is the reference for every visual and interaction decision in Mellow. Developers should be able to build any MVP screen from it without inventing a colour, a size or a sentence. When something is missing, add it here first, then implement it.

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
| Conversation | `dm` / `group_dm` | A direct message with one person (1:1) or a small group. |

---

## 1. Design principles

Each principle states what we do better than Discord and how to test it. A design or a pull request that breaks a principle needs an explicit justification in its description.

### 1.1 Content is the brightest thing on screen

The chrome (server rail, channel list, member list) is one quiet plane; messages sit on a single, slightly lighter plane. Discord uses four or five grey levels side by side, which makes the frame compete with the conversation.

- Test: the frame uses exactly one surface token (`color.surface.canvas`); the message pane uses `color.surface.content`. No other surface appears in the default layout without a reason (composer, popover, dialog).
- Test: no chromatic colour appears in the frame except selection, unread and mention indicators, and presence.

### 1.2 Red means danger, and only danger

Unread mentions use the apricot accent, not an alarm red. Red (`color.danger.*`) is reserved for destructive actions, errors, failed sends and the "Do not disturb" presence. A red badge should make people careful, not anxious.

- Test: grep the generated token usages; `color.danger.*` only appears in destructive buttons, error banners and fields, failed-message states and the DND presence indicator.

### 1.3 Readable first

Message text uses Atkinson Hyperlegible Next at 16 px with a 1.5 line height, lines are capped at a comfortable measure, and no text in the product is smaller than 12 px at 100 % scale. Section labels are sentence case, never tracked uppercase.

- Test: message text is 16/24 px by default; the message text block is at most `size.message-measure` (736 px, about 85 characters) wide unless the user turns "Limit line length" off.
- Test: every text/background pair listed in section 2.3 meets WCAG 2.2 AA; the token generator fails the build otherwise (section 3.5).

### 1.4 Density and text size are separate, explicit choices

People choose a density (cozy or compact) and, independently, a text size from 80 % to 200 %. Neither choice breaks the layout: nothing is truncated that the user needs to read, and nothing overlaps.

- Test: every screen in section 5 is checked at compact and cozy, at 100 % and 200 % text size, in light and dark. Message content, author names and buttons never clip at 200 %.

### 1.5 Never colour alone, always reachable by keyboard

Presence has four distinct shapes, unread state uses weight and a marker, permission states use icons and words. Every action available on hover is also available from a context menu, a keyboard shortcut and the screen reader's actions.

- Test: the UI is fully usable in greyscale; every hover action appears in the context menu (`Shift+F10` or the Menu key) and in the accessibility actions of the element.

### 1.6 Say what is happening

Mellow always tells you which instance you are talking on, whether you are connected, and whether your message was delivered. Pending, failed and queued messages are labelled in words. Discord often hides connection problems behind a spinner.

- Test: every screen in section 5 defines loading, empty, error, offline and reconnecting states; no state is a lone spinner or a blank area.
- Test: when the user is signed in to more than one instance, the composer placeholder and every conversation header name the instance.

---

## 2. Visual identity

### 2.1 Personality

Mellow is a warm room in the evening: soft light, good chairs, people talking at a normal volume. It is calm without being sleepy, warm without being cute, and precise without being cold. The interface feels unhurried: it never flashes, bounces or shouts for attention, but it answers instantly when touched.

Three words guide every choice: **warm, legible, unhurried**.

What this means in practice:

- Warm neutrals instead of blue-grey. The dark theme is warm graphite, the light theme is warm paper.
- One soft accent (apricot) used sparingly for "this is for you" and "this is the main action".
- A soft serif (Fraunces with its `SOFT` axis at maximum) for a handful of welcoming moments, never for chrome.
- People are round, places are square: avatars are circles, server and instance icons are rounded squares. This removes Discord's ambiguity where servers and people both look like circles.

### 2.2 Palette

The reference palette has seven named families. Components never use them directly; they use the semantic tokens in 2.3.

| Family | Role | Dark anchor | Light anchor |
|---|---|---|---|
| Ink | Warm graphite neutrals for dark surfaces and light-theme text | `#161312` to `#F3ECE4` | `#211C19` (text) |
| Paper | Warm light neutrals for light surfaces | — | `#E7E0D8` to `#FFFFFF` |
| Apricot | Brand accent: primary action, selection, mentions | `#F2A46E` | `#9A4A14` (text and fill), `#F2A46E` (container) |
| Lagoon | Links, information, focus ring | `#86C8E3`, focus `#8FD6F2` | `#1D6683`, focus `#0E6A90` |
| Sage | Success, online presence | `#86CFA0` | `#2C7449` |
| Saffron | Warning, idle presence | `#E9C46A` | `#7F5C00` (text), `#946B00` (idle) |
| Rosehip | Danger, errors, do-not-disturb presence | `#F29191` | `#B02E3B` |

Apricot and Saffron are close in hue. They never carry meaning alone: warnings always come with an icon and a sentence, idle presence always has its crescent shape.

### 2.3 Semantic colour tokens

Dark ("Dusk") is the default theme. Light ("Daylight") is offered and follows the system setting when the user picks "Match system".

#### Surfaces and borders

| Token | Dark | Light | Used for |
|---|---|---|---|
| `color.surface.canvas` | `#161312` | `#EFE9E2` | The frame: server rail, channel list, member list, settings navigation |
| `color.surface.content` | `#1E1A18` | `#FAF7F3` | Message pane, settings content, admin web page background |
| `color.surface.raised` | `#28231F` | `#FFFFFF` | Composer, attachment cards, profile cards, banners |
| `color.surface.overlay` | `#312B27` | `#FFFFFF` (with shadow) | Menus, popovers, dialogs, tooltips |
| `color.surface.sunken` | `#110F0E` | `#E7E0D8` | Text fields, code blocks, quoted text background |
| `color.border.subtle` | `#3A332E` | `#DDD5CC` | Decorative dividers only (never the only boundary of a control) |
| `color.border.strong` | `#756A62` | `#8E847B` | Text field and checkbox outlines, which need 3:1 |
| `color.scrim` | `#000000` at 56 % | `#211C19` at 40 % | Behind modal dialogs and drawers |

#### Interaction state layers

State layers are a translucent `color.text.primary` laid over the surface, so they work on every surface in both themes.

| Token | Opacity of `color.text.primary` | Used for |
|---|---|---|
| `color.state.hover` | 6 % | Pointer hover on rows and buttons |
| `color.state.pressed` | 10 % | Pressed or active |
| `color.state.selected` | 10 % | Selected channel, selected conversation, selected settings page |
| `color.state.dragged` | 14 % | Item being dragged (role reorder, channel reorder) |

The selected state is never conveyed by the layer alone: selected rows also get `color.text.primary` text and, in the rail, the selection pill.

#### Text

| Token | Dark | Light | Used for |
|---|---|---|---|
| `color.text.primary` | `#F3ECE4` | `#211C19` | Message text, names, titles, unread channels |
| `color.text.secondary` | `#C9BEB3` | `#4C433D` | Read channels, custom status, descriptions |
| `color.text.tertiary` | `#A39890` | `#6A5F57` | Timestamps, placeholders, "edited", muted channels, helper text |
| `color.text.disabled` | `#6E645C` | `#A39890` | Disabled controls only (exempt from contrast requirements, always paired with a reason) |

#### Accent, link, focus and mention

| Token | Dark | Light | Used for |
|---|---|---|---|
| `color.accent.primary` | `#F2A46E` | `#9A4A14` | Primary button fill, rail selection pill, unread divider, accent text and icons |
| `color.accent.on-primary` | `#2B1608` | `#FFFFFF` | Text and icons on `accent.primary` |
| `color.accent.container` | `#5A3520` | `#F2A46E` | Mention count badge, @mention chips in messages, selected navigation indicator on Android |
| `color.accent.on-container` | `#FFDCC4` | `#2B1608` | Text on `accent.container` |
| `color.mention.background` | `#3C2D24` | `#F8E6D8` | Background of a message that mentions you (always with a 2 px `accent.primary` bar on the leading edge) |
| `color.link` | `#86C8E3` | `#1D6683` | Links in messages and in copy (also underlined) |
| `color.focus` | `#8FD6F2` | `#0E6A90` | Keyboard focus ring, 2 px wide with a 2 px gap |

The focus ring is Lagoon, not Apricot, so that "focused" and "selected" are never confused. The 2 px gap means the ring always sits against the surrounding surface, not against the control's own fill.

#### Feedback

| Token | Dark | Light | Used for |
|---|---|---|---|
| `color.danger.primary` | `#F29191` | `#B02E3B` | Destructive buttons, error text, failed-send icon |
| `color.danger.on-primary` | `#2B0A0E` | `#FFFFFF` | Text on destructive buttons |
| `color.danger.container` | `#4A1F23` | `#F9DEDF` | Error banner background |
| `color.danger.on-container` | `#FFD9DA` | `#5C1119` | Error banner text |
| `color.warning.primary` | `#E9C46A` | `#7F5C00` | Warning icon and text |
| `color.warning.container` | `#3D3015` | `#F7E8C2` | Warning banner background |
| `color.warning.on-container` | `#F7E4B0` | `#3D2C00` | Warning banner text |
| `color.success.primary` | `#86CFA0` | `#2C7449` | Success icon and text |
| `color.success.container` | `#1C3B28` | `#D6EFDF` | Success banner background |
| `color.success.on-container` | `#CFF0DA` | `#0F3A21` | Success banner text |
| `color.info.container` | `#173847` | `#D7EEF7` | Information and offline banners |
| `color.info.on-container` | `#CDEBF7` | `#0B3445` | Text on information banners |

#### Presence

| Token | Dark | Light | Shape (never colour alone) |
|---|---|---|---|
| `color.presence.online` | `#86CFA0` | `#2C7449` | Filled circle |
| `color.presence.idle` | `#E9C46A` | `#946B00` | Crescent moon |
| `color.presence.dnd` | `#F29191` | `#B02E3B` | Filled circle with a horizontal bar cut out |
| `color.presence.offline` | `#8C827A` | `#7A7068` | Hollow ring |

The presence indicator is drawn with a 2 px cut-out ring of the surface it sits on, so it separates from the avatar image.

#### Verified contrast ratios (WCAG 2.2 AA)

Ratios were computed with the WCAG relative luminance formula. Body text needs 4.5:1; large text (24 px, or 18.66 px bold) and non-text UI elements need 3:1.

| Pair | Dark | Light | Requirement |
|---|---|---|---|
| `text.primary` on `surface.content` | 14.74 | 15.80 | 4.5 |
| `text.primary` on `surface.canvas` | 15.78 | 14.00 | 4.5 |
| `text.primary` on `surface.overlay` | 11.91 | 16.87 | 4.5 |
| `text.primary` on `mention.background` | 11.26 | 13.90 | 4.5 |
| `text.secondary` on `surface.content` | 9.46 | 9.03 | 4.5 |
| `text.secondary` on `surface.canvas` | 10.12 | 8.00 | 4.5 |
| `text.tertiary` on `surface.content` | 6.13 | 5.80 | 4.5 |
| `text.tertiary` on `surface.canvas` | 6.56 | 5.14 | 4.5 |
| `text.tertiary` on `surface.raised` | 5.52 | 6.20 | 4.5 |
| `text.tertiary` on `surface.sunken` (placeholders) | 6.78 | 4.74 | 4.5 |
| `text.tertiary` on `mention.background` | 4.68 | 5.11 | 4.5 |
| `accent.on-primary` on `accent.primary` | 8.44 | 6.24 | 4.5 |
| `accent.on-container` on `accent.container` | 8.29 | 8.44 | 4.5 |
| `accent.primary` on `surface.canvas` (selection pill, unread divider) | 9.08 | 5.18 | 3.0 |
| `link` on `surface.content` | 9.36 | 5.99 | 4.5 |
| `link` on `mention.background` | 7.15 | 5.27 | 4.5 |
| `focus` on `surface.content` | 10.75 | 5.65 | 3.0 |
| `focus` on `surface.canvas` | 11.50 | 5.00 | 3.0 |
| `danger.primary` on `surface.content` | 7.59 | 5.96 | 4.5 |
| `danger.on-primary` on `danger.primary` | 8.01 | 6.36 | 4.5 |
| `danger.on-container` on `danger.container` | 10.71 | 10.68 | 4.5 |
| `warning.primary` on `surface.content` | 10.33 | 5.73 | 4.5 |
| `success.primary` on `surface.content` | 9.41 | 5.31 | 4.5 |
| `info.on-container` on `info.container` | 9.94 | 10.97 | 4.5 |
| `border.strong` on `surface.content` | 3.28 | 3.43 | 3.0 |
| `presence.idle` on `surface.canvas` | 11.06 | 3.99 | 3.0 |
| `presence.offline` on `surface.canvas` | 4.92 | 4.01 | 3.0 |

`color.border.subtle` (1.39 dark, 1.36 light) is intentionally below 3:1. It is decorative and must never be the only thing that shows where a control is.

#### User-chosen colours (role colours, default avatars)

Role colours are chosen by server admins and shown on author names. Discord shows them as-is, which often produces unreadable names. Mellow keeps the hue but adjusts lightness in OKLCH until the colour reaches 4.5:1 against the surface it is drawn on, separately for each theme. This adjustment lives in the shared Rust core so both clients render the same result. The role editor shows a live preview on both themes and says "Adjusted for readability" when the displayed colour differs from the picked one.

Default avatars (no uploaded picture) use the first grapheme of the display name on one of eight muted tints derived from the palette, picked deterministically from the user ID. Each tint/initial pair is validated at 4.5:1 by the token generator.

### 2.4 Typography

All fonts are bundled with the apps and the admin web UI, so the product looks the same everywhere and works offline.

| Family | Licence | Role | Why |
|---|---|---|---|
| Atkinson Hyperlegible Next | SIL OFL 1.1 | All UI and message text | Designed by the Braille Institute for maximum character distinction (`Il1`, `O0`, `rn`/`m`), which matters in usernames, invite codes and code-ish chat. Variable weight 200–800. Distinct from the default Inter look of most apps. |
| Atkinson Hyperlegible Mono | SIL OFL 1.1 | Inline code, code blocks, invite codes, IDs in the admin UI | Same design logic and metrics family as the UI face, so code sits naturally inside messages. |
| Fraunces (axes `SOFT` 100, `WONK` 0) | SIL OFL 1.1 | Display moments only: first-run welcome, empty-channel hero, empty states, admin dashboard title | The `SOFT` axis rounds the serif's terminals, which literally expresses "mellow". Restricted to large sizes so it never hurts legibility. |
| Noto Sans (subsets as needed) | SIL OFL 1.1 | Fallback for scripts not covered by Atkinson (Cyrillic, Greek, CJK, Arabic, etc.) | Broad coverage and harmonious metrics. |
| Noto Color Emoji | SIL OFL 1.1 | Emoji rendering | Consistent emoji across Linux, Windows and macOS; to be confirmed for binary size (section 9). |

Rules:

- Fraunces is used for at most one element per screen and never below 24 px.
- Weights in the UI are limited to 400, 600 and 700. Emphasised styles (Material 3 Expressive) add 100 to the weight.
- Numbers in tables (admin UI, member counts) use tabular figures (`tnum`) where the font provides them.
- Text is never set in all caps. Section headers in lists ("Text channels", "Online — 12") are sentence case at `type.label-md`.

#### Type scale tokens

Sizes are logical pixels on desktop and the admin web, and `sp` on Android. Line heights are in the same unit.

| Token | Family | Weight | Size / line height | Tracking | Used for | Compose `Typography` slot |
|---|---|---|---|---|---|---|
| `type.hero` | Fraunces Soft | 600 | 32 / 40 | -0.25 | First-run welcome, empty-channel hero | `displaySmall` (also `displayMedium`, `displayLarge` at 40/48 and 48/56 on expanded windows) |
| `type.headline` | Fraunces Soft | 600 | 24 / 32 | 0 | Empty-state titles, admin dashboard title | `headlineLarge`, `headlineMedium` |
| `type.title-lg` | Atkinson Next | 700 | 20 / 28 | 0 | Channel header name, dialog titles, settings page titles | `titleLarge`, `headlineSmall` |
| `type.title` | Atkinson Next | 600 | 16 / 22 | 0 | Author names (cozy), card titles, server name in channel list header | `titleMedium` |
| `type.title-sm` | Atkinson Next | 600 | 14 / 20 | 0.1 | Profile popover sections, table headers | `titleSmall` |
| `type.body-message` | Atkinson Next | 400 | 16 / 24 | 0 | Message text, composer text | `bodyLarge` |
| `type.body` | Atkinson Next | 400 | 15 / 22 | 0 | Channel names, member names, settings text, admin body | `bodyMedium` |
| `type.body-sm` | Atkinson Next | 400 | 13 / 18 | 0.1 | Custom status, helper text, descriptions | `bodySmall` |
| `type.label` | Atkinson Next | 600 | 14 / 20 | 0.1 | Buttons, tabs, segmented controls | `labelLarge` |
| `type.label-md` | Atkinson Next | 600 | 13 / 18 | 0.2 | List section headers, badges | `labelMedium` |
| `type.label-sm` | Atkinson Next | 500 | 12 / 16 | 0.2 | Timestamps, "edited", typing indicator | `labelSmall` |
| `type.code` | Atkinson Hyperlegible Mono | 400 | 14 / 22 | 0 | Inline code, code blocks | Extended token `MellowTheme.typography.code` |

All tokens scale with the user's text size setting (desktop: 80–200 % in the app; Android: system font scale, including non-linear scaling on Android 14+).

### 2.5 Spacing

A 4 px base grid. Components use these tokens only.

| Token | Value | Typical use |
|---|---|---|
| `space.3xs` | 2 | Icon-to-badge offset, presence cut-out |
| `space.2xs` | 4 | Gap between grouped messages (compact), icon-to-label in tight rows |
| `space.xs` | 8 | Row inner padding, gap between buttons |
| `space.sm` | 12 | List item horizontal padding, gap between avatar and message |
| `space.md` | 16 | Pane padding, gap between message groups (cozy) |
| `space.lg` | 24 | Section spacing in settings and dialogs |
| `space.xl` | 32 | Page margins in settings, empty-state blocks |
| `space.2xl` | 48 | Hero spacing, first-run layout |
| `space.3xl` | 64 | Admin dashboard top spacing on wide screens |

Fixed sizes:

| Token | Value | Use |
|---|---|---|
| `size.rail-width` | 72 | Server rail on desktop and Android |
| `size.channel-list-width` | 248 (resizable 200–360) | Channel list default |
| `size.member-list-width` | 248 (resizable 200–320) | Member list default |
| `size.header-height` | 52 | Pane headers |
| `size.message-measure` | 736 | Maximum width of message text when "Limit line length" is on |
| `size.settings-measure` | 720 | Maximum width of settings forms |
| `size.hit-min-desktop` | 32 | Minimum pointer target on desktop (WCAG 2.5.8 requires 24) |
| `size.hit-min-web` | 44 | Minimum target in the admin web UI |
| `size.hit-min-android` | 48 | Minimum touch target on Android |

#### Density tokens (desktop)

Density changes spacing and avatar visibility only, never font size.

| Token | Cozy (default) | Compact |
|---|---|---|
| `density.message.avatar-size` | 40 | Hidden (20 if "Show avatars in compact mode" is on) |
| `density.message.group-gap` | 16 | 4 |
| `density.message.row-padding-y` | 2 | 1 |
| `density.message.timestamp` | After the author name | Inline before the author name, fixed-width column |
| `density.list.row-height` | 36 | 32 |
| `density.member.row-height` | 44 | 32 |
| `density.rail.icon-size` | 48 | 40 |

Android uses cozy only, because touch targets cannot shrink below 48 dp.

### 2.6 Radius and shape

| Token | Value | Use |
|---|---|---|
| `radius.xs` | 4 | Inline code, small badges, focus ring corners on tight elements |
| `radius.sm` | 6 | Text fields, checkboxes, tooltips |
| `radius.md` | 10 | Cards, attachments, code blocks, menus |
| `radius.lg` | 16 | Popovers, dialogs, composer |
| `radius.xl` | 24 | Android bottom sheets, first-run panel |
| `radius.full` | 9999 | Buttons (pill), avatars, badges, chips |
| `radius.place` | 30 % of the icon size (14 at 48) | Server and instance icons ("places are square") |

Buttons are pills on every platform. This matches Material 3 Expressive on Android and gives the desktop the same soft identity. Cards and containers use `radius.md` or `radius.lg`, so there is a visible difference between "something you press" and "something that holds content".

Android signature shape: the selected server icon morphs from `radius.place` to a slightly softer cookie-like `MaterialShapes` shape with the theme's spring motion. This is the one expressive flourish on the server list; everything else stays quiet. On desktop the selected server keeps its shape and gets the selection pill on the leading edge.

### 2.7 Elevation

Dark themes show depth by surface tone; shadows only separate floating layers.

| Token | Surface | Shadow (dark) | Shadow (light) | Use |
|---|---|---|---|---|
| `elevation.0` | `surface.canvas` / `surface.content` | none | none | Panes |
| `elevation.1` | `surface.raised` | none | 0 1 2 rgba(33,28,25,0.06) | Composer, cards |
| `elevation.2` | `surface.overlay` | 0 4 16 rgba(0,0,0,0.32) | 0 4 16 rgba(33,28,25,0.12) | Menus, popovers, tooltips |
| `elevation.3` | `surface.overlay` | 0 12 32 rgba(0,0,0,0.44) | 0 12 32 rgba(33,28,25,0.18) | Dialogs, drawers, toasts |

Values are `x y blur colour`. Slint maps them to `drop-shadow-offset-y`, `drop-shadow-blur` and `drop-shadow-color`; Compose maps levels to `surfaceContainer*` roles and `shadowElevation`; CSS maps them to `box-shadow`.

### 2.8 Motion

Mellow's motion is unhurried but never slow: short, soft deceleration, no bounce in the chrome. Motion only answers a user action or explains where something came from or went.

| Token | Value | Use |
|---|---|---|
| `motion.duration.instant` | 0 ms | Reduced-motion replacement for spatial motion |
| `motion.duration.fast` | 120 ms | Hover, press, focus ring, checkbox, switch |
| `motion.duration.base` | 200 ms | Popovers, menus, tooltips, member list collapse, unread pill |
| `motion.duration.slow` | 320 ms | Dialogs, drawers, pane transitions on Android, jump-to-message highlight fade start |
| `motion.duration.highlight` | 1600 ms | Fade-out of the highlight on a message you jumped to |
| `motion.easing.standard` | cubic-bezier(0.2, 0, 0, 1) | Most transitions |
| `motion.easing.enter` | cubic-bezier(0, 0, 0, 1) | Elements appearing |
| `motion.easing.exit` | cubic-bezier(0.3, 0, 1, 1) | Elements leaving (use `fast` duration) |

Rules:

- No user-initiated transition is longer than 320 ms.
- No skeleton shimmer. Loading placeholders are static shapes (see 6.10).
- No per-item entrance animation in lists. New messages appear in place.
- Animated GIFs, animated avatars and animated emoji play on hover or focus by default, not continuously.
- Android uses `MotionScheme.standard()` for the whole app (calmer than the expressive scheme) and the expressive spring only for the selected-server shape morph and the send button.

Reduced motion:

- Desktop reads the OS preference where available (macOS "Reduce motion", Windows "Show animations in Windows", GNOME `enable-animations`) and offers an in-app override: "Match system", "Reduce motion", "Allow motion".
- Android follows "Remove animations" (animator duration scale 0) and the same in-app override.
- The admin web UI uses `@media (prefers-reduced-motion: reduce)`.
- When reduced motion is on: every spatial movement (slide, scale, morph, smooth scroll) becomes `motion.duration.instant`; opacity cross-fades are kept but capped at 100 ms; "jump to message" scrolls instantly and keeps the highlight as a static background for `motion.duration.highlight`; animated media never autoplays; the typing indicator shows static dots.

### 2.9 Iconography

One icon set everywhere: **Phosphor Icons** (MIT licence), "Regular" weight, at 20 px in lists and 24 px in headers and on Android. The rounded joins match the soft identity, the set is large enough for chat, moderation and admin needs, and it is not tied to another platform's look. Icons are exported as SVG for Slint and the admin web UI, and converted to `ImageVector` for Compose. Emoji are never used as icons.

---

## 3. Token architecture

### 3.1 Single source of truth

All tokens live in one file: `design/tokens.json`, in the W3C Design Tokens Community Group format (`$value`, `$type`, `$description`). A generator (a `cargo xtask tokens` command in the workspace) produces the platform files. Nobody edits generated files by hand.

```
design/
  tokens.json              # source of truth (palette, semantic, density, type, motion)
  fonts/                   # bundled font files (OFL texts included)
  icons/                   # Phosphor SVG subset actually used
generated (committed, checked in CI):
  crates/mellow-desktop/ui/theme/tokens.slint
  android/core/designsystem/src/main/kotlin/.../theme/MellowTokens.kt
  crates/mellow-server/assets/admin/tokens.css
```

The generated paths are indicative and follow the layout in ARCHITECTURE.md; `design/tokens.json` is fixed.

### 3.2 Three tiers

1. **Reference tokens** (`ref.*`): raw values with no meaning, for example `ref.apricot.300 = #F2A46E`, `ref.ink.900 = #161312`. Only semantic tokens may reference them.
2. **Semantic tokens** (`color.*`, `type.*`, `space.*`, `radius.*`, `elevation.*`, `motion.*`, `size.*`, `density.*`): the tokens documented in section 2. Colour tokens have a `dark` and a `light` value. Components use only this tier.
3. **Component tokens** (`component.<name>.*`): created only when a component needs a value that differs from the semantic default, for example `component.rail.selection-pill.height`. Keep this tier small.

### 3.3 Naming convention

- Source names are dot-separated, lowercase, kebab-case within a segment: `color.accent.on-primary`, `density.message.group-gap`.
- Themes are values, not names: `color.text.primary` has a dark and a light value. There is no `color.dark.text`.
- Names describe the role, never the hue: `color.danger.primary`, not `color.red`.

| Platform | Transformation | Example for `color.accent.on-primary` |
|---|---|---|
| Slint | Global per category, property name is the rest in kebab-case | `MellowColor.accent-on-primary` |
| Compose | Material slot when one exists (3.4), otherwise a camelCase property in `MellowTheme` | `MaterialTheme.colorScheme.onPrimary` |
| CSS | Custom property prefixed with `--mw-`, dots become dashes | `--mw-color-accent-on-primary` |

### 3.4 Platform mapping

#### Slint (desktop)

The generator writes one `.slint` file with these globals: `MellowColor`, `MellowType`, `MellowSpace`, `MellowSize`, `MellowRadius`, `MellowElevation`, `MellowMotion`, `MellowDensity`.

- `MellowColor` has an `in-out property <bool> dark` set by the Rust side from the user's theme setting and the OS appearance. Every colour property is generated as `dark ? <dark value> : <light value>`, so switching theme is one property change.
- `MellowDensity` has an `in-out property <bool> compact`; each density property is generated the same way.
- `MellowType` exposes font family, size, weight and line height per type token, multiplied by an `in-out property <float> text-scale` (0.8–2.0).
- `MellowMotion` durations are generated as `reduce-motion ? 0ms : <value>`, with `in-out property <bool> reduce-motion`.
- We do not use the std-widgets styles (Fluent, Material, Cupertino) for visible components; we build Mellow components on top of Slint primitives so the identity is consistent across operating systems. We keep std-widgets only where they provide behaviour we cannot reasonably rebuild (for example scrolling), styled with our tokens.
- Name collision: Slint's std-widgets already export a `Palette` global, which is why ours is called `MellowColor`.

#### Compose (Android, Material 3 Expressive)

The generator writes `MellowTokens.kt` containing the light and dark `ColorScheme`, the `Typography`, the `Shapes`, and an extended `MellowColors`/`MellowSpacing`/`MellowSizes` object provided through a `CompositionLocal`. `MellowTheme { }` wraps `MaterialExpressiveTheme` (or `MaterialTheme` if the expressive theme is not available in the pinned version) with `motionScheme = MotionScheme.standard()`.

Dynamic colour (Material You) is off: Mellow's identity is the warm palette. See section 9.

`ColorScheme` mapping:

| Material role | Dark | Light |
|---|---|---|
| `primary` / `onPrimary` | `accent.primary` / `accent.on-primary` | `accent.primary` / `accent.on-primary` |
| `primaryContainer` / `onPrimaryContainer` | `accent.container` / `accent.on-container` | `accent.container` / `accent.on-container` |
| `secondary` / `onSecondary` | `accent.primary` / `accent.on-primary` | same |
| `secondaryContainer` / `onSecondaryContainer` (navigation indicator) | `accent.container` / `accent.on-container` | same |
| `tertiary` / `onTertiary` | `link` / `#0E2A36` | `link` / `#FFFFFF` |
| `tertiaryContainer` / `onTertiaryContainer` | `info.container` / `info.on-container` | same |
| `error` / `onError` | `danger.primary` / `danger.on-primary` | same |
| `errorContainer` / `onErrorContainer` | `danger.container` / `danger.on-container` | same |
| `background`, `surface` / `onBackground`, `onSurface` | `surface.content` / `text.primary` | same |
| `onSurfaceVariant` | `text.secondary` | `text.secondary` |
| `surfaceDim`, `surfaceContainerLowest` | `surface.sunken` | `surface.sunken` |
| `surfaceContainerLow` | `surface.canvas` | `surface.canvas` |
| `surfaceContainer` | `surface.content` | `surface.content` |
| `surfaceContainerHigh` | `surface.raised` | `surface.raised` |
| `surfaceContainerHighest`, `surfaceBright` | `surface.overlay` | `surface.overlay` |
| `surfaceVariant` | `surface.sunken` | `surface.sunken` |
| `outline` / `outlineVariant` | `border.strong` / `border.subtle` | same |
| `inverseSurface` / `inverseOnSurface` / `inversePrimary` | light theme's `surface.content` / `text.primary` / `accent.primary` | dark theme's equivalents |
| `scrim` | `scrim` | `scrim` |

Extended colours (`MellowTheme.colors`): `textTertiary`, `link`, `focus`, `mentionBackground`, `success*`, `warning*`, `presenceOnline`, `presenceIdle`, `presenceDnd`, `presenceOffline`.

`Shapes` mapping: `extraSmall = radius.xs`, `small = radius.sm`, `medium = radius.md`, `large = radius.lg`, `extraLarge = radius.xl`. Buttons keep Material's full-round default, which equals `radius.full`.

Spacing and sizes: `MellowTheme.spacing.md`, `MellowTheme.sizes.railWidth`, etc. Screens never contain raw `dp` values.

#### Admin web (CSS custom properties)

The generator writes `tokens.css`:

- `:root` holds the dark values (dark is the default), `@media (prefers-color-scheme: light)` and `[data-theme="light"]` hold the light values, `[data-theme="dark"]` forces dark.
- Every token becomes `--mw-<path>`, for example `--mw-space-md: 1rem`. Spacing and type sizes are emitted in `rem` (16 px base) so browser zoom and font settings work.
- Motion durations are wrapped in `@media (prefers-reduced-motion: reduce)` overrides.
- Fonts are embedded as WOFF2 files served by the server binary, with `font-display: swap`.

### 3.5 Generator guarantees (acceptance criteria, not code)

- `cargo xtask tokens --check` fails if a generated file is stale. CI runs it.
- The source file declares the contrast pairs of section 2.3 with their required ratio; the generator fails if any pair falls below its threshold in either theme.
- The generator fails if a semantic token references a missing reference token, or if a colour token lacks a dark or light value.

---

## 4. Layout and information architecture

### 4.1 Mental model

```
Instance (chat.example.org)            ← account lives here
 ├─ Home: conversations (DMs, groups)
 └─ Servers
     └─ Server "Rust Lyon"
         ├─ Category "General"
         │   ├─ #welcome
         │   └─ #general
         └─ Category "Projects"
             └─ #mellow
```

One client can be signed in to several instances. The rail groups servers by instance; Home merges conversations from all instances and labels each with its instance when more than one is connected.

### 4.2 Desktop (Slint)

#### Window and breakpoints

Minimum window size: 480 × 560 px, so Mellow fits in a half-screen tile on a 1366 px laptop and in tiling window managers. Default size on first launch: 1280 × 800, or 80 % of the screen if smaller.

| Window width | Layout |
|---|---|
| ≥ 1280 | Rail + channel list + message pane + member list (member list visible by default) |
| 960–1279 | Rail + channel list + message pane; member list hidden by default, opens beside the pane if the pane keeps ≥ 560 px, otherwise as an overlay panel on the trailing edge |
| 640–959 | Rail + channel list + message pane; member list always an overlay panel |
| 480–639 | Message pane only; rail and channel list open together as a drawer from the leading edge (header button or `Ctrl+Shift+D`) |

#### Wireframe (≥ 1280, cozy, dark)

```
┌──────┬──────────────────┬───────────────────────────────────────────┬──────────────────┐
│ Home │ Rust Lyon      ⌄ │ # general · Talk about anything      ⌕ ⚑ ☰│ Moderators — 2   │
│──────│──────────────────│───────────────────────────────────────────│ (A) Ana          │
│ inst │ General          │            Today                          │ (L) Léo   ◐      │
│ (RL)▌│  # welcome       │ (A) Ana  14:02                            │                  │
│ (MB) │ ▌# general     • │     Has anyone tried Slint 1.x on Wayland?│ Online — 12      │
│──────│  # off-topic     │     It works for me with the winit backend│ (P) Paul         │
│ inst │ Projects         │ ── New since 14:32 ────────────────────── │ ...              │
│ (XY) │  # mellow    (2) │ ▌(L) Léo  14:33                           │                  │
│      │                  │ ▌    @Paul can you review the PR?         │ Offline — 40     │
│  +   │──────────────────│                         [ Jump to present ]│ ...              │
│      │ [connection dock]│ ┌───────────────────────────────────────┐ │                  │
│      │ (P) Paul  ●  ⚙   │ │ + Message #general                    │ │                  │
│      │                  │ └───────────────────────────────────────┘ │                  │
│      │                  │ Ana is typing…                            │                  │
└──────┴──────────────────┴───────────────────────────────────────────┴──────────────────┘
 rail 72  channel list 248   message pane (text capped at 736)          member list 248
```

Notes:

- `▌` on the rail is the selection pill (`accent.primary`); a short pill means "unread", a tall pill means "selected". In the channel list, `▌` is the selected row marker and `•` an unread dot.
- `inst` marks an instance group in the rail: a thin divider and the instance icon (rounded square, smaller) whose tooltip and accessible name give the domain. With one instance only, the group header is hidden.
- The **connection dock** above the user panel is a reserved slot. In the MVP it shows connection and sync status (section 5.2). When voice arrives, it becomes the voice status area, so voice will not require a layout change.
- The header holds the channel name (`type.title-lg`), the topic (single line, `text.secondary`, opens fully on click), and actions: search (reserved slot, see section 9), pinned messages (reserved), member list toggle.

#### Resizable panes

- The channel list and member list have a 4 px drag handle on their inner edge (hit area 8 px), with a resize cursor. Keyboard: focus the handle with `F6` cycling, then `Left`/`Right` resize by 16 px.
- Widths are clamped to the ranges in 2.5 and saved per window.
- Double-clicking a handle restores the default width.
- The member list toggle (`Ctrl+U`) remembers its state per server.

#### Density

Cozy (default): avatar and author name on the first message of a group, following messages aligned under the text. Compact: one line per message, timestamp and author inline, no avatars by default, aligned columns for scanning busy channels. Density is set in Appearance settings and applies to the message list, channel list, member list and rail.

### 4.3 Android (Compose, adaptive)

Android is designed for thumbs first, not as a shrunken desktop. We use window size classes with `NavigationSuiteScaffold` and the adaptive list-detail and supporting-pane scaffolds.

Top-level destinations (navigation suite):

| Destination | Content |
|---|---|
| Servers | The rail and channel list of the current server |
| Messages | Conversations from all connected instances |
| Mentions | Recent mentions and replies across instances (the inbox) |
| You | Profile, status, settings, instances |

#### Compact width (< 600 dp, phones)

- Bottom navigation bar with the four destinations.
- Servers: the 72 dp rail on the leading edge plus the channel list fill the screen. Tapping a channel opens the chat full screen (list-detail, single pane), hiding the navigation bar so the composer has room.
- Chat: top app bar with channel name and member-list action; members open as a full-screen sheet. Back (including predictive back) returns to the channel list.
- The composer sits above the keyboard with correct IME insets; edge-to-edge with system bar insets handled.

```
Phone, Servers                Phone, chat
┌────┬──────────────────┐     ┌───────────────────────────┐
│Home│ Rust Lyon      ⌄ │     │ ←  # general         [M]  │  [M] = members action
│(RL)│ General          │     │      Today                │
│(MB)│  # welcome       │     │ (A) Ana  14:02            │
│    │ ▌# general     • │     │     Has anyone tried…     │
│ +  │ Projects         │     │ ── New since 14:32 ────── │
│    │  # mellow    (2) │     │ ▌(L) Léo  14:33           │
│    │                  │     │ ▌    @Paul can you…       │
├────┴──────────────────┤     │ ┌───────────────────────┐ │
│ Servers Messages @ You│     │ │ + Message #general  ➤ │ │
└───────────────────────┘     └─┴───────────────────────┴─┘
```

#### Medium width (600–839 dp, foldables unfolded in portrait, small tablets)

- Navigation rail replaces the bottom bar.
- Two panes: rail + channel list on the leading side, chat on the trailing side (list-detail, both visible).
- Members open as a modal side sheet.

#### Expanded and larger (≥ 840 dp, tablets, foldables in landscape, ChromeOS)

- Navigation rail, server rail, channel list, chat.
- From 1200 dp, the member list is a supporting pane, visible by default, togglable.
- Book posture on foldables: the pane split follows the hinge so no content sits on the fold.
- Content width of the message text is capped at `size.message-measure`.

#### Android-specific patterns

- Message actions: long-press opens a bottom sheet with Reply, Edit, Copy text, Mark unread, Delete, Report (later). The same actions are exposed as TalkBack custom actions.
- Swipe a message towards the trailing edge to reply. With reduced motion on, the gesture still works and the message snaps back instantly.
- Pull-to-refresh is not used in chat; history loads automatically when scrolling up.
- Expressive moments, one per screen: the selected-server shape morph on Servers; the send button, which morphs from a circle to a wider pill when there is content to send and shows a `LoadingIndicator` while attachments upload.

#### Push notifications

- One notification channel group per instance, each with channels "Direct messages", "Mentions and replies", "Other messages" (off by default) and "Connection problems" (low importance).
- `MessagingStyle` with the sender's avatar, conversation title `#general · Rust Lyon` or the conversation name, inline reply and "Mark as read" actions.
- Lock screen: the content is hidden by default ("New message from Ana") until the user allows previews in settings. This is a privacy choice for a self-hosted product.
- The first time a user signs in, Mellow explains notifications before showing the Android 13+ permission prompt. If permission is denied, You → Notifications shows "Notifications are off for Mellow. You won't hear about mentions or messages." with "Open Android settings".

### 4.4 Admin web UI (server-rendered HTML + htmx)

Purpose: instance administrators check health and manage users, servers, invites and settings. Usage is occasional, often from a laptop, sometimes from a phone.

Layout:

```
Wide (≥ 960 px)                                   Narrow (< 960 px)
┌──────────────┬───────────────────────────────┐  ┌──────────────────────────┐
│ ◼ chat.ex.org│ Overview                      │  │ ◼ chat.ex.org     Menu ⌄ │
│              │                               │  │ Overview                 │
│ Overview   ▌ │ ┌────────┐┌────────┐┌────────┐│  │ ┌──────────────────────┐ │
│ Users        │ │Healthy ││Users   ││Storage ││  │ │ Healthy              │ │
│ Servers      │ │v0.4.1  ││1,204   ││12/50 GB││  │ └──────────────────────┘ │
│ Invites      │ └────────┘└────────┘└────────┘│  │ ┌──────────────────────┐ │
│ Storage      │ Recent sign-ups               │  │ │ Users 1,204          │ │
│ Backups      │ ┌───────────────────────────┐ │  │ └──────────────────────┘ │
│ Updates      │ │ table                     │ │  │ ...                      │
│ Settings     │ └───────────────────────────┘ │  │                          │
│ Paul · Exit  │                               │  │                          │
└──────────────┴───────────────────────────────┘  └──────────────────────────┘
```

- Left navigation on `surface.canvas`, content on `surface.content` capped at 1120 px. Under 960 px the navigation becomes a `<details>` disclosure menu in the header.
- Every page works without JavaScript for reading (plain links and forms); htmx enhances searching, pagination, inline actions and the health refresh. Progressive enhancement keeps the admin usable when htmx fails to load.
- The health summary refreshes every 15 s with `hx-trigger="every 15s"` only while the page is visible.
- Destructive actions use a native `<dialog>` with the confirmation pattern in section 8, and a server-side confirmation page when JavaScript is off.

---

## 5. Key screens and states

Common state rules for every screen:

- **Loading**: static skeletons shaped like the real content, shown after 300 ms (shorter loads show nothing, to avoid flicker). Never a lone centred spinner.
- **Empty**: a short title, one sentence that explains, one action.
- **Error**: what happened, why if known, what to do, and a retry action. Errors keep any content already loaded visible.
- **Offline**: cached content stays readable; actions that need the network are disabled with a reason; messages typed offline are queued.
- **Reconnecting**: shown in the connection dock (desktop) or a slim banner under the top app bar (Android), never as a modal.

### 5.1 Instance connect, sign in, register

Flow: Welcome → Instance address → Instance card → Sign in or Create account → (first server or invite) → Home.

**Welcome (first run)**: `type.hero` in Fraunces: "Chat that you can host yourself." One sentence: "Connect to a Mellow instance to sign in or create an account." Primary button "Connect to an instance". Secondary text button "I have an invite link".

**Instance address**: one field "Instance address", placeholder `chat.example.org`. We accept full URLs and strip the scheme and path. Primary button "Continue".

| State | Presentation |
|---|---|
| Checking | Button shows a progress indicator and the label "Checking chat.example.org…"; field stays editable; "Cancel" appears |
| Not reachable | Field error: "We couldn't reach chat.example.org. Check the address, or try again if your connection is down." |
| Not a Mellow instance | "chat.example.org is online but doesn't look like a Mellow instance. Check the address with the person who invited you." |
| Incompatible version | "chat.example.org runs Mellow 0.3. This app needs 0.5 or later. Ask the instance admin to update." |
| Certificate problem | "The security certificate of chat.example.org isn't valid, so we didn't connect. The instance admin needs to fix it." No bypass in the MVP (section 9). |
| Offline | Field disabled, info banner: "You're offline. Connect to the internet to add an instance." |

**Instance card**: icon (rounded square), name, description, member count if public, registration mode ("Open to everyone", "Invite only", "Registrations closed"), link to rules. Actions: "Sign in" (primary when the account likely exists, for example coming from "Add an account"), "Create account".

**Sign in**: "Username or email", "Password" (show/hide toggle with accessible label "Show password"), primary "Sign in", link "Forgot your password?" (only when the instance has email configured).

| State | Copy |
|---|---|
| Wrong credentials | "That username and password don't match. Check them and try again." |
| Rate limited | "Too many attempts. You can try again in 5 minutes." (time from the server) |
| Account suspended | "This account is suspended on chat.example.org. Contact the instance admin if you think this is a mistake." |

**Create account**: username (live availability check after 400 ms idle: "Available" with success icon, or "Taken. Try another one."), display name (optional, "Shown to others. You can change it later."), email (if the instance requires it), password with its requirements listed up front ("At least 12 characters"), invite code (prefilled from an invite link, shown only when invite-only), and a checkbox "I agree to the rules of chat.example.org" with a link, only when the instance has rules. Primary "Create account".

**Adding another instance**: the `+` at the bottom of the rail opens a menu: "Create a server", "Join with an invite", "Connect another instance". The connect flow is the same, presented in a dialog on desktop and a full-screen flow on Android.

### 5.2 Server rail

Contents, top to bottom: Home (conversations, with a mention badge aggregating all instances), then for each instance: an instance header (only when ≥ 2 instances) and its servers, then `+`.

| Element state | Presentation |
|---|---|
| Default | Server icon (or initials on a muted tint) with `radius.place` |
| Hover / focus | State layer, tooltip with server name (and instance when ≥ 2) |
| Unread messages | Short selection pill (8 px high) on the leading edge |
| Mentions | Count badge (`accent.container` / `accent.on-container`) on the bottom trailing corner; "99+" beyond 99 |
| Selected | Tall pill (32 px), icon unchanged on desktop; shape morph on Android |
| Muted server | Icon at full opacity, no unread pill; the mention badge is still shown, because direct mentions are never muted |
| Instance offline | The instance header shows an offline ring; its servers are shown at 60 % opacity with tooltip "chat.example.org is unreachable. Showing saved messages." |
| Loading (first sync) | Static rounded-square skeletons for the expected number of servers |
| Empty (no servers) | Only Home and `+`; Home shows the empty state in 5.6 |

Reordering: drag and drop with pointer, `Ctrl+Shift+Up/Down` with keyboard, "Move up"/"Move down" in the context menu. Folders are out of scope for the MVP.

**Connection dock** (desktop, bottom of the channel list) and slim banner (Android):

| State | Copy |
|---|---|
| Connected | Hidden |
| Reconnecting | "Reconnecting to chat.example.org…" with a small progress indicator; after 30 s: "Still trying to reconnect. Messages you send will wait until you're back." with "Retry now" |
| Offline (device) | "You're offline. Showing saved messages." |
| Back online | "Back online" for 2 s, then hidden; missed messages are fetched and the unread divider marks them |
| Session expired | "Your session on chat.example.org has ended. Sign in again to keep chatting." with "Sign in" |

### 5.3 Channel list

Header: server name (`type.title`) with a chevron opening the server menu (Invite people, Server settings, Notification settings, Mark as read, Leave server). Then categories and channels, then the connection dock and the user panel.

| Row | Presentation |
|---|---|
| Read channel | `#` icon and name in `text.secondary` |
| Unread channel | Name in `text.primary`, weight 600, unread dot on the trailing edge |
| Channel with mentions | As unread, plus mention count badge instead of the dot |
| Selected | `state.selected` layer, `text.primary`, 3 px `accent.primary` marker on the leading edge |
| Muted | `text.tertiary`, muted icon after the name, no unread dot |
| Locked (cannot send) | Lock icon replaces `#` with accessible label "read-only" |
| Category | `type.label-md` sentence case, chevron, collapsible; collapsed categories still show unread channels |

States:

- Loading: 6–8 static row skeletons with varied widths.
- Empty server (no channels, user can manage): "No channels yet" + "Create a channel". User cannot manage: "No channels you can see yet. Ask a moderator for access."
- Error: inline banner "Couldn't load channels for Rust Lyon." with "Try again", while keeping cached channels.

User panel: avatar with presence, display name, `@username` (and `@instance` when ≥ 2 instances) in `text.tertiary`, settings button. Clicking the avatar opens the status menu (Online, Idle, Do not disturb, Invisible, plus custom status).

### 5.4 Message list

#### Structure and grouping

- Messages from the same author are grouped when they follow each other within 7 minutes, with no day divider, no unread divider and no reply in between.
- The first message of a group shows avatar (cozy), author name coloured by the top hoisted role (readability-adjusted, 2.3), role icon if any, and timestamp (`type.label-sm`, `text.tertiary`; absolute time today, date for older; full date and time in the tooltip and accessible name).
- Following messages show their time in the leading gutter on hover and focus.
- Day dividers: centred label "Today", "Yesterday", or a locale-formatted date ("Monday 13 October 2026"), on a `border.subtle` rule.

#### Unread divider and jump to present

- When opening a channel with unread messages, the list scrolls to the first unread message and shows the divider: an `accent.primary` rule with the label "New since 14:32" (text `accent.primary`).
- A banner pinned to the top of the pane: "12 new messages since 14:32" with "Mark as read" (`Esc`). It disappears when the divider has been seen and the user reaches the bottom.
- When the user is scrolled up and new messages arrive, a "Jump to present" pill appears above the composer, with the count of new messages ("Jump to present · 3 new"). `End` (list focused) or `Esc` (composer focused) jumps. The list never auto-scrolls while the user reads older messages.

#### Replies

- A reply shows a one-line reference above the message: a thin connector, the replied author's 16 px avatar and name, and a single-line excerpt in `text.secondary`. Clicking or pressing `Enter` on it jumps to the original and highlights it with `mention.background` fading over `motion.duration.highlight`.
- If the original was deleted: "Original message was deleted" in `text.tertiary`, not clickable.
- If the original is not loaded yet, jumping loads the surrounding history first (with a skeleton) and keeps the "Jump to present" pill available.

#### Mentions

- A message that mentions you (directly, via a role you have, or `@everyone`) gets `mention.background` and a 2 px `accent.primary` bar on the leading edge. The bar ensures the state is not carried by the background colour alone; the accessible name starts with "Mentions you".
- Mention chips inside text (`@Ana`, `@Moderators`, `#general`) use `accent.container` / `accent.on-container` with `radius.xs`. They are focusable and open the profile, role description or channel.

#### Edited, deleted, pending, failed

| State | Presentation |
|---|---|
| Edited | "(edited)" after the text, `type.label-sm`, `text.tertiary`; tooltip and accessible description give the edit time |
| Deleted by author | Removed from the list for everyone |
| Deleted by a moderator | Removed; the moderation log keeps the record (server settings) |
| Pending | Text in `text.tertiary`; after 2 s a "Sending…" label appears |
| Queued offline | `text.tertiary`, label "Will send when you're back online", with "Cancel" |
| Failed | `danger.primary` icon, "Not sent." followed by "Retry" and "Delete" buttons; the text stays intact so nothing is lost |

#### Attachments

- Images: shown inline, aspect ratio preserved, max 400 × 320 (cozy) or 320 × 240 (compact), `radius.md`, placeholder in a dominant-colour block while loading. Click or `Enter` opens the viewer (zoom, next/previous, "Save image", "Copy link", close with `Esc`). Alt text, when present, is the accessible name and is shown in the viewer; when absent, the accessible name is "Image, no description, file name.jpg".
- Animated images: first frame shown; play on hover or focus (or always, per accessibility setting).
- Other files: a `surface.raised` card with a file-type icon, file name (middle-truncated, full name in tooltip and accessible name), size, and a "Download" icon button with label "Download file name.pdf".
- Loading failed: "Couldn't load this image." with "Try again", the card keeps its size.
- Removed by the server (for example, retention policy): "This file is no longer available."

#### Hover and focus actions

On hover or focus, a small toolbar appears on the trailing top of the message: Reply, Edit (own), More (menu). The menu adds Copy text, Copy message link, Mark unread, Delete (own, or with "Manage messages"), and moderation actions for moderators. All are reachable by keyboard (7.2).

#### List states

| State | Presentation |
|---|---|
| Initial load | 5 static message skeletons (avatar circle, name bar, 1–3 text bars) |
| Loading older history | 3 skeletons at the top; scroll position is kept stable |
| Beginning of channel | Hero at the top: Fraunces `type.headline` "This is the start of #general", topic below, then "Invite people" (if allowed) and "Edit channel" (if allowed) |
| Empty channel | Same hero, plus composer placeholder "Write the first message in #general" |
| Error (initial) | Replaces the skeletons: "Couldn't load messages in #general. The instance didn't respond." with "Try again" |
| Error (older history) | Inline at the top: "Couldn't load older messages." with "Try again"; newer messages stay visible |
| Offline | Cached messages shown; info pill at the top: "Offline. Showing saved messages."; nothing is hidden |
| No access (permissions changed) | "You no longer have access to #general." with "Go to #welcome" (first accessible channel) |

### 5.5 Composer

Layout: a `surface.raised` field with `radius.lg`, attach button on the leading side, send button on the trailing side (always visible on Android; on desktop the send button appears when there is content, since `Enter` sends).

- Placeholder: "Message #general", or "Message #general as @paul on chat.example.org" when ≥ 2 instances are connected; "Message Ana" in conversations.
- Markdown subset (rendered in messages, typed as plain text in the composer): `**bold**`, `*italic*`, `~~strikethrough~~`, `` `inline code` ``, fenced code blocks with optional language, `> quote`, `||spoiler||`, `- list` and `1. list`, automatic links, `@mentions`, `#channels`. Headings are not supported, to keep conversations at one volume.
- Shortcuts: `Ctrl+B`, `Ctrl+I`, `Ctrl+Shift+X` (strikethrough), `Ctrl+E` (inline code) wrap the selection.
- Autocomplete popovers for `@`, `#` and `:` (emoji shortcodes), navigable with arrows, `Tab`/`Enter` to insert, `Esc` to close.
- Sending: `Enter` sends, `Shift+Enter` inserts a new line (desktop). Android: the keyboard's Enter inserts a new line by default and the send button sends; a setting swaps this for hardware keyboards.
- Edit mode: `Up` in an empty composer edits your last message inline. The message shows "Esc to cancel · Enter to save". Saving an empty message asks to delete it.
- Reply mode: a bar above the field: "Replying to Ana", a switch "Mention Ana" (on by default), and a close button (`Esc`).
- Attachments: attach button, drag and drop anywhere on the message pane (overlay "Drop to attach to #general"), or paste. An attachment tray above the field shows thumbnails with "Add a description" (alt text), "Mark as spoiler" and "Remove". Upload progress is shown per file. Upload errors: "photo.heic is 48 MB. chat.example.org accepts files up to 25 MB."
- Length: a counter appears at 90 % of the instance limit ("3,620 / 4,000"); beyond the limit the counter turns `danger.primary`, the send button is disabled and its accessible description says "Message is 120 characters too long."
- Typing indicator: below the composer, `type.label-sm`, `text.secondary`: "Ana is typing…", "Ana and Léo are typing…", "Ana, Léo and 2 others are typing…", "Several people are typing…" (beyond 5). It is not announced by screen readers.
- Read-only channel: the composer is replaced by "You can't send messages in #announcements." on `surface.raised`.
- Offline: the composer stays usable; sent messages are queued (5.4).
- Draft: the unsent text and reply state are kept per channel across channel switches and app restarts.

### 5.6 Conversations (DMs and groups)

- Home shows conversations in the channel-list column: a search field "Find or start a conversation", then "New message" (primary in this column), then conversations sorted by last activity, each with avatar (group: two overlapping avatars), name, last message preview (one line, `text.secondary`), unread state, and the instance label when ≥ 2 instances are connected.
- New message: a picker to choose one or more people (from shared servers and previous conversations on the same instance), with a visible limit "Up to 9 people" for groups. Conversations cannot mix instances; the picker explains: "People from different instances can't be in the same conversation yet." (federation is out of scope).
- Group conversation header: name (auto-generated from members until renamed), member avatars, actions "Add people", "Rename", "Leave conversation".
- Empty: "No conversations yet. Start one with someone from your servers." with "New message".
- Error and offline: as in 5.4.

### 5.7 Member list and profile

- Groups: hoisted roles in role order ("Moderators — 2"), then "Online — 12", then "Offline — 40". Sentence case, `type.label-md`.
- Row: avatar with presence indicator, display name (role colour, adjusted), custom status on a second line in `text.secondary` (cozy only).
- Accessible name: "Ana, online, Moderator, custom status: Reviewing PRs".
- Large servers: offline members are not listed above 1,000 members ("Offline members are hidden in large servers.").
- Loading: row skeletons. Error: "Couldn't load members." with "Try again".
- Profile popover (click, `Enter`, or `Shift+F10` → "View profile"): avatar 80 px, display name, `@username@instance`, about text, roles as chips, member since, actions "Message" (primary), "Mention", and for moderators a "Moderation" menu (Kick, Ban, Manage roles).

### 5.8 Server settings

Navigation on `surface.canvas` (left on desktop, a list screen on Android compact): Overview, Roles, Members, Channels, Invites, Bans, Moderation log, Delete server (only for the owner, at the bottom, `danger.primary` text).

**Unsaved changes**: a bar slides up at the bottom of the content: "You have unsaved changes." with "Reset" and "Save changes". Leaving the page with unsaved changes opens a dialog: "Leave without saving? Your changes to Roles will be lost." with "Keep editing" and "Leave without saving".

**Roles editor**:

- Left: role list in hierarchy order, each with colour swatch and member count. Reorder by drag and drop, with `Ctrl+Shift+Up/Down`, or with "Move up"/"Move down" buttons. Roles above your highest role are shown but locked, with the reason in the tooltip and accessible description: "You can only edit roles below your highest role."
- Right: tabs "Display", "Permissions", "Members".
- Display: name, colour picker (with the readability preview on both themes), "Show separately in the member list", "Allow anyone to mention this role".
- Permissions: grouped (General, Membership, Text channels, Moderation, Advanced). Each permission is a switch row with a plain-language description, for example "Ban members — Remove people and stop them from rejoining." Dangerous permissions (Administrator, Ban members, Manage roles) carry a warning icon and the word "Sensitive".
- Channel overrides (in channel settings): each permission is a three-state segmented control "Allow / Inherit / Deny" with icons (check, slash, cross) and text labels, never colours alone.
- Empty (only @everyone): "Roles let you group people and give them permissions." with "Create a role".

**Invites**: table of active invites (code in mono, created by, uses / maximum, expires). "Create invite" dialog: "Expire after" (30 minutes, 1 hour, 1 day, 7 days, Never), "Maximum uses" (No limit, 1, 5, 10, 25, 100), result with the full link, "Copy link" (confirmation "Link copied"). Revoking asks for confirmation (section 8). Empty: "No active invites." with "Create invite".

**Bans**: searchable list (avatar, name, reason, banned by, date). Action "Revoke ban". Empty: "No one is banned from Rust Lyon."

**Kick and ban dialogs**: see section 8; the ban dialog includes "Delete their recent messages" (Don't delete, Last hour, Last 24 hours, Last 7 days) and an optional reason shown in the moderation log.

### 5.9 User settings

Sections: My account (per instance), Profile, Appearance, Accessibility, Notifications, Instances, Keyboard shortcuts (desktop), Language, About.

- My account: display name, username, email, password, active sessions with "Sign out of this session", and "Delete account" in a separate danger zone.
- Profile: avatar upload with crop (circle preview), about text with counter.
- Appearance: Theme (Match system, Dark, Light), Density (Cozy, Compact, desktop only), Text size slider 80–200 % with a live message preview, "Limit line length", "Show avatars in compact mode".
- Accessibility: Reduce motion (Match system, Reduce motion, Allow motion), Play animated images (On hover or focus, Always, Never), Underline links (on by default, see section 9), "Announce new messages in the open channel" (screen readers).
- Notifications: per instance, defaults for new servers ("Mentions and conversations" by default), sounds, lock-screen previews (Android).
- Instances: list with status (Connected, Reconnecting, Signed out), "Sign out", "Connect another instance".
- About: version, open-source licences, link to the source code (AGPL-3.0 requires offering it), instance versions.

### 5.10 Admin web dashboard

- Overview: health card (status in words: "Healthy", "Degraded: database is slow", "Down: storage unavailable"), version and update notice, uptime, users (total, active in the last 7 days), servers, storage used and limit with a plain bar, push gateway status, active connections.
- Users: searchable table (username, display name, email if any, created, last seen, status). Actions: "Suspend", "Reset password link", "Delete account" (typed confirmation).
- Servers: table (name, owner, members, created). Actions: "View", "Transfer ownership", "Delete server".
- Invites and registration: registration mode, instance-wide invite codes.
- Storage and backups: usage by type (attachments, avatars), upload size limit, last backup time and result, with "Run backup now".
- Updates: current and latest version, release notes link, and the update status.
- Settings: instance name, description, icon, rules text, email, push gateway.
- States: loading uses static table-row skeletons for htmx swaps; empty tables say what would appear and how ("No users yet. Share an invite to let people sign up."); errors render inline above the table with "Try again"; a lost connection to the server shows a top banner "Can't reach the server. Data on this page may be out of date."

---

## 6. Components inventory

All components use tokens only, support light and dark themes, both densities where relevant, text scaling to 200 %, keyboard focus and screen readers. Desktop components live in `crates/mellow-desktop`; Android components live in `:core:designsystem`; admin web components are HTML partials with CSS classes.

| # | Component | Variants | States | Notes |
|---|---|---|---|---|
| 6.1 | Button | Primary (`accent.primary`), Secondary (tonal: `surface.raised` + `border.strong`), Ghost (text only), Danger (`danger.primary`) · sizes Medium (36 desktop / 48 Android / 44 web), Small (32 desktop) | Default, hover, pressed, focus, disabled, loading (label kept, progress inline) | Pill shape. One primary button per view. Labels are verbs ("Save changes"). |
| 6.2 | IconButton | Standard, Toggle (pressed state), Danger | Same as Button, plus selected | Always has a tooltip and an accessible label. Hit area ≥ `size.hit-min-*`. |
| 6.3 | TextField | Single line, Multiline, Password (show/hide), Search (with clear) | Default, hover, focus, filled, error (message below, `danger.primary` icon + text), disabled, read-only | Label above the field, never placeholder-only. `surface.sunken` + `border.strong`. |
| 6.4 | Select / Menu button | Single select | Closed, open, focus, error, disabled | Uses the Menu component for options. |
| 6.5 | Checkbox, Switch, Radio group | — | Off, on, mixed (checkbox), focus, disabled | Switch for immediate settings, checkbox in forms that need "Save". |
| 6.6 | SegmentedControl | 2–4 options; tri-state permission variant (Allow / Inherit / Deny) | Selected, focus, disabled | Icon + text in each segment. |
| 6.7 | Slider | Continuous, Stepped (text size: 80, 90, 100, 115, 130, 150, 175, 200 %) | Focus, dragging, disabled | Arrow keys step; value announced. |
| 6.8 | Avatar | Sizes 16, 24, 32, 40, 80, 120 · image, initials fallback, group (two overlapping) | Loading (tint block), with presence, animated (play on hover/focus) | Always a circle. |
| 6.9 | PresenceIndicator | Online, idle, dnd, offline | — | Shape + colour; surface cut-out ring. |
| 6.10 | Skeleton | Text line, avatar, message, row, server icon, table row | Static | No shimmer; `color.state.hover` fill on the current surface; hidden from screen readers, container announces "Loading messages". |
| 6.11 | ServerIcon | Server, instance header, Home, Add (`+`) | Default, hover, focus, unread, mention badge, selected, offline, dragging | `radius.place`; morph on Android when selected. |
| 6.12 | ChannelRow | Text channel, category header | Read, unread, mentions, selected, muted, locked, hover, focus, dragging | Category collapse with `Left`/`Right`. |
| 6.13 | ConversationRow | 1:1, group | Read, unread, selected, offline instance | Instance label when ≥ 2 instances. |
| 6.14 | MessageRow | Group start (cozy/compact), continuation, system message (joins, pins) | Default, hover, focus, mentioned, highlighted (jump target), pending, queued, failed, editing | Accessible name includes the author even on continuations. |
| 6.15 | ReplyReference | Normal, original deleted, original unloaded | Hover, focus | Jumps to original. |
| 6.16 | MentionChip | User, role, channel, `@everyone` | Hover, focus | `accent.container` colours. |
| 6.17 | AttachmentImage | Static, animated, spoiler (blurred with "Show spoiler") | Loading, loaded, error, removed | Alt text as accessible name. |
| 6.18 | AttachmentFile | — | Default, downloading (progress), downloaded, error | Middle-truncated file name. |
| 6.19 | CodeBlock | Inline, block | — | `type.code`, `surface.sunken`, "Copy code" button on blocks, horizontal scroll inside the block only. |
| 6.20 | Divider | Day, unread ("New since …") | — | Unread divider in `accent.primary`. |
| 6.21 | JumpToPresentPill | With / without new count | Hover, focus | Above the composer, trailing side. |
| 6.22 | Composer | Channel, conversation, edit inline, read-only | Empty, typing, with attachments, uploading, too long, offline, disabled | Includes reply bar and attachment tray. |
| 6.23 | AttachmentTray item | Image, file | Uploading, uploaded, error, spoiler on | "Add a description" for alt text. |
| 6.24 | TypingIndicator | 1, 2, 3–5, many | Static under reduced motion | Not announced to screen readers. |
| 6.25 | Autocomplete popover | Users, channels, emoji | Results, no results ("No one called anx in Rust Lyon"), loading | Listbox semantics. |
| 6.26 | MemberRow | Cozy (two lines), compact | Hover, focus, offline (60 % opacity on avatar only, text stays at full contrast) | Group headers sentence case. |
| 6.27 | ProfilePopover | Self, other, other with moderation | Loading, error | `elevation.2`. |
| 6.28 | Menu / ContextMenu | With icons, with danger items (at the bottom, separated) | Item hover, focus, disabled with reason | Arrow keys, type-ahead, `Esc`. |
| 6.29 | Tooltip | — | Shown after 500 ms hover or immediately on focus | Never contains the only copy of essential information. |
| 6.30 | Dialog | Standard, destructive, typed confirmation | Default, submitting, error inline | Focus trapped, `Esc` closes (except while submitting), focus returns to the trigger. Bottom sheet on Android compact. |
| 6.31 | Toast / Snackbar | Info, success, with action ("Undo") | Auto-dismiss after 5 s (paused on hover/focus), dismissible | Polite live region; never used for errors that need action. |
| 6.32 | Banner | Info, offline, warning, error, success | With / without action, dismissible or persistent | Container tokens from 2.3. |
| 6.33 | ConnectionDock | Hidden, reconnecting, offline, back online, session expired | — | Future voice slot. |
| 6.34 | EmptyState | Hero (Fraunces), compact | With one primary action | One title, one sentence, one action. |
| 6.35 | Badge | Count (mention), dot (unread) | — | Count capped at "99+"; accessible text "3 mentions". |
| 6.36 | RoleChip | With colour dot | Removable (in role assignment) | Colour dot plus name, never colour alone. |
| 6.37 | PermissionRow | Switch (role), tri-state (channel override) | Sensitive marker, locked (above your role) | Plain-language description. |
| 6.38 | SettingsNav | Desktop sidebar, Android list | Selected, focus, danger item | — |
| 6.39 | UnsavedChangesBar | — | Visible, saving, error | `elevation.3`. |
| 6.40 | Tabs | Standard | Selected, focus | Arrow keys move between tabs. |
| 6.41 | PaneResizeHandle | Vertical | Hover, dragging, focus | Keyboard resize. |
| 6.42 | AdminTable (web) | With search, pagination, row actions | Loading rows, empty, error | `<table>` with `<caption>` and `scope` headers. |
| 6.43 | AdminStatCard (web) | Health, number, storage bar | Normal, degraded, down | Status in words plus icon. |

Before adding a component, check this list. A new component is added here first, with variants and states.

---

## 7. Accessibility

Target: WCAG 2.2 level AA on all three surfaces, verified with real assistive technologies.

### 7.1 Contrast and colour

- Text pairs and non-text pairs in section 2.3 meet AA in both themes and are enforced by the token generator.
- Information is never carried by colour alone: presence shapes, unread weight and markers, mention bar, icons in permission states, words in status messages.
- Role colours are adjusted for readability (2.3).

### 7.2 Keyboard (desktop)

`Ctrl` means `Cmd` on macOS. Shortcuts are listed in Settings → Keyboard shortcuts and in a cheat sheet (`Ctrl+/`).

| Area | Keys | Action |
|---|---|---|
| Global | `F6` / `Shift+F6` | Move focus between regions: rail → channel list → message list → composer → member list (and header) |
| Global | `Ctrl+K` | Quick switcher (servers, channels, conversations across instances) |
| Global | `Alt+Up` / `Alt+Down` | Previous / next channel or conversation |
| Global | `Alt+Shift+Up` / `Alt+Shift+Down` | Previous / next unread channel |
| Global | `Ctrl+Alt+Up` / `Ctrl+Alt+Down` | Previous / next server |
| Global | `Ctrl+U` | Toggle member list |
| Global | `Ctrl+Shift+D` | Open the navigation drawer (narrow windows) |
| Global | `Ctrl+N` | New message (conversation) |
| Global | `Ctrl+,` | Settings |
| Global | `Ctrl+=` / `Ctrl+-` / `Ctrl+0` | Text size up / down / reset |
| Global | `Esc` | Close the topmost popover, menu or dialog; otherwise mark the channel as read and jump to present |
| Global | `Shift+Esc` | Mark the whole server as read |
| Rail, channel list, member list | `Up` / `Down`, `Home` / `End` | Move focus (roving focus, one tab stop per list) |
| Rail, channel list | `Enter` / `Space` | Open |
| Channel list | `Left` / `Right` | Collapse / expand category |
| Rail, role list, channel list (settings) | `Ctrl+Shift+Up` / `Ctrl+Shift+Down` | Move the focused item up or down |
| Any focusable item | `Shift+F10` or Menu key | Context menu |
| Message list | `Up` / `Down` | Previous / next message |
| Message list | `PageUp` / `PageDown`, `End` | Scroll, jump to present |
| Message list (focused message) | `R` reply, `E` edit (own), `Delete` delete (with confirmation), `Shift+Delete` delete own message without confirmation, `C` copy text, `Enter` open actions menu | Message actions |
| Message list | `Tab` | Move into the focused message's links, chips, attachments |
| Composer | `Enter` / `Shift+Enter` | Send / new line |
| Composer | `Up` (empty) | Edit last message |
| Composer | `Ctrl+Shift+U` | Attach a file |
| Composer | `Ctrl+B` / `Ctrl+I` / `Ctrl+E` / `Ctrl+Shift+X` | Bold / italic / inline code / strikethrough |
| Dialogs | `Tab` cycle, `Esc` cancel, `Enter` primary action when the focus is in a field | — |

Focus visibility: a 2 px `color.focus` ring with a 2 px gap, following the element's radius, shown on keyboard focus only (`:focus-visible` behaviour), never removed. Focused elements are never obscured by sticky bars (WCAG 2.4.11): the message list scrolls the focused message clear of the composer and the unread banner.

### 7.3 Screen readers

Desktop (Slint with AccessKit: VoiceOver, NVDA/Narrator, Orca):

- Every interactive element sets `accessible-role`, `accessible-label` and, where useful, `accessible-description` and `accessible-checked`/`accessible-value`.
- Regions have names: "Servers", "Channels in Rust Lyon", "Messages in #general", "Message composer", "Members of Rust Lyon".
- Message accessible name pattern: "[Mentions you.] Ana, Moderator, 14:32: Has anyone tried Slint on Wayland? Edited. 2 attachments. Replying to Léo." Continuation messages repeat the author.
- Badges are text: "Rust Lyon, 3 mentions, unread".
- New messages in the open channel are announced politely when the user is at the bottom, if the accessibility setting is on (requires live-region support, section 9).
- Decorative elements (skeleton bars, dividers, presence cut-outs) are hidden from the tree.

Android (TalkBack, Switch Access, keyboards):

- Message rows merge descendants; custom accessibility actions expose Reply, Edit, Copy text, Mark unread, Delete.
- `contentDescription` on meaningful icons and images, `null` on decorative ones; presence is in the row's description ("online").
- Headings semantics on the channel name and settings section titles; pane titles for list-detail panes.
- Typing indicators are excluded from announcements; send results use polite live regions ("Message sent" only when sending was slow or failed before).

Admin web: semantic HTML first (`<nav>`, `<main>`, `<table>`, `<label>`), `aria-live="polite"` on the htmx-swapped status regions, focus moved to the new content heading after a page-level htmx swap, skip link "Skip to content".

### 7.4 Targets

| Surface | Minimum target | Notes |
|---|---|---|
| Android | 48 × 48 dp (`minimumInteractiveComponentSize`) | Visual size can be smaller; the touch area is not. |
| Admin web | 44 × 44 CSS px | Includes table row actions. |
| Desktop | 32 × 32 px (24 px absolute minimum, WCAG 2.5.8) | Pointer-first. Touchscreen laptops are an open question (section 9). |

Drag-and-drop actions (reordering servers, roles, channels) always have a non-drag alternative (WCAG 2.5.7).

### 7.5 Text scaling and reflow

- Desktop: 80–200 % text size; all containers grow with content; no fixed-height text boxes; at 200 % the channel list wraps long names to two lines rather than truncating them in settings, and truncates with a tooltip only in the rail tooltip and narrow lists.
- Android: system font scale up to 200 % (non-linear on Android 14+); layouts tested at `fontScale = 2f` in previews.
- Admin web: works at 400 % zoom (reflow at 320 px width, WCAG 1.4.10).

### 7.6 Motion, time and media

- Reduced motion as in 2.8.
- No time limits on reading; toasts with actions stay while hovered or focused and actions remain available elsewhere (for example "Undo delete" is also possible from the message's menu for 10 s).
- Spoilers and animated media require a user action to reveal or play by default.

---

## 8. UX copy: voice and tone

Mellow speaks like a calm friend who also happens to run the server: plain, warm, direct. It never blames the user, never apologises theatrically, and never jokes when something went wrong.

Rules:

- Sentence case everywhere, including buttons and titles.
- Active voice, second person ("You can't send messages here"), first person plural for the app only when it acts ("We couldn't reach…").
- Buttons name the action: "Save changes", "Ban Ana", "Create invite". Never "OK", "Submit", "Yes".
- Results echo the action: "Changes saved", "Invite link copied", "Ana was banned".
- Errors say what happened, why if we know, and what to do. No "Oops", no "Something went wrong" without a next step, no error codes in the main sentence (codes go in a "Details" disclosure for admins).
- Empty states invite the next action in one sentence.
- No exclamation marks. No emoji in system copy.
- Use the user's words: "server", "channel", "conversation", "instance". Never "guild", "DM" in headings ("Messages" is fine), "payload", "websocket", "token".
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
| Empty server | "No channels yet. Create the first one to get the conversation going." · "Create a channel" |
| Empty conversations | "No conversations yet. Start one with someone from your servers." · "New message" |
| Empty bans | "No one is banned from Rust Lyon." |
| Kick confirmation | Title "Kick Ana from Rust Lyon?" · Body "Ana will be removed from the server. They can rejoin with a new invite." · Buttons "Cancel" / "Kick Ana" |
| Ban confirmation | Title "Ban Ana from Rust Lyon?" · Body "Ana will be removed and won't be able to rejoin, even with an invite." · Field "Reason (optional, visible to moderators)" · Select "Delete their recent messages" · Buttons "Cancel" / "Ban Ana" |
| Delete channel | Title "Delete #old-announcements?" · Body "All 1,204 messages in this channel will be deleted for everyone. This can't be undone." · Field "Type old-announcements to confirm" · Buttons "Cancel" / "Delete channel" (enabled when the name matches) |
| Delete own message | Title "Delete this message?" · Body "It will be removed for everyone." · Buttons "Cancel" / "Delete message" · Hint "Tip: hold Shift while deleting to skip this next time." |
| Revoke invite | Title "Revoke this invite?" · Body "The link will stop working. People who already joined stay in the server." · Buttons "Cancel" / "Revoke invite" |
| Session expired | "Your session on chat.example.org has ended. Sign in again to keep chatting." · "Sign in" |
| Notifications denied | "Notifications are off for Mellow. You won't hear about mentions or messages." · "Open Android settings" |

Destructive confirmation pattern:

- The title is a question naming the object.
- The body states the consequence and whether it can be undone.
- The destructive button repeats the verb and the object, uses the Danger variant, and is never the default focus. Default focus goes to "Cancel".
- Typed confirmation only for irreversible actions affecting many people (deleting a channel, a server, an account, an instance user).

---

## 9. Open questions and items to validate

Product and vocabulary:

1. **"Server" vs "instance"**: we use "server" for guilds (familiar) and "instance" for installations (jargon). Validate with five users that "instance" is understood after the one-line explanation on the welcome screen; alternative candidates are "host" and "community".
2. **Message search**: post-MVP according to ARCHITECTURE.md (SQLite FTS5). The header reserves the slot and `Ctrl+F` is kept free for it; confirm the timing, since users expect it early.
3. **Group conversation size**: "Up to 9 people" (10 including you) is assumed; confirm the server limit.
4. **Cross-instance conversations**: excluded (no federation). Confirm the copy and that the picker should hide people from other instances rather than disable them.
5. **Default notification level** for new servers ("Mentions and conversations"): confirm with the product owner; it is calmer than Discord's "All messages" for small servers.
6. **Emoji picker, reactions, link previews and custom emoji** are not in the MVP list; autocomplete for `:shortcodes:` is specified. Confirm scope.
7. **Self-signed certificates**: self-hosters often use them. The MVP refuses invalid certificates without a bypass. Decide whether to add certificate pinning with an explicit trust-on-first-use flow later (needs a security review).

Platform and technical:

8. **Slint rich text in the composer**: Slint's text input does not render inline styles today, so the composer shows raw Markdown. Check the Slint version's styled-text support; inline formatting preview would improve the experience.
9. **Slint live regions**: verify that the Slint/AccessKit version in use exposes live-region announcements on all three desktop platforms; otherwise "Announce new messages" is Android-only in the MVP.
10. **Variable fonts in Slint**: confirm Slint renders variable font axes (`SOFT`, `wght`). If not, the build generates static instances (Atkinson Next 400/600/700, Fraunces Soft 600). Check whether Fraunces's OFL declares Reserved Font Names, which would require renaming a modified instance.
11. **Reduced-motion detection on Linux desktops** beyond GNOME (KDE, others): find the reliable signals or rely on the in-app setting.
12. **Bottom-anchored, virtualised message list in Slint**: confirm `ListView` can keep scroll position stable when prepending history and anchoring at the bottom; a custom list may be needed. Performance must be measured with 10,000 loaded messages.
13. **Emoji font size**: Noto Color Emoji adds about 10 MB to the desktop binary. Decide between bundling it, using the system emoji font, or using a smaller subset.
14. **Script coverage**: decide which Noto subsets to bundle (Cyrillic, Greek, CJK, Arabic, Hebrew, Devanagari) versus relying on system fallback, and validate right-to-left message rendering.
15. **Android dynamic colour**: off for brand identity. Consider an opt-in "Use wallpaper colours" setting after the MVP if users ask.
16. **Material 3 Expressive APIs**: verify `MaterialExpressiveTheme`, `MaterialShapes` morphing and `LoadingIndicator` in the pinned Compose Material 3 version before implementing; fall back to stable APIs if they are still experimental and unstable.
17. **Touchscreen desktops** (Windows tablets, touch laptops): consider a "Touch" density that raises desktop targets to 44 px.
18. **Link underlines**: links are underlined in messages by default in both themes in this draft; validate that this does not make code-heavy channels noisy, and keep the setting.

Visual validation still to do:

19. Render the palette on real screens (OLED phone, low-quality laptop IPS, sunlight) and check that `surface.canvas` and `surface.content` are distinguishable in dark mode on cheap panels (the step is intentionally small).
20. Check Apricot versus Saffron distinguishability for the common colour-vision deficiencies (protanopia, deuteranopia, tritanopia) with simulation, even though neither carries meaning alone.
21. Usability test the multi-instance rail grouping with users who have two or more instances.
22. Test with real assistive technologies: VoiceOver (macOS), NVDA and Narrator (Windows), Orca (Linux), TalkBack and Switch Access (Android).

---

## 10. Defaults we rejected

For each, the reason it was rejected for Mellow:

| Default | Why we rejected it | What we do instead |
|---|---|---|
| Discord's blurple and cool blue-grey surfaces | Copies the product we want to improve on, and cool greys feel clinical | Warm Ink/Paper neutrals with an Apricot accent |
| Red mention badges | Red signals danger and creates anxiety for routine pings | Apricot mention badges; red reserved for danger |
| Four or five grey levels in the frame | Visual noise competing with messages | Two planes (canvas and content) |
| Inter or the system UI font | Generic, and weaker character distinction for usernames and codes | Atkinson Hyperlegible Next and Mono, Fraunces Soft for rare display moments |
| Tracked uppercase section labels | Harder to read, shouty | Sentence-case labels at 13 px semibold |
| Circular server icons | Confuses places with people | People are round, places are square |
| Shimmering skeletons and per-item entrance animations | Restless, contradicts "unhurried" | Static skeletons, no list animations |
| Material baseline purple and dynamic colour on Android | Template look, loses the identity | Brand colour scheme generated from tokens |
| Expressive motion everywhere on Android | Too bouncy for a calm product | `MotionScheme.standard()`, expressive only for two signature moments |
| Glassmorphism, gradients, glow | Decoration without function | Flat warm surfaces, shadows only on floating layers |
| Hover-only message actions | Inaccessible to keyboard and touch users | Hover, keyboard, context menu and accessibility actions |
| Silent connection problems | Users lose messages and trust | Connection dock, labelled pending/queued/failed messages |
