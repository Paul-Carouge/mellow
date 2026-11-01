# 0031. Process the microphone in the web client, with RNNoise for noise suppression

- Status: Accepted (by the owner, 2026-11-01)
- Date: 2026-11-01

## Context
After trying voice comfort (`v0.24.0`), the owner asked for better noise suppression and a cleaner, "streamer" sound: an equaliser or a light setting that makes voices sound good, which can be turned on or off and adjusted a little. ROADMAP item 21 already listed RNNoise and a "streamer" voice preset; the owner pulled both forward on 2026-11-01.

The browser's own processing (`noiseSuppression`, `echoCancellation`, `autoGainControl`) is all the client used so far. Chrome's noise suppression is light: it leaves fans, keyboards and room noise audible. RNNoise, a small recurrent neural network from Xiph, removes them well at a low CPU cost, but in a browser it runs only as WebAssembly inside an AudioWorklet.

The web client's Content Security Policy is `script-src 'self'` with `worker-src 'none'` (ADR-0019). WebAssembly compilation needs `'wasm-unsafe-eval'` in `script-src`. An AudioWorklet module loaded from the instance's own origin is governed by `script-src 'self'` and is not a worker, so `worker-src 'none'` stays.

## Decision
- **Processing runs in the sender's browser**, on the microphone track, before WebRTC sends it: everyone hears the processed voice, and the server keeps relaying Opus untouched (ADR-0030). No server work, no extra latency on the relay.
- **The chain**, all optional: RNNoise noise suppression, then a voice enhancement built only from native Web Audio nodes: a high-pass filter (rumble), a three-band equaliser (bass, mids, treble) with presets ("Naturel", "Streamer", "Chaleureux", "Clair"), a compressor (steady loudness) and a limiter (no clipping). The browser's automatic gain control is turned off while the compressor is on, since the two fight; echo cancellation stays on.
- **RNNoise comes from `@sapphi-red/web-noise-suppressor` 0.4.1** (MIT, no dependencies, by a Vite core team member; it ships `shiguredo/rnnoise-wasm` builds, about 150 KB, with and without SIMD), pinned exactly in `web/package-lock.json`. Its WebAssembly binary and worklet module are bundled by Vite and served by the instance like the rest of the client; nothing is fetched from elsewhere. The binary is passed to the worklet, so the Emscripten loader's own `fetch`/`importScripts` paths are not used.
- **The Content Security Policy gains `'wasm-unsafe-eval'`** in `script-src`, and nothing else: JavaScript `eval` and inline scripts stay forbidden, and `worker-src 'none'` stays. A server test guards it.
- **Settings** live with the other voice settings, per browser (`mellow.voice-settings`). RNNoise replaces the browser's noise suppression when on (both together muffle the voice).

## Consequences
- Much cleaner voices without any service or server cost; the processed sound is what others hear, as with a streamer's local filters.
- The policy is looser by one keyword: a script that can already run on the origin could now compile WebAssembly. Running script on the origin is the real barrier, and it is unchanged; still, uploads must keep being served with `nosniff` and their own types, so they can never be loaded as scripts.
- One more front-end dependency to follow (pinned, audited by `npm audit` in CI's web job, licences checked).
- Processing costs some CPU on the sender: RNNoise processes 10 ms frames, about 1–2 % of a laptop core; the Web Audio nodes are negligible. Low-end phones may prefer it off.
- AudioWorklet runs on the audio thread, so it keeps working in a background tab, unlike timers.

## Alternatives considered
- **Browser noise suppression only:** no cost, but the owner judged it not good enough.
- **Server-side processing:** the SFU would have to decode, process and re-encode every voice: far more CPU, more latency, and against ADR-0030's forward-only design.
- **`@jitsi/rnnoise-wasm` or `@shiguredo/rnnoise-wasm` directly:** the same model, but each needs our own AudioWorklet glue and frame buffering; the chosen package already provides a tested worklet node.
- **Bigger models (GTCRN, DeepFilterNet):** better on hard noise, but several times the size and CPU; RNNoise is the proven light choice. The chosen package also ships GTCRN, which can be evaluated later without a new dependency.
