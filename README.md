# Mellow

A calm, self-hostable alternative to Discord, written in Rust.

- **Self-hosting in minutes:** one server binary, one data directory, no Docker, no external database, one-command updates.
- **Feels like Discord:** same layout and habits (servers, channels, DMs, roles, attachments), with Mellow's own brand.
- **Clients:** a web client served by the instance itself (Svelte), installable as an app from Chrome. A native desktop app in Rust with GPUI is being built at parity with it ([ADR-0051](docs/adr/0051-gpui-desktop-client.md)); a mobile client comes later.
- **Private by default:** TLS everywhere, data encrypted at rest, no telemetry.

> Status: the web client is the focus. Servers, channels, roles and permissions, moderation, direct messages and groups, friends, reactions, search, custom emoji, notifications and voice channels work end to end in it. See [ROADMAP.md](ROADMAP.md).

## Documentation
- [ARCHITECTURE.md](ARCHITECTURE.md) — how the system is built.
- [DESIGN.md](DESIGN.md) — visual identity and UX.
- [docs/adr/](docs/adr/) — architecture decision records.
- [CLAUDE.md](CLAUDE.md) — instructions for AI-assisted development.

## Development
Prerequisites: [rustup](https://rustup.rs) (the pinned toolchain in `rust-toolchain.toml` installs itself), a C compiler, `nasm` on x86-64 (the server's AVIF encoder assembles its fast paths with it), `cmake` (it builds libopus, which encodes audio attachments: `brew install cmake` on macOS, `apt install cmake` on Debian and Ubuntu), the ALSA headers on Linux for the desktop voice engine's audio devices (`apt install libasound2-dev` on Debian and Ubuntu), `make`, `curl` and `tar` for the desktop's video engine, which downloads libvpx 1.17.0 (checked against its SHA-256) and builds it statically (`nasm` or `yasm` on x86-64; for an offline or packaged build, point `MELLOW_LIBVPX_SOURCE_DIR` at an unpacked copy of the release), and to build the vendored OpenSSL used by SQLCipher: `perl` and `make` on Linux, Visual Studio Build Tools (`nmake`) and Strawberry Perl on Windows.

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo deny check
cargo xtask tokens --check
```
Web client ([`web/`](web/), Svelte 5 + Tailwind 4, Node 24 and npm): `cd web && npm ci`, then `npm run check`, `npm run test`, `npm run build`. `npm run dev` serves it on `http://127.0.0.1:5173` and proxies the API and the gateway to a local instance on `127.0.0.1:18080` (`MELLOW_DEV_INSTANCE` to change it). `cargo build -p mellow-server --features web-client` embeds the built client (`web/dist`) in the server, which then serves it at `/`; release builds always do.

The certificate test against [Pebble](https://github.com/letsencrypt/pebble), the ACME test authority, is ignored by default: `MELLOW_PEBBLE_BINARY=/path/to/pebble cargo test -p mellow-server --lib tls::tests -- --ignored`.

## Releases
Each tag `vX.Y.Z` publishes, on the [releases page](https://github.com/Paul-Carouge/mellow/releases), `mellow-server` for `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`. Each binary, and the `mellow-X.Y.Z-SHA256SUMS` digest list, has a `.minisig` signature whose trusted comment is the signed file's own name. Linux builds need glibc 2.35 or newer (Debian 12, Ubuntu 22.04). Check a file by hand with the public key in [`release/minisign.pub`](release/minisign.pub), then make sure the printed `Trusted comment:` is exactly that file's name (otherwise the signature belongs to another file):
```bash
minisign -V -p release/minisign.pub -m mellow-server-0.22.0-x86_64-unknown-linux-gnu
```
Updating an instance is one command: `sudo mellow-server update` installs the latest release in place of the running executable after checking its signature (the replaced one is kept as `mellow-server.previous`), then restart the service; `--check` only compares versions and `--rollback` puts the previous executable back, not the database ([ADR-0018](docs/adr/0018-roll-back-the-executable-only-and-guard-the-install-location.md)). Keep the executable in a root-owned directory such as `/usr/local/bin` (`root:root`, mode 0755) and run the service as a dedicated account: the updater refuses an executable or directory that another account could change. To go back to a version older than a migration, stop the server, set `mellow.db`, `mellow.db-wal` and `mellow.db-shm` aside, and copy `backups/pre-migration-<version>.db` to `mellow.db`. With Docker Compose, use [`deploy/docker`](deploy/docker/README.md) and its `mellow-update` instead.

To publish a release (maintainer): bump `version` in `Cargo.toml`, move the `CHANGELOG.md` entries under the new version, merge, then `git tag vX.Y.Z origin/main && git push origin vX.Y.Z` and approve the `release` environment in the workflow run ([ADR-0016](docs/adr/0016-sign-releases-in-a-protected-ci-environment.md)). Before approving, check that the run's commit is the tip of `main` you meant to release and that `release.yml` did not change unexpectedly: the approved job receives the signing key.

## Run a development instance
```bash
cargo run -p mellow-server -- init --config dev/mellow.toml --data-directory dev/data --master-key-file dev/keys/master.key
cargo run -p mellow-server -- admin create-user --config dev/mellow.toml --username admin --email admin@example.org --administrator
cargo run -p mellow-server -- serve --config dev/mellow.toml
curl http://127.0.0.1:8080/api/v1/health
```
An instance needs about 1.5 GB of memory at its peak: compressing a 16 MP picture attachment takes up to 1.25 GB while an avatar or emoji upload is processed beside it (ADR-0033); 2 GB of RAM is the recommended minimum.
Build the server with `--features web-client` (after `npm run build` in `web/`) to open the web client at `http://127.0.0.1:8080`, or run `npm run dev` in `web/` against an instance on `127.0.0.1:18080`.
Registration is closed by default (`registration_mode = "open"` in `mellow.toml` to allow sign-ups). Only instance administrators (accounts made with `admin create-user --administrator`) create servers by default; `guild_creation = "everyone"` lets every account create its own, and `admin set-administrator --username <name>` promotes an existing account (ADR-0027). The API so far (all JSON, `Authorization: Bearer <token>` except register/login):
- Accounts: `POST /api/v1/auth/register`, `POST /api/v1/auth/login` (with `X-Mellow-Session-Transport: cookie`, the session goes into an `HttpOnly` cookie usable only from the instance's own origin: the web client), `POST /api/v1/auth/logout`, `GET|PATCH /api/v1/users/@me` (display name; username with the current password), `POST /api/v1/users/@me/password` (signs out the other sessions).
- Guilds: `POST /api/v1/guilds`, `GET /api/v1/users/@me/guilds`, `GET /api/v1/guilds/{id}`, `GET /api/v1/guilds/{id}/members`, `DELETE /api/v1/guilds/{id}/members/@me`.
- Channels: `POST /api/v1/guilds/{id}/channels`, `PATCH|DELETE /api/v1/channels/{id}`.
- Invites: `POST /api/v1/guilds/{id}/invites`, `POST /api/v1/invites/{code}`.
- Messages: `GET|POST /api/v1/channels/{id}/messages` (history with `before`/`after`/`limit`, chronological), `PATCH|DELETE /api/v1/channels/{id}/messages/{message_id}`.
- Read states: `PUT /api/v1/channels/{id}/read-state`, `GET /api/v1/users/@me/read-states`.
- Typing: `POST /api/v1/channels/{id}/typing`.
- Direct messages: `POST /api/v1/users/@me/channels` (`recipient_id`, someone who shares a server), then the messages routes above.
- Real time: WebSocket `GET /gateway?v=1` — `hello`, then `identify` (or `resume`) with the session token, `ready`, numbered `dispatch` events (messages, channels, members, guilds, typing, presence, user renames, voice states), `heartbeat` every 30 s; `voice_join`, `voice_leave` and `voice_state` for voice channels.
### Backups
`mellow-server backup --config /etc/mellow/mellow.toml` copies the database and the attachment files (avatars, icons, emoji and sounds included) into a new `backups/backup-<unix milliseconds>/` directory of the data directory, or under `--output-directory <dir>`, while the server runs. The database copy is a consistent snapshot (`VACUUM INTO`), still encrypted with the instance's key, and the files are copied sealed; the backup checks that every file its database points at was copied, and refuses (leaving nothing) if one was deleted meanwhile, so just run it again. After `mellow-server update`, restart the server before a backup: the backup never migrates the database and refuses while migrations wait. By default the backup sits in the data directory and doubles the space it takes until moved. The master key file is not in it: keep a copy of it elsewhere, since neither the instance nor a backup can be decrypted without it. Move the backup to another disk. To restore, stop the server, put the backup's `mellow.db` and `attachments/` in the data directory in place of the current ones (set `mellow.db-wal` and `mellow.db-shm` aside), and start it with the same master key.

### Recompressing older pictures
Pictures stored before visually lossless compression ([ADR-0033](docs/adr/0033-visually-lossless-image-compression.md)) keep their lossless form until an administrator asks. `mellow-server admin recompress-images --config <file> --dry-run` counts them (avatars and banners, guild and channel icons, picture attachments) and estimates the space saved. `--confirm` refuses unless `mellow-server backup` finished in the last hour, since no original is kept; then the running server (or the next one started) recompresses them in the background, one at a time after new uploads, and gives old picture attachments their preview. It can be stopped and resumed at any time: each picture is marked before it is tried, so a restart continues with the next one. Avatars, banners and icons get new ids and clients are told (`USER_UPDATE`, `GUILD_UPDATE`, `CHANNEL_UPDATE`). Custom emoji are left as they are: their id names them in messages and reactions. Run `--dry-run` again to follow the progress.

### Production: built-in TLS
On a server with a public domain, let Mellow get its own Let's Encrypt certificate (no reverse proxy, no certbot):
```bash
mellow-server init --config /etc/mellow/mellow.toml --data-directory /var/lib/mellow --master-key-file /etc/mellow/keys/master.key --domain chat.example.org --contact-email admin@example.org
mellow-server serve --config /etc/mellow/mellow.toml
```
Point the domain's DNS at the server and open port 443 (TCP); nothing else is needed, port 80 stays closed. The server listens on `[::]:443` (use `--listen-address 0.0.0.0:443` on hosts without IPv6), so it needs permission to bind a low port. In its systemd unit, set `AmbientCapabilities=CAP_NET_BIND_SERVICE`, `LimitNOFILE=65536` (the server accepts up to 10 000 connections) and `UMask=0077`. Certificates and the ACME account key live in `certificates/` next to the master key (private to the service user, outside the data directory). Add `--use-staging-certificates` to try a setup against Let's Encrypt staging without hitting its rate limits; the resulting certificate is not trusted by clients. Wildcard names are not supported (ADR-0013).

### Voice channels
Voice is relayed by the server itself over one UDP port, 50000 by default ([ADR-0030](docs/adr/0030-voice-channels-with-an-sfu-in-the-server-binary.md)). For people outside your network to talk, forward UDP 50000 on your router (box) to the server and open it in the host firewall (for example `sudo ufw allow 50000/udp`); a reverse proxy is not involved. Nothing needs configuring: clients send audio to the name they reached the instance at. Optional `[voice]` table in `mellow.toml`: `enabled` (default `true`), `udp_port` (default `50000`), `public_address` (an IPv4 address or a DNS name, when the name clients use does not resolve to this server's public IPv4 address), `max_participants` (default `50` across the instance) and `video_upload_mbps` (default `100`: the upload cameras and shared screens may take, shared fairly among the people watching; a watcher whose connection loses packets gets smaller pictures first; `0` turns video off, ADR-0035 and ADR-0037). For many cameras, also raise the kernel's UDP buffers (`sysctl -w net.core.rmem_max=4194304 net.core.wmem_max=4194304`): the server asks for 4 MiB and logs what it got. If the port cannot be opened, the server logs it once and keeps serving text chat. Audio is encrypted between each client and the server; the server relays it without storing or logging it, and could technically hear it, like any server without end-to-end encryption.

Links posted in messages get a preview (title, description, picture), fetched once by the server for everyone ([ADR-0041](docs/adr/0041-link-previews-fetched-by-the-server.md)): sites see the instance's address, never its members'. The server only reaches public addresses on ports 80 and 443, never your own network. To keep the instance from fetching anything, add `[link_previews]` with `enabled = false` to `mellow.toml`.

GIFs come from Klipy and are off until you set a key ([ADR-0042](docs/adr/0042-gifs-from-klipy-in-the-browser.md)): create an app key on Klipy's partner panel (turn ads off for it), then add `[gifs]` with `klipy_app_key = "…"` to `mellow.toml` and restart. Klipy's terms require members' browsers to search and load GIFs themselves, so Klipy sees the address of members who search for or view a GIF; Mellow sends it no account, name or message. A test key allows about 100 searches per hour for the whole instance.

### Behind a reverse proxy
Without `--domain`, the server listens on `127.0.0.1:8080` over plain HTTP. Put a TLS reverse proxy (Caddy, nginx) on the same host in front of it and set `behind_tls_reverse_proxy = true` whenever one does, including on loopback: rate limits then use the client address from the proxy's `X-Forwarded-For` (trusted from loopback, and from `trusted_proxy_networks` below). When the proxy runs in a container (Caddy or nginx in Docker), it reaches Mellow from its container address rather than loopback: list that address in `trusted_proxy_networks` (e.g. `trusted_proxy_networks = ["172.18.0.10/32"]`, with the proxy's address fixed by `ipv4_address` in compose). Any peer in a trusted network can choose the client address, so do not trust the whole Docker network: it includes every other container on it and the gateway (`.1`, the Docker host, which is also the source of connections forwarded by Docker's userland proxy). Never publish Mellow's port. If the proxy reaches Mellow over IPv6, list its IPv6 address too, or every client shares the proxy's rate limit. Only private networks are accepted. Without the flag or `[tls]`, a non-loopback listen address is refused, since session tokens would travel unencrypted. Do not publish Mellow's plain HTTP port directly.

The web client signs in with a session cookie that Mellow accepts only from its own origin, which it checks against the `Host` header: the proxy must pass the browser's `Host` unchanged, port included. Caddy does by default; nginx needs `proxy_set_header Host $http_host;` (plus `proxy_http_version 1.1;` and the `Upgrade`/`Connection` headers for `/gateway`); Apache needs `ProxyPreserveHost On`. The proxy should also send `Strict-Transport-Security` (Mellow sends it itself only with built-in TLS).

### Keys and platforms
Back up the master key file separately from the data directory; without it the database cannot be decrypted. In production, disable core dumps for the service (`LimitCORE=0` in its systemd unit): a dump would contain the key. The server currently runs on Linux and macOS only: on Windows it refuses to create or read a master key until the key file can be restricted to the service account.

## License
AGPL-3.0-or-later.
