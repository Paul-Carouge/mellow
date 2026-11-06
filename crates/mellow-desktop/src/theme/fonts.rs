//! Figtree and Source Code Pro, embedded from `design/fonts` (their single source) so the app
//! looks the same on every machine (DESIGN.md §2.4).

use std::borrow::Cow;

use anyhow::{Context as _, Result};
use gpui::App;

/// The family names inside the font files, as `font.family.sans` and `font.family.mono` list them.
pub const SANS_FONT_FAMILY: &str = "Figtree";
pub const MONOSPACE_FONT_FAMILY: &str = "Source Code Pro";

const EMBEDDED_FONT_FILES: [&[u8]; 3] = [
    include_bytes!("../../../../design/fonts/Figtree[wght].ttf"),
    include_bytes!("../../../../design/fonts/Figtree-Italic[wght].ttf"),
    include_bytes!("../../../../design/fonts/SourceCodePro[wght].ttf"),
];

/// # Errors
/// When the text system rejects a font file.
pub fn register_embedded_fonts(cx: &App) -> Result<()> {
    cx.text_system()
        .add_fonts(EMBEDDED_FONT_FILES.into_iter().map(Cow::Borrowed).collect())
        .context("cannot register the embedded fonts")
}
