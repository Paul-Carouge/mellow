# Voice SFU spike (ADR-0030)

Throwaway code that measured the voice design of [ADR-0030](../../docs/adr/0030-voice-channels-with-an-sfu-in-the-server-binary.md) before any feature code. It has its own Cargo workspace, so it adds nothing to Mellow's dependencies, builds or CI. Do not build on it: stage 16.2 writes the real `voice` module in `mellow-server`.

- `sfu`: one room, audio only, `str0m` with ICE-lite on one UDP port, 1 send and K receive m-lines per participant (no renegotiation), Opus forwarded as is. Prints its CPU use and the time a frame spends inside it every 5 s. Serves `web/index.html` for browser tests.
- `load`: N headless participants, of which `--speakers` send a 20 ms, 160-byte frame (64 kbit/s) without pause. Each frame carries its send time, so the bots, which share one clock, measure the true one-way latency of every frame they hear.

```bash
cargo build --release
./run-series.sh                      # fresh local SFU per room, rooms as participants:speakers
SLOTS=16 ./run-series.sh 100:5 50:16 # with 16 receive slots per participant
./target/release/sfu --ip 192.0.2.10                # then open http://192.0.2.10:3000 in several tabs
./target/release/load --server http://192.0.2.10:3000 --bots 25 --speakers 25 --seconds 20
```

The SFU binds the UDP port 50000 and the HTTP port 3000 on the given address (by default the one that routes to the internet). It is not hardened: run it on a trusted network only.
