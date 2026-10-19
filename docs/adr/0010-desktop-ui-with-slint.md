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

## Spike results (2026-10-19)

Prototype: `crates/mellow-desktop` (Slint 1.18.1), one channel with 10 000 deterministic fake messages (author groups, replies, mentions, inline formatting, links, long wrapped text, code blocks, quotes, emoji) and a composer, styled with the DESIGN.md dark tokens. Measured on an Apple M3 (16 GB, macOS 27.0.1), release build, default Slint renderer.

| Capability | Result | Notes |
|---|---|---|
| Virtualized list, 10 000 messages | **Works** | `ListView` instantiates only visible rows. Scrolling the whole history (517 frames): render time p50 1.2 ms, p99 1.8 ms, max 1.9 ms, 0 frames over 16.7 ms. First frame 23 ms warm (176 ms on a cold first launch). Peak RSS 114 MB. Release binary 16 MB. Run `MELLOW_SPIKE_SCROLL_BENCHMARK=1 cargo run --release -p mellow-desktop`. |
| Bottom-anchored list (open at latest message) | **Works** | Set `content-y` to `visible-height - content-height` one tick after the first layout. Variable row heights are estimated by `ListView`, so "jump to message N" will need care. |
| Inline rich text | **Works** | `StyledText` + `slint::StyledText::from_markdown` render bold, italic, strikethrough, inline code, links (with `link-clicked`) and color emoji. |
| Block constructs (code blocks, quotes, headings, lists) | **Workaround** | `from_markdown` returns an error for code blocks, block quotes and headings. M1 must split a message body into blocks in Rust (paragraph / code block / quote) and render each block with its own element; the spike falls back to plain text, which shows raw backticks. |
| Raw HTML in user content | **Workaround (done)** | Slint markdown accepts `<font color>` and `<u>`, which would let senders restyle text (spoofing). Bodies escape `<` before parsing; covered by a unit test. |
| Text selection | **Partial** | `StyledText` is not selectable. A read-only `TextInput` (F2 toggles it in the spike) allows selection and copy within one message, but only of plain text. Selection across messages is not possible. Proposed product answer: per-message "Copy text" action plus selectable mode; record it in DESIGN.md. |
| Keyboard navigation of the list | **Not done** | List rows are not focusable yet; roving focus with arrow keys must be built (FocusScope per row). |
| Screen reader (AccessKit) | **Not verified** | Rows expose `accessible-role: list-item` and a label; a polite `accessible-live-region` announces sent messages. Nobody has listened with VoiceOver/NVDA/Orca yet: this is the open item before M1 UI work. |
| Bundled variable font (Atkinson Hyperlegible Next) | **Not verified** | The font files were not downloaded during the spike; the prototype uses the system font. An uppercase accented initial ("É" in "Élise") looked clipped in one snapshot: re-check once the real font is bundled. |
| Licensing | **OK** | Slint crates are `GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`. Mellow uses them under GPL-3.0-only, which may be combined with AGPL-3.0 (GPLv3 §13). Consequence: the desktop client cannot move to a future (A)GPL v4 unless Slint does. |
| Tooling | **OK, with exceptions** | Slint-generated code needs a local `allow(unsafe_code)` (the workspace now uses `deny` instead of `forbid`) and its module opts out of our clippy restriction lints. Transitive winit/AccessKit crates carry RustSec "unmaintained" notices (no vulnerabilities); `deny.toml` now fails on unmaintained crates only for direct dependencies. |

**Conclusion:** no blocker. Performance is far inside budget and inline rich text works. The decision stands, with three M1 obligations: block-level message rendering in Rust, a selection/copy design that does not rely on cross-message selection, and a real screen-reader pass before the message list is considered done. Switching to Iced is not justified: to our knowledge it has no rich-text selection either, and its screen-reader support is weaker (not re-measured in this spike).
