# 0001. Use one Cargo workspace with a shared protocol crate

- Status: Accepted
- Date: 2026-10-19

## Context
The server, the desktop client and the Android client (through a Rust core) must agree on every REST payload and gateway event. Hand-maintained duplicates of these types drift and cause runtime bugs.

## Decision
Mellow is one Cargo workspace with five crates: `mellow-protocol`, `mellow-server`, `mellow-client-core`, `mellow-ffi`, `mellow-desktop`. All wire types live in `mellow-protocol`, which has no I/O and no dependency on other workspace crates. The Android Gradle project lives in `android/` in the same repository.

## Consequences
- A protocol change is one commit that updates the server and clients together and fails to compile if one side is forgotten.
- One `Cargo.lock`, one set of lints, one CI pipeline.
- The repository mixes Rust and Kotlin tooling; CI needs both.

## Alternatives considered
- **Separate repositories per component:** independent release cycles, but protocol drift and cross-repo changes for every feature.
- **Schema-first protocol (OpenAPI/protobuf) with code generation:** useful for third-party clients later, but adds a toolchain for no MVP benefit. An OpenAPI description can be generated from the Rust types later.
