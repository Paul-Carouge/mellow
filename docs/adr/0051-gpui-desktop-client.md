# 0051. A native desktop client in GPUI, at parity with the web client

- Status: Accepted
- Date: 2026-11-06
- Supersedes: [ADR-0036](0036-web-client-only-and-native-clients-later.md) ("native clients come back later"; its removals stay done), the CI and release rules of [ADR-0023](0023-release-only-the-linux-server-during-the-web-phase.md) as carried by ADR-0036
- Reinstates, amended below: [ADR-0011](0011-shared-client-core-with-uniffi.md) (shared client core, without UniFFI for now), [ADR-0017](0017-desktop-app-updates-itself-from-signed-releases.md) and the desktop parts of [ADR-0018](0018-roll-back-the-executable-only-and-guard-the-install-location.md)
- Amends: [ADR-0044](0044-provider-embeds.md) (players on the desktop), [ADR-0046](0046-videos-compressed-in-the-uploaders-browser.md) (compression on the desktop)

## Context
On 2026-11-06 the owner judged the web client done apart from comfort additions and asked for the desktop app now: written in Rust with GPUI, "ultra optimised and fluid", the same design, every feature of the web client, voice, camera and screen sharing working natively, the same server, and **no Electron, web view or browser engine**. The owner also authorised relicensing the reused parts of Vesper, their own capture and encoding engine, so that Mellow reuses it.

A research pass (2026-11-06) established the facts this ADR relies on:
- **GPUI on crates.io.** Zed's own `gpui` 0.2.2 (2025-10) is stale; Zed's newer platform crates are not published. Longbridge publishes weekly Apache-2.0 snapshots of Zed's GPUI as `gpui-pre` (0.3.11, snapshot of Zed `f816e1f`, 2026-11-02, with its platform crates) and builds `gpui-component` 0.9.0 (Apache-2.0: multi-line IME text input with selection and undo, menus, popovers, dialogs, tooltips, virtual lists, a chat scroller, focus traps, reduced motion) on it. A probe on Rust 1.99.1 passed `cargo deny` with Mellow's rules except one licence (`0BSD`, `enum-iterator`); no git source and no GPL code from Zed (Zed's `ui` and `editor` crates are GPL and not used).
- **GPUI's limits.** AccessKit accessibility (macOS, AT-SPI, Windows) is integrated since 2026 but young. Pictures: PNG, JPEG, WebP, GIF and SVG, but **not AVIF**, the format the server stores (ADR-0033, 10-bit, Display P3). Zero-copy video surfaces exist on macOS only; on Linux and Windows each frame is uploaded as an image, as Zed does for calls. No tray icon, no global hotkeys, no camera; screen capture on X11 and macOS only (Wayland unimplemented). System notifications are built in.
- **The server needs no change for a native client.** Bearer tokens are accepted on every route, login returns the token when no cookie is asked for, native clients send no `Origin` and pass the origin check, files are plain authenticated routes with ranges and ETags.
- **Voice is one WebRTC connection per session** to the server's `str0m` SFU (ADR-0030): the client sends the offer over the gateway (`voice_join`), the server is ICE-lite on one UDP port, media is DTLS-SRTP with BUNDLE and rtcp-mux, Opus (PT 111, FEC, DTX) with the RFC 6464 audio-level extension, VP8 with RTX and three simulcast layers `q`, `l`, `h` (ADR-0035, ADR-0037), and a fixed order of m-lines checked by `domain/voice_rules.rs`. The server's tests already drive it with a `str0m` client.
- **`mellow-client-core`** has a sound architecture (typed REST client, gateway with resume and jittered backoff, pure state reducer, outbox) but covers the v0.8 feature set: 39 of the server's 103 routes, 17 of 44 gateway events, and no outbound gateway frames (voice cannot be signalled).
- **The web client** is 57 000 lines in 14 feature modules; its reference model is `web/src/lib/client.svelte.ts`. It keeps no service worker, Web Push or IndexedDB state: eleven `localStorage` keys and per-user notification settings.

## Decision

### Toolkit and crates
- A new crate **`crates/mellow-desktop`** (binary `mellow`) built on **`gpui-component` = 0.9.0 and `gpui-pre` = 0.3.11** (`package = "gpui-pre"`, lib name `gpui`), with `gpui-pre-platform` features `wayland`, `x11`, `font-kit`. Versions are pinned with `=` and upgraded together, deliberately, at most weekly; crates.io only, so `deny.toml` keeps `unknown-git = "deny"`. `0BSD` joins the licence allow-list. Fallback if the snapshot stops: the same code at a pinned Zed revision with an `allow-git` entry (a new ADR).
- gpui-component gives behaviour (text input, overlays, lists, focus); **every visible component is restyled to Dusk** from the generated tokens, and Mellow writes its own primitives where gpui-component's shadcn look fights DESIGN.md. The desktop looks like the web client: same layout (the Navigator, ADR-0022), colours, type, icons, motion, French and English text.
- A new crate **`crates/mellow-media`**: the voice, camera and screen engine, without UI. It owns the client's `str0m` peer, audio I/O and processing, codecs, capture and the frames handed to the UI. Testable headless against an in-process server, as `mellow-client-core` is.
- **`mellow-client-core`** grows to the whole protocol: every REST route, every gateway event, outbound frames (voice), and the web client's model (unread and mention counts, read states, relationships, presences, reactions, polls, checklists, progress bars, link previews, custom emoji, soundboard, voice states), ported with the web client's tests as Rust tests. Pure client logic shared by both clients' behaviour (Markdown parser, search query parser, notification rules, formatting) lives there too, without UI types. Its public API stays plain Rust (owned types, flat error enums); UniFFI waits for a mobile client.
- **Threads**: a Tokio runtime on two background workers runs `mellow-client-core` and network I/O; the UI holds the client state in GPUI entities on the main thread, fed by a bounded channel and batched to one repaint per burst; every answer is checked against a session generation so a late one is dropped (as the Slint app did, and as the web's stale-answer guards do). Media runs on its own real-time threads (audio callbacks never allocate, lock or block); decoding, colour conversion and picture decoding run off the main thread.

### Design
- `cargo xtask tokens` writes a third output, `crates/mellow-desktop/src/theme/tokens.rs`, plain data with no GPUI types (colours per theme as `u32`, lengths in px, durations distinct from delays, breakpoints), checked by `--check` in CI like the web stylesheet. `design/tokens.json` stays the single source.
- Figtree and Source Code Pro from `design/fonts` and the Phosphor icons from `design/icons` are embedded in the binary. The system emoji font is used, as on the web. Melli is drawn from its separate layers.
- The interface strings are generated from the web client's catalogues (`web/src/i18n/en.ts`, `fr.ts`) by an `xtask` step, typed against English so a missing French key fails to compile; the two clients cannot drift. Language follows the OS locale; theme and reduced motion follow the OS, as the web follows the browser.
- Accessibility targets the web's level (DESIGN.md §1.4, §7.3) through AccessKit roles, names, focus order and visible focus; each screen is checked with VoiceOver and Orca. Where GPUI cannot reach it yet, the gap is recorded in DESIGN.md, not hidden.

### Pictures and files
- The desktop decodes AVIF itself: `avif-parse` and `rav1d` (pure Rust), then `moxcms` converts Display P3 10-bit to the display's sRGB 8-bit, off the main thread, into GPUI `RenderImage`s. No server variant is added.
- Downloads go through one authenticated client with the server's limits in mind (120 file requests per user per minute); profile images and emoji are cached on disk in the user cache directory; attachments are cached in memory only and nothing of a conversation stays on disk after sign-out. Animated pictures are decoded with a frame and memory cap, still frames (`/still`) when animation is off.

### Voice, camera and screen
- **WebRTC**: `str0m` on the client too (sans-IO, pure Rust, the server's stack), with Mellow's own UDP loop, jitter buffer with loss concealment, clock-drift steering and lip sync. The offer follows the server's fixed m-line order byte for byte. libwebrtc bindings are rejected: a 110 to 265 MB prebuilt download per target at build time, C++, opaque to `cargo deny`.
- **Audio**: `cpal` (CoreAudio, PipeWire/ALSA, WASAPI), `opus` (already built from source by cmake), `rubato` (already in the workspace), echo cancellation, noise suppression and gain control by `sonora` (pure Rust port of WebRTC's audio processing; the C++ `webrtc-audio-processing` is the fallback if measurements require it), RNNoise by `nnnoiseless` for the "advanced" suppression the web offers. Voice activity, push-to-talk, per-person volume, mute, deafen, soundboard, join sounds and voice cues behave as on the web. Push-to-talk works while the app is unfocused: `global-hotkey` on macOS, X11 and Windows, the GlobalShortcuts portal on Wayland.
- **Camera**: VP8 with three simulcast layers, as the server and browsers expect (ADR-0035), encoded by libvpx built from source and linked statically. Capture per OS behind one trait (AVFoundation, PipeWire/V4L2, Media Foundation).
- **Screen sharing reuses Vesper.** Its capture and encoding pipeline (Linux: PipeWire, DMA-BUF, hardware encoders) is relicensed by its author under AGPL-3.0-or-later for Mellow and brought in as a dependency; macOS uses ScreenCaptureKit with VideoToolbox behind the same interface. What Vesper sends must stay decodable by browsers watching from the web client; the codec, Vesper's integration and whether the SFU forwards a hardware codec (H.264) next to VP8 are decided in their own ADR once Vesper's code is studied. Until then a share uses the camera's VP8 path, so it works on every platform from the first release of the feature. The share's sound is captured natively (ScreenCaptureKit audio, PipeWire monitor or application streams), a gain over the browser.
- **Receiving video**: VP8 decoded off the main thread; frames shown zero-copy on macOS (`surface` with a `CVPixelBuffer`), uploaded as images on Linux and Windows, converted and scaled to the tile's displayed size first; the cost of a 1080p share is measured, and a GPUI texture path is added if it is too high.

### Things a native app does differently
- **Providers' players** (YouTube, YouTube Music, ADR-0044): no web view, so the cards open the player in the system browser.
- **Video compression before upload** (ADR-0046): the first desktop releases send the original, which ADR-0046 already allows; native hardware encoders (VideoToolbox, then VA-API and Media Foundation) follow with the same output contract.
- **Video attachments and MP4 GIFs** play through the same decoding stack as calls once it exists; until then they show their poster and open in the system's player.
- **Klipy** (ADR-0042) is called by the app itself, as ADR-0042 allows for "the user's app", with the same answer checks and the same attribution.
- **Session**: the bearer token lives in the OS keychain (`keyring`); settings live in one validated file in the OS config directory, written atomically. The app asks for an instance address, checks `/api/v1/health` and refuses instances older than the oldest it supports, with a sentence saying so.
- **Notifications** use the OS (GPUI's system notifications), with a dock or taskbar badge and a tray icon (`tray-icon` on macOS and Windows, `ksni` on Linux, no GTK); the notification rules are the web's, ported.
- DESIGN.md §7.2's shortcuts the web lacks (Alt+Up/Down, F6, Ctrl+/, Ctrl+, , zoom) are added on the desktop.

### Platforms, CI, releases
- **macOS (Apple silicon and Intel) and Linux (X11 and Wayland)** are first-class. **Windows** follows once parity holds on the other two (libvpx's build and code signing are the work there).
- CI builds and tests `mellow-desktop` and `mellow-media` on Linux, and builds them on macOS (Metal shaders through `runtime_shaders` where Xcode's Metal toolchain is missing). The other jobs stay as they are.
- Releases add a macOS `.app` bundle (zip) and a Linux tarball (executable, `.desktop` file, icons, install script into `~/.local`), signed with minisign like the server. The app updates itself from signed releases through `mellow-update`, which gets back a desktop program and installs a whole bundle on macOS (ADR-0017, ADR-0018 reinstated for the desktop). Apple Developer ID signing and notarisation are the owner's decision; without them macOS asks again for microphone, camera and screen permissions after each update.

### Order of work
Foundations (client-core, media engine skeleton, design tokens, primitives, sign-in) → shell and navigation → messages and composer → conversations, friends, notifications, context menus → rich messages and composer → search and settings → voice → video receive, camera, screen sharing → media playback, compression, packaging. Each step is merged and usable, tracked in ROADMAP.md M3.

## Consequences
- Every protocol change touches the server, `mellow-client-core`, the web client and the desktop; features reach both clients again ([ADR-0036](0036-web-client-only-and-native-clients-later.md)'s reason for stopping). Logic moved into `mellow-client-core` is written once.
- The workspace gains a large dependency graph (GPUI: about 760 crates) and C builds (libvpx, already libopus); CI gets a macOS job. Build times grow; the server binary is unaffected.
- A single maintainer publishes `gpui-pre`, and gpui-component breaks between minors: pinning and deliberate upgrades are mandatory.
- Media quality now depends on Mellow's own jitter buffer, echo cancellation and congestion handling instead of the browser's; each is measured against the web client in real calls before it ships.
- Vesper's reused crates become AGPL-3.0-or-later; the rest of Vesper keeps its licence unless its author decides otherwise.

## Alternatives considered
- **Electron, Tauri or any web view**: refused by the owner.
- **Raw GPUI without gpui-component**: a text editor with IME, overlays and focus management from scratch, for no gain.
- **Zed's GPUI from git**: always current, but a git source in `deny.toml` and the whole Zed repository at build time; kept as the fallback.
- **libwebrtc (LiveKit's bindings)**: behaves like a browser out of the box, but its prebuilt C++ archive cannot be audited and dwarfs the rest of the build.
- **Slint again**: dropped by the owner in ADR-0036.
- **A server-side WebP or sRGB picture variant for native clients**: a protocol change and more storage, while decoding AVIF in the client is pure Rust.
