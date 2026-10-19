# 0011. Share the client core with Android through UniFFI

- Status: Accepted
- Date: 2026-10-19

## Context
The Android app is written in Kotlin with Compose Material 3 Expressive. Re-implementing the protocol, sync, cache and outbox logic in Kotlin would duplicate the hardest client code.

## Decision
`mellow-client-core` holds all client logic. `mellow-ffi` exposes it to Kotlin with UniFFI (async functions and callback-based event streams). Native libraries are built with `cargo-ndk` for `arm64-v8a` and `x86_64`. Kotlin contains only UI, ViewModels and platform integrations (push, keystore, notifications, file picker).

## Consequences
- One implementation of protocol and sync behaviour, tested once in Rust.
- The exported API must stay FFI-friendly: owned types, no lifetimes or generics, errors as flat enums.
- The Android build requires the Rust toolchain and NDK; CI must cache both.
- Debugging across the FFI boundary is harder; the core logs through `tracing` forwarded to Logcat.

## Alternatives considered
- **Pure Kotlin client (Ktor/OkHttp):** simpler build, duplicated logic and divergent behaviour.
- **Kotlin Multiplatform shared with desktop:** contradicts the 100% Rust desktop requirement.
