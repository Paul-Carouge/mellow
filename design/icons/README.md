# Icons

[Phosphor Icons](https://phosphoricons.com) 2.1.1 (`@phosphor-icons/core`, MIT, see `LICENSE`), copied as SVG with `fill="currentColor"`, so each icon takes its text colour from the design tokens.

Weight: **Fill** (DESIGN.md §2.9), except `at.svg`, `check.svg`, `hash.svg`, `list.svg`, `magnifying-glass.svg`, `minus.svg`, `plus.svg` and `x.svg`, which use **Bold**: their Fill versions are a solid square or disc with the glyph cut out, unlike Discord's open mention, check, channel, menu, add-server and close glyphs (`minus` sits beside `check` and `x` in the channel permission control, so it takes their weight; the Fill magnifier's solid lens reads as a target, so search uses the open Bold lens). The "/" menu's formatting glyphs (`code`, `code-block`, `link-simple`, `list-bullets`, `list-numbers`, `text-b`, `text-h-one`, `text-h-two`, `text-h-three`, `text-italic`, `text-strikethrough`, `text-underline`) use Bold too: their Fill versions are the glyph cut out of a solid square, which reads as a toolbar key rather than the mark it writes. The web client (`web/`) reads them from here.

To add an icon, copy `assets/fill/<name>-fill.svg` from the same package version as `<name>.svg`.
