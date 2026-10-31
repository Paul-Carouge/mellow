# Voice channels on the web (ROADMAP stage 16.2)

Status: web client built on `feat/voice-channels-web`, DESIGN.md v0.30, against the contract of ADR-0030 and stage 16.2. The server side is built in parallel on another branch; until it lands, everything here is checked with unit tests and the development gallery (`npm run dev`, then `/dev/voice.html`), never with a real call.

## Brief

The owner: "La voix doit fonctionner vraiment comme un Discord", on by default. Mellow's people are small communities on a self-hosted instance, often the owner's friends, who hang out in a voice channel while they play or work, with the web client open in a tab. The job: join a channel in one click, see at a glance who is there and who speaks, mute or deafen without thinking, and trust the connection (or be told plainly why it failed and what to do).

Real content: "Rust Lyon" with "Salons textuels" (général, annonces) and "Salons vocaux" (Salon, Jeux limited to 5, Réunion du jeudi soir); Ana speaking in Salon, Bob muted there, Chloé deafened in Jeux, Dimitri without the Speak permission.

## Flows and states

**Join.** A voice channel row is a button: click, Enter or Space joins; joining another channel moves. The microphone is asked for then, never on load. If the browser refuses, finds no microphone, finds it busy, or the page is not HTTPS, the connection still opens to listen, the microphone shows muted, and the panel says why and how to fix it ("Ton navigateur bloque le micro. Autorise-le dans les réglages du site, à côté de l'adresse, puis réactive le micro."). Unmuting then asks the browser again; once allowed, sound starts without renegotiating (`replaceTrack`).

**Connected.** The voice panel shows above the user card, wherever the reader goes in the app. Its signal bars follow the round trip read from `RTCPeerConnection.getStats()` every 2 s (selected candidate pair, else a nominated pair that succeeded): good up to 100 ms (three bars, green), fair up to 250 ms (two, amber), poor above (one, red); the milliseconds are the tooltip and are read after the state. The channel line opens its server. Disconnect leaves.

**Toggles.** Mute stops sending at once (`track.enabled = false`) and sends `voice_state`; deafen mutes every incoming voice and implies mute; undeafening restores the previous mute (the existing `voice-toggles` rules). Both work before joining, stored per browser (`mellow.voice-toggles`), with `Ctrl+Shift+M` and `Ctrl+Shift+D` anywhere.

**Others.** Everyone connected to a channel the reader can see is listed under it, in join order, with mute, deafen and "cannot speak" (suppressed, shown muted). Speaking lights an apricot ring. Others joining or leaving your channel play a soft cue and are read out.

**Reconnecting.** "Reconnexion…" while a fresh `voice_join` (same channel, new offer: the server swaps the media session) follows ICE `failed`, ICE `disconnected` for more than 3 s, the browser's `online` event while sound is not flowing, a new gateway session (`ready`), a resumed gateway session without sound, or the instance dropping your own voice state (after a 1 s grace in which `VOICE_DISCONNECTED` may explain it). Up to 5 silent attempts, then "La connexion a été perdue et n'est pas revenue." with Try again. Frames are only sent on an identified gateway session; while the gateway is down, the offer waits for its next `ready` or `resumed`.

**Problems.** In the panel's place, with `role="alert"`: refused (`missing_permissions`, `channel_full`, `instance_full`, `invalid_offer`, `voice_unavailable`, `rate_limited`, each with its sentence; Try again for the three that may pass), no answer in 15 s, answered but no sound in 15 s or ICE failed on the first join ("Son port vocal est peut-être fermé : préviens son administrateur."), connection lost, disconnected by the instance (`permissions_changed`, `channel_deleted`, `removed_from_guild`, `media_timeout` after one silent retry), browser without WebRTC. `moved_elsewhere` (the same user joined from another tab or device) is a quiet notice, "Voix déplacée", never retried.

**Create and settings.** "Create channel" offers Texte, Vocal and Catégorie (Catégorie only from the server menu); a voice channel has a name and a user limit ("Illimitée" or 1 to 25, a native select). Channel settings → Overview shows Name and User limit for a voice channel. Permissions (channel overwrites and roles) gain "Salons vocaux": Se connecter and Parler; a voice channel's editor leaves out the message permissions and a text channel's the voice ones, as on Discord. The channel menu starts with Rejoindre or Quitter, and has no notification entries (a voice channel holds no messages).

## Design plan

Tokens (two new component groups, no new colour; contrast pairs added: `text.positive` and `text.normal` on `surface.sidebar-footer`, 4.5:1, both themes pass):

- `component.voice-panel.*`: the user card's surface, radius and margin; `status-connected-text` (`text.positive`), `status-pending-text` (`text.secondary`), `channel-text` / `-hover`, `signal-good` (`text.positive`), `signal-medium` (`status.idle`), `signal-poor` (`text.danger`), `signal-unknown` (`text.muted`), `signal-icon-size` 16, `hang-up-icon-hover` (`text.danger`), problem and notice colours.
- `component.voice-participant.*`: `row-height` 28, `indent` 26, `avatar-size` 20, `gap`, `name-text` (`text.muted`) / `name-text-speaking` (`text.heading`), `state-icon` and its 14 px size, `speaking-ring` (`accent.indicator`), its 2 px width and 2 px gap, `limit-text`.
- Motion: only the speaking ring fades, over `motion.duration.fast`, which reduced motion sets to 0.
- Icons (Phosphor 2.1.1, Fill): `speaker-high`, `speaker-slash`, `microphone-slash`, `phone-disconnect`, `cell-signal-high`, `-medium`, `-low`.

```
Navigator, wide                          Problem
+--+-----------------------------+       +-----------------------------+
|  | v Salons vocaux           + |       | (!) Impossible de rejoindre (x)
|  | ))) Salon                   |       |     Jeux                    |
|  |     (A) Ana   <- apricot ring|       |     Le salon est plein.     |
|  |     (B) Bob          [mic/] |       |     Réessaie quand…         |
|  |     (P) Paul                |       | [        Réessayer        ] |
|  | ))) Jeux                2/5 |       +-----------------------------+
|  |     (C) Chloé       [spk/]  |
|  | ))) Réunion du jeudi soir   |
|  +-----------------------------+
|  | |||| Voix connectée     [hang up]
|  |  Salon / Rust Lyon          |
|  +-----------------------------+
|  | (P) Paul    [mic][hp][gear] |
+--+-----------------------------+
```
Phones: the same navigator in the drawer; the panel and the card stay at its foot, every control at least 44 px under a coarse pointer.

## Defaults rejected

- **Discord's green speaking ring.** Speaking is "happening now", which Dusk lights in apricot (DESIGN.md §2.1); green stays for presence and success. The name brightens with the ring, so colour is not the only sign.
- **A separate voice screen or a call view.** 16.2 is audio only: the navigator already holds everything, so voice never takes the conversation's place, and the reader keeps chatting while talking.
- **A selected wash on the channel you are in.** It would compete with the open text channel; the voice row reads bright instead, and the participants under it say the rest.
- **Discord's text channels first, then voice.** Managers order channels freely, mixed by position, so a drag never snaps back.
- **An error dialog for voice problems.** A modal would interrupt the conversation; the panel's slot already sits where the reader looks for voice, and it stays until closed.
- **Interactive participant rows.** No profile or per-person volume in 16.2 (comfort features come in 16.3): fewer tab stops until there is something to do.

## Implementation

- `web/src/lib/protocol.ts`: `ChannelKind` `"voice"`, `Channel.user_limit`, `VoiceState`, `Ready.voice_states`, the voice events and frames, `CONNECT` and `SPEAK` (channel-scoped), `VOICE_RECEIVE_SLOTS`.
- `web/src/lib/voice-session.svelte.ts`: one `RTCPeerConnection` per connection: a `sendonly` transceiver (the microphone, or none), then 16 `recvonly`; non-trickle (gathering complete or 1 s); reconnection, timers, toggles, level meter and round trip. Browser objects stay in plain fields; browser access goes through `VoicePorts` (`voice-browser.ts`: `getUserMedia` with echo cancellation, noise suppression, auto gain, mono; an `<audio>` element per voice; an `AnalyserNode` meter, since the policy keeps `worker-src 'none'`).
- `web/src/lib/voice.ts`: latency quality, round trip from stats, microphone errors, who joined or left, waiting for ICE gathering.
- `web/src/lib/client.svelte.ts`: voice states by user (join order), speaking set, announcements and cues for your channel, events routed to the session; `selectGuild` never opens a voice channel; the quick switcher and search leave voice channels out.
- Components: `VoicePanel.svelte` (+ `voice-panel.ts`), the voice rows in `ChannelSidebar.svelte`, `UserCard.svelte`, `CreateChannelDialog.svelte`, `ChannelOverviewSettings.svelte`, `UserLimitField.svelte`, `voice-sounds.ts` (through `chime-sound.ts`'s context), the channel menu in `context-menus.ts`, the permission groups in `server-settings.ts` and `channel-settings.ts`.
- Development gallery: `web/dev/voice.html#state=connected|connecting|reconnecting|no-microphone|refused|moved|timeout|idle&theme=light|dark`.

## Checked, and left to check

Checked: unit tests for the session (offer shape, answer, toggles, receive-only join, refusals, moves, media timeout, ICE grace, gateway down, resume, own state dropped, unsupported browser), the reducers, the panel's words and the permission groups; `npm run check` and `npm test`; the gallery in headless Chrome in French, dark and light, connected, connecting, reconnecting, no microphone, refused, moved, media timeout.

Not checked: a real call (the server is not ready), real `getStats` values in Firefox and Safari, autoplay of incoming voices in Safari, iOS and Android touch, screen readers, 200 % text, the cue volumes on real speakers, and the shortcuts against each browser's own (Chrome may keep `Ctrl+Shift+M` for its profile menu on some systems).
