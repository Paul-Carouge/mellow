# Brand marks

Other companies' marks, shown only where their own rules allow it (DESIGN.md 5.4, "Link cards"). They are not Mellow's icons: never recoloured with Mellow's tokens, never used as a control's glyph, never the most prominent thing on a card.

| File | Mark | Source | Rules followed |
|---|---|---|---|
| `youtube-icon.svg` | YouTube's icon (the rounded rectangle with the play triangle cut out) | Path data from [Simple Icons](https://simpleicons.org) (CC0 1.0), which traces the published mark; the mark itself is a trademark of Google LLC | [YouTube API Services branding guidelines](https://developers.google.com/youtube/terms/branding-guidelines) and the [YouTube brand resources](https://brand.youtube/): shown unaltered in its white monochrome version (`#FFFFFF`, `component.link-preview.brand-mark-color`) on the dark media stage, at `brand-mark-height` (20 px), next to the YouTube content it attributes, as a link to that video on YouTube, smaller than the card's title and play key, and never suggesting that YouTube endorses Mellow. |

The SVG uses `fill="currentColor"`, so a client sets the mark's colour from `brand-mark-color` and nothing else; the triangle is a hole (`fill-rule="evenodd"`), so the stage shows through it.
