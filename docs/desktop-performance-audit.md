# Desktop client performance audit (phase 1: measurements)

Scope: `crates/mellow-desktop`, `crates/mellow-media`, `crates/mellow-client-core`, measured on a
release build of the working tree of 2026-11-13 (v0.49.0 plus the uncommitted `MELLOW_PROFILE`
work). Nothing in the repository was changed for this audit; every number below comes from a run
described in "Method". Phase 2 (the fixes) is the plan at the end.

## Summary

The desktop client is already cheap in steady state: no continuous redraw, 0 frames in 30 s at
rest, frames of 1 to 2 ms while scrolling 1 445 messages, about 36 to 46 MiB of anonymous memory
that does not grow over 5 minutes. The real costs are elsewhere:

1. **Start-up waits on the system font scan**: GPUI's Linux text system loads every installed font
   on the main thread before the window exists. 8 to 20 ms when the font files are in the page
   cache, **380 to 610 ms when they are not** (first launch after login, memory pressure). With warm
   fonts the history is on screen 124 ms after `main`.
2. **Background mode does not work on Linux and Windows**: closing the window quits the app (GPUI's
   default quit mode), although Mellow logs "keeps running in the background".
3. **Two full frames per incoming message** in the open channel instead of one, and a full redraw
   per second while someone types.
4. **Release profile**: no `[profile.release]`; fat LTO with one codegen unit makes the shipped
   (stripped) binary 18 % smaller (86.2 to 70.7 MB).
5. **Scaling risks not visible with test data**: no view is cached, so every notification re-renders
   the whole window; the member sheet sorts and builds every member row on every frame.

## Environment and method

- Machine: Ryzen 7 9800X3D, Radeon RX 9070 XT (RADV, Vulkan), 30 GiB RAM, CachyOS, 1 102 installed
  font files (1.0 GB). The machine was shared with other agents' builds during the whole session
  (load average 2 to 27, swap full at times): **wall-clock times are marked "(loaded)"** and only
  their best or median values are given; counters (frames, wake-ups, invalidations) do not depend on
  load and are the numbers to trust.
- Server: the local test instance on `127.0.0.1:8080` (debug build). A `perf` text channel in the
  Atelier guild was filled with 1 445 varied messages (Markdown, code blocks, quotes, lists, links,
  emoji; three authors). The hourly message limit stopped the seed short of 2 000; the client keeps
  at most 500 messages per channel (`MAXIMUM_CACHED_MESSAGES_PER_CHANNEL`), so scrolling to the start
  goes through about 29 history pages either way.
- App: `cargo build --release -p mellow-desktop` in a separate target directory, run with
  `MELLOW_PROFILE=perf` in headless gamescope (1440 x 900), on X11 (Xwayland) and on gamescope's
  nested Wayland (`--expose-wayland`). Input driven with `xdotool` (X11 only).
- Two scratch-only changes, never in the repository: `SHOULD_STORE_LOOPBACK_SESSIONS = true`
  (`session_store.rs:20` keeps loopback sessions only in debug builds, so a release build could not
  start "already signed in" against `127.0.0.1`), and an instrumented copy.
- Instrumented copy: GPUI's `profiler` feature plus a probe that drains
  `cx.foreground_journal().collector()` every 200 ms (with `gpui::profiler::set_trace_enabled(true)`)
  and writes every drawn frame (duration, layout, prepaint and paint phases, invalidations), every
  foreground task poll of 1 ms or more with its spawn location, every input dispatch, and start-up
  milestones. This is the cheapest way to get per-frame numbers out of GPUI 0.3.11 and could become a
  `--features perf-probe` of the app later.
- CPU, wake-ups and memory: `/proc/<pid>/task/*/stat` and `status` (utime + stime, voluntary and
  involuntary context switches per thread, `VmRSS`, `RssAnon`, `RssFile`, `VmHWM`). `perf` is not
  installed and ptrace is not allowed (Yama), so no sampling profiler was available.

## Measurements

### Binary and build

| Build | Size | Notes |
|---|---|---|
| release (current, no `[profile.release]`) | 119.3 MB, 86.2 MB stripped, 32.4 MB gzip | `.text` 61.8 MB, `.strtab` + `.symtab` 33 MB, `.eh_frame` + `.gcc_except_table` 11 MB |
| release + `lto = "fat"`, `codegen-units = 1` | 84.9 MB, **70.7 MB stripped (-18 %)**, 30.0 MB gzip (-7 %) | |
| cold build time | 4 min 23 s current, 9 min 42 s fat LTO (loaded, `nice`) | about 2.2 times longer |
| dynamic loader | 7.6 M cycles (about 2 ms), 34 778 relocations, 144 shared libraries | negligible |

The release workflow already strips the Linux binary (`.github/workflows/release.yml:176`), so the
shipped size is the "stripped" column. Largest symbol groups: `core`/`alloc` generics (11 MB),
`zvariant` + `zbus` (7.3 MB, D-Bus for portals, the keyring and the tray), `gpui` (4.5 MB),
`mellow_desktop` (3.9 MB), `rav1d`, `naga`, `tokio` (about 1.2 MB each). No duplicated heavy crate
versions.

### Start-up with a stored session (instrumented build, X11)

Milestones in milliseconds from the first line of `main`:

| Phase | Fonts in page cache | Fonts not in page cache (loaded) |
|---|---|---|
| `cosmic_text::FontSystem::new` (inside `gpui_platform::application()`) | 8.7 to 20 | **380 to 610** |
| rest of platform creation (X11 connection, executors) | 12 to 25 | 7 to 10 |
| `start` up to `open_window` | 3 to 5 | 3 to 5 |
| window opened (wgpu instance, 3 adapters enumerated, swapchain) | 42 to 93 (X11), 134 (Wayland) | same |
| stored session read from the secret store | under 10 after the window | same |
| first real frame | 19 to 25 (prepaint 18 to 23: shaping, glyph rasterisation, atlas) | same |
| gateway `Ready` (local server) | +25 to 30 | same |
| first history page drawn (frame of 11.6 to 13.5 ms) | **124** | 507 to 759 |

Ten consecutive launches under load: exec to history drawn 466 to 2 262 ms, median about 630 ms
(loaded). The exec-to-`main` gap was 18 ms when the machine was idle. A second `FontSystem::new` in
the same process takes 8 ms: the cost is reading the font files, not parsing them.

### Memory (base build, X11)

| Moment | RSS | RssAnon | Peak (`VmHWM`) |
|---|---|---|---|
| signed in, `#perf` open | 74 to 90 MiB | 36 to 41 MiB | 123 to 156 MiB |
| after visiting 4 channels, Home and a direct message | 88 MiB | 36 MiB | 152 MiB |
| after 5 more minutes at rest | 89 MiB | 36 MiB | 152 MiB |
| after scrolling up through all 1 445 messages | 84 MiB | 46 MiB | 149 MiB |

`RssFile` moves between 6 and 117 MiB from run to run: it is the memory-mapped font files and GPU
driver, shared and reclaimable. The anonymous part is the app's own cost and did not grow over time.
The picture caches allow up to 192 MiB of decoded pictures plus 64 MiB of files in memory
(`pictures/mod.rs:39`, `pictures/loader.rs:33`); the test data had no uploaded pictures, so they
were not exercised.

### CPU and wake-ups at rest

| Situation | CPU | Wake-ups per second | Frames |
|---|---|---|---|
| X11, window open, nothing happening (5 min) | 0.05 % | 65 (60 on the main thread) | 0 in 30 s |
| nested Wayland, window open, nothing happening | 0.00 % | 5.5 (0.2 on the main thread) | 0 |
| sign-in screen (caret blinking) | 0.3 % | 94 | 2 per second |
| window closed | not measurable: the app quits (see causes) | | |

The 60 main-thread wake-ups on X11 are GPUI's X11 refresh timer, which runs at the display rate while
a window is visible and stops when it is hidden (`gpui-pre-linux` `x11/client.rs:1969`); Wayland
parks its frame loop. The remaining wake-ups come from two `blocking-*` threads (2 per second each,
smol's thread pool behind zbus) and one PipeWire thread (1 per second, started about 0.5 s after
launch, most likely cpal's PipeWire host); none uses measurable CPU.

### Live traffic (counters)

| Event | Frames drawn |
|---|---|
| message arriving in the open channel | **2** (the second one frame later, same cost) |
| message arriving in another channel | 1 |
| someone starts typing in the open channel | 9 over the notice's 8 s lifetime (one per second) |
| 1.7 to 3 messages per second in the open channel | 0.7 % CPU |

### Scrolling 1 445 messages (wheel, about 60 notches per second)

| Run | Frames | Draw time p50 / p95 / p99 / max | Frames over 8.3 ms |
|---|---|---|---|
| 400 notches up, 400 down | 48 per second | 1.4 / 1.9 / 2.1 / 2.7 ms | 0 |
| 2 000 notches up to the channel's start (29 history pages) | 1 949 | 1.39 / 2.08 / 3.23 / 7.89 ms | 0 |

Mean phases: layout 0.27 ms, prepaint 0.71 ms, paint 0.37 ms. No foreground task took 1 ms or more
while history pages arrived. Process CPU while scrolling: 8 to 15 % (loaded; base median 14.3 % up
and 13.0 % down over 3 runs, fat LTO 9.4 % and 10.6 %, the spread between runs, 8 to 15 %, is larger
than the difference, so LTO's runtime effect is not established).

### Typing in the composer (180 characters at 25 per second, never sent)

One frame per keystroke; draw time p50 1.13 ms, p95 1.67 ms, max 2.2 ms; input dispatch p95 0.03 ms;
process CPU 4.3 to 7.0 % (about 2 ms of CPU per keystroke, loaded). Each keystroke also wakes GPUI's
timer thread and its 8 background workers (about 136 and 18 wake-ups per second), for no measurable
CPU.

### Web client, for reference

The web client in Chrome on the same instance: 10 MB of JavaScript heap, 551 DOM nodes. Chrome's
multi-process memory is not comparable with one native process, so no memory comparison is made.

## Causes found in the code

| # | Cause | Where | Effect measured |
|---|---|---|---|
| 1 | GPUI's Linux platform builds its text system with `FontSystem::new()`, which reads every system font on the main thread before the first window; Mellow embeds the two families it draws and needs system fonts only as fallbacks (emoji, other scripts). `CosmicTextSystem::new_without_system_fonts` already exists. | `gpui-pre-linux-0.3.11/src/linux/platform.rs:130`, `gpui-pre-wgpu-0.3.11/src/cosmic_text_system.rs:131` and `:147` | 380 to 610 ms of start-up with cold fonts, 8 to 20 ms warm |
| 2 | `QuitMode` is never set, so GPUI quits when the last window closes on Linux and Windows (`QuitMode::Default`), after `background.rs` has decided to keep running. | `gpui-pre-0.3.11/src/app.rs:3301`; `crates/mellow-desktop/src/background.rs:77`; `lib.rs` `start` | background mode broken; "window closed" CPU cannot exist |
| 3 | `note_reader_scrolled` notifies unconditionally; the list's tail-follow scroll after each new message calls it, which costs a second full frame. `mark_channel_read` then emits `ReadStates`, which rebuilds every row of the open channel. | `conversation/mod.rs:624` to `:647`, `:483`; `conversation/rows.rs:250`; `session/messages.rs:372` | 2 frames per incoming message instead of 1 |
| 4 | The conversation observes every session notification and redraws (`cx.observe(&session, …)` then `cx.notify()`), and the session notifies after every gateway burst even when nothing changed for the open view. | `conversation/mod.rs:233`; `session/mod.rs:410` | one full frame per gateway burst of any kind |
| 5 | The typing line is refreshed by a 1 s loop that redraws the whole window while anyone types. | `conversation/mod.rs:508` | 9 frames per typing notice |
| 6 | No view is cached (`Entity::cached` is never used), so any `cx.notify()` re-renders the navigator, the rail, the conversation and the sheets. Cheap today (1 to 2 ms frames), proportional to the window's content tomorrow. | whole crate (`rg "\.cached\(" crates/mellow-desktop` finds nothing) | scales with server size |
| 7 | The member sheet sorts all members, clones each user and builds every row as an element on every frame, without a virtual list. | `shell/member_sheet.rs:159` | not measurable with 4 members; O(n log n) plus n elements per frame |
| 8 | No `[profile.release]`: 16 codegen units, no LTO. `panic = "abort"` must stay off: decoders isolate panics with `catch_unwind` (`vesper-capture/src/camera.rs:376`, `mellow-media/src/video_file.rs:359`, `sound_source.rs:60`, `clip_player.rs:558`). | `Cargo.toml` | 18 % larger shipped binary |
| 9 | wgpu enumerates every backend, GL included, and tests an adapter before the first window. | `gpui-pre-wgpu-0.3.11/src/wgpu_context.rs:525` | 42 to 134 ms of window opening |

Checked and found fine: periodic loops stop when idle (poll clock `conversation/cards.rs:134`,
typing line, draft writes debounced to 1 s `composer/mod.rs:515`); picture decoding and disk I/O run
off the main thread with byte-bounded LRU caches; history pages apply in under 1 ms; the message
body cache is cleared on each channel switch; the client keeps at most 500 messages per channel.

## Plan (phase 2), by measured or estimated gain

Each lot is independent: separate files, separate PRs, each measured before and after with the
same scenario.

### Lot 1: start-up without the system font scan (gain: 370 to 600 ms on cold launches)
- Done (ADR-0059): `gpui-pre-wgpu` vendored instead (`gpui-pre-linux` unchanged). Launch to first
  render, median of 5, release: cold 387 to 45 ms, warm 52 to 46 ms (681 to 127 ms cold under load).
- Patch `gpui-pre-linux` (vendored like `str0m`, or upstream first) so the Linux text system starts
  with `new_without_system_fonts` and loads the system font database on a background thread,
  swapping it in when ready; fallback shaping before that uses the embedded fonts only.
- Files: `vendor/gpui-pre-linux` (the call site) and likely `vendor/gpui-pre-wgpu` (`CosmicTextSystem` needs a way to add the system fonts later), `Cargo.toml` `[patch.crates-io]`, an ADR.
- Measure: launches with the font files evicted from the page cache
  (`posix_fadvise(POSIX_FADV_DONTNEED)` on every `fc-list` file, no root needed) and warm.
- Needs the owner's decision (a second vendored dependency).

### Lot 2: background mode on Linux and Windows (correctness; then idle CPU when hidden)
- `cx.set_quit_mode(QuitMode::Explicit)` in `start` (`crates/mellow-desktop/src/lib.rs`); the
  window-closed handler in `background.rs` already calls `cx.quit()` when it should.
- Test first: a GPUI test that closes the last window with a signed-in session and the tray
  available and asserts the app did not quit.
- Measure: CPU and wake-ups 60 s after closing the window (expected near 0 on both X11, whose
  refresh timer stops for hidden windows, and Wayland).

### Lot 3: one frame per event (gain: -50 % frames per incoming message, -80 % per typing notice)
- Done. Frames counted with the probe of "Method" (debug build, X11, `#perf` open, 5 messages or
  3 notices each, identical on every run): a message in the open channel 2 to 1 per message
  (-50 %), in another channel 1 to 1, a typing notice in the open channel 9 to 2 (-78 %), in
  another channel 1 to 0, opening a channel with 30 unread messages 5 to 3. The second frame per
  message was not `note_reader_scrolled` (GPUI's list calls its scroll handler only on wheel input,
  and notifies the view itself right after): it was the read mark, deferred after the draw. The
  conversation now marks the channel read from the `StateChanged` handler, before the frame; rows
  ignore `ReadStates` (their divider is the visit's marker); the session, the shell and the
  conversation do not notify for typing, which the conversation redraws itself when the line's
  names change, with one timer to the next notice's end (`ClientState::next_typing_notice_end`).
  `note_reader_scrolled` and the conversation's `observe` were left as they are: their notifies
  never cost a frame of their own. Tests count frames through a counting window root.
- `note_reader_scrolled`: notify only when something it owns changed (the at-latest flag, the unread
  marker, `latest_id_when_leaving_present`) (`conversation/mod.rs:624`).
- `ReadStates`: rebuild rows only when the open channel's read position moved
  (`conversation/mod.rs:483`), not on every channel's read state.
- Session: skip `cx.notify()` after a gateway burst that produced no change and no status change
  (`session/mod.rs:410`); the conversation's `observe` keeps only the history-status check.
- Typing line: one timer to the next notice's expiry instead of a 1 s loop
  (`conversation/mod.rs:508`).
- Tests count frames or renders (no clock thresholds): a GPUI test that applies one
  `MESSAGE_CREATE` to the open channel and asserts one render of the conversation.

### Lot 4: release profile (gain: -15.5 MB shipped binary, -18 %)
- Done: the desktop jobs of `release.yml` build with the server's `[profile.dist]` (`target/dist/`);
  `release` is unchanged and `panic` stays "unwind". Measured on 2026-11-14 (`CARGO_BUILD_JOBS=2`,
  `nice`): stripped 87.2 to 71.7 MB (-17.8 %), gzip 32.6 to 30.3 MB (-7.0 %), cold build 8 min 32 s
  to 9 min 59 s with two jobs (more on CI's runners, where `release` builds 16 units in parallel).
- Add a `[profile.dist]` inheriting `release` with `lto = "fat"`, `codegen-units = 1`,
  `strip = true`, used by `release.yml` (`cargo build --profile dist`), so everyday release builds
  stay at about 4 minutes. Keep `panic = "unwind"`.
- Files: `Cargo.toml`, `.github/workflows/release.yml` (artefact path `target/dist/`).
- Measure: binary size, CI build time, and the scroll scenario on an idle machine (the runtime gain
  could not be separated from noise here).

### Lot 5: scale with big servers (estimated, to measure with seeded data first)
- Seed a guild with 2 000 members and 200 channels (needs a seeding path that bypasses the
  registration rate limit on the test instance, for example an admin command), then measure frame
  time with the member sheet open.
- Cache the stable regions: `Entity::cached` for the server rail, the navigator and the member sheet,
  so a message or a keystroke re-renders only the conversation or the composer.
- Member sheet: keep the sorted groups between frames (recomputed on `StateChange::Members` or
  presence of that guild) and draw rows with `uniform_list`.
- Files: `shell/member_sheet.rs`, `shell/mod.rs`, `shell/channel_sidebar.rs`,
  `shell/server_rail.rs`.

### Lot 6: smaller items (gain under 50 ms or under 1 % CPU)
- First frame prepaint of 18 to 23 ms (glyph rasterisation of the first screen): acceptable; look
  again only if Lot 1 makes it the largest start-up cost.
- wgpu backend enumeration (cause 9): upstream request to try Vulkan before enumerating GL.
- Typing wakes all 8 background workers per keystroke: upstream GPUI scheduler, no CPU cost today.
- Picture cache budgets (256 MiB together) could shrink while the window is hidden; measure with
  real pictures first.

## Reproducing

The scripts (seed, launcher, `/proc` sampler, probe analysis) lived in the session scratchpad and
are not kept; the method above is enough to rebuild them: seed through the REST API under the rate
limits (30 messages per channel per 10 s, 600 per user per hour), run the app in
`gamescope --backend headless` with `MELLOW_PROFILE` set and `WAYLAND_DISPLAY` unset (X11) or
`--expose-wayland` (Wayland, after checking that `WAYLAND_DISPLAY` names gamescope's socket), drive
it with `xdotool`, and read `/proc/<pid>/task/*`. Never sample with `pgrep -f` on a pattern that also
matches the sampling command itself.
