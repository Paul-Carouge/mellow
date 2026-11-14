# Test fonts

System fonts cut down for the font tests in `src/theme/fonts_tests.rs` (ADR-0059), as they
come on a Linux desktop: the colour emoji font, and the monochrome text font cosmic-text tries
first for a character the interface font lacks.

- `NotoColorEmoji-subset.ttf`: Noto Color Emoji 2.057 (noto-emoji 20260911, SIL Open Font
  License 1.1, `NotoColorEmoji-OFL.txt`) with `#`, `1`, ©, U+20E3, ☺ and 😀 and their keycap
  ligatures, at their glyph ids (GPUI skips glyph 3 of an emoji font). It keeps the PostScript
  name `NotoColorEmoji` and has no "m", as the real font.
- `DejaVuSans-subset.ttf` and `DejaVuSans-Bold-subset.ttf`: DejaVu Sans and DejaVu Sans Bold 2.37
  (Bitstream Vera licence and public domain, `DejaVuSans-LICENSE.txt`) with `#`, `1`, ©, U+FE0F, ☺
  and a monochrome 😀.

```sh
pyftsubset NotoColorEmoji.ttf --unicodes=U+1F600,U+263A,U+00A9,U+FE0F,U+20E3,U+0031,U+0023 \
  --layout-features='*' --name-IDs='*' --name-languages='*' --notdef-outline \
  --retain-gids --output-file=NotoColorEmoji-subset.ttf
pyftsubset DejaVuSans.ttf --unicodes=U+1F600,U+263A,U+00A9,U+FE0F,U+0031,U+0023 \
  --layout-features='*' --name-IDs='*' --name-languages='*' --notdef-outline \
  --output-file=DejaVuSans-subset.ttf
# and the same for DejaVuSans-Bold.ttf into DejaVuSans-Bold-subset.ttf
```
