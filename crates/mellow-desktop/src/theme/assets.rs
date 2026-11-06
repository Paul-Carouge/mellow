//! Serves the embedded icons to GPUI, which loads `svg().path(…)` through the app's asset source.

use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

use super::icons::IconName;

/// Every asset is compiled into the binary: the app reads nothing from disk to draw itself.
#[derive(Debug, Clone, Copy, Default)]
pub struct EmbeddedAssets;

impl AssetSource for EmbeddedAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
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
        assert_eq!(
            EmbeddedAssets.list("icons/").unwrap().len(),
            IconName::ALL.len()
        );
    }
}
