# 0052. H.264 next to VP8 in the voice SFU

- Status: Accepted
- Date: 2026-11-06
- Amends: [ADR-0035](0035-video-in-voice-channels.md) (its codec section, "VP8 only, for now") and [ADR-0037](0037-screen-sharing-on-the-camera-path.md) (offers with "VP8 and Opus only")
- Builds on: [ADR-0051](0051-gpui-desktop-client.md), which asks for this ADR before shares use Vesper's GPU encoder

## Context
The SFU answers and forwards VP8 only (ADR-0035): one codec for a whole channel, so it never has to choose. The desktop client (ADR-0051) captures screens on the GPU and encodes them there (Vesper's VAAPI path on Linux, VideoToolbox on macOS, Media Foundation on Windows). No common GPU encodes VP8; every one of them encodes H.264. Without H.264 in the SFU, a native share must be read back to the CPU and encoded by libvpx: one to two cores for a 1440p screen with its three layers, more latency, and battery on laptops.

Facts this decision relies on (checked in `str0m` 0.24.1, the version in `Cargo.lock`):
- `str0m` packetises and depacketises H.264 in packetisation mode 1 (STAP-A, FU-A) and hands frames in Annex B form. `MediaData::is_keyframe` is true for an access unit holding an IDR slice.
- Its FU-A reassembly (`H264Depacketizer`) keeps every fragment until one carries the end bit, with no size limit and across frames, so a sender that never sets it grows the server's memory without bound. Upstream `main` does the same.
- Its payload matching (`PayloadParams::match_score`) requires the same packetisation mode and the same profile, and tolerates any level. `Writer::match_params` matches against the session's codec configuration, not the m-line's: forwarding must itself make sure a watcher's line negotiated the codec.
- Constrained Baseline has no B-frames, so frames arrive in decoding order with increasing timestamps, which an SFU that forwards frames as they come needs. Every browser Mellow supports decodes it (Chromium, Safari, Firefox through its OpenH264 plugin); some Chromium builds without proprietary codecs, and a Firefox that has not downloaded the plugin yet, do not.

## Decision
### What the SFU accepts
- The server's answer offers **H.264 Constrained Baseline (`profile-level-id` `42e01f`), packetisation mode 1, with RTX**, next to VP8 and its RTX. No other profile, no mode 0, no VP9 or AV1. The level is not negotiated down; the keyframe checks below bound the picture instead.
- **A sender's codec is fixed for its media session.** It is VP8 whenever the camera line negotiated VP8 (every client decodes it), otherwise H.264. A client that wants to send H.264 offers only H.264 on its camera line. Frames in another codec than the session's are dropped.
- H.264 takes the same path as VP8: the same simulcast RIDs `h`, `l`, `q`, the same per-layer byte budgets, keyframe requests per layer, layer switches at keyframes, silent cameras and lost layers, packet budgets.
- **Keyframe size checks** are the H.264 equivalent of VP8's (`domain::h264_keyframes`): an IDR access unit must carry a sequence parameter set that is Constrained Baseline (`profile_idc` 66 with `constraint_set1_flag`) and whose cropped picture fits the layer (`q` 640 × 360, `l` 1280 × 720, `h` 2560 × 1440). Anything else reaches no one. The server reads a few dozen bits of the SPS (exp-Golomb, emulation prevention removed) and never decodes a picture.
- **`str0m` is patched** (`vendor/str0m`, through `[patch.crates-io]`): a start fragment begins a new FU-A unit, fragments without one are dropped, any other packet ends an unfinished unit, and a unit over 2 MiB is refused with a packet error. The SRTP payload is encrypted, so the server cannot bound this before `str0m`. The rest of the crate is 0.24.1 as published, without its examples and integration tests; the patch goes once an upstream release bounds the buffer.

### Who receives what
- **Receivers say what they decode in their offer**: the codecs their video receive m-lines negotiate. The server counts a codec as decoded when every receive line carries it, so any free slot can take a picture in it.
- **The sender's codec is announced in its voice state**: `VoiceState.video_codec` (`"vp8"` or `"h264"`), an additive field, absent when the offer has no video lines and from older servers (meaning VP8). Released v1 clients ignore it. Rust clients read a codec from a newer server as `unknown` (ADR-0044's convention), never as a broken voice state.
- **The server never forwards a picture its watcher cannot decode.** A camera in a codec the watcher's lines do not carry gets no receive slot (no `VOICE_SLOT_ASSIGNED`, no frames, not counted in the watcher's allocation or in the camera's watched layers). When a camera reconnects, its watchers' slots are given again, so a codec change takes or gives slots back at once.
- The clear reason is the voice state: a client seeing `video_codec` it does not decode shows that it cannot play this picture, instead of waiting for it. No new event is needed.

### The web client
- It keeps **sending VP8** (the browser's encoder, simulcast checked in ADR-0035).
- Its video receive slots offer **VP8, then H.264 Constrained Baseline in packetisation mode 1 where `RTCRtpReceiver.getCapabilities("video")` lists it, then RTX**. A browser without H.264 offers VP8 alone, as before.
- A tile or a watched screen whose sender's `video_codec` the browser does not decode says so ("Image non prise en charge par ce navigateur"; on a watched screen, that it can be watched from another browser).

### Senders (the desktop client)
- Constrained Baseline, packetisation mode 1, no B-frames, SPS and PPS before every IDR, an IDR on each keyframe request (per RID), the three layers at ADR-0037's sizes and rates, and only H.264 offered on the camera line.

## Consequences
- Native shares can stay on the GPU from capture to network; the server's cost per forwarded packet does not change (it reads an SPS only on keyframes; not measured separately).
- A watcher whose browser cannot decode H.264 sees a sentence instead of a native sharer's picture. It is rare (see above) and visible; the fix is on the sender's side (send VP8) or the watcher's (another browser), never transcoding on the server (ADR-0035 rejected it for its cost).
- Browsers that only received VP8 now also decode H.264 bitstreams written by other members, so their H.264 decoders become reachable from a channel. That is the usual cost of WebRTC; release notes say so.
- **Offer size.** H.264 on the 9 receive slots adds one codec with its RTX to each, a few hundred bytes per line: estimated 3 to 6 kB more for Chromium, inside the 48 KiB cap (`MAXIMUM_VOICE_OFFER_BYTES`). To be measured in a real browser when the web change ships.
- One codec per sender, not per channel: two watchers of one camera may need different codecs only if the sender could send both, which this decision forbids (VP8 wins when both are negotiated). Simulcast in two codecs at once would double the sender's uplink; not done.
- ADR-0035's codec section ("VP8 only") and ADR-0037's "VP8 and Opus only" are amended: VP8 stays the default and what browsers send.

## Alternatives
- **H.264 for the whole channel** (every client sends H.264): browsers' H.264 simulcast is uneven, some Linux browsers cannot decode it, and nothing is gained for browsers that already have hardware encoders behind WebRTC.
- **A refusal event to watchers** (`VOICE_VIDEO_UNDECODABLE`): one more event and a race with the watch list, when the voice state can say it before anyone asks.
- **Refusing H.264 senders while a watcher cannot decode it**: one old browser in a channel would silently push every native sharer back to CPU encoding.
- **Transcoding on the server**: rejected by ADR-0035 (several milliseconds of CPU per frame and watcher).
- **AV1 or HEVC**: Safari decodes AV1 only on recent hardware, Firefox has no HEVC in WebRTC; later, with their own ADR.
