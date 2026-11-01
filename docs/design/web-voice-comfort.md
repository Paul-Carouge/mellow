# Voice comfort on the web (ROADMAP stage 16.3)

Status: web client built on `feat/voice-comfort`, DESIGN.md v0.31, on top of stage 16.2 ([web-voice-channels.md](web-voice-channels.md), ADR-0030). The server's `VOICE_SLOT_ASSIGNED` event was added on the same branch for per-person volume. Checked with unit tests and the development gallery; not yet in a real call.

## Brief

The owner tested 16.2 ("ça fonctionne très bien") and wants voice to "work really like Discord". Discord's Voice & Video settings are the reference for the concepts: devices, volumes, input mode, push-to-talk, sensitivity, noise suppression, a microphone test, and per-person volume with "Mute for me" in a participant's menu. The look stays Dusk's (ADR-0022). Real people: small communities on a self-hosted instance, in a browser tab, often with a headset, sometimes on laptop speakers; some prefer push-to-talk because of a noisy room or a mechanical keyboard.

The job of the settings page: choose the right microphone and headset, be heard well (not too quiet, not the keyboard), and check it before joining. The job of the participant menu: turn one loud or quiet friend up or down, or silence them for yourself only.

## Flows and states

**Settings → Voix.** Opened from the user card's gear, under "App settings" before Notifications. Every change applies at once, during a call too, without renegotiating.

- Devices hidden (no microphone permission yet): one sentence and "Autoriser le micro". The browser asks; the microphone is released right after; the selects appear with names. Refused: the existing microphone sentences, `role="alert"`.
- Devices listed: "Par défaut (name)" first, then each device. A remembered device that is unplugged stays selected as "Appareil débranché" with the reason under it; the default is used meanwhile and the remembered one comes back by itself when replugged (`devicechange`).
- No output choice (Safari): a sentence instead of the output select.
- Microphone test outside a call: "Tester le micro" opens the microphone from that click, the meter moves, "Arrêter le test" or leaving the page frees it. During a call, the meter reads the call's microphone and the button goes away.
- Push-to-talk without a key: a warning on the page and in the voice panel. Recording: the next key pressed on the button is the key (Esc cancels, Tab leaves, a blur stops recording).

**In a call.**

- Push-to-talk: the microphone sends only while the key is held, then for the release delay. It works while typing (capture-phase listener on the window, as Discord's), and a key that types a character does not type it. Losing focus or hiding the tab releases the key. Muted stays muted whatever the key does. Optional 60 ms ticks on press and release.
- Manual sensitivity: the client gates the sent track above the threshold (level sampled every 20 ms after the input volume), with a 250 ms hold. Automatic: no client gate.
- Your speaking ring follows what leaves: the gate itself when manual, else being heard (key held, not muted) and loud.
- Device unplugged mid-call: the microphone track ends, the default one takes over with `replaceTrack`, and the panel says so; replugged, the remembered one comes back. The same for the output, through `setSinkId`.
- Per-person volume and "Rendre muet pour moi": from a participant row's menu (click, Enter, right-click, long press, Shift+F10); applied to whichever receive slot carries them, at once.

## Design plan

Dusk tokens, extended, no new colour:

| Token group | Role |
|---|---|
| `component.voice-meter.*` | 8 px pill on `surface.field`; level under the threshold `text.muted`, past it `accent.indicator`; threshold mark 4 × 20 px `text.heading` |
| `component.slider.*` | `accent` (`accent.fill`) for native ranges; `value-text` (`text.secondary`) |
| `component.key-chip.*` | 32 px key cap on `surface.field`, `border.strong`, `text.heading`; `border-recording` `accent.indicator` while waiting for a key |
| `component.settings.voice-column-min-width` | 260 px: microphone and output columns side by side, else stacked |
| `component.voice-participant.local-mute-icon` / `background-hover` | `text.danger` speaker-x; `state.hover` on the now interactive rows |
| `component.voice-panel.hint-text` | `text.secondary` for the push-to-talk line and device notices |

Contrast pairs added and enforced by `cargo xtask tokens`: `text.muted` and `accent.indicator` on `surface.field` (3:1, the meter's parts), `border.strong` on `surface.chat` (3:1, the key cap's edge). Type: existing roles only (`type-category` labels, `type-body` helpers, `type-button` key cap). Motion: none added; the meter redraws every frame without transitions, so reduced motion changes nothing. Icon added: Phosphor 2.1.1 `speaker-x` (Fill).

Layout, wide (content capped at 740 px) and phone (one column, 16 px margins):

```
Wide                                              Phone (390)
Voix                                              Voix
Ces réglages restent dans ce navigateur.          Ces réglages restent…
Micro              | Sortie audio                 Micro
[Yeti…          v] | [Casque…              v]     [Yeti…           v]
Volume du micro 120 %| Volume de sortie 100 %     Volume du micro 120 %
[=====o------]     | [====o--------]              [=====o--------]
Mode d'entrée                                     Sortie audio …
(o) Activité vocale                               Mode d'entrée …
( ) Appuyer pour parler                           …
Sensibilité du micro                              Test du micro
Réglage automatique                    [ x]       [meter.................]
Test du micro                                     [Tester le micro]
Seuil                                 −32 dB      Traitement de la voix …
[▓▓▓▓▓▓▓▓|███░░░░░░░░]   [Tester le micro]
Traitement de la voix
Suppression du bruit                   [✓ ]
```

Participant menu (the existing person menu, with a header):

```
(A) Ana
+-----------------------------+
| Volume de l'utilisateur 150 %|
| [==============o-----]      |
| Rendre muet pour moi    [ ] |
|-----------------------------|
| Envoyer un message      (o) |
| …                           |
```

## Defaults rejected, and why

- **Discord's yellow-to-green sensitivity bar.** Green is Discord's speaking colour. In Dusk, "now" is apricot: the part of the level past the mark lights apricot like your speaking ring, because that part is what others hear. This meter is the page's one expressive element; everything else is plain rows and native controls.
- **Custom-drawn sliders everywhere.** The browser's range input with an accent colour keeps keyboard, touch, screen readers and forced colours for free; only the threshold mark is drawn, because it sits on the meter.
- **A default push-to-talk key.** A letter or Space would stop typing while in voice. No default, and both the page and the panel say it is missing.
- **A separate "devices hidden" state per select.** Without the permission, neither the microphone nor the output has a name; one sentence and one button replace both.
- **Playing your microphone back during the test (Discord's "Let's check").** On laptop speakers it howls; the meter answers the same question silently. Left for later if people ask.
- **Icons on the input mode rows.** Neither a microphone nor headphones says "push to talk"; the descriptions do.
- **Routing every voice through Web Audio.** Chrome's echo cancellation hears what plays through media elements; Web Audio output may escape it, which would bring echo to people on speakers. Up to 100 % each voice stays on its `<audio>` element (16.2's proven path, `volume` for less than 100 %), and a `GainNode` takes over only above 100 %.

## Implementation

- `web/src/lib/voice-settings.ts`: the settings and per-person settings (types, defaults, validated read, write), dB conversion, push-to-talk key recording rules, device choices and presence, `playbackGain`.
- `web/src/lib/voice-session.svelte.ts`: settings applied live (device and noise suppression switch with `replaceTrack`; input volume through an amplifier track; output device and gains on every slot), push-to-talk with release delay, the manual gate, the microphone test, device watching and fallback, `assignSlot` for `VOICE_SLOT_ASSIGNED` (forgotten on each new answer and on leaving).
- `web/src/lib/voice-browser.ts`: `getUserMedia` with the chosen device (`exact`) and noise suppression; per-slot playback (element up to 100 %, shared Web Audio graph above, routed by `setSinkId` where it exists); the meter on a clone of the track (the sent track is the one gated); the input amplifier; device listing and `devicechange`.
- `web/src/lib/protocol.ts`, `client.svelte.ts`: `VOICE_SLOT_ASSIGNED { channel_id, mid, user_id }`, routed to the session.
- Components: `VoiceSettings.svelte`, `VoiceLevelMeter.svelte`, `RangeField.svelte`, `PushToTalkKeyRecorder.svelte`, `push-to-talk-keys.ts` (the window listener), the header slot in `ContextMenu.svelte` and `data-menu-control` in `menu-keyboard.ts`, the voice header in `PersonContextMenu.svelte`, interactive participant rows in `ChannelSidebar.svelte`, the panel's lines in `VoicePanel.svelte`, sounds routed to the chosen output in `chime-sound.ts`, push-to-talk ticks in `voice-sounds.ts`.
- Gallery: `web/dev/voice.html#view=settings&mode=auto|manual|ptt&key=0&devices=hidden&output=unsupported&missing=1&theme=light&wide=0`, and `#view=menu` for Ana's menu. A wavering oscillator stands in for the microphone (headless Chromium never answers `getUserMedia`).

## Checked, and left to check

Checked: unit tests for the settings model (validation, dB, keys, devices, gains) and the session (slot volumes and local mute, reassignment, mappings forgotten on a new answer, output volume and deafen, output fallback and return, push-to-talk with delay and sounds, mute over push-to-talk, the manual gate and its hold, the gate after input volume, the amplifier and `replaceTrack`, live device and noise-suppression switch, remembered microphone unplugged at join and mid-call, microphone test and its refusal, storage), the push-to-talk key handler; `npm run check`, `npm test`, `cargo xtask tokens --check`. The gallery in headless Chrome: settings in manual, automatic and push-to-talk modes, no key, unplugged devices, devices hidden, no output choice, dark and light, 1100 px and 390 px; the participant menu dark and light; the panel with the key and without.

Not checked: a real call with the new server event; real `enumerateDevices`, `setSinkId` and `devicechange` in Firefox, Safari and Chromium; whether Chrome's echo cancellation still holds above 100 % (Web Audio path) on speakers; push-to-talk in a background tab (by design it cannot work) and the release on blur on each browser; the gate's 20 ms sampling in a throttled background tab; iOS, where `HTMLMediaElement.volume` is read-only (volumes under 100 % have no effect there); screen readers on the meter and on a slider inside a menu (ARIA has no slider inside `menu`; Discord does the same); 200 % text.
