# Web client: every screen size, and installable as an app

DESIGN.md §4.5 (web client), §2.10 (icons) and §7.4–7.5 (targets, reflow) in the Dusk direction ([ADR-0022](../adr/0022-give-mellow-its-own-visual-language-on-a-familiar-layout.md)). The owner's request: make the web app perfectly responsive, and installable from Chrome as an app. This page records what was checked, what changed and why, so later screens keep the same rules.

## Brief

- **Who**: members of small self-hosted communities who open their instance on a phone as often as on a laptop, many of them from a link someone sent; the owner wants it to feel like an app in the dock or on the home screen, without an app store.
- **Job**: every screen usable from a 320 px phone to a 1920 px monitor, in portrait and landscape, with the soft keyboard open and with text at 200 %; one tap to open Mellow like any other app.
- **Constraints**: the layouts and breakpoints of DESIGN.md §4.2 and §4.5 stay (they are the accepted Dusk navigator); tokens only; trivial self-hosting (ADR-0019: the server binary serves everything, with its Content-Security-Policy); nothing personal may ever be cached by the browser on the client's behalf.

## How the screens were checked

A headless Chrome, driven over the DevTools protocol against a local instance seeded with long names, a long topic, a long message with an unbreakable word and a long URL, a fenced code block, a quote, a spoiler, a reply, an image and a file with a long name, seven reactions, a pin, friends, a pending request and a conversation. For each width (320, 360, 390, 414, 600, 768, 820, 1024, 1280, 1440 and 1920 px) and each state (sign-in, chat, navigator drawer, members sheet, member profile, Home with Friends, Pending, Add friend, a conversation, quick switcher, emoji picker, user settings and Profile, server menu, server settings and Roles, channel settings and Permissions, status menu, pins, create channel, add a server, a message's More menu, the "New message" people picker, a group with a long name, its members sheet and its Rename and Leave dialogs, the composer with a long draft, long content, attachments and reactions, and a phone with the keyboard open), the script took a screenshot and listed:

- the page scrolling sideways;
- any interactive element past a window edge, or cut by a parent that hides its overflow;
- under a coarse pointer (touch emulation, so `pointer: coarse` matches), any target under 44 × 44 px.

Then again in phone landscape (740 × 360, 844 × 390), and with the root font size at 200 % (every size token is in `rem`, so this is the browser's text-size setting at its worst: text grows inside the same window).

## What changed, per screen

| Screen | Problem found | Change |
|---|---|---|
| Navigator drawer (< 816 px) | Its card kept the full 72 + 240 px while the drawer is capped at the window minus the 56 px peek: at 320 px the server menu's chevron, the channel gear and the user card's settings button were cut off and unreachable; at 360 px the rows touched the edge. | In the drawer the channel list column is `minmax(0, component.sidebar-width)`: it gives up width and truncates on the narrowest phones. |
| Home, Friends tabs | "All" was 32 px wide under touch. | Tabs are at least 44 px wide (and centred) under a coarse pointer. |
| Navigator, Friends row and conversation rows | 42 px high under touch (the member row height). | 44 px under a coarse pointer, like channel rows. |
| Quick switcher | With a long server name the channel's own name vanished (the server name never shrank). | The trailing "where" label truncates and takes at most half the row; the channel or person keeps the rest. The placeholder ends with an ellipsis instead of a cut word. Its offset and height use `dvh`. |
| Chat, file cards | A long file name lost its extension on phones: it was cut to 40 characters in the middle, then CSS cut the rest at the end ("un-fichier-a…"). | The name is split for CSS (`fileNameStartAndEnd`): the start truncates, the extension and four characters before it always show ("un-fi…nale.txt"), at any width. |
| Chat, unread bar and jump bar | "You're viewing older …" and "12 new messages since …" were cut on 320 px phones (longer still in French). | The sentence wraps onto a second line (the bar's height becomes a minimum); the action keeps its place. |
| Chat, reactions | The Add reaction chip was 34 px wide under touch. | At least 44 px wide under a coarse pointer. |
| Chat header (channel, conversation or group) | The title block never shrank: with a long group name on a phone, or beside "Go to…" between 816 and about 900 px, the last header button (Member list, Add friends to the group) was pushed past the edge. | The title truncates further instead; the buttons always fit. |
| Home, Friends header (816–900 px) | Beside the title the tabs scrolled sideways with no scrollbar, hiding Add friend (touch sizes, French labels). | The tabs wrap onto a second row and the header grows, as on phones. |
| Group members sheet, people picker | The full-window group sheet had no safe-area padding; the picker's chips were 28 px under touch. | Same rules as the server members sheet and every other target. |
| Message More menu, friend More menu | Opened under the button even near the bottom of the window, so on a phone the menu of the last message or friend fell below the screen. | `menu-placement.ts` opens the menu above the button when there is no room below (and never past the top). |
| Every screen, notched phones and the installed app | No `viewport-fit=cover`: on iOS the page was letterboxed, and nothing reserved room for a status bar or a home indicator. | The page runs under them; the shell, the drawer's navigator, the full-window members sheets (server and group), the phone settings sheet and the bottom sheets (emoji picker, pins) pad themselves by `env(safe-area-inset-*)` (`safe-area-padding` utilities in `app.css`). |
| Chat on phones, keyboard open | The keyboard could cover the composer where the browser overlays it on the page. | `interactive-widget=resizes-content`: the keyboard shrinks the layout (Chrome and Firefox on Android); the shell is `100dvh`, the message list stays on the latest message (its `ResizeObserver`), so the composer sits right above the keyboard. |

Everything else held at every width: the sign-in card, the chat (long words and URLs wrap, code blocks scroll inside their block, reactions wrap; an inline spoiler stays a line of text tall, as WCAG 2.5.8 allows for targets inside a sentence), the composer and its tray, the side sheet docked, floating and full-window, the profile, the settings sheets (list page then section page on phones), the dialogs (`100dvh` caps, scrolling body), the emoji picker and the pins panel (bottom sheets on phones), the status menu and the server menu.

## Installable app

- **Manifest** (`web/public/manifest.webmanifest`, generated by `cargo xtask tokens`): `id`, `start_url` and `scope` `/`, `display: standalone`, name and short name "Mellow", `theme_color` and `background_color` the ground (`color.surface.rail`) of the dark theme, Mellow's default. The manifest has one colour per field, so light is carried by the page: `index.html` has one `theme-color` meta per `prefers-color-scheme`, written by the same command between two marker comments, and an installed window's title bar is the ground it frames in either theme.
- **Icons** (`web/public/icons/`, same command, same favicon pipeline as the desktop icons): 192 and 512 px (the rounded tile, `purpose: any`), a 512 px **maskable** tile with square corners (Android launchers cut their own shape; Melli stays inside the 40 % safe zone), and a 180 px Apple touch icon from the maskable tile.
- **No service worker.** Chrome (since version 108 on Android and 112 on desktop) and Edge install a site with a valid manifest without one. A worker would only add risk: a cached client could keep talking to a newer server after an update, and a cache could keep something personal on a shared device. Without it, an installed window always loads the client the server serves, like a tab; the API stays versioned (`/api/v1`) for the moment between an update and the next reload. An offline shell, if ever wanted, needs its own ADR.
- **Server** (`crates/mellow-server/src/http/web_client.rs`): `.webmanifest` is served as `application/manifest+json`; the icons as `image/png`; both with `no-cache` like every unhashed file, `nosniff`, and the same Content-Security-Policy, which now names `manifest-src 'self'` (it was already allowed through `default-src`; naming it keeps it allowed if `default-src` is ever narrowed). No `worker-src`, since no worker is served. A unit test checks the type, the cache header and the policy.
- **No install banner.** Chrome and Edge show their own install button in the address bar, Android Chrome in its menu. An "Install app" entry in user settings (`beforeinstallprompt`) is possible later if the owner wants it.

### How installation was checked

Against an instance built from this branch (`cargo build -p mellow-server --features web-client`), headless Chrome 156 reported the manifest with no errors (`Page.getAppManifest`) and no installability errors (`Page.getInstallabilityErrors`), with the app id `/`. Headless Chrome has no `PWA.install`, so the installed window was approximated by Chrome's app window (`--app`) with `display-mode: standalone` emulated: the matching `theme-color` meta follows the scheme (`#110B11` dark, `#F5F1F6` light), and with emulated safe-area insets (47 px top, 34 px bottom) the shell and the drawer keep clear of them while the ground fills them.

## Defaults rejected

| Default | Why not | Instead |
|---|---|---|
| Tailwind's `sm`/`md`/`lg` breakpoints for phone tweaks | They match none of Mellow's panes | The token-summed breakpoints of §4.2; components adapt inside them |
| A separate mobile layout (tab bar, stacked pages) | The owner accepted the navigator; a second layout doubles every screen | The same navigator as a drawer, the side sheet full-window |
| A service worker and an offline shell | Not needed to install; stale client and cached personal data risks | None |
| An install banner or a first-run "Add to home screen" sheet | Nags, and duplicates the browser | The browser's own install entry |
| `100vh` everywhere | Ignores the mobile URL bar and the keyboard | `100dvh` and `interactive-widget=resizes-content` |

## Known limit

Every screen reflows down to 320 px of layout width, which is what WCAG 1.4.10 asks. Below that, the navigator drawer cannot hold the 72 px strip and readable channel names: a 360 px phone with text at 200 % lays out as 180 px, and there the drawer shows the strip, channel icons only and a crowded user card (its settings button is cut off in development builds, where the microphone and headset toggles show). The chat, settings and dialogs stay usable at that size. A narrower strip or a stacked navigator for such widths would be a layout change for the owner to decide.

## Not verified

- Real devices: an Android phone with Chrome (keyboard resizing, the maskable icon on the launcher, the standalone status bar), an iPhone with Safari (safe areas with the notch and the home indicator, "Add to Home Screen", whether the keyboard covers the composer: Safari ignores `interactive-widget` and overlays the keyboard, so the composer may need a `visualViewport` follow-up there), a touchscreen laptop.
- An installation through Chrome's real install flow (address bar button, Android "Install app"): headless Chrome cannot install, so the window above is an approximation.
