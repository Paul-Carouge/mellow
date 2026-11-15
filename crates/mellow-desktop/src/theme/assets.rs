//! Serves the embedded icons and brand marks to GPUI, which loads `svg().path(…)` through the
//! app's asset source.

use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

use super::icons::IconName;

/// YouTube's own mark (`design/brands/youtube-icon.svg`, rules in `design/brands/README.md`): a
/// link card's attribution, never a control's glyph, so it is not among the icons.
pub const YOUTUBE_MARK_PATH: &str = "brands/youtube-icon.svg";
/// The mark's width for its height: its file's `viewBox` (24 × 17), checked by a test.
pub const YOUTUBE_MARK_ASPECT_RATIO: f32 = 24.0 / 17.0;
const YOUTUBE_MARK_SVG: &[u8] = include_bytes!("../../../../design/brands/youtube-icon.svg");

/// Every asset is compiled into the binary: the app reads nothing from disk to draw itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmbeddedAssets;

impl AssetSource for EmbeddedAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        if path == YOUTUBE_MARK_PATH {
            return Ok(Some(Cow::Borrowed(YOUTUBE_MARK_SVG)));
        }
        Ok(IconName::from_asset_path(path).map(|icon| Cow::Borrowed(icon.svg_bytes())))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(IconName::ALL
            .iter()
            .map(|icon| icon.asset_path())
            .filter(|asset_path| asset_path.starts_with(path))
            .map(SharedString::from)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_loads_as_an_svg_and_unknown_paths_load_nothing() {
        for icon in IconName::ALL {
            let bytes = EmbeddedAssets.load(icon.asset_path()).unwrap().unwrap();
            assert!(bytes.starts_with(b"<svg"), "{icon:?}");
        }
        assert!(EmbeddedAssets.load("icons/missing.svg").unwrap().is_none());
        let mark = EmbeddedAssets.load(YOUTUBE_MARK_PATH).unwrap().unwrap();
        let mark = std::str::from_utf8(&mark).unwrap();
        // The drawn box keeps the file's proportions.
        assert!(mark.contains(r#"viewBox="0 3.5 24 17""#), "{mark}");
        assert_eq!(
            EmbeddedAssets.list("icons/").unwrap().len(),
            IconName::ALL.len()
        );
    }
}
