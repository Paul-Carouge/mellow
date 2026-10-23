# Mellow

A calm, self-hostable alternative to Discord, written in Rust.

- **Self-hosting in minutes:** one server binary, one data directory, no Docker, no external database, one-command updates.
- **Feels like Discord:** same layout and habits (servers, channels, DMs, roles, attachments), with Mellow's own brand.
- **Clients:** a web client served by the instance itself (Svelte), desktop (Linux, macOS, Windows) in Rust with Slint, Android in Kotlin with Material 3 Expressive.
- **Private by default:** TLS everywhere, data encrypted at rest, no telemetry.

> Status: M2 in progress. Servers, channels, invites, roles and permissions, moderation and direct messages work end to end in the desktop app; the web client is being built. See [ROADMAP.md](ROADMAP.md).

## Documentation
- [ARCHITECTURE.md](ARCHITECTURE.md) — how the system is built.
- [DESIGN.md](DESIGN.md) — visual identity and UX.
- [docs/adr/](docs/adr/) — architecture decision records.
- [CLAUDE.md](CLAUDE.md) — instructions for AI-assisted development.

## Development
Prerequisites: [rustup](https://rustup.rs) (the pinned toolchain in `rust-toolchain.toml` installs itself), a C compiler, and to build the vendored OpenSSL used by SQLCipher: `perl` and `make` on Linux, Visual Studio Build Tools (`nmake`) and Strawberry Perl on Windows. Android (later): JDK 21, Android SDK, NDK, `cargo-ndk`.

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
cargo deny check
cargo xtask tokens --check
```
Web client ([`web/`](web/), Svelte 5 + Tailwind 4, Node 24 and npm): `cd web && npm ci`, then `npm run check`, `npm run test`, `npm run build`. `npm run dev` serves it on `http://127.0.0.1:5173` and proxies the API and the gateway to a local instance on `127.0.0.1:18080` (`MELLOW_DEV_INSTANCE` to change it). `cargo build -p mellow-server --features web-client` embeds the built client (`web/dist`) in the server, which then serves it at `/`; release builds always do.

The certificate test against [Pebble](https://github.com/letsencrypt/pebble), the ACME test authority, is ignored by default: `MELLOW_PEBBLE_BINARY=/path/to/pebble cargo test -p mellow-server --lib tls::tests -- --ignored`.

## Install the desktop app
Linux (x86_64, aarch64; glibc 2.35 or newer) and macOS (Apple Silicon), for the current user, without root:
```bash
curl -fsSL https://raw.githubusercontent.com/Paul-Carouge/mellow/main/install.sh | sh
```
It downloads the latest release, checks its minisign signature (or, without `minisign` installed, the digest from the signed `SHA256SUMS`), and installs `~/.local/bin/mellow-desktop` with a menu entry on Linux, or `~/Applications/Mellow.app` on macOS. Later versions install from the app: User settings → About. `sh -s -- --uninstall` removes it. On Linux, a Secret Service provider (GNOME Keyring, KWallet) keeps you signed in between launches.

## Releases
Each tag `vX.Y.Z` publishes, on the [releases page](https://github.com/Paul-Carouge/mellow/releases), `mellow-server` and `mellow-desktop` for `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin` and `x86_64-pc-windows-msvc`. Each binary, and the `mellow-X.Y.Z-SHA256SUMS` digest list, has a `.minisig` signature whose trusted comment is the signed file's own name. Linux builds need glibc 2.35 or newer (Debian 12, Ubuntu 22.04); the desktop app also needs fontconfig, xkbcommon and D-Bus at run time. Check a file by hand with the public key in [`release/minisign.pub`](release/minisign.pub), then make sure the printed `Trusted comment:` is exactly that file's name (otherwise the signature belongs to another file):
```bash
minisign -V -p release/minisign.pub -m mellow-server-0.10.0-x86_64-unknown-linux-gnu
```
The macOS and Windows builds are not signed by Apple or Microsoft yet, so the first launch shows a warning (macOS: right-click → Open).
Updating an instance is one command: `sudo mellow-server update` installs the latest release in place of the running executable after checking its signature (the replaced one is kept as `mellow-server.previous`), then restart the service; `--check` only compares versions and `--rollback` puts the previous executable back, not the database ([ADR-0018](docs/adr/0018-roll-back-the-executable-only-and-guard-the-install-location.md)). Keep the executable in a root-owned directory such as `/usr/local/bin` (`root:root`, mode 0755) and run the service as a dedicated account: the updater refuses an executable or directory that another account could change. To go back to a version older than a migration, stop the server, set `mellow.db`, `mellow.db-wal` and `mellow.db-shm` aside, and copy `backups/pre-migration-<version>.db` to `mellow.db`. With Docker Compose, use [`deploy/docker`](deploy/docker/README.md) and its `mellow-update` instead.

To publish a release (maintainer): bump `version` in `Cargo.toml`, move the `CHANGELOG.md` entries under the new version, merge, then `git tag vX.Y.Z origin/main && git push origin vX.Y.Z` and approve the `release` environment in the workflow run ([ADR-0016](docs/adr/0016-sign-releases-in-a-protected-ci-environment.md)). Before approving, check that the run's commit is the tip of `main` you meant to release and that `release.yml` did not change unexpectedly: the approved job receives the signing key.

## Run a development instance
```bash
cargo run -p mellow-server -- init --config dev/mellow.toml --data-directory dev/data --master-key-file dev/keys/master.key
cargo run -p mellow-server -- admin create-user --config dev/mellow.toml --username admin --email admin@example.org --administrator
cargo run -p mellow-server -- serve --config dev/mellow.toml
curl http://127.0.0.1:8080/api/v1/health
cargo run -p mellow-desktop
```
In the desktop app, use `http://127.0.0.1:8080` as the instance address (plain HTTP is accepted only on loopback; any other address must use `https://`). `MELLOW_DEMO_WORKSPACE=1 cargo run -p mellow-desktop` opens the shell on fake data instead, without an instance.
Registration is closed by default (`registration_mode = "open"` in `mellow.toml` to allow sign-ups). The API so far (all JSON, `Authorization: Bearer <token>` except register/login):
- Accounts: `POST /api/v1/auth/register`, `POST /api/v1/auth/login` (with `X-Mellow-Session-Transport: cookie`, the session goes into an `HttpOnly` cookie usable only from the instance's own origin: the web client), `POST /api/v1/auth/logout`, `GET|PATCH /api/v1/users/@me` (display name; username with the current password), `POST /api/v1/users/@me/password` (signs out the other sessions).
- Guilds: `POST /api/v1/guilds`, `GET /api/v1/users/@me/guilds`, `GET /api/v1/guilds/{id}`, `GET /api/v1/guilds/{id}/members`, `DELETE /api/v1/guilds/{id}/members/@me`.
- Channels: `POST /api/v1/guilds/{id}/channels`, `PATCH|DELETE /api/v1/channels/{id}`.
- Invites: `POST /api/v1/guilds/{id}/invites`, `POST /api/v1/invites/{code}`.
- Messages: `GET|POST /api/v1/channels/{id}/messages` (history with `before`/`after`/`limit`, chronological), `PATCH|DELETE /api/v1/channels/{id}/messages/{message_id}`.
- Read states: `PUT /api/v1/channels/{id}/read-state`, `GET /api/v1/users/@me/read-states`.
- Typing: `POST /api/v1/channels/{id}/typing`.
- Direct messages: `POST /api/v1/users/@me/channels` (`recipient_id`, someone who shares a server), then the messages routes above.
- Real time: WebSocket `GET /gateway?v=1` — `hello`, then `identify` (or `resume`) with the session token, `ready`, numbered `dispatch` events (messages, channels, members, guilds, typing, presence, user renames), `heartbeat` every 30 s.
### Production: built-in TLS
On a server with a public domain, let Mellow get its own Let's Encrypt certificate (no reverse proxy, no certbot):
```bash
mellow-server init --config /etc/mellow/mellow.toml --data-directory /var/lib/mellow --master-key-file /etc/mellow/keys/master.key --domain chat.example.org --contact-email admin@example.org
mellow-server serve --config /etc/mellow/mellow.toml
```
Point the domain's DNS at the server and open port 443 (TCP); nothing else is needed, port 80 stays closed. The server listens on `[::]:443` (use `--listen-address 0.0.0.0:443` on hosts without IPv6), so it needs permission to bind a low port. In its systemd unit, set `AmbientCapabilities=CAP_NET_BIND_SERVICE`, `LimitNOFILE=65536` (the server accepts up to 10 000 connections) and `UMask=0077`. Certificates and the ACME account key live in `certificates/` next to the master key (private to the service user, outside the data directory). Add `--use-staging-certificates` to try a setup against Let's Encrypt staging without hitting its rate limits; the resulting certificate is not trusted by clients. Wildcard names are not supported (ADR-0013).

### Behind a reverse proxy
Without `--domain`, the server listens on `127.0.0.1:8080` over plain HTTP. Put a TLS reverse proxy (Caddy, nginx) on the same host in front of it and set `behind_tls_reverse_proxy = true` whenever one does, including on loopback: rate limits then use the client address from the proxy's `X-Forwarded-For` (trusted from loopback, and from `trusted_proxy_networks` below). When the proxy runs in a container (Caddy or nginx in Docker), it reaches Mellow from its container address rather than loopback: list that address in `trusted_proxy_networks` (e.g. `trusted_proxy_networks = ["172.18.0.10/32"]`, with the proxy's address fixed by `ipv4_address` in compose). Any peer in a trusted network can choose the client address, so do not trust the whole Docker network: it includes every other container on it and the gateway (`.1`, the Docker host, which is also the source of connections forwarded by Docker's userland proxy). Never publish Mellow's port. If the proxy reaches Mellow over IPv6, list its IPv6 address too, or every client shares the proxy's rate limit. Only private networks are accepted. Without the flag or `[tls]`, a non-loopback listen address is refused, since session tokens would travel unencrypted. Do not publish Mellow's plain HTTP port directly.

The web client signs in with a session cookie that Mellow accepts only from its own origin, which it checks against the `Host` header: the proxy must pass the browser's `Host` unchanged, port included. Caddy does by default; nginx needs `proxy_set_header Host $http_host;` (plus `proxy_http_version 1.1;` and the `Upgrade`/`Connection` headers for `/gateway`); Apache needs `ProxyPreserveHost On`. The proxy should also send `Strict-Transport-Security` (Mellow sends it itself only with built-in TLS).

### Keys and platforms
Back up the master key file separately from the data directory; without it the database cannot be decrypted. In production, disable core dumps for the service (`LimitCORE=0` in its systemd unit): a dump would contain the key. The server currently runs on Linux and macOS only: on Windows it refuses to create or read a master key until the key file can be restricted to the service account.

## License
AGPL-3.0-or-later.
