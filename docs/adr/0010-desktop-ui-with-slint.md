# 0010. Build the desktop client with Slint

- Status: Accepted
- Date: 2026-10-19

## Context
The desktop client must be native Rust on Linux, macOS and Windows, accessible (WCAG 2.2 AA, screen readers), and themable with a distinct visual identity. Candidates were Iced and Slint.

## Decision
Use Slint. Presentation lives in `.slint` files; Rust adapts client-core state to Slint models. Design tokens are generated into a Slint `global`.

## Consequences
- Screen reader support through AccessKit on all three platforms.
- Live preview of `.slint` files speeds up visual iteration with the design system.
- Long virtualized lists use `ListView`; message rendering (rich text, selection across messages) must be validated early with a prototype, since chat is the hardest screen.
- Licensing: Slint is available under GPLv3 or its Royalty-free license. Mellow is AGPL-3.0, which is compatible with GPLv3. Re-check the Slint license terms at each major Slint upgrade.

## Alternatives considered
- **Iced:** pure Rust and proven (COSMIC), but screen reader support is less mature today.
- **Dioxus desktop:** webview-based, less native, larger runtime.
- **egui:** immediate mode, hard to reach a polished consumer look.
