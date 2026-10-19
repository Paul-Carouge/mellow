# Mellow

A calm, self-hostable alternative to Discord, written in Rust.

- **Self-hosting in minutes:** one server binary, one data directory, no Docker, no external database, one-command updates.
- **Feels like Discord:** same layout and habits (servers, channels, DMs, roles, attachments), with Mellow's own brand.
- **Native clients:** desktop (Linux, macOS, Windows) in Rust with Slint, Android in Kotlin with Material 3 Expressive.
- **Private by default:** TLS everywhere, data encrypted at rest, no telemetry.

> Status: M1 in progress. The server initialises and serves a health check; the desktop shell runs on fake data. See [ROADMAP.md](ROADMAP.md).

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
```
Registration is closed by default (`registration_mode = "open"` in `mellow.toml` to allow sign-ups). The API so far (all JSON, `Authorization: Bearer <token>` except register/login):
- Accounts: `POST /api/v1/auth/register`, `POST /api/v1/auth/login`, `POST /api/v1/auth/logout`, `GET /api/v1/users/@me`.
- Guilds: `POST /api/v1/guilds`, `GET /api/v1/users/@me/guilds`, `GET /api/v1/guilds/{id}`, `GET /api/v1/guilds/{id}/members`, `DELETE /api/v1/guilds/{id}/members/@me`.
- Channels: `POST /api/v1/guilds/{id}/channels`, `PATCH|DELETE /api/v1/channels/{id}`.
- Invites: `POST /api/v1/guilds/{id}/invites`, `POST /api/v1/invites/{code}`.
- Messages: `GET|POST /api/v1/channels/{id}/messages` (history with `before`/`after`/`limit`, chronological), `PATCH|DELETE /api/v1/channels/{id}/messages/{message_id}`.
- Read states: `PUT /api/v1/channels/{id}/read-state`, `GET /api/v1/users/@me/read-states`.
- Typing: `POST /api/v1/channels/{id}/typing`.
- Real time: WebSocket `GET /gateway?v=1` — `hello`, then `identify` (or `resume`) with the session token, `ready`, numbered `dispatch` events (messages, channels, members, guilds, typing, presence), `heartbeat` every 30 s.
The server listens on `127.0.0.1:8080` over plain HTTP until built-in TLS lands (M1). Put a TLS reverse proxy (Caddy, nginx) on the same host in front of it and set `behind_tls_reverse_proxy = true` whenever one does, including on loopback: rate limits then use the client address from the proxy's `X-Forwarded-For` (trusted only from loopback). Without the flag, a non-loopback listen address is refused, since session tokens would travel unencrypted. Do not publish Mellow's port directly. Back up the master key file separately from the data directory; without it the database cannot be decrypted. In production, disable core dumps for the service (`LimitCORE=0` in its systemd unit): a dump would contain the key. The server currently runs on Linux and macOS only: on Windows it refuses to create or read a master key until the key file can be restricted to the service account.

## License
AGPL-3.0-or-later.
