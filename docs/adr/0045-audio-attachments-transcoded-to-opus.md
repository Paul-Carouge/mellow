# 0045. Audio attachments transcoded to Opus

- Status: Proposed. Not wired in: libopus needs cmake to build, and the owner's development machine has none (see "Obstacle").
- Date: 2026-11-05
- Supersedes the "No transcoding" consequence of [ADR-0038](0038-play-audio-and-video-attachments-inline.md) for audio only. Video stays as uploaded, and the rest of ADR-0038 (sniffed types, inline serving, byte ranges, throughput floor) is unchanged.

## Context
ADR-0038 plays audio attachments inline and transcodes nothing. It rejected transcoding on the server because it would need FFmpeg or a codec stack in the binary and heavy CPU on small hosts. For audio, that cost is small, and the files are large: a 2-minute WAV recording is 21 MB, close to the 25 MB default limit, and lossless or high-bitrate uploads fill each member's storage quota (ADR-0020). ADR-0033 already stores pictures in their smallest form that looks the same, without keeping the original. The owner asked for the same with media (audit finding media-4, 2026-11-05).

Opus at 96 kbit/s stereo is close to transparent for most listeners and most music, and 64 kbit/s is plenty for mono speech. Every browser the web client supports decodes Opus. The questions were which encoder, which container, and what the build costs.

**Encoder.** The pure-Rust Opus encoders (`rusty-opus` 1.0.2 and `opus-pure` 0.2.2, created in July and August 2026, about 70 000 downloads each) are too young for files we keep forever without an original. `unsafe-libopus` (libopus 1.3 transpiled by c2rust) has needed no C compiler since 2024, but it is unmaintained and would bring unsafe calls into our crate. Every maintained binding to libopus (`opusic-sys`, `libopus_sys`, `opus-codec`) builds the C sources with **cmake**. So we use the `opus` crate 0.4.0 (MIT or Apache-2.0) over `opusic-sys` 0.7.7 (BSD-3-Clause), which bundles libopus 1.6.1 (BSD-3-Clause), builds it with cmake and links it statically. It detects CPU features (SSE, AVX2, NEON) at run time.

**Container and Safari.** Ogg Opus (RFC 7845) plays in Chrome, Firefox and Edge. WebKit added "Ogg container support for both Opus and Vorbis audio" in Safari 18.4 (macOS 15.4, iOS and iPadOS 18.4, March 2025; [WebKit features in Safari 18.4](https://webkit.org/blog/16574/webkit-features-in-safari-18-4/)). Can I Use lists iOS Safari 18.4 and later as supported, and earlier versions as partial (Opus in CAF only). The web client targets Safari 16.4 and later (ADR-0033), so a reader on Safari 16.4 to 18.3 cannot play a transcoded file. The web client already shows its download card when the browser cannot decode an attachment (ADR-0038), and that card plays in any desktop player. Opus in MP4 or WebM might play on some of those versions, but no source documents it for `<audio>` and no such device was at hand to try, so it is not chosen on hope. Keeping AAC for Safari would need a second encoder, and FDK AAC's licence is not compatible with AGPL-3.0. Every iPhone that runs iOS 18 can update to 18.4 or later, and current releases are iOS 26 and 27.

**Measurements** (2026-11-05, release build, one core of an Apple M3, decode + resample + encode + Ogg, the whole `transcode_to_opus` call):

| Upload | Type, source | Size | Opus | Ratio | Time |
|---|---|---|---|---|---|
| "Barely Alive" (Commons, CC BY 4.0), 110.8 s | WAV 44.1 kHz stereo | 19 539 740 | 1 344 303 | 6.9 % | 1.53 s |
| Same, encoded by LAME at 320 kbit/s | MP3 | 4 433 545 | 1 344 304 | 30.3 % | 1.64 s |
| Same, encoded by LAME at 128 kbit/s | MP3 | 1 773 443 | left as it is | | 0.000 s |
| "King of Jungle" (Commons, CC BY 4.0), 134.0 s | WAV 44.1 kHz stereo | 23 641 228 | 1 626 039 | 6.9 % | 1.52 s |
| Same, encoded by FFmpeg AAC at 256 kbit/s | M4A | 4 343 338 | 1 626 273 | 37.4 % | 1.70 s |
| Viardot, "Havanaise" (Commons, CC BY-SA 4.0), 284.1 s | FLAC 44.1 kHz stereo | 21 693 473 | 3 446 460 | 15.9 % | 4.00 s |
| A phone dictation, 120.5 s | M4A (AAC 247 kbit/s, 48 kHz mono) | 3 725 409 | 979 649 | 26.3 % | 0.48 s |
| Two generated tones, 140 s | WAV 44.1 kHz stereo | 24 696 078 | 1 698 389 | 6.9 % | 2.00 s |

That is about 70 times faster than real time for 44.1 kHz stereo, which is resampled, and 250 times for the 48 kHz mono dictation, which is not. FFmpeg decodes every output without error. Each output lasts as long as its source, to the sample (the `.wav`, `.mp3` and `.flac` sources), and its integrated loudness (EBU R128) matches the source within 0.1 LU. The M4A sources come out 29 ms longer, because symphonia keeps the AAC encoder's priming samples. No listening test was run.

## Obstacle (2026-11-05)
The implementation below was written and passes `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace` (583 server tests, 11 of them new) and `cargo deny check`, but only with a cmake binary on the path. On the owner's Mac there is none (`cmake not found`; Homebrew is installed but has no cmake). The repository's commit hook runs clippy and the tests there, so every commit fails with `failed to run custom build command for opusic-sys` / `is cmake not installed?`. Once merged, every local build and every agent worktree would fail the same way. CI and the release workflow are not affected: GitHub's Ubuntu images ship cmake, and the change installs it explicitly. Self-hosters are not affected either: they download the signed binary or the Docker image built from it.

Installing a system tool on the owner's machine is the owner's call, so nothing is wired in yet.

Options, in order of preference:
1. **The owner installs cmake** (`brew install cmake`, or `pip install cmake`). Then the implementation is applied as it is. It adds cmake next to nasm in `SERVER_BUILD_PACKAGES` (CI and release) and to the README prerequisites. This is the recommended option.
2. **`unsafe-libopus`** (libopus 1.3 transpiled to Rust by c2rust): no cmake and no C compiler, so the build stays as it is. But it has been unmaintained since 2024, has no SIMD (slower, not measured), and puts `unsafe` calls in our crate, against the workspace's `unsafe_code = "deny"`.
3. **Our own `-sys` crate building libopus with `cc`** (already in the tree for libwebp and SQLCipher): no cmake, but about 100 C files and a `config.h` to vendor and track per release, for one encoder.
4. **Wait for a pure-Rust encoder** with a track record (`rusty-opus`, `opus-pure`) and swap `OggOpusEncoder` then.

## Decision (proposed)
- **What is transcoded.** The background attachment worker (ADR-0033) takes the sniffed types `audio/wav` and `audio/flac` always. It takes `audio/mpeg`, `audio/aac` and `audio/mp4` (M4A, M4B) only when their average bitrate is above 160 kbit/s: below that, a second lossy generation saves less than a third of the size. The bitrate is read from the container's declared length when there is one, else checked after decoding. `audio/ogg` (already Opus or Vorbis), video, and audio with more than two channels or a sample rate outside 8 to 384 kHz stay as uploaded.
- **How.** symphonia 0.6 (pure Rust, MPL-2.0; WAV, PCM, FLAC, MP3, AAC and ISO-MP4 only) decodes one packet at a time. rubato 5 (pure Rust, MIT or Apache-2.0, sinc resampler, no FFT) resamples to 48 kHz. libopus encodes in 20 ms frames: `Application::Audio`, VBR, complexity 10, 96 kbit/s stereo or 64 kbit/s mono. The `ogg` crate (BSD-3-Clause) writes pages of about one second, with a correct pre-skip and an end granule that cuts the padding, so the file plays exactly as long as its source. No tags are carried over: titles, artists, cover art and any location go.
- **Budgets.** At most 2 hours of decoded audio (`MAXIMUM_TRANSCODED_AUDIO_SECONDS`), checked against the declared length before decoding and against the decoded length after each packet: a few kilobytes of FLAC can decode to hours of silence. At most 120 s per job (`AUDIO_TRANSCODING_TIME_BUDGET`), checked after each packet. The encode also stops as soon as its output reaches the upload's size. Memory therefore follows the output, which is smaller than the upload, plus a few small buffers; decoded audio is never held whole. The job takes one of the two picture-processing slots, so heavy media work stays bounded together.
- **Stored form.** The result replaces the file only when it is strictly smaller, as for pictures: a new file id, the row switched in one transaction, the old file deleted after it, and no original kept. The row takes `audio/ogg`, its size, and its name with `.opus` in place of the audio extension (`take 3.WAV` becomes `take 3.opus`). Undecodable, corrupt (a packet that fails to decode is not skipped), over-budget or not-smaller uploads keep their file. Only an encoder or resampler failure is logged, without the file's name.
- **Build.** cmake becomes a build prerequisite, like nasm: `SERVER_BUILD_PACKAGES` installs it in CI and in the release workflow, and the README lists it (`brew install cmake` on macOS). Self-hosters download the signed binary or the Docker image built from it, so nothing changes for them. libopus is linked statically: the binary needs no system library.

## Consequences
- Lossless and high-bitrate audio takes 3 to 15 times less space and bandwidth. A 25 MB limit now holds about 35 minutes of stereo audio instead of 2.5 minutes of WAV, for later uploads. Uploads are still checked against the limit before the transcode.
- The original is gone. A musician sharing a FLAC to have it mastered gets Opus back. shortcut: no "send the original" choice; add an upload flag the worker honours (a protocol change) if members ask for it.
- Readers on Safari 16.4 to 18.3 get the download card for transcoded audio instead of the player. Files uploaded before this change, and files left as they are, play as before.
- The name and type shown in a message come from the row. A client that loaded the message before the swap shows the old name and size until it reloads, as for compressed pictures.
- A job lost to a full queue (64) or a restart is not picked up again: the worker's catch-up query and its partial index (migration 0027) list picture types only. shortcut: a dropped audio job keeps its upload; add the audio types to that index with a migration if this shows up in practice. Audio uploaded before this change is never transcoded, and there is no `admin` command to do it.
- cmake is one more tool to install for anyone building Mellow from source. The GitHub-hosted Ubuntu images already ship it, and the workflow installs it explicitly.
- New C code on untrusted-input paths: none. libopus only encodes PCM that symphonia, in Rust, decoded from the upload. symphonia's decoders read untrusted bytes, within its own limits and our budgets. A security audit of the new decoder surface is still required before this is accepted.
- New crates: `opus`, `opusic-sys`, `ogg`, `rubato` (with `audioadapter`, `audioadapter-buffers`, `audioadapter-sample`, `num-complex`, `windowfunctions`, `visibility`), `symphonia` and its parts (with `extended`, `regex-lite`, `audio-codec-algorithms`). All pass `cargo deny`. BSD-3-Clause asks that binary distributions reproduce libopus's copyright notice. The server binary has no third-party notice file yet, for this or for the C libraries it already bundles (libwebp, SQLCipher, OpenSSL, aws-lc). That gap is recorded, not closed here.

## Alternatives considered
- **No transcoding** (ADR-0038): the status quo. Simple, but a WAV costs 15 times its Opus form in storage, quota and download time.
- **A pure-Rust Opus encoder**: no C toolchain, but the crates are three to four months old, and the files are kept without an original. Revisit once one has a track record. Swapping the encoder would only change `OggOpusEncoder`.
- **`unsafe-libopus`** (c2rust transpile): no cmake, but unmaintained since 2024, libopus 1.3 without SIMD, and unsafe calls in our crate.
- **AAC in MP4 for Safari**: plays on every Safari, but there is no AGPL-compatible encoder of comparable quality, and two outputs per upload would double the work.
- **Opus in WebM or MP4**: not documented to play on the Safari versions that lack Ogg, and not tried on them; Ogg is the format RFC 7845 defines and every other browser plays.
- **Transcoding in the browser before upload**: there is no WebCodecs Opus encoder in Safari, and a Wasm libopus would add hundreds of kilobytes to the client. The server already has a worker and the budgets.
- **FFmpeg on the host**: breaks the one-binary promise.
