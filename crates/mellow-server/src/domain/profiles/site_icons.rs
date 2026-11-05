//! A site's icon, fetched by the server for profile links (ADR-0050): the icon its home page
//! names, else `/favicon.ico`. Browsers never ask the site themselves.

use url::Url;

use crate::domain::link_previews::fetcher::{FetchError, Fetched, LinkFetcher};

const ICO_HEADER_BYTES: usize = 6;
const ICO_ENTRY_BYTES: usize = 16;
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];

/// The bytes of `host`'s icon, not yet checked as a picture; `None` when the site has none the
/// picture pipeline can read.
///
/// # Errors
/// The last fetch's error when it may succeed later (`FetchError::is_transient`), so the caller
/// tries again sooner than for a site that simply has no icon.
pub(super) async fn fetch_site_icon(
    fetcher: &LinkFetcher,
    host: &str,
) -> Result<Option<Vec<u8>>, FetchError> {
    let home = Url::parse(&format!("https://{host}/")).map_err(|_| FetchError::NotAllowed)?;
    let mut transient_error = None;
    match fetcher.fetch_page_or_picture(home.clone()).await {
        Ok(Fetched::Page(details)) => {
            if let Some(icon_url) = details.icon_url {
                match fetcher.fetch_picture(icon_url).await {
                    Ok(bytes) => {
                        if let Some(picture) = readable_picture(bytes) {
                            return Ok(Some(picture));
                        }
                    }
                    Err(error) if error.is_transient() => transient_error = Some(error),
                    Err(_) => {}
                }
            }
        }
        Err(error) if error.is_transient() => transient_error = Some(error),
        Ok(Fetched::Picture(_)) | Err(_) => {}
    }
    let favicon = home
        .join("/favicon.ico")
        .map_err(|_| FetchError::NotAllowed)?;
    match fetcher.fetch_picture(favicon).await {
        Ok(bytes) => Ok(readable_picture(bytes)),
        Err(error) if error.is_transient() => Err(error),
        Err(_) => transient_error.map_or(Ok(None), Err),
    }
}

/// The picture to process: an ICO file's largest PNG image, `None` for an ICO holding only
/// bitmaps (the pipeline reads no BMP), anything else as it is.
fn readable_picture(bytes: Vec<u8>) -> Option<Vec<u8>> {
    if bytes.starts_with(&[0, 0, 1, 0]) {
        largest_png_in_ico(&bytes).map(<[u8]>::to_vec)
    } else {
        Some(bytes)
    }
}

/// The largest PNG image an ICO file holds; every offset and length is checked against the file.
fn largest_png_in_ico(file: &[u8]) -> Option<&[u8]> {
    let count = usize::from(u16::from_le_bytes([*file.get(4)?, *file.get(5)?]));
    let entries = file.get(ICO_HEADER_BYTES..ICO_HEADER_BYTES + count * ICO_ENTRY_BYTES)?;
    entries
        .as_chunks::<ICO_ENTRY_BYTES>()
        .0
        .iter()
        .filter_map(|entry| {
            // 0 stands for 256 pixels.
            let width = match entry[0] {
                0 => 256,
                width => u32::from(width),
            };
            let length = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]);
            let offset = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]);
            let start = usize::try_from(offset).ok()?;
            let end = start.checked_add(usize::try_from(length).ok()?)?;
            let image = file.get(start..end)?;
            image.starts_with(&PNG_SIGNATURE).then_some((width, image))
        })
        .max_by_key(|(width, _)| *width)
        .map(|(_, image)| image)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An ICO file holding these images, each with its declared width.
    pub(crate) fn ico_file(images: &[(u8, &[u8])]) -> Vec<u8> {
        let mut file = vec![0, 0, 1, 0];
        file.extend_from_slice(&u16::try_from(images.len()).unwrap().to_le_bytes());
        let mut offset = ICO_HEADER_BYTES + images.len() * ICO_ENTRY_BYTES;
        for (width, image) in images {
            file.extend_from_slice(&[*width, *width, 0, 0, 1, 0, 32, 0]);
            file.extend_from_slice(&u32::try_from(image.len()).unwrap().to_le_bytes());
            file.extend_from_slice(&u32::try_from(offset).unwrap().to_le_bytes());
            offset += image.len();
        }
        for (_, image) in images {
            file.extend_from_slice(image);
        }
        file
    }

    #[test]
    fn the_largest_png_of_an_ico_file_is_kept_and_bitmaps_or_broken_entries_give_nothing() {
        let small: Vec<u8> = PNG_SIGNATURE.iter().copied().chain([1]).collect();
        let large: Vec<u8> = PNG_SIGNATURE.iter().copied().chain([2, 2]).collect();
        let bitmap = [40_u8, 0, 0, 0];
        let file = ico_file(&[(16, &small), (0, &large), (32, &bitmap)]);
        assert_eq!(readable_picture(file), Some(large));
        assert_eq!(readable_picture(ico_file(&[(16, &bitmap)])), None);
        let mut truncated = ico_file(&[(16, &small)]);
        truncated.pop();
        assert_eq!(readable_picture(truncated), None);
        let mut too_many_entries = ico_file(&[(16, &small)]);
        too_many_entries[4] = 0xFF;
        assert_eq!(readable_picture(too_many_entries), None);
        assert_eq!(readable_picture(small.clone()), Some(small));
    }
}
