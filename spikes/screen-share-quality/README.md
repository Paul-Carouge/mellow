# Screen share bench

Measures the desktop client's GPU screen encoder (ADR-0054, ADR-0058) on reproducible screens: the
bytes it puts on the network, how the pacing spaces pictures, and what a watcher sees. Used for the
2026-11-15 amendment of ADR-0058 (refining a still screen).

`measure.py` renders a 60 Hz scene from text Mellow's own work does not edit (vendored str0m code,
the licence), paces it as `crates/mellow-media/src/h264_encoder_linux.rs` does (the source's rate
measured with ADR-0058's hysteresis, the three layers reopened at 30 or 60 with a keyframe each,
token buckets for the top layer at the encoder's rate and the lower layers at 30, newest picture
wins, a still screen repeated once a second, and with `--refine 50,1000` the refining repeats),
encodes every layer with FFmpeg's `h264_vaapi` and Mellow's settings (Constrained Baseline, no
B-frames, CBR with half a second of buffer, `async_depth` 1, radeonsi quality level 21) and counts
bytes as they go on the network: str0m's packetizer drops the filler data AMD's constant bit rate
pads pictures with (pinned by `h264_stream::tests::str0m_never_sends_filler_data`), so the filler
is only printed apart. A packet counts 40 bytes of RTP header, extensions and SRTP tag per 1 100
bytes of payload; retransmissions are not counted.

Scenes (`--scene`): `desktop` (still code, a scroll at 240 px/s, still, a document page, typing at
10 characters a second, a 640 × 360 video region over code, 12 s), `switch` (the video region
changing 60 times a second for 5 s, then 10), `heavy-switch` (the whole screen moving finely, 60
then 10 a second), `fifteen` (the region at 15 a second), `heavy` (the whole screen at 60).

Needs FFmpeg with VA-API, a VA-API H.264 encoder on `/dev/dri/renderD128`, Python 3 with NumPy and
Pillow, DejaVu Sans Mono and Noto Sans. `--out-dir` is required and refused inside the repository:

```bash
B=spikes/screen-share-quality/measure.py
python3 $B --out-dir /tmp/bench --scene desktop --rate 30 --quality --name before
python3 $B --out-dir /tmp/bench --scene desktop --rate 30 --quality --refine 50,1000 --name after
python3 $B --out-dir /tmp/bench --scene heavy-switch --refine 50,1000 --name switch
```

Output, one JSON line: the network rate, the worst one-second window of all three layers (the
server allows 1.5 MB per sender), the filler str0m dropped, the encoder rates opened (`segments`),
and per phase the network and top-layer rates, the worst second lying within the phase, top-layer encodes and new pictures
per second, the delay of new pictures (mean, worst) and the jitter of their spacing against the
source's (standard deviation). With `--quality`, per phase `[PSNR-Y, SSIM-Y, PSNR-Y text crop,
SSIM-Y text crop]` of each shown top-layer picture against the source picture it encodes.
Radeonsi gave bit-identical streams on repeated runs within a session; across sessions the bytes
without filler moved by up to a third.
