# 0009. Build the admin UI with server-rendered HTML and htmx

- Status: Accepted
- Date: 2026-10-19

## Context
Instance admins need a web page to manage settings, users, storage, backups and updates. It must ship inside the server binary and stay 100% Rust on the server side.

## Decision
Render admin pages on the server with `askama` templates, use htmx for partial updates, and embed all assets (CSS generated from design tokens, htmx, icons) in the binary at compile time. No JavaScript build step.

## Consequences
- No Node toolchain, no separate frontend deployment, small attack surface.
- Rich client-side interactions are limited; acceptable for an admin console.
- htmx is vendored at a pinned version with its hash checked in CI.

## Alternatives considered
- **Leptos/Dioxus WASM SPA:** 100% Rust but heavier build and larger bundle for a low-traffic admin page.
- **CLI only:** acceptable for experts, contradicts "hyper simple" self-hosting.
