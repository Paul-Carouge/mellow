# Video SFU spike (ADR-0035)

Throwaway code that measured the video design of [ADR-0035](../../docs/adr/0035-video-in-voice-channels.md) before any feature code. Like [`spikes/voice-sfu`](../voice-sfu/), it has its own Cargo workspace, so it adds nothing to Mellow's dependencies, builds or CI. Do not build on it: stage 16.5 extends the real `voice` module in `mellow-server`.

- `sfu`: one room, VP8 video only, `str0m` 0.24.1 with ICE-lite on one UDP port. Each participant offers at most one camera m-line (simulcast or not) and K receive m-lines. Frames are forwarded as they are, never decoded. Each watcher gets one simulcast layer per camera: `--focus-layer` for the camera in focus (the speaker view's large tile), `--other-layer` for the others; the focus moves to the next camera every `--focus-seconds` (4 by default), so layer switches, keyframe requests (PLI) and keyframes happen all the time. `--bwe-desired-kbps` turns on `str0m`'s send-side bandwidth estimation (TWCC) towards each watcher. Every 5 s it prints its CPU use, its UDP packets and bit rates in and out, how long a frame spends inside it, how many keyframes it asked for and how long a layer switch waited for its keyframe. Serves `web/index.html` for browser tests.
- `load`: N headless participants, of which `--cameras` send a synthetic VP8 camera at 30 frames/s on each of `--layers` (`rid:kbit/s`; one entry means no simulcast). A keyframe is three times a normal frame, sent first and on each keyframe request. Every bot watches up to `--slots` cameras. Each frame carries its send time, so the bots, which share one clock, measure the true one-way latency of every frame they receive.

```bash
cargo build --release
./run-series.sh                                   # one layer at 720p, 1.5 Mbit/s, rooms as participants:cameras
LAYERS=h:1500,l:500,q:150 FOCUS=h OTHER=q ./run-series.sh 10:5 25:10 50:10   # simulcast, speaker view
LAYERS=h:1500,l:500,q:150 FOCUS=l OTHER=l ./run-series.sh 10:5 25:10         # simulcast, grid view
LAYERS=h:1500,l:500,q:150 FOCUS=h OTHER=q SLOTS=9 ./run-series.sh 25:25      # 25 cameras, 9 watched
LAYERS=h:1500,l:500,q:150 FOCUS=h OTHER=q BWE=3000 ./run-series.sh 25:10     # with bandwidth estimation
./target/release/sfu --ip 192.0.2.10 --focus-layer h --other-layer q   # then open http://192.0.2.10:3000 in two tabs
```

Each row of the series is a fresh SFU and 20 s of measurement (`DURATION`) after a 3 s warm-up. The SFU binds the UDP port 50000 and the HTTP port 3000 on the address that routes to the internet (`--ip` to choose). It is not hardened: run it on a trusted network only.

**On the owner's server** (Linux, i7-8700T), as for the voice spike: the host firewall drops LAN traffic to unopened ports, so run the bots on the same machine, for example in a throwaway container on the host network with the Rust toolchain (`docker run --rm -it --network host -v "$PWD":/spike -w /spike rust:1 sh`), then the same commands. Linux caps socket buffers at `net.core.rmem_max` (often 208 KiB); raising it for the run (`sysctl -w net.core.rmem_max=4194304 net.core.wmem_max=4194304`) avoids measuring kernel drops instead of the SFU.

`web/chrome-check.cjs` drives headless Chromium with Playwright (any local install, given as `PLAYWRIGHT=/path/to/node_modules/playwright`): `PAGES=3 node web/chrome-check.cjs http://192.0.2.10:3000` against an SFU started with `--focus-layer h --other-layer q` prints what each page sends and receives (resolution, frame rate, loss, keyframe requests), then the size of offers carrying video m-lines. It uses the page's synthetic camera, since headless Chromium on macOS never answers `getUserMedia`.
