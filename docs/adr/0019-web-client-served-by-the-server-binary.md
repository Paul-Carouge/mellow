# 0019. Serve a web client from the server binary

- Status: Accepted (2026-10-21, by the project owner: Svelte with Tailwind)
- Date: 2026-10-21

## Context
A Discord replacement must be usable from any browser, without installing anything: this is how most people join a community for the first time. [ARCHITECTURE.md](../../ARCHITECTURE.md) §1 lists web clients as a non-goal for the MVP; the project owner now wants one. The promise of trivial self-hosting must hold: one binary, no separate frontend deployment.

The web client talks to the same REST API (`/api/v1`) and gateway (`/gateway?v=1`) as the native clients. It cannot reuse `mellow-client-core` as is: its networking and secret storage target native platforms.

## Decision
Build the web client as a single-page application in Svelte 5 (TypeScript, strict mode) with Tailwind CSS 4, in a `web/` directory, without SvelteKit (no server rendering: the Rust server serves static files). CI builds it once and the build output is embedded in `mellow-server` at compile time, so a release binary serves the client at `/` with no extra file or service. Design tokens come from `design/tokens.json` as CSS custom properties, with a Tailwind `@theme` block over them (generator in `xtask`, like the Slint one), so utility classes only read tokens.

The session token never reaches page scripts: with `X-Mellow-Session-Transport: cookie`, sign-in sets an `HttpOnly`, `Secure`, `SameSite=Strict` cookie instead of returning the token. The cookie counts only on requests from the instance's own origin (`Sec-Fetch-Site`, `Origin` against `Host`), and the gateway refuses upgrades from other origins; native clients keep bearer tokens. The server serves the client with a strict Content-Security-Policy (no inline script, same-origin connections only) and HSTS when it terminates TLS. These server changes need `security-auditor` before merge.

The web client starts after the desktop client reaches parity on text chat, roles and settings, and covers the same screens (DESIGN.md), adapted to browser layouts.

## Consequences
- Self-hosters still deploy one binary. Contributors need Node to build the client; release builds do it in CI. The bundle is not committed: the `web-client` cargo feature embeds `web/dist`, and without it `cargo build` needs no Node toolchain.
- Client logic is written twice (Rust core, TypeScript). The protocol crate stays the single source of truth; the TypeScript types are written by hand for now and must be generated from it before the protocol grows further.
- The server gains a static-asset surface and cookie handling, widening the attack surface.
- Supersedes the web non-goal of ARCHITECTURE.md §1.

## Alternatives considered
- **Slint compiled to WebAssembly:** reuses the desktop UI, but poor accessibility (no real DOM), large bundles and weak text selection and input handling in browsers.
- **Leptos/Dioxus (Rust WASM) reusing `mellow-client-core`:** one language, but the core would need a WASM networking port first, and the bundle size and build time are worse than a Svelte SPA. [ADR-0009](0009-admin-ui-server-rendered-htmx.md) made the same call for the admin page.
- **Separate hosted web app:** contradicts the single-binary promise.
