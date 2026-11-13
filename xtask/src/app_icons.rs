//! App icons (DESIGN.md §2.10): the favicon (Melli on its dusk tile) rasterised to the installed
//! web app's PNGs, so every icon comes from the tokens like the favicon does.

use anyhow::{Context as _, Result, anyhow};
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;

/// Rasterisers may round a few pixels differently on another CPU (CI runs on `x86_64`): `--check`
/// accepts that much difference per channel instead of comparing bytes.
pub const MAXIMUM_CHANNEL_DIFFERENCE_ON_CHECK: u8 = 2;

/// The installed web app's icons (`manifest.webmanifest`), served from the web client's `public/`.
pub const WEB_ICONS_DIRECTORY: &str = "web/public/icons";
/// Chrome's install criteria ask for 192 and 512 px; Android launchers prefer the maskable one.
const WEB_ICON_SIZES: [u32; 2] = [192, 512];
pub const WEB_MASKABLE_ICON_SIZE: u32 = 512;
/// iOS "Add to Home Screen" draws its own rounded square and fills transparency with black, so it
/// gets the full-bleed tile.
const APPLE_TOUCH_ICON_SIZE: u32 = 180;
pub const APPLE_TOUCH_ICON_FILE_NAME: &str = "apple-touch-icon.png";

pub fn png_file_name(size: u32) -> String {
    format!("mellow-{size}.png")
}

pub fn maskable_png_file_name(size: u32) -> String {
    format!("mellow-maskable-{size}.png")
}

/// The web app's icon files, as (path under `WEB_ICONS_DIRECTORY`, contents).
pub fn generate_web_app_icons(
    favicon_svg: &str,
    maskable_svg: &str,
) -> Result<Vec<(String, Vec<u8>)>> {
    let favicon = usvg::Tree::from_str(favicon_svg, &usvg::Options::default())
        .context("the favicon is not valid SVG")?;
    let maskable = usvg::Tree::from_str(maskable_svg, &usvg::Options::default())
        .context("the maskable icon is not valid SVG")?;
    let mut icons = Vec::new();
    for size in WEB_ICON_SIZES {
        icons.push((png_file_name(size), render_png(&favicon, size)?));
    }
    icons.push((
        maskable_png_file_name(WEB_MASKABLE_ICON_SIZE),
        render_png(&maskable, WEB_MASKABLE_ICON_SIZE)?,
    ));
    icons.push((
        APPLE_TOUCH_ICON_FILE_NAME.to_owned(),
        render_png(&maskable, APPLE_TOUCH_ICON_SIZE)?,
    ));
    Ok(icons)
}

pub fn render_png(tree: &usvg::Tree, size: u32) -> Result<Vec<u8>> {
    render_png_with_margin(tree, size, 0.0)
}

/// The picture drawn `margin_fraction` of `size` away from each edge, on a transparent canvas.
pub fn render_png_with_margin(
    tree: &usvg::Tree,
    size: u32,
    margin_fraction: f32,
) -> Result<Vec<u8>> {
    let mut pixmap = Pixmap::new(size, size).ok_or_else(|| anyhow!("empty icon size"))?;
    // Icon sizes are small integers: the f32 conversion is exact.
    #[allow(clippy::cast_precision_loss)]
    let size_in_pixels = size as f32;
    let margin = size_in_pixels * margin_fraction;
    let scale = (size_in_pixels - 2.0 * margin) / tree.size().width();
    resvg::render(
        tree,
        Transform::from_row(scale, 0.0, 0.0, scale, margin, margin),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .with_context(|| format!("cannot encode the {size} px icon"))
}

/// Whether a PNG holds only what the encoder writes (`IHDR`, `IDAT`, `IEND`) and nothing after
/// its end: pixels within tolerance must not let other bytes hide in a committed icon.
fn has_only_image_chunks(png: &[u8]) -> bool {
    const SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
    const LENGTH_AND_TYPE: usize = 8;
    const CHECKSUM: usize = 4;
    let Some(mut rest) = png.strip_prefix(SIGNATURE) else {
        return false;
    };
    while let Some(header) = rest.get(..LENGTH_AND_TYPE) {
        let Ok(Ok(length)) = header[..4]
            .try_into()
            .map(u32::from_be_bytes)
            .map(usize::try_from)
        else {
            return false;
        };
        let kind = &header[4..];
        if !matches!(kind, b"IHDR" | b"IDAT" | b"IEND") {
            return false;
        }
        let Some(after_chunk) = rest.get(LENGTH_AND_TYPE + length + CHECKSUM..) else {
            return false;
        };
        if kind == b"IEND" {
            return after_chunk.is_empty();
        }
        rest = after_chunk;
    }
    false
}

/// Whether a committed icon matches a fresh render: only image chunks, and pixels within the
/// rasteriser's rounding. A missing or malformed icon is out of date.
pub fn is_icon_up_to_date(committed: &[u8], generated: &[u8]) -> Result<bool> {
    if committed == generated {
        return Ok(true);
    }
    are_pngs_alike(committed, generated)
}

fn are_pngs_alike(committed: &[u8], generated: &[u8]) -> Result<bool> {
    if !has_only_image_chunks(committed) {
        return Ok(false);
    }
    let Ok(committed) = Pixmap::decode_png(committed) else {
        return Ok(false);
    };
    let generated = Pixmap::decode_png(generated).context("the generated icon is not a PNG")?;
    if committed.width() != generated.width() || committed.height() != generated.height() {
        return Ok(false);
    }
    Ok(committed
        .data()
        .iter()
        .zip(generated.data())
        .all(|(a, b)| a.abs_diff(*b) <= MAXIMUM_CHANNEL_DIFFERENCE_ON_CHECK))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10"><rect width="10" height="10" fill="#3F332D"/></svg>"##;

    #[test]
    fn the_web_app_gets_192_512_maskable_and_apple_icons() {
        let rounded = SQUARE.replace("<rect ", r#"<rect rx="3" "#);
        let icons = generate_web_app_icons(&rounded, SQUARE).unwrap();
        let size_of = |name: &str| {
            let (_, png) = icons.iter().find(|(file, _)| file == name).unwrap();
            Pixmap::decode_png(png).unwrap()
        };
        assert_eq!(size_of("mellow-192.png").width(), 192);
        assert_eq!(size_of("mellow-512.png").width(), 512);
        assert_eq!(size_of(APPLE_TOUCH_ICON_FILE_NAME).width(), 180);
        // Rounded corners stay transparent on the plain icon; the maskable one fills them.
        assert_eq!(size_of("mellow-512.png").pixel(0, 0).unwrap().alpha(), 0);
        let maskable = size_of(&maskable_png_file_name(WEB_MASKABLE_ICON_SIZE));
        assert_eq!(maskable.pixel(0, 0).unwrap().alpha(), u8::MAX);
    }

    #[test]
    fn small_rounding_differences_pass_the_check_but_other_artwork_does_not() {
        let icons = generate_web_app_icons(SQUARE, SQUARE).unwrap();
        let (_, png) = &icons[0];
        let mut nudged = Pixmap::decode_png(png).unwrap();
        nudged.data_mut()[0] = nudged.data()[0].saturating_sub(1);
        assert!(is_icon_up_to_date(png, &nudged.encode_png().unwrap()).unwrap());

        let other_square = SQUARE.replace("#3F332D", "#FFFAF3");
        let other = generate_web_app_icons(&other_square, &other_square).unwrap();
        assert!(!is_icon_up_to_date(png, &other[0].1).unwrap());
        assert!(!is_icon_up_to_date(&[], png).unwrap());
        assert!(!is_icon_up_to_date(b"not a picture", png).unwrap());
    }

    #[test]
    fn bytes_hidden_in_an_icon_with_the_right_pixels_fail_the_check() {
        let icons = generate_web_app_icons(SQUARE, SQUARE).unwrap();
        let (_, png) = &icons[0];
        let mut appended = png.clone();
        appended.extend_from_slice(b"hidden");
        assert!(!is_icon_up_to_date(&appended, png).unwrap());

        // A text chunk right after IHDR (8-byte signature, 25-byte IHDR chunk).
        let mut with_text = png[..33].to_vec();
        with_text.extend_from_slice(&[0, 0, 0, 1]);
        with_text.extend_from_slice(b"tEXtx");
        with_text.extend_from_slice(&[0; 4]);
        with_text.extend_from_slice(&png[33..]);
        assert!(!is_icon_up_to_date(&with_text, png).unwrap());
    }
}
