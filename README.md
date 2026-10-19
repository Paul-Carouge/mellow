# Mellow

A calm, self-hostable alternative to Discord, written in Rust.

- **Self-hosting in minutes:** one server binary, one data directory, no Docker, no external database, one-command updates.
- **Familiar:** servers, channels, direct messages, roles and permissions, attachments.
- **Native clients:** desktop (Linux, macOS, Windows) in Rust with Slint, Android in Kotlin with Material 3 Expressive.
- **Private by default:** TLS everywhere, data encrypted at rest, no telemetry.

> Status: design phase. Nothing is runnable yet. See [ROADMAP.md](ROADMAP.md).

## Documentation
- [ARCHITECTURE.md](ARCHITECTURE.md) — how the system is built.
- [DESIGN.md](DESIGN.md) — visual identity and UX.
- [docs/adr/](docs/adr/) — architecture decision records.
- [CLAUDE.md](CLAUDE.md) — instructions for AI-assisted development.

## Prerequisites (planned)
- Rust stable (see `rust-toolchain.toml` once added).
- Android: JDK 21, Android SDK, NDK, `cargo-ndk`.

## License
AGPL-3.0-or-later.
