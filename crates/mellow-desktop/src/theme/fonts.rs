//! Figtree and Source Code Pro, embedded from `design/fonts` (their single source) so the app
//! looks the same on every machine (DESIGN.md §2.4).

use std::borrow::Cow;

use anyhow::{Context as _, Result};
use gpui::App;

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

/// # Errors
/// When the text system rejects a font file.
pub fn register_embedded_fonts(cx: &App) -> Result<()> {
    cx.text_system()
        .add_fonts(EMBEDDED_FONT_FILES.into_iter().map(Cow::Borrowed).collect())
        .context("cannot register the embedded fonts")
}

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
