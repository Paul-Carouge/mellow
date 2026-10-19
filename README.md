# Mellow

A calm, self-hostable alternative to Discord, written in Rust.

- **Self-hosting in minutes:** one server binary, one data directory, no Docker, no external database, one-command updates.
- **Feels like Discord:** same layout and habits (servers, channels, DMs, roles, attachments), with Mellow's own brand.
- **Native clients:** desktop (Linux, macOS, Windows) in Rust with Slint, Android in Kotlin with Material 3 Expressive.
- **Private by default:** TLS everywhere, data encrypted at rest, no telemetry.

> Status: M1 in progress. The server handles accounts, guilds, channels, messages and a real-time gateway; the desktop client signs in to an instance and chats live. See [ROADMAP.md](ROADMAP.md).

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
- Accounts: `POST /api/v1/auth/register`, `POST /api/v1/auth/login`, `POST /api/v1/auth/logout`, `GET /api/v1/users/@me`.
- Guilds: `POST /api/v1/guilds`, `GET /api/v1/users/@me/guilds`, `GET /api/v1/guilds/{id}`, `GET /api/v1/guilds/{id}/members`, `DELETE /api/v1/guilds/{id}/members/@me`.
- Channels: `POST /api/v1/guilds/{id}/channels`, `PATCH|DELETE /api/v1/channels/{id}`.
- Invites: `POST /api/v1/guilds/{id}/invites`, `POST /api/v1/invites/{code}`.
- Messages: `GET|POST /api/v1/channels/{id}/messages` (history with `before`/`after`/`limit`, chronological), `PATCH|DELETE /api/v1/channels/{id}/messages/{message_id}`.
- Read states: `PUT /api/v1/channels/{id}/read-state`, `GET /api/v1/users/@me/read-states`.
- Typing: `POST /api/v1/channels/{id}/typing`.
- Real time: WebSocket `GET /gateway?v=1` — `hello`, then `identify` (or `resume`) with the session token, `ready`, numbered `dispatch` events (messages, channels, members, guilds, typing, presence), `heartbeat` every 30 s.
### Production: built-in TLS
On a server with a public domain, let Mellow get its own Let's Encrypt certificate (no reverse proxy, no certbot):
```bash
mellow-server init --config /etc/mellow/mellow.toml --data-directory /var/lib/mellow --master-key-file /etc/mellow/keys/master.key --domain chat.example.org --contact-email admin@example.org
mellow-server serve --config /etc/mellow/mellow.toml
```
Point the domain's DNS at the server and open port 443 (TCP); nothing else is needed, port 80 stays closed. The server listens on `[::]:443` (use `--listen-address 0.0.0.0:443` on hosts without IPv6), so it needs permission to bind a low port. In its systemd unit, set `AmbientCapabilities=CAP_NET_BIND_SERVICE`, `LimitNOFILE=65536` (the server accepts up to 10 000 connections) and `UMask=0077`. Certificates and the ACME account key live in `certificates/` next to the master key (private to the service user, outside the data directory). Add `--use-staging-certificates` to try a setup against Let's Encrypt staging without hitting its rate limits; the resulting certificate is not trusted by clients. Wildcard names are not supported (ADR-0013).

### Behind a reverse proxy
Without `--domain`, the server listens on `127.0.0.1:8080` over plain HTTP. Put a TLS reverse proxy (Caddy, nginx) on the same host in front of it and set `behind_tls_reverse_proxy = true` whenever one does, including on loopback: rate limits then use the client address from the proxy's `X-Forwarded-For` (trusted only from loopback). Without the flag or `[tls]`, a non-loopback listen address is refused, since session tokens would travel unencrypted. Do not publish Mellow's plain HTTP port directly.

### Keys and platforms
Back up the master key file separately from the data directory; without it the database cannot be decrypted. In production, disable core dumps for the service (`LimitCORE=0` in its systemd unit): a dump would contain the key. The server currently runs on Linux and macOS only: on Windows it refuses to create or read a master key until the key file can be restricted to the service account.

## License
AGPL-3.0-or-later.
