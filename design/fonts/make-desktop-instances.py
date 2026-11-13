"""Writes the static Figtree instances the desktop client embeds (design/fonts/desktop).

GPUI's Linux text system registers a variable font as one face at its default weight
(Figtree's is 300), so every weight would render Light; static faces give each weight its own
face. fontTools names each instance from the font's STAT table, as the OpenType model expects.
Run with fontTools 4.68 or newer: `python3 design/fonts/make-desktop-instances.py`.
"""
from pathlib import Path

from fontTools.ttLib import TTFont
from fontTools.varLib.instancer import instantiateVariableFont

FONTS = Path(__file__).resolve().parent
OUTPUT = FONTS / "desktop"
ROMAN_WEIGHTS = {400: "Regular", 500: "Medium", 600: "SemiBold", 700: "Bold", 800: "ExtraBold"}
ITALIC_WEIGHTS = {400: "Italic", 600: "SemiBoldItalic", 700: "BoldItalic"}

OUTPUT.mkdir(exist_ok=True)
for source, weights in (("Figtree[wght].ttf", ROMAN_WEIGHTS), ("Figtree-Italic[wght].ttf", ITALIC_WEIGHTS)):
    for weight, style_name in weights.items():
        font = instantiateVariableFont(
            TTFont(FONTS / source), {"wght": weight}, updateFontNames=True, static=True
        )
        font.save(OUTPUT / f"Figtree-{style_name}.ttf")
