//! App icons (DESIGN.md §2.10): the favicon's still idle Melli rasterised to PNGs, and a macOS
//! `.icns` built from them, so every icon comes from the tokens like the favicon does.

use anyhow::{Context as _, Result, anyhow, bail, ensure};
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;

/// Square PNG sizes in `release/icons/`, full bleed: the drawing already has its own margin.
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

pub fn png_file_name(size: u32) -> String {
    format!("mellow-{size}.png")
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

fn icns_chunks(icns: &[u8]) -> Result<Vec<([u8; 4], &[u8])>> {
    ensure!(icns.starts_with(b"icns"), "not an .icns file");
    let mut chunks = Vec::new();
    let mut rest = icns
        .get(ICNS_HEADER_LENGTH..)
        .ok_or_else(|| anyhow!("truncated .icns header"))?;
    while !rest.is_empty() {
        let header = rest
            .get(..ICNS_HEADER_LENGTH)
            .ok_or_else(|| anyhow!("truncated .icns chunk header"))?;
        let kind: [u8; 4] = header[..4].try_into()?;
        let length = usize::try_from(u32::from_be_bytes(header[4..].try_into()?))?;
        let chunk = rest
            .get(ICNS_HEADER_LENGTH..length)
            .ok_or_else(|| anyhow!("truncated .icns chunk"))?;
        chunks.push((kind, chunk));
        rest = &rest[length..];
    }
    Ok(chunks)
}

/// Whether a committed icon matches a fresh render, within the rasteriser's rounding.
pub fn is_icon_up_to_date(file_name: &str, committed: &[u8], generated: &[u8]) -> Result<bool> {
    if committed == generated {
        return Ok(true);
    }
    if committed.is_empty() {
        return Ok(false);
    }
    if file_name == ICNS_FILE_NAME {
        let (committed_chunks, generated_chunks) =
            (icns_chunks(committed)?, icns_chunks(generated)?);
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
        return Ok(true);
    }
    are_pngs_alike(committed, generated)
}

fn are_pngs_alike(committed: &[u8], generated: &[u8]) -> Result<bool> {
    let committed = Pixmap::decode_png(committed).context("a committed icon is not a PNG")?;
    let generated = Pixmap::decode_png(generated)?;
    if committed.width() != generated.width() || committed.height() != generated.height() {
        return Ok(false);
    }
    Ok(committed
        .data()
        .iter()
        .zip(generated.data())
        .all(|(a, b)| a.abs_diff(*b) <= MAXIMUM_CHANNEL_DIFFERENCE_ON_CHECK))
}

pub fn check_icon_is_up_to_date(file_name: &str, committed: &[u8], generated: &[u8]) -> Result<()> {
    if !is_icon_up_to_date(file_name, committed, generated)? {
        bail!("{ICONS_DIRECTORY}/{file_name} is out of date; run `cargo xtask tokens`");
    }
    Ok(())
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
    }
}
