# Voice processing on the web: RNNoise and the enhanced voice (ADR-0031)

Status: web client built on `feat/voice-enhancement`, DESIGN.md v0.32, on top of voice comfort ([web-voice-comfort.md](web-voice-comfort.md), stage 16.3). Checked with unit tests, the development gallery and a real Chrome against a release build under the production Content Security Policy; not yet in a real call between two people.

## Brief

After trying `v0.24.0`, the owner (2026-11-01): « la suppression de bruit pourrait être mieux réglée et faire un son plus streamer/propre avec un EQ par exemple ou bien un paramètre léger qui rend les voix bien, activable ou non et un peu configurable ». He chose RNNoise as well. ADR-0031 (accepted) fixes the architecture: processing in the sender's browser, before WebRTC; RNNoise from `@sapphi-red/web-noise-suppressor` 0.4.1; then native Web Audio nodes only; the browser's automatic gain off while the compressor runs; echo cancellation always on.

Real people: small communities in a browser tab, often a USB or headset microphone in an untreated room, a fan or a mechanical keyboard nearby. They want to sound "like a streamer" without knowing what a compressor is, and to hear the difference before others do.

The job of the section: pick how noise is removed, switch on a good-sounding voice in one click, adjust it a little if wanted, and hear it.

## Flows and states

**Settings → Voix → Traitement de la voix**, now just above "Test du micro", so the test answers the settings.

- "Suppression du bruit", three 6.55 choice rows: **Avancée** (RNNoise, the default), **Navigateur** (the browser's `noiseSuppression`), **Désactivée**. With Avancée the browser's own suppression is turned off (both together muffle).
- RNNoise cannot run (no AudioWorklet or WebAssembly, known before any microphone opens; or Firefox refusing a 48 kHz graph for a microphone at another rate, `NotSupportedError`, which would fail every time): "Avancée" stays chosen, the browser's filter is used, and a `warning-circle` line says so (`role="status"`). Loading or starting it failed (network, policy), or it died after starting (its worklet failed, or a watchdog saw it give exact silence for a second while the microphone was heard): the same, with "Choisis-la de nouveau pour réessayer"; choosing another mode and back retries. The microphone is reopened with the browser's suppression, so nobody is left with none.
- RNNoise (about 155 KB) loads ahead of the first join, once the instance says it has voice and when Voice settings open; a join waits at most 3 seconds for it, then uses the browser's filter for that microphone without giving up on RNNoise for the next one.
- Muted or deafened with processing on, the device's track is disabled as well as the processed one, so the browser's microphone indicator goes out; a closed gate or push-to-talk leaves it on to feed the meter.
- "Voix améliorée" switch, off by default. On: the browser's automatic gain is turned off and the chain runs. Below it, "Style" (Naturel, Streamer, Chaleureux, Clair; Streamer first chosen) as one segmented choice, a sentence for the chosen style, then "Graves", "Médiums", "Aigus" from −12 to +12 dB. A style fills the three sliders; moving a slider makes it "Personnalisé" (shown at the end of the Style line, no style chosen, "Tes propres réglages. Choisis un style pour repartir de lui.").
- Every change applies live, during a call too: a slider changes the running filters; a mode or the switch opens the microphone again with the right `getUserMedia` processing and `replaceTrack`s it, without renegotiation. Everything off: no Web Audio at all, the device's track is sent as in 16.2.
- "S'écouter" under the meter, off by default: plays what others would hear on the chosen output; in the test, the processed microphone at the input volume; in a call, the sent track itself, which carries the input volume, push-to-talk and the gate. The description warns to use headphones. Outside a call it starts the test (it is a click); "Arrêter le test", hanging up or leaving the page turns it off.

## Design plan

Tokens: one new group, `component.voice-enhancement.*`, aliasing existing roles; no new colour, contrast pairs already enforced (`text.secondary` on `surface.input`, `text.heading` on `state.selected`).

| Token | Value | Role |
|---|---|---|
| `preset-track-background` / `-padding` / `-radius` | `surface.input`, `space.3xs`, `radius.sm` | the styles' track |
| `preset-height` | `size.hit.web-min` (44 px) | each style is a full target with any pointer |
| `preset-padding-inline`, `preset-radius` | `space.sm`, `radius.xs` | |
| `preset-text` / `-text-hover` | `text.secondary` / `text.heading` | |
| `preset-background-checked` / `preset-text-checked` | `state.selected` / `text.heading`, semibold | chosen style, never colour alone |

Type: existing roles (`type-channel-name` for "Suppression du bruit" and the switch labels, `type-category` for Style and the bands, `type-body` for the styles and helpers). Motion: none added (the styles' `transition-colors` uses `motion.duration.fast`). Sliders: `RangeField` (6.7) with a `<datalist>` tick at 0 dB, so cutting and boosting read either side of flat.

The processing chain (`web/src/lib/voice-processing.ts`), in order: RNNoise (48 kHz AudioContext) → high-pass 90 Hz (Butterworth) → low shelf 120 Hz (Graves) → peaking 350 Hz, Q 0.9 (Médiums: boxiness) → high shelf 5 kHz (Aigus: presence and air) → compressor 3:1 above −24 dBFS, knee 10 dB, 5 ms / 200 ms (Web Audio adds its own make-up gain, about +9 dB) → limiter 20:1 above −2 dBFS (peaks just under −1 dBFS with its make-up). Presets in dB (bass / mids / treble): Naturel 0/0/0, Streamer +3/−3/+4, Chaleureux +5/−1/−1, Clair −2/−2/+5.

```
Wide (content capped at 740 px)                    Phone (390)
Traitement de la voix                              Traitement de la voix
Suppression du bruit                               Suppression du bruit
(o) Avancée     Un petit réseau neuronal…          (o) Avancée
( ) Navigateur  Le filtre de ton navigateur…           Un petit réseau…
( ) Désactivée  Ta voix part telle que…            ( ) Navigateur …
(!) fallback reason, when there is one             ( ) Désactivée …
Voix améliorée                          [✓ ]       Voix améliorée    [✓ ]
Égalise ta voix et lui donne de la présence…       Égalise ta voix…
Style                              Personnalisé    Style      Personnalisé
[Naturel|Streamer|Chaleureux|Clair]                [Naturel|Streamer|Chal…|Clair]
Plus de présence, moins de résonance de la pièce.  (full width, four equal parts)
Graves                                   +3 dB     Graves           +3 dB
[==========o=====|===============]                 [=======o==|=======]
Médiums …  Aigus …                                 …
Test du micro                                      Test du micro
[meter......................]  [Arrêter le test]  [meter..............]
S'écouter                               [ x]       [Arrêter le test]
Entends ta voix… Mets un casque…                   S'écouter …
```

## Defaults rejected, and why

- **A graphic equaliser with many bands, or a drawn frequency curve.** That is a studio tool; the owner asked for "un paramètre léger … un peu configurable". Three bands named in words people use (Graves, Médiums, Aigus) and four styles are enough; the curve would be a second signature on a page whose signature is the meter.
- **A switch for RNNoise beside the browser's switch.** Two switches allow the muffling combination ADR-0031 forbids. One choice of three rows makes the trade-off explicit and impossible to get wrong.
- **Hiding "Avancée" where it cannot run.** The reader would not learn why their choice is missing. It stays, falls back softly, and says so in one line.
- **Style as a select, or as four big choice rows.** A select hides the options; four rows with descriptions would double the section. A segmented choice (native radios, arrow keys) shows all four, and one sentence describes the chosen one.
- **A fifth "Personnalisé" segment.** Clicking it would do nothing. "Personnalisé" is a state, so it is written where the value goes, like a slider's value.
- **Native range fill as the only reference.** Chromium fills from the left, which reads as "more" for a bipolar control; the 0 dB tick restores the centre without drawing a custom slider.
- **Monitoring on by default, or anywhere but the settings.** Voice comfort rejected playback because laptop speakers howl. The owner now wants to hear the result, so it is opt-in, warned, and turned off when the page closes.
- **"L'annulation d'écho reste toujours active."** Drafted under the choices, removed in the self-critique: echo cancellation is never a choice, and the line answered a question nobody asks.

## Implementation

- `web/src/lib/voice-settings.ts`: `noiseSuppression` (`off` / `browser` / `advanced`; the older `isNoiseSuppressionOn: false` reads as `off`, `true` as the new default), `isVoiceEnhancementOn`, `voiceEnhancementPreset` (`natural` / `streamer` / `warm` / `clear` / `custom`), `equalizerDecibels` (a preset's values always come from the preset; a custom curve is clamped to ±12 dB), `withVoiceEnhancementPreset`, `withEqualizerBand`.
- `web/src/lib/voice-processing.ts`: pure. The `getUserMedia` choice and constraints, the processing wanted, the stages in order with their parameters, and whether a change needs the microphone reopened.
- `web/src/lib/voice-browser.ts`: `prepareNoiseSuppressor` (the package is imported lazily, since it extends `AudioWorkletNode` at load; the binary is fetched once and compiled once on the main thread to catch a policy refusal before the microphone opens), `processMicrophone` (one AudioContext per chain, 48 kHz with RNNoise, `addModule` per context, nodes built from the stages, mono destination track; `stop` destroys RNNoise's WebAssembly memory and closes the context).
- `web/src/lib/voice-session.svelte.ts`: the processor sits between the device's track and the existing meter, amplifier and gate (which now act on the processed track; the device's track stays enabled to feed it). A RNNoise start failure reopens the microphone with the browser's suppression; `noiseSuppressorProblem`, `isMonitoring`, `setMonitoring` (reusing `playTrack`), the microphone test and its restart keep listening.
- `web/src/components/VoiceSettings.svelte`, `web/src/i18n/{fr,en}.ts`, `design/tokens.json`.
- Gallery: `web/dev/voice.html#view=settings&enhance=1&preset=custom&rnnoise=unsupported|failed&listen=1`. Processing runs for real on the gallery's oscillator; RNNoise removes a steady tone as noise, so the meter stays low with "Avancée" (use `rnnoise=unsupported` to see it move).

Licences: the package is MIT, `shiguredo/rnnoise-wasm` Apache-2.0, RNNoise BSD-3-Clause; their notices are in `web/public/third-party-notices.txt`, served with the client and linked from Settings, About ("Licences des composants tiers").

## Follow-ups

- Up to three AudioContexts run in series on the microphone today (the chain, the input amplifier, the meter), each hop through a `MediaStreamTrack`. Merging them into one graph would save CPU and a little latency; for `performance-optimizer`, with measurements before and after.

## Checked, and left to check

Checked: unit tests for settings (migration, presets, custom clamp, round trip), the pure chain (topology per setting, band gains, limiter ceiling, constraints, when to reopen) and the session (RNNoise sent with the browser's suppression off, fallback when unsupported, reopen when it fails to start and retry, nothing built or loaded when all is off, enhancement live with gain control off and no renegotiation, sliders live without reopening, gate and mute on the processed track, listening during the test, its volume, following a restart, off when the page closes). `npm run check`, `npm test`, `cargo xtask tokens --check`. The gallery in headless Chrome: dark and light, 1100 px and 390 px, enhanced, custom, fallback reasons, listening.

Under the production policy (release `mellow-server --features web-client`, throwaway instance under the scratchpad, Chrome driven by Playwright): the page's `Content-Security-Policy` is `script-src 'self' 'wasm-unsafe-eval'` with `worker-src 'none'`; Settings, Voix, "Tester le micro" with Avancée loaded `/assets/dist-*.js` and `/assets/rnnoise_simd-*.wasm` (200, served as `application/octet-stream`, which `loadRnnoise` reads as bytes), the worklet node was created in a 48 kHz context, no `securitypolicyviolation`, no `processorerror`, no console warning. White noise at about −25 dBFS RMS came out 33 to 45 dB lower within a second, so RNNoise is running, not passing through.

Not checked: a real voice through a real microphone (by ear: the presets and the compressor's make-up gain need tuning on real voices and rooms); Firefox (the 48 kHz context with a microphone at another rate is expected to fail and fall back), Safari (AudioWorklet with WebAssembly in a worklet); CPU and added latency on a real device (RNNoise works on 10 ms frames, plus one Web Audio hop: expect 10 to 20 ms more); whether Chrome's echo cancellation copes with "S'écouter" on speakers (the warning says to use headphones); screen readers on the segmented style choice; 200 % text.
