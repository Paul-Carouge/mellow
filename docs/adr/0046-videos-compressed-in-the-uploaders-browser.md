# 0046. Videos compressed in the uploader's browser

- Status: Proposed (the Content-Security-Policy change below waits for the owner's approval)
- Date: 2026-11-05
- Amends: [ADR-0038](0038-play-audio-and-video-attachments-inline.md) (videos are no longer always stored as uploaded), [ADR-0019](0019-web-client-served-by-the-server-binary.md) (the policy's `worker-src`)

## Context
The owner wants videos "as small as possible at near-original quality". Phones record 1080p or 4K H.264 or HEVC at 15 to 50 Mbit/s, so a 10 s clip is 20 to 60 MB and a minute does not fit the instance's 25 MiB default limit. ADR-0038 stores and serves videos as uploaded, with the `moov` box often after `mdat` (players then fetch the end first). Transcoding on the server was rejected there and stays rejected: no pure-Rust H.264 decoder, an FFmpeg dependency breaks the one-binary promise, and minutes of CPU per clip on small hosts. The uploader's browser has hardware encoders behind WebCodecs (`VideoEncoder`), and the composer already uploads each file at once into its tray (ADR-0020) with an abortable request.

## Decision
- **Where.** The web client compresses a video (`file.type` is `video/*`) before it uploads, in a dedicated module Worker (`web/src/lib/video-compression.worker.ts`), so decoding, scaling, encoding and muxing never touch the main thread. Vite builds the Worker and its library into their own chunk, loaded only when a video is attached. A browser without `Worker` or `VideoEncoder` (Firefox on Android, older Safari) sends the original without loading anything.
- **Library: mediabunny 1.65.2** (MPL-2.0, by Vanilagy and contributors; 1.65.2 published 2026-11-04, releases every few weeks; zero runtime dependencies, its two dependencies are type packages). It reads MP4, QuickTime, WebM and Matroska, drives WebCodecs, and writes a faststart MP4. The Worker chunk is 461 kB minified, 93 kB Brotli. MPL-2.0 is file-level copyleft and compatible with the AGPL-3.0: the package is bundled unmodified, its per-file licence headers stay in the chunk, and its notice and licence are in `third-party-notices.txt`. It replaced `mp4-muxer` and `webm-muxer`, which its author deprecated in its favour. Pinned exactly in `web/package-lock.json`; `npm audit` reports nothing new.
- **Output: H.264 video, AAC or Opus audio, in a faststart MP4** (`fastStart: "in-memory"`, `ftyp`, `moov`, `mdat`), without the source's metadata tags (`tags: {}`: no location, device or dates; the server's ADR-0038 walk stays as the boundary check). H.264 is the only video codec every reader's browser decodes: AV1 is about half the size at equal quality (SVT-AV1 at 2 Mbit/s matched H.264 at 4 Mbit/s on the test clips) but Safari plays it only with AV1 hardware (iPhone 15 Pro, M3 and later), so many readers would get the download card; VP9 in realtime mode overshot its rate by 90 % and gains little over hardware H.264. Revisit AV1 when the readers' floor moves. Audio: AAC or Opus already in the source is copied; other audio is encoded to AAC at 128 kbit/s, or Opus at 96 kbit/s where the browser has no AAC encoder (Chrome on Linux).
- **Size and rate.** Never upscaled. Scaled down to fit a 1920 × 1080 box in the video's own orientation (portrait clips stay 1080 wide; rotation is baked into the frames). Frame rate kept up to 60, brought to 60 above. Key frame every 2 s.
- **Bits per pixel: 0.1** (`TARGET_BITS_PER_PIXEL`): about 6.2 Mbit/s at 1080p30, 12.4 Mbit/s at 1080p60, 2.8 Mbit/s at 720p30, in variable bitrate (static content, such as screen recordings, comes out smaller). Chosen on the measurements below: 0.07 (the research's first figure, measured with FFmpeg's VideoToolbox) gave only VMAF 86 and 71 with Chrome's own encoder; 0.1 gives 90 and 76 at about a quarter of the original size.
- **The instance's limit** (`Ready.maximum_attachment_bytes`). When a clip at the target would not fit, the bitrate is lowered to fit 90 % of the limit, down to a floor of 0.05 bits per pixel (`MINIMUM_BITS_PER_PIXEL`); below the floor the original is kept (and refused by the size check if it is too large) rather than sending something smeared. The size check runs on the compressed file, before any byte is sent.
- **The original is kept** when: the browser cannot decode the video (HEVC on most non-Apple systems), the duration is unknown, the source is HDR (`pq` or `hlg` transfer: tone mapping to SDR H.264 washes it out), the browser has no H.264 encoder, the audio can be neither copied nor encoded, a track would be dropped, the clip is already near the target and not H.264, the conversion fails (logged, never lost), or a transcode is not at least 20 % smaller. A source already near the target (at most 1.25 times the target total rate, no scaling, at most 60 fps) in H.264 is only remuxed (packets copied: milliseconds), and that remux is kept when it is not larger, for the faststart and the dropped tags.
- **Tray.** The item says "Compression… 42 %" / "Compressing… 42%" with the preview dimmed as while uploading, and a "Envoyer l'original" / "Send the original" link (the Retry link's style, labelled "Send the original of clip.mov") that stops compression and uploads the file as it is. The item's Remove button cancels: it stops the Worker and nothing is uploaded. Send waits for compressing files as for uploading ones. No new screen or token.
- **Content-Security-Policy: `worker-src 'self'`** instead of `'none'`. Browsers load a worker script only when it is served with a JavaScript type; on this origin only the client's own hashed assets are, since uploads are sniffed to media types or `application/octet-stream` and served with `nosniff` (ADR-0020, ADR-0038), so an upload still cannot become a Worker or a service worker. **Not applied in this change**: without it the Worker is refused, the composer logs it and sends the original, so the feature is dormant until the owner approves the policy change and `web_client.rs` (both policies and their tests) is updated.

## Measurements (2026-11-05)
Headless Google Chrome on macOS (Apple silicon, VideoToolbox behind WebCodecs), the built Worker driven by Playwright; VMAF by libvmaf against the source, frames compared by index.

| Clip | Original | Output | Ratio | Time | VMAF |
|---|---|---|---|---|---|
| jellyfish, 10 s 1080p29.97 H.264 + AAC, 24 Mbit/s | 31.5 MB | 7.80 MB | 25 % | 1.6 s | 89.6 |
| Big Buck Bunny, 10 s 1080p30 H.264, 24 Mbit/s | 30.7 MB | 7.89 MB | 26 % | 1.8 s | 75.6 |
| generated 4K30 H.264 + 192 kbit/s AAC with noise, 6 s | 34.8 MB | 4.73 MB (1080p) | 14 % | 1.0 s | – |
| generated 1080p60 HEVC, rotated 90°, PCM, 6 s | 16.8 MB | kept (encoder overshot on noise, under 20 % saved) | – | 1.8 s | – |
| generated 720p30 H.264 at 1 Mbit/s + AAC, 6 s | 811 kB | 807 kB (remux, faststart) | 99.6 % | 0.02 s | – |
| generated 4K30 HEVC Main10, PQ, 4 s | 20.2 MB | kept (HDR) | – | 0.1 s | – |

Bitrate against quality with Chrome's encoder (same harness, video only):

| Clip | Encoder | 4.35 Mbit/s (0.07) | 6 Mbit/s (0.1) | 8 Mbit/s |
|---|---|---|---|---|
| jellyfish | hardware | 85.8 | 89.3 | 91.4 |
| jellyfish | software (`prefer-software`, OpenH264) | 81.1 | 86.9 | – |
| Big Buck Bunny | hardware | 71.4 | 75.2 | – |
| Big Buck Bunny | software | 66.4 | – | – |

For reference, FFmpeg's `h264_videotoolbox` at 4 Mbit/s scored 88.3 and 80.0 on the same clips: Chrome's hardware path is a few points below it at equal rate, and software OpenH264 (Chrome on Linux) a few more. Not measured yet: real phone clips (high motion, iPhone HEVC portrait), Chrome on Linux and Windows, Safari and Firefox by hand.

## Consequences
- Most phone videos upload four times smaller and start playing at once (faststart); a minute of 1080p fits the 25 MiB default where it did not.
- The uploader's device does the work: about 1.6 s for 10 s of 1080p with hardware encoding, several times longer in software, and battery on phones. "Send the original" and Remove are always there.
- The output is a second lossy generation: VMAF 76 to 90 at the target, so fine detail in high-motion clips softens. Readers who need the exact file use "Send the original".
- iPhone HEVC that is SDR becomes H.264 and plays in every browser; HDR iPhone clips (the default since iPhone 12) stay as they are, HEVC that Firefox and most Chrome builds cannot play. shortcut: HDR sources are kept; tone-map to SDR (or keep HEVC when every reader's browser decodes it) when HDR uploads are common.
- `BufferTarget` holds the output in the Worker's memory: at most the instance's limit plus the encoder's buffers.
- The server is unchanged: it still checks size, type and metadata of whatever arrives, compressed or not.
- shortcut: the 0.1 bits-per-pixel target was measured on two reference clips with Chrome on macOS only; measure real phone clips and Chrome with OpenH264 on Linux before treating it as settled.

## Alternatives considered
- **Transcoding on the server**: rejected by ADR-0038 (FFmpeg or a codec stack in the binary, CPU on small hosts).
- **FFmpeg compiled to WebAssembly**: about 30 MB of WebAssembly, single-threaded software x264, and LGPL/GPL build questions.
- **AV1 or VP9 output**: smaller (AV1) but not playable in every reader's Safari; VP9 overshoots in realtime mode. See the output decision.
- **A fixed quality preset (`QUALITY_HIGH`)** instead of bits per pixel: mediabunny's presets are tuned for its own targets and give no control tied to the instance's limit.
- **Compressing on the main thread**: simpler, but decoding and encoding callbacks would compete with typing and scrolling.
