# 0035. Video in voice channels: VP8 simulcast forwarded by the built-in SFU

- Status: Proposed (2026-11-02, awaiting the owner's decision and the measurement on the owner's server)
- Date: 2026-11-02
- Builds on [ADR-0030](0030-voice-channels-with-an-sfu-in-the-server-binary.md), which decided and measured audio only. What it decided still holds: `str0m` 0.24 inside the server binary, one UDP port, ICE-lite, no TURN, signalling over the gateway, one media thread, the server never decodes media.

## Context
ROADMAP stage 16.5 adds cameras to voice channels, with a grid and a speaker view, "exactly like Discord" (the owner's direction for voice). ADR-0030 left these questions open: the codec, how the SFU forwards video, how many cameras are sent and watched at once, the permission, and what video costs the server in CPU and bandwidth. The product promise does not change (ADR-0002): one binary, nothing else to install or run.

The forces are those of ADR-0030, with two differences of scale:
- **Video is about 25 times heavier than voice.** A 720p camera at 30 frames per second sends about 1.5 Mbit/s and 180 packets per second, against 64 kbit/s and 50 packets for a voice. The SFU's cost is per forwarded packet (ADR-0030 measured about 12 µs per packet on the Mac and 22 µs on the owner's server), so what each person watches, and at which quality, decides the cost.
- **The host's upload becomes the limit.** Every watcher receives their own copy of every camera they watch. Many instances, the owner's included, run at home, where the upload link is shared with everything else.
- **The SFU forwards and never transcodes.** Every watcher must decode what the camera's browser encoded, so codecs and qualities are chosen by the senders, not by the server.

## Decision

### Codec: VP8 only, for now
- Cameras are sent in **VP8**, with its retransmission stream (RTX). The server's answer offers VP8 and nothing else on video m-lines (`enable_vp8` next to the Opus configuration of ADR-0030). Every browser the web client supports encodes and decodes VP8. VP8 simulcast is checked below in Chromium; Firefox and Safari support it too, which stage 16.5 checks for real.
- Clients restrict their video transceivers to VP8 and RTX (`RTCRtpTransceiver.setCodecPreferences`) and their audio ones to Opus. This halves the offer: the measurement below shows why it matters.
- **One codec for a whole channel** avoids the only hard question an SFU has with codecs: a camera in a codec some watcher cannot decode. Since every client sends and receives VP8, the server never has to choose a common codec, renegotiate when someone joins, or refuse a camera.
- **H.264** is not offered. It would save battery where the encoder is in hardware (Macs, phones), but simulcast with H.264 is uneven across browsers, its profiles and levels complicate negotiation, and some Linux browsers must download a plugin first. It can be added later by a new ADR, once phones or measurements ask for it; `str0m` supports it.
- **VP9 and AV1** compress 30 to 50 % better and replace simulcast with scalable layers (SVC), but they cost senders much more CPU, AV1 encoding is missing from Safari, and their SVC forwarding in `str0m` would be a second code path. Not now, for the same reasons; the bandwidth they would save is measured below as the main cost of video, so they are the first candidate when it matters.

### Forwarding: simulcast with three layers, chosen per watcher
- Each camera sends **three simulcast layers** (`a=simulcast` with RIDs): `h` 1280 × 720 at up to 1.5 Mbit/s, `l` 640 × 360 at up to 500 kbit/s and `q` 320 × 180 at up to 150 kbit/s, all at 30 frames per second. The capture is capped at 720p30, Discord's camera quality. Uplink per camera: about 2.2 Mbit/s.
- **`str0m` 0.24 supports this.** It receives simulcast (the answer inverts the offer's send layers, and `MediaData::rid` names the layer of each frame), asks for keyframes per layer (`Writer::request_keyframe(Some(rid), Pli)`), reports watchers' keyframe requests (`Event::KeyframeRequest`), and re-packetizes each frame it writes with its own sequence numbers and VP8 picture ids, so a watcher's stream stays continuous when its layer changes. The spike and a real Chromium (below) confirmed all of it. Its repository's `chat` example forwards simulcast the same way, with a fixed layer.
- **Each watcher gets one layer per camera**, the one that fits the tile: the client says, for each camera it shows, whether the tile is large (`high`), medium or small, and the server picks `h`, `l` or `q`, lowered by the bandwidth estimate below. A watcher only switches to a layer at a **keyframe** of that layer: the server keeps forwarding the old layer, asks the camera for a keyframe (PLI), and switches when it arrives. The same happens when someone starts watching a camera. Keyframe requests are throttled to one per camera layer every 300 ms, and a watcher's own PLI (after a loss) is passed to the camera it is watching, for the layer it receives. Measured: a switch waits one to two frame intervals (33 to 66 ms) on a local network; on the internet, add one round trip.
- **Layers that disappear.** When a sender's uplink is short, its browser stops sending its top layers by itself. A layer with no frame for one second counts as gone, and its watchers fall back to the next lower layer still arriving.
- **Layers nobody watches are paused.** The server tells each camera which of its layers someone watches (an event to that session only), and the client sets `active: false` on the other encodings. A laptop in a grid of small tiles then encodes and uploads only `q` and `l`. Without it, every camera uploads its 2.2 Mbit/s even when nobody watches it large; the spike's 25 cameras sent 57 Mbit/s to the server for that reason.
- **Frame mode.** `str0m` reassembles each frame and re-packetizes it for each watcher, as for voice. Its lower-level RTP mode would skip that work, but the spike shows it is not where the time goes (the cost per packet stays that of voice), and frame mode gives continuity across layer switches for free. Revisit only if profiling on the owner's server says otherwise.

### Bandwidth estimation and slow links
- **Towards watchers**, the server turns on `str0m`'s send-side bandwidth estimation (`RtcConfig::enable_bwe`, transport-wide congestion control feedback, which browsers send) for every participant who watches video. It sets the desired bit rate to the sum of the layers the watcher asked for, and receives `Event::EgressBitrateEstimate`. `str0m` also reports REMB estimates, but browsers prefer TWCC when it is negotiated, so TWCC is the one used.
- **Allocation.** Whenever the estimate changes, the server fits the watcher's cameras into 90 % of it: it starts from the layers asked for, then lowers the smallest tiles one layer at a time before the large one, and when every tile is at `q` and it still does not fit, it **pauses** tiles, smallest first and the large tile last. A paused tile shows the person's avatar with a "connection too slow" mark. **Audio is never paused**: it costs a few percent of the budget. Layers go back up only after the estimate has stayed above the need, plus 20 %, for 5 seconds, so a link at the edge does not flap.
- **Towards the server**, the sender's browser runs its own estimation and drops its top layers, as above. The server receives retransmission requests (NACK) from watchers and answers them from `str0m`'s send buffer, and asks cameras for the packets it lost on their uplink, so a lost packet costs one retransmission, not a keyframe.
- **Cost.** The spike measured bandwidth estimation at about 11 points of a core in a room of 25 with 10 cameras (18 % without, 29 % with). It is worth it: without it, a watcher on a slow link receives more than it can take, loses packets and keeps asking for keyframes, which costs every other watcher of that camera too.

### Limits
- **Cameras per channel: anyone connected**, so up to 25, the channel cap of ADR-0030 (Discord also allows video in channels of up to 25 people).
- **Cameras watched at once: at most 9 per person.** Each participant's offer carries 9 video receive m-lines, assigned by the server like the 16 audio receive slots of ADR-0030, so cameras turning on and off need no renegotiation. Like the 16 voices, this bound keeps the cost per participant flat however many cameras are on: the spike's 25 cameras watched 9 at a time cost the same as 10 cameras.
- **Views.** The **grid** is the default, as on Discord: up to 9 tiles at once (3 × 3), the others shown as avatars on a further page or row. The **speaker view** shows one large tile, the person speaking or the one picked, and up to 8 small ones. With the tile sizes of a 1080p window, a grid of 1 tile receives `h`, 2 to 4 tiles `l`, and 5 to 9 tiles `q`; the speaker view receives `h` for the large tile and `q` for the others.
- **Resolution and frame rate:** at most 1280 × 720 at 30 frames per second per camera, enforced by the client's capture constraints and encodings, and on the server by a packet and byte budget per participant: the 200 packets per second of the voice-only media loop become 1 500 packets and 4 Mbit/s averaged over a second for a participant sending a camera (three layers, keyframes three times larger, retransmissions), and stay 200 otherwise. Larger datagrams are still dropped unread.
- **Per instance:** the 50 participants of ADR-0030 stay. Two caps are added:
  - **25 cameras sending at once** on the instance, a constant like the channel cap. It bounds what the server receives (25 cameras with three layers is 7 300 packets and 57 Mbit/s per second) and what the instance's download carries. The 26th camera gets a refusal ("too many cameras on this server"), and the person stays in the call.
  - **A video upload budget**, `[voice] video_upload_mbps` in `mellow.toml`, default **100 Mbit/s** (to be confirmed by the owner, see below). The allocation above uses it: each watcher's budget is the smaller of its own estimate and its fair share of the instance's budget, so a home connection is never saturated by video, and voice, the web client and messages keep working. The administrator of an instance with a small upload sets it lower; `0` turns cameras off.
- **The worst case under these caps** is 50 participants each watching 9 tiles: about 23 000 packets per second out (speaker view) to 27 000 (grid of medium tiles), plus 7 300 in. The Mac forwards that on a third of a core. On the owner's server it is near the 40 000 packets per second ADR-0030 found for one media thread once voice is added; the measurement there decides whether these caps hold or must come down until the media loop is spread over several threads.

### Permission: `STREAM`
- A new bit, **`STREAM = 1 << 21`**, the next free one after `USE_SOUNDBOARD` (bit 20). It mirrors Discord's "Video" permission (`STREAM` in Discord's API): turning a camera on in a voice channel, and later sharing a screen (16.6). The interface calls it "Vidéo". It is **channel-scoped** (`CHANNEL_SCOPED`) and in **`DEFAULT_EVERYONE`**.
- **Migration 0033** grants it to every existing `@everyone` role, as 0022 and 0023 did: `UPDATE roles SET permissions = permissions | 2097152 WHERE id = guild_id;`.
- **The domain decides** (ADR-0007): `domain::permissions` gains a pure `may_send_video(channel_permissions)`, true when `voice_entitlement` is not `Refused` and `STREAM` is held. `SPEAK` is not required: as on Discord, a member who may not speak may still show their camera. Watching needs no permission beyond being connected. `Guilds::voice_access` returns it with the voice entitlement, so every path that already re-checks voice access (role, overwrite and member edits, kicks, bans, deletions) re-checks cameras too. A member who loses `STREAM` stops being forwarded at once, and their camera shows as off.
- Unknown bits from clients are still rejected at the boundary until the bit is defined; older clients keep it as an unknown bit, as they already do.

### Protocol sketch (gateway `v1`, additive)
Released clients keep working: every field is optional with a default, every frame and event is new, and an audio-only session is answered exactly as today.
- **`VoiceState.self_video: bool`** (`#[serde(default)]`): the person's camera is on and forwarded. The server sets it false when it stops forwarding (permission lost, instance cap), and a client seeing its own state turn false stops its camera. The client's `voice_state` frame gains the same optional field to turn the camera on or off; turning it on is refused with a reason when not allowed.
- **`voice_renegotiate { offer_id, sdp_offer }`**, a new client frame: a new offer for the existing session, answered by the existing `VOICE_SESSION_DESCRIPTION` (same `offer_id` rule) or `VOICE_JOIN_REFUSED`. The client sends it once per session, the first time it turns a camera on or opens a video view, adding one `sendonly` simulcast video transceiver and 9 `recvonly` ones to its 17 audio m-lines. A client may also include them in its first `voice_join`. The client stays the offerer, so the server never sends offers.
- **`VOICE_SLOT_ASSIGNED`** is reused for video receive slots: it already names a `mid` and the `user_id` whose media it carries.
- **`voice_video_watch { cameras: [{ user_id, size: "large" | "medium" | "small" }] }`**, a new client frame, at most 9 entries: the cameras this session shows and how large. The server assigns slots and layers from it. It replaces the whole previous list and is coalesced on the client (one frame per layout change, not per resize event).
- **`VOICE_VIDEO_LAYERS { active: ["q", "l"] }`**, a new event to a camera's session only: the layers someone watches, so the client pauses the others.
- **Offer size.** Measured in Chromium 156 with the candidates of this Mac: 17 audio m-lines, a simulcast camera and 9 video receive m-lines make an offer of **73.6 kB with every codec, 37.7 kB with VP8 only, and 33.9 kB with VP8 and Opus only**; the same with 4 video receive slots is 25.4 kB. The cap of `MAXIMUM_VOICE_OFFER_BYTES` (28 KiB) rises to **48 KiB** for offers carrying video, and the frame cap of identified clients from 32 to 52 KiB; frames before `identify` stay at 4 KiB. Firefox and Safari offers are measured in stage 16.5 before the number is fixed.

## Measurements
The spike in [`spikes/video-sfu/`](../../spikes/video-sfu/) copies the voice spike's layout and stays out of the workspace: its own Cargo workspace and lock file, no dependency added to the product. It has three parts:
- `sfu`, a one-room `str0m` 0.24.1 SFU built like the design above: ICE-lite, one UDP port, VP8 only, a camera m-line and K receive m-lines per participant, a layer per watcher and camera (`--focus-layer` for the one in focus, `--other-layer` for the others), the focus moving to the next camera every 4 seconds so watchers switch layers and ask for keyframes all the time, keyframe requests throttled at 300 ms, and `str0m`'s bandwidth estimation on demand. It reports its CPU time (`getrusage`), its UDP packets and bit rates, the time a frame spends inside it, and how long layer switches wait for their keyframe.
- `load`, headless `str0m` participants. Cameras send synthetic VP8 frames at 30 per second on each layer, sized for the layer's bit rate (720p 1.5 Mbit/s, 360p 500 kbit/s, 180p 150 kbit/s), with keyframes three times larger on request. Every frame carries its send time, so the bots, which share one clock, measure the one-way latency of every frame they receive and count the frames that never arrive.
- `web/index.html` and `web/chrome-check.cjs`, for real browsers.

Each row is a fresh SFU and 20 s of measurement after a 3 s warm-up, on an **Apple M3 laptop** (8 cores), SFU and bots on the same machine through its LAN address; figures are the two 5 s reports of each run (a range when they differ). In every row, every participant watches every other camera up to its slots, and no frame was lost.

| Participants : cameras | Layers | Each watcher receives | SFU CPU (one core) | Packets/s in / out | Mbit/s in / out | Inside the SFU p50 / p99 | One-way latency p50 / p99 |
|---|---|---|---|---|---|---|---|
| 2 : 2 | one, 720p | 1 × 720p | 5 to 6 % | 400 / 400 | 3.1 / 3.1 | 0.4 to 0.6 / 1.8 ms | 3.0 / 5.7 ms |
| 10 : 5 | one, 720p | 4 to 5 × 720p | 18 to 26 % | 1 110 / 8 220 | 8.0 / 70.6 | 0.8 to 1.7 / 7 to 9 ms | 4.6 / 13.2 ms |
| 25 : 10 | one, 720p | 9 to 10 × 720p | 46 % | 2 330 / 43 400 | 16.4 / 376 | 1.2 / 14 ms | 9.3 / 44.9 ms |
| 10 : 5 | simulcast | speaker: 1 `h`, others `q` | 10 % | 1 570 / 3 040 | 11.5 / 22.0 | 0.1 / 1.6 to 1.9 ms | 4.2 / 19.1 ms |
| 25 : 10 | simulcast | speaker | 18 % | 3 250 / 11 330 | 23.1 / 75.3 | 0.07 / 1.6 ms | 5.1 / 24.0 ms |
| 50 : 10 | simulcast | speaker | 33 % | 3 800 / 22 750 | 23.6 / 152 | 0.1 / 3.4 ms | 8.3 / 13.5 ms |
| 10 : 5 | simulcast | grid: all `l` | 6 to 16 % | 1 560 / 2 820 | 11.5 / 23.6 | 0.1 to 0.2 / 1.1 to 2.6 ms | 4.5 / 11.4 ms |
| 25 : 10 | simulcast | grid: all `l` | 26 % | 3 240 / 14 640 | 23.1 / 125.5 | 0.15 / 2.4 ms | 6.3 / 15.4 ms |
| 25 : 25, 9 slots | simulcast | speaker, 9 cameras | 23 to 26 % | 7 290 / 10 200 | 57.0 / 63.8 | 0.04 / 1.7 to 2.0 ms | 7.1 / 11.2 ms |
| 25 : 10, estimation on | simulcast | speaker | 29 % | 3 250 / 11 340 | 23.1 / 75.4 | 0.05 / 2.2 ms | 5.1 / 8.6 ms |

"Inside the SFU" runs from the arrival of a frame's first packet to the end of its last write, so for video it includes waiting for the frame's other packets. Layer switches waited 33 to 36 ms at the median and 37 to 66 ms at the 99th percentile in every simulcast row: one or two frame intervals, since a bot answers a keyframe request with its next frame. With estimation on, the estimates towards the 25 watchers reached 4.0 to 4.8 Mbit/s against a desired 3 Mbit/s.

**Chromium 156** (headless, Playwright, a synthetic 720p canvas camera, three pages on the Mac against the spike SFU with `--focus-layer h --other-layer q`): `str0m` accepted Chromium's simulcast offer at the first try; each page sent its three layers (1280 × 720, 640 × 360 and 320 × 180, 30 frames per second, no quality limitation), and received one camera at 1280 × 720 and the other at 320 × 180, about 415 frames decoded each in 15 s, **0 packets lost and 0 keyframe requests from the browsers**, while the focus moved every 4 s: the decoders accepted every layer switch made at a keyframe. The SFU used 3 % of a core.

What the figures say:
1. **Simulcast is not optional.** A room of 25 with 10 cameras, each watched at 720p, sends 376 Mbit/s and 43 000 packets per second. With simulcast and a speaker view, the same room sends 75 Mbit/s and 11 000 packets: five times less bandwidth and four times fewer packets, and the CPU follows (46 % against 18 % of a core).
2. **The cost per packet is that of voice.** About 10 to 15 µs per packet received or sent on the Mac in the larger rooms (the smallest ones are too short to measure precisely); ADR-0030 measured 12 µs for Opus. Encrypting a 1 200-byte packet instead of 160 bytes barely shows: the system call per packet dominates, as for voice. The owner's server was 1.8 times slower per packet for voice, so the same rooms should cost it roughly twice the Mac's CPU, **to be measured there**.
3. **Bandwidth runs out before CPU.** The 50 participants of the instance cap watching 10 cameras in speaker view take a third of the Mac's core but 152 Mbit/s of upload; a grid of medium tiles in a room of 25 takes 125 Mbit/s. A home connection with a 100 Mbit/s upload cannot carry either, hence the instance's upload budget and the per-watcher allocation.
4. **The 9 watched cameras bound the cost, as the 16 voices did.** 25 cameras watched 9 at a time cost the server 23 to 26 % of a core and 64 Mbit/s out; what grows with the number of cameras is only what the server receives (57 Mbit/s for 25 cameras sending all their layers), which pausing unwatched layers removes.
5. **Keyframes on demand are cheap and quick.** A layer switch or a new watcher waits one or two frames plus a round trip, and the keyframe requests stay at about one per second per room thanks to the throttle.
6. **Bandwidth estimation costs about 60 % more CPU** in the room measured (18 % to 29 % of a core); it stays well within budget and is what makes slow watchers possible at all.

**Not measured yet**, and to be done before the ADR is accepted or during stage 16.5:
- **The owner's server (Intel i7-8700T)**, as ADR-0030 required: the same series, run on the host itself (see below). This ADR does not claim any figure for it.
- A watcher on a slow link: the Mac cannot shape traffic without administrator rights. Stage 16.5 tests the allocation with bots that drop part of what they receive.
- Firefox and Safari, and real cameras (synthetic frames have no motion-dependent sizes; real keyframes vary more).

**To rerun on the owner's server** (as for the voice spike, the host firewall drops LAN traffic to unopened ports, so SFU and bots run on the host, for example in a throwaway container):
```bash
docker run --rm -it --network host -v "$PWD/spikes/video-sfu":/spike -w /spike rust:1 sh
sysctl -w net.core.rmem_max=4194304 net.core.wmem_max=4194304   # on the host, for the run only
cargo build --release
./run-series.sh 2:2 10:5 25:10
LAYERS=h:1500,l:500,q:150 FOCUS=h OTHER=q ./run-series.sh 10:5 25:10 50:10
LAYERS=h:1500,l:500,q:150 FOCUS=l OTHER=l ./run-series.sh 10:5 25:10
LAYERS=h:1500,l:500,q:150 FOCUS=h OTHER=q SLOTS=9 ./run-series.sh 25:25
LAYERS=h:1500,l:500,q:150 FOCUS=h OTHER=q BWE=3000 ./run-series.sh 25:10
```
The bots compete with the SFU for the CPU there, as in ADR-0030, so the largest rooms will be pessimistic.

## Consequences
- Cameras need nothing new from self-hosters: the same UDP port, no new service, no new certificate, no state on disk. Backups and updates are unchanged.
- The host's **upload bandwidth** becomes the first limit of an instance, before its CPU. `[voice] video_upload_mbps` is the one new setting; the README and `init` explain it next to the UDP port.
- The media loop gains per-participant video state: receive slots for video, a layer per watcher and camera, keyframe throttles, bandwidth allocation and the per-participant packet budget. It stays on one thread; the next step when real use approaches the ceiling is unchanged from ADR-0030 (several media threads, Linux batching first if profiling says so), measured before and after.
- The UDP socket's kernel buffers are raised (`SO_RCVBUF` and `SO_SNDBUF` to a few MiB) through `socket2`, already in the dependency tree through Tokio: keyframes arrive as bursts of dozens of packets per camera, which Linux's default 208 KiB buffer drops.
- The server can see the video it relays, as it can hear the voice (ADR-0030). End-to-end encryption remains a later, separate decision; it would not change the SFU, which never reads the payload, provided the VP8 header that marks keyframes is left unencrypted (or keyframes are signalled in a header extension).
- `Permissions` gains bit 21. Screen sharing (16.6) will reuse it, and will need its own ADR for its layers (text needs a high resolution at a low frame rate).
- **Known limits.** One codec only: a future client without VP8 could not send or watch. Cameras that nobody watches still upload their lowest layer. The 25-camera instance cap is first come, first served, with no per-guild share, like the 50 participants.

## Alternatives considered
- **An external SFU (LiveKit, mediasoup, Janus, Jitsi):** they do all of the above and more (SVC, dynacast, recording), but each is a second program to install, update and secure, against the one-binary promise; the owner excluded them in ADR-0030. The spike shows the built-in SFU needs no more than they offer for Discord-like video: `str0m` provides simulcast, keyframe requests, retransmissions and bandwidth estimation, and the rest is the allocation policy, which Mellow would have to configure in any SFU.
- **Peer-to-peer for small calls (two or three people):** saves the server's bandwidth, but reveals participants' IP addresses to each other, needs TURN on many networks, and would be a second media path to build and test, switched on and off as people come and go. Rejected, as for voice.
- **One layer per camera, without simulcast:** simplest, but measured at five times the bandwidth and four times the packets of simulcast in a room of 25 with 10 cameras, and every watcher would get the sender's quality whatever its link. A slow watcher would force the camera down for everyone (through its keyframe requests and REMB) or be overrun.
- **Transcoding on the server** (decode, scale, encode per watcher): one stream per watcher at exactly the right quality, but a realtime VP8 encode at 720p costs several milliseconds of CPU per frame, against microseconds per packet for forwarding: one encode per watcher and camera would saturate the owner's server with a handful of cameras, and add a frame or more of latency.
- **SVC with VP9 or AV1 instead of simulcast:** one encode per camera instead of three and 30 to 50 % less bandwidth, but heavier encoders, no AV1 encoding in Safari, and layer selection inside the codec's own payload. Kept as the first upgrade if bandwidth becomes the problem.
- **Renegotiating a receive m-line per camera** as cameras come and go: the classic design, rejected for voice for its cost in large rooms; 9 slots assigned by the server keep cameras turning on and off free of SDP.

## Plan after this ADR
Like stages 16.2 to 16.4, each step below is reviewed before the next, and the stage ends with a release the owner tries:
1. **The owner decides** on this ADR (see the open questions below) and runs, or lets an agent run, the spike series on the i7-8700T; the numbers are added here, and the limits are lowered if that server does not hold them.
2. **Protocol and server**, in the main thread: `STREAM` and migration 0033 with the domain function and its tests, the protocol additions with wire samples, video slots and layer selection in `voice::media`, keyframe requests, bandwidth allocation and the instance budget, the per-participant packet budget, socket buffers. **End-to-end tests with `str0m` bots** through the real gateway, as in `voice/tests.rs`: a camera seen by a watcher, a layer switch at a keyframe, a lost permission stopping the forwarding, the 9-slot and 25-camera caps, a slow watcher (dropping packets) moved down a layer and then paused.
3. **Web interface by `ux-ui-designer`** against DESIGN.md: a camera button in "Voix connectée", the camera choice and a preview in the Voix settings, the grid and the speaker view, a participant's tile with their speaking ring, the paused tile, the states (camera starting, refused, permission lost, too many cameras), keyboard and screen reader access.
4. **`code-reviewer` and `security-auditor` in parallel** (the gateway, the media loop and permissions are touched).
5. **A real two-browser check** (two Chromium contexts through a release build, as for voice, plus Firefox if possible).
6. **A release** the owner tries on their instance, then stage 16.6.

## Open questions for the owner
- **The upload budget's default.** 100 Mbit/s suits most fibre connections; a VDSL or 4G host needs far less. What is the upload of the owner's connection, and should the default be lower, or derived by `init` from a question?
- **The default view.** Grid by default as on Discord, or the speaker view?
- **Cameras for `@everyone`.** Migration 0033 grants `STREAM` to every existing `@everyone`, as Discord's default does. Keep it, or let guild owners grant it?
- **Pausing unwatched layers** (`VOICE_VIDEO_LAYERS`) in stage 16.5, or as a follow-up once cameras work?
