# Mellow

A calm, self-hostable alternative to Discord, written in Rust.

- **Self-hosting in minutes:** one server binary, one data directory, no Docker, no external database, one-command updates.
- **Feels like Discord:** same layout and habits (servers, channels, DMs, roles, attachments), with Mellow's own brand.
- **Native clients:** desktop (Linux, macOS, Windows) in Rust with Slint, Android in Kotlin with Material 3 Expressive.
- **Private by default:** TLS everywhere, data encrypted at rest, no telemetry.

> Status: M0 foundations. A desktop UI prototype runs (`cargo run -p mellow-desktop`); there is no server yet. See [ROADMAP.md](ROADMAP.md).

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

## License
AGPL-3.0-or-later.
