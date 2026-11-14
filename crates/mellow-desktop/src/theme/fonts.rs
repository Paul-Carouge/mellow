//! Figtree and Source Code Pro, embedded from `design/fonts` (their single source) so the app
//! looks the same on every machine (DESIGN.md §2.4).

use std::borrow::Cow;

use anyhow::{Context as _, Result};
use gpui::{App, Font, font};

/// The family names inside the font files, as `font.family.sans` and `font.family.mono` list them.
pub const SANS_FONT_FAMILY: &str = "Figtree";
pub const MONOSPACE_FONT_FAMILY: &str = "Source Code Pro";

/// Static faces, one per weight the tokens use: GPUI's Linux text system registers a variable
/// font as a single face at its default weight (300 for Figtree, 200 for Source Code Pro), so
/// the web's variable files would render every weight thin (`design/fonts/README.md`).
const EMBEDDED_FONT_FILES: [&[u8]; 12] = [
    include_bytes!("../../../../design/fonts/desktop/Figtree-Regular.ttf"),
    include_bytes!("../../../../design/fonts/desktop/Figtree-Medium.ttf"),
    include_bytes!("../../../../design/fonts/desktop/Figtree-SemiBold.ttf"),
    include_bytes!("../../../../design/fonts/desktop/Figtree-Bold.ttf"),
    include_bytes!("../../../../design/fonts/desktop/Figtree-ExtraBold.ttf"),
    include_bytes!("../../../../design/fonts/desktop/Figtree-Italic.ttf"),
    include_bytes!("../../../../design/fonts/desktop/Figtree-SemiBoldItalic.ttf"),
    include_bytes!("../../../../design/fonts/desktop/Figtree-BoldItalic.ttf"),
    include_bytes!("../../../../design/fonts/desktop/SourceCodePro-Regular.ttf"),
    include_bytes!("../../../../design/fonts/desktop/SourceCodePro-It.ttf"),
    include_bytes!("../../../../design/fonts/desktop/SourceCodePro-Semibold.ttf"),
    include_bytes!("../../../../design/fonts/desktop/SourceCodePro-Bold.ttf"),
];

/// The colour emoji font, tried first for a character the embedded fonts lack. GPUI's Linux text
/// system would otherwise try monochrome symbol fonts first (`DejaVu Sans`, `Noto Sans Symbols`);
/// a fallback chain may name an emoji font since ADR-0059 (`vendor/gpui-pre-wgpu`). macOS and
/// Windows find their colour emoji font by themselves.
#[cfg(target_os = "linux")]
const EMOJI_FONT_FAMILY: &str = "Noto Color Emoji";

/// Figtree, with the colour emoji font as its fallback. Set on the main window's root, so every text
/// below it inherits the fallback, the composer's input included.
#[cfg(target_os = "linux")]
#[must_use]
pub fn interface_font() -> Font {
    Font {
        fallbacks: Some(gpui::FontFallbacks::from_fonts(vec![
            EMOJI_FONT_FAMILY.to_owned(),
        ])),
        ..font(SANS_FONT_FAMILY)
    }
}

/// Figtree.
#[cfg(not(target_os = "linux"))]
#[must_use]
pub fn interface_font() -> Font {
    font(SANS_FONT_FAMILY)
}

/// # Errors
/// When the text system rejects a font file.
pub fn register_embedded_fonts(cx: &App) -> Result<()> {
    cx.text_system()
        .add_fonts(EMBEDDED_FONT_FILES.into_iter().map(Cow::Borrowed).collect())
        .context("cannot register the embedded fonts")
}

/// GPUI's Linux text system reads the system fonts on a background thread (ADR-0059), so the
/// first frames draw with the embedded fonts alone, leaving out what they lack (an emoji). Once
/// the system fonts are in, GPUI's font lookups and line layouts are dropped and the windows drawn
/// again.
#[cfg(target_os = "linux")]
pub fn redraw_with_system_fonts_once_read(cx: &App) {
    let text_system = cx.text_system().clone();
    // Waits for the system fonts: off the main thread.
    let system_fonts_read = cx
        .background_executor()
        .spawn(async move { text_system.prewarm_fonts(&[interface_font()]) });
    cx.spawn(async move |cx| {
        system_fonts_read.await;
        cx.update(|cx| {
            // Adds no font, but GPUI drops what it shaped before.
            if let Err(error) = cx.text_system().add_fonts(Vec::new()) {
                tracing::warn!("cannot redraw with the system fonts: {error:#}");
            }
            cx.refresh_windows();
        });
    })
    .detach();
}

#[cfg(test)]
#[cfg(target_os = "linux")]
#[path = "fonts_tests.rs"]
mod system_font_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::tokens;

    /// (typographic family, weight, is italic) of every embedded face.
    fn embedded_faces() -> Vec<(String, u16, bool)> {
        use skrifa::MetadataProvider as _;
        use skrifa::string::StringId;

        EMBEDDED_FONT_FILES
            .iter()
            .map(|bytes| {
                let font = skrifa::FontRef::new(bytes).expect("an embedded font parses");
                let family = [StringId::TYPOGRAPHIC_FAMILY_NAME, StringId::FAMILY_NAME]
                    .into_iter()
                    .find_map(|id| font.localized_strings(id).english_or_first())
                    .expect("an embedded font names its family")
                    .to_string();
                let attributes = font.attributes();
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let weight = attributes.weight.value().round() as u16;
                let is_italic = attributes.style != skrifa::attribute::Style::Normal;
                (family, weight, is_italic)
            })
            .collect()
    }

    #[test]
    fn every_weight_the_tokens_use_has_its_own_static_face() {
        let faces = embedded_faces();
        let has_face = |family: &str, weight: u16, is_italic: bool| {
            faces.contains(&(family.to_owned(), weight, is_italic))
        };
        for token in [
            tokens::TYPE_BODY,
            tokens::TYPE_BUTTON,
            tokens::TYPE_CATEGORY,
            tokens::TYPE_HEADING_LG,
            tokens::TYPE_DISPLAY,
        ] {
            assert!(
                has_face(SANS_FONT_FAMILY, token.font_weight, false),
                "no {SANS_FONT_FAMILY} face at {}",
                token.font_weight
            );
        }
        for weight in [400, 600, 700] {
            assert!(
                has_face(SANS_FONT_FAMILY, weight, true),
                "no italic at {weight}"
            );
        }
        for weight in [400, 700] {
            assert!(
                has_face(MONOSPACE_FONT_FAMILY, weight, false),
                "no code face at {weight}"
            );
        }
        assert!(
            faces.iter().all(|(_, weight, _)| *weight >= 400),
            "a variable file at its default (light) weight came back: {faces:?}"
        );
    }
}
