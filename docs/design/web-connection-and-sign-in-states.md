# Web: connection, notification and sign-in states (design pass WP7)

Status: implemented on `feat/web-connection-and-sign-in-states`, 2026-11-05. Findings X1 to X4 of [web-design-pass-2026-11.md](web-design-pass-2026-11.md) (section 3.13). No new token: every colour below is an existing semantic or component token, and every pair is already declared in `design/tokens.json`.

## Brief

The web client is how most people reach a self-hosted Mellow instance, often on a phone or a flaky network. The job of these states is to say, at a glance and in the reader's words, whether what they see is live and what to do about it, without taking the screen from the conversation. The sign-in card is the one screen where Melli is the hero (DESIGN.md 5.1).

## States

| State | When | What shows |
|---|---|---|
| Connecting | First connection of the session | Amber bar, `wifi-slash`, "Connexion…" (unchanged) |
| Offline | The browser reports the device offline (`online` from `svelte/reactivity/window`) | Amber bar, "Tu es hors ligne. Les messages enregistrés restent lisibles." / "You're offline. Showing saved messages." It comes first: it explains the rest, and the socket can take a heartbeat interval to notice |
| Checking | The browser says online again while the socket still looks open: the gateway sends a heartbeat to learn whether it survived (`MellowClient.isCheckingLink`, up to 10 s) | Amber bar, "Connexion…": the browser's `online` does not mean the socket is live |
| Reconnecting | The gateway lost its socket | Amber bar, "Connexion perdue. Reconnexion…" (unchanged) |
| Still reconnecting | 30 s of reconnecting (`STILL_RECONNECTING_AFTER_MILLISECONDS`) | "Toujours en reconnexion. Tes messages partiront à ton retour." with **Réessayer maintenant** / "Retry now", which skips the backoff wait (`MellowClient.reconnectNow`, the gateway's own reconnect-now made public); an attempt already under way goes on. If the button had the focus when the connection comes back, the focus goes to the composer (`fallbackFocus`), not to the page |
| Back online | A lost connection (offline or reconnecting, never the first connection) comes back, and the gateway confirmed its socket (a check keeps the trouble before it, so a network change without a loss passes quietly) | `positive.wash` bar with `text.positive`, `check`, "De retour en ligne" / "Back online" for 2 s (`BACK_ONLINE_SHOWN_MILLISECONDS`, a plain constant: reduced motion maps motion tokens to 0 ms and must not turn the confirmation into a flash) |
| Session ended | The instance refused the session (gateway identify refused, or a request answered `UNAUTHORIZED` while the session was open), not a sign-out from this tab, even when a request sent before it answers late (`MellowClient.hasSessionEndedByItself`) | The sign-in card replaces the shell and its subtitle reads "Ta session sur {instance} a pris fin. Reconnecte-toi pour continuer à discuter." / "Your session on {instance} has ended. Sign in again to keep chatting." (DESIGN.md section 8) in an alert; the card's own "Se connecter" is the action |

The rules are pure functions in `web/src/components/connection-notice.ts` (tested); `ConnectionBanner.svelte` only keeps the two timers. The bar keeps its place at the top of the window and its live region, which is always in the page.

**Notification invitation on phones** (X2): below the drawer breakpoint (`Shell`'s `hasRoomForNavigator`), `NotificationPermissionBar` is one row of 48 px (44 px button plus `space.3xs` above and below): bell, "Notifications désactivées" / "Notifications are off" (two lines at 320 px, still inside the row), a short primary "Activer" / "Turn on" and the close key. The full sentence stays the section's accessible name and "Activer les notifications" the button's, which starts with its visible label. It was about 104 px at 320 and 390 px.

**Sign-in card** (X3, X4):
- `novalidate` on the form: empty fields get Mellow's error under each field (`TextField`'s new optional `error`: `warning-circle` and `text.danger`, the field edged in `text.danger`, `aria-invalid`, joined to `aria-describedby`), focus goes to the first field to fix, which reads its label and error. A filled email the browser reads as malformed gets "Entre une adresse e-mail, comme ana@exemple.fr." Typing in a field clears its error. Rules in `web/src/components/sign-in-form.ts` (tested).
- Melli: the cloud itself is `auth-card.melli-size` (96 px; its box is 256/164 of that) and stands above the card, the card's edge passing between the cloud (row 206 of the drawing) and its shadow (rows 215 to 229), so the shadow falls on the card. It shows `connecting` (looking up at its dots) while the form is sent. No gradient or glow on the ground.

```
Wide (≥ 816)                                         Phone (< 816), 320 px
┌──────────────────────────────────────────────┐    ┌────────────────────────────┐
│  ⌁ Toujours en reconnexion. Tes…  [Réessayer]│    │ 🔔 Notifications  [Activer] ✕│ 48 px
├──────────────────────────────────────────────┤    │    désactivées              │
│ rail │ channels │ # general                  │    ├────────────────────────────┤
```

## Defaults rejected

- **A green success bar** for "Back online": `positive.fill` is Discord's green, the kind of tell WP8 removes. The calm `positive.wash` with `text.positive` (6.16:1 dark, 4.64:1 light) says it without shouting.
- **A toast in a corner** for connection states: the bar already has its place and its live region; a second place would split the story.
- **An apricot primary button on the amber bar**: two loud fills side by side. "Retry now" takes the bar's own ink as its fill (9.59:1 with the amber label) and underlines on hover; its focus ring is drawn inside (the blue ring outside was 1.2:1 on amber in dark; inside it is 8.2:1 dark, 3.0:1 light).
- **A separate "Session expired" banner with "Sign in"** in the shell: the session's data is gone the moment the instance refuses it, so the card says it where the action already is. One element fewer.
- **The browser's validation bubble**: foreign to Mellow's voice and to DESIGN 5.1.
- **A bigger Melli by a bigger box**: the token now sizes the cloud, not its empty margin.

## Checks

- Contrast: `text.danger` on `surface.field` 6.90 (dark) / 5.27 (light), on the card 6.83 / 5.74; `warning.on-fill` on `warning.fill` 9.59; `text.positive` on `positive.wash` 6.16 / 4.64.
- Playwright against a local instance (Vite on its own port), French, at 1440 dark, 1440 light, 390 dark touch, 320 light touch: the offline bar showed 5 to 8 ms after `setOffline(true)`; "De retour en ligne" showed and was gone 2.5 s later; after cutting the gateway, the bar offered "Réessayer maintenant" after 30 s, it was the shell's first Tab stop, and Enter brought "De retour en ligne" in 3.3 to 3.9 s; revoking the session (logout from the page's own `fetch`) showed the card with the session sentence; an empty submit focused the login field with `aria-describedby` reading "Entre ton nom d'utilisateur ou ton e-mail."; Melli read `connecting` while a delayed sign-in was sent, `idle` after the refusal.
- Not checked: a screen reader (the alert on the session sentence and the focus path were read from the DOM only), text at 200 %, Firefox and Safari, a real network drop on a phone (Chrome's offline emulation only).
