//! App icons (DESIGN.md §2.10): the favicon (Melli on its dusk tile) rasterised to PNGs, and a macOS
//! `.icns` built from them, so every icon comes from the tokens like the favicon does.

use anyhow::{Context as _, Result, anyhow};
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;

/// Square PNG sizes in `release/icons/`, full bleed: the tile fills the square.
/// 256 is the window icon and the Linux menu icon.
const PNG_SIZES: [u32; 7] = [16, 32, 48, 64, 128, 256, 512];
/// macOS draws app icons on a 1024 grid with the artwork inside 824 units, so Melli sits as large
/// in the Dock as the other apps' icons.
const MACOS_ARTWORK_FRACTION: f32 = 824.0 / 1024.0;
/// `.icns` chunk types holding PNG data, with their pixel size (1x and 2x, 16 to 512 points).
const ICNS_CHUNKS: [(&[u8; 4], u32); 8] = [
    (b"ic11", 32),
    (b"ic12", 64),
    (b"ic07", 128),
    (b"ic13", 256),
    (b"ic08", 256),
    (b"ic14", 512),
    (b"ic09", 512),
    (b"ic10", 1024),
];
const ICNS_HEADER_LENGTH: usize = 8;
/// Rasterisers may round a few pixels differently on another CPU (CI runs on `x86_64`): `--check`
/// accepts that much difference per channel instead of comparing bytes.
pub const MAXIMUM_CHANNEL_DIFFERENCE_ON_CHECK: u8 = 2;

pub const ICONS_DIRECTORY: &str = "release/icons";
pub const ICNS_FILE_NAME: &str = "Mellow.icns";

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
        icons.push((png_file_name(size), render_png(&favicon, size, 1.0)?));
    }
    icons.push((
        maskable_png_file_name(WEB_MASKABLE_ICON_SIZE),
        render_png(&maskable, WEB_MASKABLE_ICON_SIZE, 1.0)?,
    ));
    icons.push((
        APPLE_TOUCH_ICON_FILE_NAME.to_owned(),
        render_png(&maskable, APPLE_TOUCH_ICON_SIZE, 1.0)?,
    ));
    Ok(icons)
}

/// Every icon file, as (path under `ICONS_DIRECTORY`, contents).
pub fn generate_app_icons(favicon_svg: &str) -> Result<Vec<(String, Vec<u8>)>> {
    let tree = usvg::Tree::from_str(favicon_svg, &usvg::Options::default())
        .context("the favicon is not valid SVG")?;
    let mut icons = Vec::new();
    for size in PNG_SIZES {
        icons.push((png_file_name(size), render_png(&tree, size, 1.0)?));
    }
    let mut chunks = Vec::new();
    for (kind, size) in ICNS_CHUNKS {
        chunks.push((*kind, render_png(&tree, size, MACOS_ARTWORK_FRACTION)?));
    }
    icons.push((ICNS_FILE_NAME.to_owned(), icns_from_chunks(&chunks)?));
    Ok(icons)
}

fn render_png(tree: &usvg::Tree, size: u32, artwork_fraction: f32) -> Result<Vec<u8>> {
    let mut pixmap = Pixmap::new(size, size).ok_or_else(|| anyhow!("empty icon size"))?;
    // Icon sizes are small integers: the f32 conversion is exact.
    #[allow(clippy::cast_precision_loss)]
    let canvas = size as f32;
    let artwork = canvas * artwork_fraction;
    let scale = artwork / tree.size().width();
    let margin = (canvas - artwork) / 2.0;
    resvg::render(
        tree,
        Transform::from_row(scale, 0.0, 0.0, scale, margin, margin),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .with_context(|| format!("cannot encode the {size} px icon"))
}

fn icns_from_chunks(chunks: &[([u8; 4], Vec<u8>)]) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    for (kind, png) in chunks {
        body.extend_from_slice(kind);
        body.extend_from_slice(&length_field(ICNS_HEADER_LENGTH + png.len())?);
        body.extend_from_slice(png);
    }
    let mut icns = b"icns".to_vec();
    icns.extend_from_slice(&length_field(ICNS_HEADER_LENGTH + body.len())?);
    icns.extend_from_slice(&body);
    Ok(icns)
}

fn length_field(length: usize) -> Result<[u8; 4]> {
    Ok(u32::try_from(length)
        .context("an .icns chunk is over 4 GiB")?
        .to_be_bytes())
}

/// The chunks of a well-formed `.icns`: its length field is the file's length and the chunks
/// fill it exactly. `None` for anything else.
fn icns_chunks(icns: &[u8]) -> Option<Vec<([u8; 4], &[u8])>> {
    let total_length = u32::from_be_bytes(icns.get(4..ICNS_HEADER_LENGTH)?.try_into().ok()?);
    if !icns.starts_with(b"icns") || usize::try_from(total_length).ok()? != icns.len() {
        return None;
    }
    let mut chunks = Vec::new();
    let mut rest = &icns[ICNS_HEADER_LENGTH..];
    while !rest.is_empty() {
        let kind: [u8; 4] = rest.get(..4)?.try_into().ok()?;
        let length = u32::from_be_bytes(rest.get(4..ICNS_HEADER_LENGTH)?.try_into().ok()?);
        let length = usize::try_from(length).ok()?;
        chunks.push((kind, rest.get(ICNS_HEADER_LENGTH..length)?));
        rest = &rest[length..];
    }
    Some(chunks)
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

/// Whether a committed icon matches a fresh render: the same chunks, and pixels within the
/// rasteriser's rounding. A missing or malformed icon is out of date.
pub fn is_icon_up_to_date(file_name: &str, committed: &[u8], generated: &[u8]) -> Result<bool> {
    if committed == generated {
        return Ok(true);
    }
    if file_name != ICNS_FILE_NAME {
        return are_pngs_alike(committed, generated);
    }
    let generated_chunks =
        icns_chunks(generated).ok_or_else(|| anyhow!("the generated .icns is malformed"))?;
    let Some(committed_chunks) = icns_chunks(committed) else {
        return Ok(false);
    };
    if committed_chunks.len() != generated_chunks.len() {
        return Ok(false);
    }
    for ((committed_kind, committed_png), (generated_kind, generated_png)) in
        committed_chunks.iter().zip(&generated_chunks)
    {
        if committed_kind != generated_kind || !are_pngs_alike(committed_png, generated_png)? {
            return Ok(false);
        }
    }
    Ok(true)
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
    fn writes_every_png_size_and_an_icns_with_its_chunks_in_order() {
        let generated_icons = generate_app_icons(SQUARE).unwrap();
        for size in PNG_SIZES {
            let (_, png) = generated_icons
                .iter()
                .find(|(name, _)| *name == png_file_name(size))
                .unwrap();
            let pixmap = Pixmap::decode_png(png).unwrap();
            assert_eq!((pixmap.width(), pixmap.height()), (size, size));
        }
        let (_, icns) = generated_icons
            .iter()
            .find(|(name, _)| name == ICNS_FILE_NAME)
            .unwrap();
        let chunks = icns_chunks(icns).unwrap();
        let kinds: Vec<[u8; 4]> = chunks.iter().map(|(kind, _)| *kind).collect();
        let expected: Vec<[u8; 4]> = ICNS_CHUNKS.iter().map(|(kind, _)| **kind).collect();
        assert_eq!(kinds, expected);
        let total_length = u32::from_be_bytes(icns[4..8].try_into().unwrap());
        assert_eq!(usize::try_from(total_length).unwrap(), icns.len());
    }

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
    fn the_macos_icon_keeps_apples_margin() {
        let generated_icons = generate_app_icons(SQUARE).unwrap();
        let (_, icns) = generated_icons
            .iter()
            .find(|(name, _)| name == ICNS_FILE_NAME)
            .unwrap();
        let (_, largest) = *icns_chunks(icns).unwrap().last().unwrap();
        let pixmap = Pixmap::decode_png(largest).unwrap();
        // 100 px margin on the 1024 grid: transparent there, artwork just inside.
        assert_eq!(pixmap.pixel(50, 512).unwrap().alpha(), 0);
        assert_eq!(pixmap.pixel(110, 512).unwrap().alpha(), u8::MAX);
    }

    #[test]
    fn small_rounding_differences_pass_the_check_but_other_artwork_does_not() {
        let generated_icons = generate_app_icons(SQUARE).unwrap();
        let (name, png) = &generated_icons[0];
        let mut nudged = Pixmap::decode_png(png).unwrap();
        nudged.data_mut()[0] = nudged.data()[0].saturating_sub(1);
        assert!(is_icon_up_to_date(name, png, &nudged.encode_png().unwrap()).unwrap());

        let other = generate_app_icons(&SQUARE.replace("#3F332D", "#FFFAF3")).unwrap();
        assert!(!is_icon_up_to_date(name, png, &other[0].1).unwrap());
        let (icns_name, icns) = generated_icons.last().unwrap();
        assert!(!is_icon_up_to_date(icns_name, icns, &other.last().unwrap().1).unwrap());
        assert!(!is_icon_up_to_date(name, &[], png).unwrap());
        assert!(!is_icon_up_to_date(name, b"not a picture", png).unwrap());
    }

    #[test]
    fn bytes_hidden_in_an_icon_with_the_right_pixels_fail_the_check() {
        let generated_icons = generate_app_icons(SQUARE).unwrap();
        let (name, png) = &generated_icons[0];
        let mut appended = png.clone();
        appended.extend_from_slice(b"hidden");
        assert!(!is_icon_up_to_date(name, &appended, png).unwrap());

        // A text chunk right after IHDR (8-byte signature, 25-byte IHDR chunk).
        let mut with_text = png[..33].to_vec();
        with_text.extend_from_slice(&[0, 0, 0, 1]);
        with_text.extend_from_slice(b"tEXtx");
        with_text.extend_from_slice(&[0; 4]);
        with_text.extend_from_slice(&png[33..]);
        assert!(!is_icon_up_to_date(name, &with_text, png).unwrap());

        let (icns_name, icns) = generated_icons.last().unwrap();
        let mut longer_icns = icns.clone();
        longer_icns.extend_from_slice(b"hidden");
        assert!(!is_icon_up_to_date(icns_name, &longer_icns, icns).unwrap());
        let mut swollen_icns = longer_icns.clone();
        let swollen_length = u32::try_from(swollen_icns.len()).unwrap();
        swollen_icns[4..8].copy_from_slice(&swollen_length.to_be_bytes());
        assert!(!is_icon_up_to_date(icns_name, &swollen_icns, icns).unwrap());
    }
}
