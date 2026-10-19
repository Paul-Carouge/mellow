# 0012. Clone Discord's interface faithfully, keep Mellow's own brand

- Status: Accepted
- Date: 2026-10-19

## Context
The first DESIGN.md (2026-10-19) kept Discord's information architecture but gave Mellow a calmer, distinct visual identity (warm surfaces, its own proportions). The user then asked for an interface that is "really like Discord": people switching from Discord must feel at home immediately, with no relearning. Copying a competitor's interface raises two questions: what makes adoption easier, and what is legally and ethically someone else's identity.

## Decision
- **Copy faithfully:** layout and pane dimensions (server rail, channel sidebar, headers, user panel, member list), density, message grouping and hover actions, interaction patterns (selection pill, squircle morph, collapsible categories, context menus, quick switcher and other keyboard shortcuts), status conventions, and the layered dark-grey palette family.
- **Keep Mellow's own brand:** Mellow's accent colour (never Discord's blurple or a near-identical hue), name, logo and home-button artwork, and an open-licence bundled font (Discord's gg sans is proprietary). No Discord logo, wordmark, mascot or marketing artwork.
- **Keep Mellow's guarantees:** WCAG 2.2 AA contrast, keyboard access and screen-reader support win over pixel-perfect fidelity. Where a Discord grey fails AA for a text role, the token is adjusted minimally and the deviation is documented in DESIGN.md.

## Consequences
- Switching cost for Discord users is close to zero; Mellow competes on self-hosting, privacy and openness, not on a new interface.
- DESIGN.md and `design/tokens.json` are rewritten; the previous warm, "anti-Discord" identity is superseded.
- Discord's interface can change; Mellow follows its conventions as of 2026 and does not chase every redesign.
- Trade-dress risk is limited by the brand rules above; any public release must be checked against them (accent hue, logo, fonts, marketing screenshots).

## Alternatives considered
- **Pixel-perfect clone including blurple and fonts:** maximum familiarity, but uses Discord's brand identity (trade dress risk) and gg sans cannot be licensed.
- **Discord structure with a distinct Mellow look (previous direction):** rejected by the user as not familiar enough.
