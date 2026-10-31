# 0030. Relay voice through an SFU built into the server binary

- Status: Accepted (by the owner, 2026-10-31)
- Date: 2026-10-31

## Context
The owner wants voice channels (ROADMAP item 16), with an SFU inside the server binary, WebRTC and no external service. The product promise still holds (ADR-0002): one binary, one data directory, no Docker required, nothing else to install or run. Stage 16.1 decides how, and measures it, before any feature code. The later stages build on this decision: audio only (16.2), comfort features (16.3), a soundboard (16.4), video (16.5) and screen sharing (16.6).

The forces are these:
- **Browsers only speak WebRTC.** The web client is the only client being built (ADR-0023). Media therefore travels as SRTP over DTLS and ICE, with Opus for audio. The server must implement that stack.
- **Self-hosters are not network engineers.** Many instances, the owner's included, run at home behind a box. Each extra port, certificate or service is a support burden.
- **Small machines.** The owner's server is an Intel i7-8700T (6 cores, 12 threads, low-power), already hosting other services. Voice must not starve the HTTP server, the gateway or the SQLite writer (ADR-0024).
- **Permissions stay in one place** (ADR-0007): who may join and who may speak is decided by the shared pure function, never ad hoc.
- **Privacy.** Logs never contain content (CLAUDE.md), and voice is content.

## Decision

### Library: `str0m`
The SFU uses [`str0m`](https://github.com/algesten/str0m) 0.24 (MIT or Apache-2.0), a WebRTC library in "sans-IO" style:
- it owns no sockets, threads or runtime: Mellow feeds it UDP packets and time, and sends what it returns. One socket can serve every participant, and the media loop stays entirely under Mellow's control;
- its crypto backend is `aws-lc-rs`, already in the dependency tree for TLS and attachment encryption (ADR-0020). It is used with `default-features = false, features = ["aws-lc-rs"]`, which leaves out the example web server;
- it supports what stages 16.2 to 16.5 need: ICE-lite, ICE over TCP candidates, Opus, RTP header extensions (audio level), RTCP feedback, simulcast and renegotiation. Its repository ships a working multi-party SFU example, which the spike follows.

### Topology: a selective forwarding unit
The server forwards each speaker's Opus packets to the other participants of the channel. It never decodes, mixes or transcodes audio, so CPU use stays low and latency is a fraction of a millisecond. Peer-to-peer is not used: it would reveal every participant's IP address to the others, and would not hold beyond a few people.

### Network: one UDP port, ICE-lite, no TURN
- **One UDP port** for all voice traffic of the instance, `[voice] udp_port` in `mellow.toml`, default **50000**. Participants are told apart by their address and ICE credentials (`Rtc::accepts`, then a map from source address to participant).
- **ICE-lite on the server**, with a single host candidate: the instance's public address. It is given as `[voice] public_address`, or derived from the instance's domain when not set. The socket may listen on `0.0.0.0` behind Docker or NAT while announcing the public address.
- **Why this traverses NATs.** Clients always open the connection themselves (the browser is the controlling ICE agent), so their own box or carrier NAT is not a problem, exactly as with HTTPS. The only requirement is the one an instance already meets for TCP 443: the server's UDP port must be reachable. Self-hosters at home forward one UDP port on their box, and open it in the host firewall: during the spike, the owner's server dropped LAN traffic to unopened ports. `init`, the README and the Docker kit (`50000:50000/udp`) will say so. Voice setup checks are planned for stage 16.2: the admin page shows whether the UDP port answers.
- **No built-in TURN server.** TURN only helps clients on networks that block outgoing UDP (some workplaces, some public Wi-Fi). It would be a second service to secure, with credentials and relay abuse to handle. If users meet such networks, the next step is **ICE over TCP** (RFC 6544) on a TCP port of the server, which `str0m` supports. It would get its own ADR, written once a real need is observed.

### Signalling over the existing gateway
Joining, leaving, the SDP offer and answer, and who is in which channel all travel over the authenticated WebSocket gateway (ADR-0004). There is no new HTTP endpoint and no separate token. The exact frames are designed in stage 16.2. The principle: a client asks to join a channel with its SDP offer; the server checks permissions and limits, creates the participant, and answers with its SDP; voice state events reach the members who can see the channel (ADR-0015).

Each client offers **one send m-line for its voice and 16 receive m-lines**. The server assigns those receive slots to whoever is speaking, so audio needs **no renegotiation** while people come and go. Video (16.5) will need renegotiation, over the same gateway.

### Audio
- **Opus only**, 48 kHz, mono, `useinbandfec=1` (packet-loss concealment) and `usedtx=1` (a silent participant sends almost nothing). Default target bitrate is 64 kbit/s, Discord's default. The server drops packets far larger than an Opus frame.
- **At most 16 voices heard at once** per listener. When more than 16 people talk at the same time, the server keeps the 16 loudest, using the audio level header extension (RFC 6464) that browsers send with each packet. The same extension drives the speaking indicators, so the server never decodes audio. This bound is the main finding of the spike (see below): it keeps the cost per participant flat.

### Encryption
- Media are encrypted on every hop with **DTLS-SRTP** (AES-GCM through `aws-lc-rs`), as WebRTC requires. The DTLS certificate's fingerprint travels in the SDP over the TLS-protected gateway, so nobody can sit between the client and the server unnoticed.
- **The server can hear the audio**: it decrypts each packet and re-encrypts it for each listener, as every SFU without end-to-end encryption does (Discord's servers could too, before its DAVE protocol in 2024). Mellow never stores, records or logs audio; packets live in memory for less than a millisecond. The instance's administrator already has full access to its messages (ADR-0006 protects data at rest, not against the operator), so this changes nothing in the trust model.
- **End-to-end encryption** (SFrame with WebRTC encoded transforms, as in Discord's DAVE protocol) is possible later without changing the SFU, because the SFU never reads the payload. It needs its own ADR, covering key exchange, group membership changes and browser support.
- **Participants' IP addresses** are known only to the server, never to other participants.

### Permissions
Two new bits in `Permissions` (ADR-0007), both **channel-scoped** (`CHANNEL_SCOPED`) and both in `DEFAULT_EVERYONE`:
- `CONNECT = 1 << 18`: join a voice channel. `VIEW_CHANNEL` is also required.
- `SPEAK = 1 << 19`: be heard. Without it, the server does not forward the member's packets. The client's own mute is a convenience, not the enforcement.

A member who loses `VIEW_CHANNEL` or `CONNECT` while connected is disconnected by the server. A member who loses `SPEAK` stops being forwarded, at the next permission snapshot (ADR-0024). Moderation bits (`MUTE_MEMBERS`, `DEAFEN_MEMBERS`, `MOVE_MEMBERS`) come with the stage that needs them, not before.

### Limits
- **Per channel**: an optional user limit set with `MANAGE_CHANNELS` (1 to 25). The **hard cap is 25 participants per channel**.
- **Per instance**: `[voice] max_participants` in `mellow.toml`, default **50** connected people across all voice channels.
- **Per user**: one voice connection at a time. Joining another channel, or from another device, moves the connection. Join attempts are rate-limited like other gateway actions.

The measurements below justify these numbers, for one media thread on a server like the owner's. They can rise later, once the media loop is spread over several threads (see Consequences).

### Process layout
The media loop runs on **one dedicated OS thread** inside `mellow-server`, outside the Tokio runtime: it blocks on the UDP socket with a timeout, as in the spike. It talks to the gateway through channels: joins, leaves, permission changes in; voice states and speaking events out. It never touches SQLite. The rule for modules stays `gateway → domain`: the new `voice` module sits beside `gateway`, and permission decisions come from `domain`.

## Measurements
The spike in [`spikes/voice-sfu/`](../../spikes/voice-sfu/) is outside the workspace and adds no dependency to the product. It has three parts:
- `sfu`, a one-room `str0m` SFU built like the design above: ICE-lite, one UDP port, receive slots, Opus forwarded as is;
- `load`, a generator of headless `str0m` participants that send one 20 ms frame of 160 bytes (64 kbit/s) each, and measure the one-way latency of every frame they hear (all bots share one clock);
- a web page for real browsers.

The SFU reports its own CPU time (`getrusage`) and how long each frame spends inside it, from its UDP read to the end of the last listener's send. Each row is a fresh SFU and 20 s of measurement after a 3 s warm-up. "Speaking" means sending without pause, the worst case: real conversations, with DTX, cost far less.

**Apple M3 laptop** (SFU and bots on the same machine, LAN address):

| Participants | Speaking | Receive slots | SFU CPU (one core) | Forwarded packets/s | Inside the SFU p50 / p99 | One-way latency p50 / p99 | Loss |
|---|---|---|---|---|---|---|---|
| 2 | 2 | 1 | 1 % | 100 | 0.02 / 0.2 ms | 2.6 / 5.3 ms | 0 % |
| 5 | 5 | 4 | 12 % | 1 000 | 0.3 / 1.9 ms | 2.9 / 6.8 ms | 0 % |
| 10 | 10 | 9 | 22 % | 4 500 | 0.25 / 2.0 ms | 4.0 / 10.5 ms | 0 % |
| 25 | 25 | 24 | 38 % | 30 000 | 0.29 / 0.37 ms | 4.9 / 9.8 ms | 0 % |
| 50 | 50 | 49 | saturated | 83 000 | 150 / 610 ms | 1 600 / 2 150 ms | 33 % |
| 50 | 5 | 49 | 27 % | 12 250 | 0.9 / 1.2 ms | 3.6 / 7.1 ms | 0 % |
| 100 | 5 | 99 | saturated | 20 000 | 70 / 390 ms | 2 150 / 2 740 ms | 23 % |
| 100 | 5 | 10 | 29 % | 24 750 | 1.0 / 1.3 ms | 4.0 / 7.8 ms | 0 % |
| 50 | 16 | 16 | 41 % | 39 200 | 0.48 / 0.56 ms | 4.9 / 9.3 ms | 0 % |
| 100 | 5 | 16 | 33 % | 24 750 | 1.1 / 1.4 ms | 4.2 / 7.7 ms | 0 % |
| 100 | 16 | 16 | 87 % | 79 200 | 1.0 / 1.3 ms | 9.3 / 17.7 ms | 0 % |

**The owner's server**, Intel i7-8700T. The SFU ran in a throwaway container on the host network, and the bots ran on the same machine over loopback: the host firewall drops LAN traffic to the spike's ports, and opening them was not worth a change to the server. The bots compete with the SFU for the CPU, so latency and the largest rooms are pessimistic.

| Participants | Speaking | Receive slots | SFU CPU (one core) | Forwarded packets/s | Inside the SFU p50 / p99 | One-way latency p50 / p99 | Loss |
|---|---|---|---|---|---|---|---|
| 2 | 2 | 1 | 2.6 % | 100 | 0.06 / 0.16 ms | 12 / 24 ms | 0 % |
| 5 | 5 | 4 | 7 % | 1 000 | 0.16 / 0.29 ms | 8.6 / 20 ms | 0 % |
| 10 | 10 | 9 | 26 % | 4 500 | 0.43 / 0.65 ms | 8.4 / 20 ms | 0 % |
| 25 | 25 | 24 | 66 % | 30 000 | 0.38 / 1.5 ms | 12 / 42 ms | 0 % |
| 50 | 16 | 16 | 75 % | 39 100 | 0.66 / 2.7 ms | 15 / 301 ms | 0.3 % |
| 100 | 5 | 16 | 59 to 68 % | 24 500 | 0.5 / 5.7 ms | 14 / 295 ms | 0.7 % |
| 100 | 16 | 16 | saturated | 53 000 | 234 / 472 ms | 318 / 513 ms | 32 % |

**Chrome** (two tabs on the Mac, joined with a test tone through the spike page): connected to the `str0m` SFU at the first try, heard each other, 0 packets lost out of about 390, 2 ms jitter, 1 ms round trip to the SFU, and a jitter buffer of 22 to 39 ms. The browser's jitter buffer, not the SFU, dominates the latency people hear.

What the figures say:
1. **The SFU adds well under a millisecond** while it is not saturated. The latency people hear comes from the network and the browser's jitter buffer.
2. **The cost is per forwarded packet.** The Mac spent about 12 µs per forwarded packet and the server about 22 µs. Most of it is one `sendto` system call and one SRTP encryption per packet and per listener. One media thread on the owner's server handles about 40 000 forwarded packets per second before latency suffers: 25 people all talking non-stop, or 50 people with 16 talking.
3. **Receive slots must be bounded.** With one receive m-line per other participant, 100 people with only 5 talking saturated the Mac: each `Rtc` handles timers for every stream, so the work grew with the square of the room even when silent. With 10 or 16 slots, the same room took about 30 % of a core. Hence the cap of 16 voices heard at once.
4. **The limits follow.** Under the caps (25 per channel, 50 per instance, 16 voices heard), the worst case on one thread is 50 listeners each receiving 16 voices: about 40 000 packets/s, which the owner's server sustains. Real conversations, with DTX and one or two people talking at a time, cost a few percent of a core.

## Consequences
- Voice needs **one UDP port** reachable from the internet. That is the only new step for self-hosters: a port forward on the box and a firewall rule. Docker users publish it in the compose file. Instances that cannot open a UDP port will not have voice until ICE over TCP exists.
- No new service, no TURN, no new certificate (DTLS uses a self-signed certificate whose fingerprint travels over the authenticated gateway). Backups and updates are unchanged: voice keeps no state on disk.
- The server can hear what it relays. Mellow says so plainly in its documentation, and end-to-end encryption remains a later, separate decision.
- **One media thread is the ceiling** of the first stages: about 40 000 forwarded packets per second on the owner's server. When real use approaches it, the next step is several media threads with each channel pinned to one, sharing the socket through a dispatcher. Linux batching (`sendmmsg`, UDP GSO) comes first if profiling shows the system calls dominate. Either change is measured before and after.
- `str0m` becomes a core dependency. It must pass `cargo deny` when it enters the workspace in stage 16.2. Its version is pinned, and upgrades follow its changelog, as for `sqlx`.
- `Permissions` gains bits 18 and 19. Older clients keep unknown bits (they already do), and the server's boundary check accepts them once they are defined.

## Alternatives considered
- **`webrtc-rs` (`webrtc` crate):** a port of Go's Pion, with its own async tasks per peer connection and a large API mirroring the browser's. One socket for all participants and control over the media loop are harder to get, and it brings many more dependencies. Its authors are moving to a sans-IO design (`rtc`), which is not mature yet.
- **An external SFU** (LiveKit, mediasoup, Janus, Jitsi): mature and full-featured, but a second program to install, update and secure. That breaks the one-binary promise (ADR-0002), and the owner excluded it.
- **Peer-to-peer mesh:** no server load, but each participant uploads one copy of their voice per listener. It reveals IP addresses to other participants and needs TURN in many networks.
- **An MCU (decode, mix, encode on the server):** one stream per listener, but Opus decoding and encoding for every channel costs one to two orders of magnitude more CPU, and adds latency.
- **A built-in TURN server:** helps only networks that block UDP. It is a second protocol to secure, and an open relay risk. Deferred in favour of ICE over TCP if the need appears.
- **One receive m-line per participant, renegotiated as people join:** the classic design. The spike measured its cost in large rooms. Bounded slots chosen by audio level are cheaper, and they match what people can follow anyway.

## Implementation status
Stage 16.2 (server and protocol, branch `feat/voice-channels-server`, 2026-10-31) implements the decision as written, with these specifics:
- **Address announced.** The single host candidate is `[voice] public_address` when set, otherwise the host of the gateway's `Host` header (port removed, checked for length and characters, resolved to IPv4 with Tokio's resolver at each join, since home addresses change). Behind Caddy the browser's `Host` reaches Mellow unchanged, so a box forwarding UDP 50000 needs no configuration. The socket listens on IPv4 only (`0.0.0.0`): a name resolving only to IPv6 gets `voice_unavailable`. The "derived from the instance's domain" of the decision became this Host rule, which also covers instances behind a reverse proxy without `[tls]`.
- **Opus parameters.** `str0m` lets the server write its own Opus format parameters, so the answer carries `useinbandfec=1` and `usedtx=1` (`minptime=10`, stereo flag as browsers send it). No bitrate is imposed: browsers default to about 32 kbit/s for mono speech rather than the 64 kbit/s of the decision; a `maxaveragebitrate` can be added in 16.3 if listening tests ask for it.
- **"16 loudest voices" approximated.** A receive slot keeps its speaker while they talk; once all 16 are taken, a new speaker takes the slot of the one silent the longest, if silent for over 300 ms. No ranking by audio level happens between people talking at the same time: with DTX, people who stop talking free their slot within 300 ms, which is what matters in practice. When a slot changes speaker, the listener's stream continues on the same SSRC with the new speaker's timestamps.
- **Speaking.** From the RFC 6464 audio level: louder than -50 dBov starts it, 300 ms without a voiced packet stops it; only edges are sent, never for muted or suppressed people. A packet without the extension counts as voiced.
- **Permission changes.** Every path that can change who views, connects or speaks (role and overwrite edits, member roles, kicks, bans, leaves, channel and guild deletion) notes the guild in the event hub; the voice service then re-checks its participants there through `Guilds::voice_access` (`domain`), disconnecting those who lost `VIEW_CHANNEL` or `CONNECT` and toggling `suppressed` for `SPEAK`. Lowering a channel's `user_limit` disconnects nobody, as on Discord.
- **Moves.** A move is one `VOICE_STATE_UPDATE` with the new channel; viewers of the old channel who cannot view the new one get a `VOICE_STATE_DELETE` instead. The same session joining its own channel again (an ICE restart after a network change) replaces the media session and announces nothing.
- **Not done in 16.2.** The admin-page check that the UDP port answers (planned above for 16.2) waits for the admin UI; `init` does not print the port reminder yet (the README and the Docker kit do). Members who gain `VIEW_CHANNEL` on a voice channel while people are in it learn who is there only at their next `Ready`. Moderation bits (`MUTE_MEMBERS`, `DEAFEN_MEMBERS`, `MOVE_MEMBERS`) are not defined yet, as decided.

