//! Draws the component gallery without a screen into a PNG, to look at it:
//! `cargo run -p mellow-desktop --example gallery_picture -- /tmp/gallery.png [dark|light]`.
//! GPUI's macOS platform must be created on the main thread, which a test is not.

use std::sync::Arc;

use anyhow::{Context as _, Result};
use gpui::{HeadlessAppContext, WindowAppearance, px, size};
use mellow_desktop::components::gallery::{forced_appearance, open_gallery};
use mellow_desktop::i18n::{Language, use_language};
use mellow_desktop::theme::{EmbeddedAssets, register_embedded_fonts};

/// Wide enough for the three columns, tall enough for the sample messages under them.
const PICTURE_WIDTH: f32 = 1600.0;
const PICTURE_HEIGHT: f32 = 2600.0;

fn main() -> Result<()> {
    let mut arguments = std::env::args().skip(1);
    let path = arguments
        .next()
        .context("give the PNG to write, then dark or light")?;
    let appearance = arguments
        .next()
        .and_then(|theme| forced_appearance(&theme))
        .unwrap_or(WindowAppearance::Dark);
    let text_system = gpui_platform::current_platform(true).text_system();
    let mut cx = HeadlessAppContext::with_platform(
        text_system,
        Arc::new(EmbeddedAssets),
        gpui_platform::current_headless_renderer,
    );
    cx.update(|cx| -> Result<()> {
        gpui_component::init(cx);
        register_embedded_fonts(cx)?;
        use_language(Language::English, cx);
        Ok(())
    })?;
    let window = cx.open_window(size(px(PICTURE_WIDTH), px(PICTURE_HEIGHT)), |window, cx| {
        open_gallery(Some(appearance), window, cx)
    })?;
    cx.run_until_parked();
    cx.capture_screenshot(window.into())?.save(&path)?;
    Ok(())
}
