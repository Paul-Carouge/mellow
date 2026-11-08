//! GIFs from Klipy (ADR-0042): `lib/klipy.ts` (request addresses, the checks on every answer, the
//! rule that shows a message holding only a Klipy media address as the GIF itself) and the
//! picker's rules of `components/gif-picker.ts` (arrow keys over justified rows, the favourites'
//! order and paging, a tile's shape). Klipy's answers are untrusted: a field that is missing or of
//! another shape skips its item, and only media from Klipy's own hosts is ever kept.
//!
//! Left out on purpose: the blurred previews (base64 pictures the desktop does not draw yet).

use mellow_protocol::GifSettings;
use serde_json::Value;
use url::Url;

const KLIPY_API_ORIGIN: &str = "https://api.klipy.com";
/// The hosts the web's Content-Security-Policy opens when GIFs are on (ADR-0042).
const KLIPY_MEDIA_HOSTS: [&str; 3] = ["static.klipy.com", "static1.klipy.com", "static2.klipy.com"];
/// One page of the picker: Klipy's own default, three or four rows of tiles.
pub const KLIPY_PAGE_SIZE: usize = 24;
/// Klipy's slugs, as the instance accepts them for favourites (`domain::gifs::validate_slug`).
const MAXIMUM_SLUG_LENGTH: usize = 64;
const MAXIMUM_TITLE_LENGTH: usize = 200;
/// Klipy's `locale` when the reader's gives no two-letter code.
const DEFAULT_KLIPY_LOCALE: &str = "us";

/// A picture or clip on Klipy's hosts, with the size Klipy gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KlipyMedia {
    pub url: String,
    pub width: u32,
    pub height: u32,
    pub is_video: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KlipyGif {
    pub slug: String,
    /// Klipy's title, or empty: the picker names untitled GIFs itself.
    pub title: String,
    /// The web's tile: small animated WebP, or MP4, or GIF.
    pub tile: KlipyMedia,
    /// The tile for a client that plays no video: the web's choices without the MP4.
    pub picture_tile: Option<KlipyMedia>,
    /// The small JPEG, shown while animated pictures are kept still.
    pub still: Option<KlipyMedia>,
    /// What a message carries (ADR-0042): the medium WebP, else the medium GIF, else a small one.
    pub message_url: String,
}

/// An advertisement Klipy may slip in when ads are on for the key (they are off on Mellow's).
/// Its HTML is never kept: it keeps its place as a tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KlipyAd {
    /// Its place in the answer.
    pub position: usize,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KlipyItem {
    Gif(KlipyGif),
    Ad(KlipyAd),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KlipyPage {
    /// In Klipy's order, never filtered or reordered beyond dropping what cannot be shown.
    pub items: Vec<KlipyItem>,
    pub has_next: bool,
}

fn positive_integer(value: Option<&Value>) -> Option<u32> {
    let value = value?;
    let number = value.as_u64().or_else(|| {
        // JSON's `220.0` is an integer for JavaScript's `Number.isInteger` too.
        let float = value.as_f64()?;
        // Checked just above: a whole number within u32.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        (float.fract() == 0.0 && float > 0.0 && float <= f64::from(u32::MAX))
            .then_some(float as u64)
    })?;
    u32::try_from(number).ok().filter(|number| *number > 0)
}

/// The address itself when it is `https` on one of Klipy's media hosts, without credentials or
/// port.
#[must_use]
pub fn klipy_media_url(candidate: &str) -> Option<String> {
    let url = Url::parse(candidate).ok()?;
    let is_klipy_media = url.scheme() == "https"
        && url
            .host_str()
            .is_some_and(|host| KLIPY_MEDIA_HOSTS.contains(&host))
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none();
    is_klipy_media.then(|| url.to_string())
}

const VIDEO_EXTENSIONS: [&str; 2] = [".mp4", ".webm"];
const PICTURE_EXTENSIONS: [&str; 5] = [".webp", ".gif", ".jpg", ".jpeg", ".png"];

/// The path's extension, lowercased, with its dot; empty without one.
fn extension_of(url: &str) -> String {
    let path = Url::parse(url)
        .map(|url| url.path().to_lowercase())
        .unwrap_or_default();
    path.rfind('.')
        .map(|dot| path[dot..].to_owned())
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy)]
enum KlipySize {
    Md,
    Sm,
    Xs,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KlipyFormat {
    Gif,
    Webp,
    Mp4,
    Jpg,
}

impl KlipySize {
    fn key(self) -> &'static str {
        match self {
            Self::Md => "md",
            Self::Sm => "sm",
            Self::Xs => "xs",
        }
    }
}

impl KlipyFormat {
    fn key(self) -> &'static str {
        match self {
            Self::Gif => "gif",
            Self::Webp => "webp",
            Self::Mp4 => "mp4",
            Self::Jpg => "jpg",
        }
    }
}

fn format_of(file: &Value, size: KlipySize, format: KlipyFormat) -> Option<KlipyMedia> {
    let media = file.get(size.key())?.get(format.key())?;
    if !media.is_object() {
        return None;
    }
    Some(KlipyMedia {
        url: klipy_media_url(media.get("url")?.as_str()?)?,
        width: positive_integer(media.get("width"))?,
        height: positive_integer(media.get("height"))?,
        is_video: format == KlipyFormat::Mp4,
    })
}

/// The first of `choices` the file has, in order of preference.
fn first_format(file: &Value, choices: &[(KlipySize, KlipyFormat)]) -> Option<KlipyMedia> {
    choices
        .iter()
        .find_map(|(size, format)| format_of(file, *size, *format))
}

// The picker's tiles are about 100 px high: the small WebP is sharp on dense screens and an
// eighth of the medium one's weight; the extra small one only when nothing else is there.
const TILE_CHOICES: [(KlipySize, KlipyFormat); 5] = [
    (KlipySize::Sm, KlipyFormat::Webp),
    (KlipySize::Sm, KlipyFormat::Mp4),
    (KlipySize::Sm, KlipyFormat::Gif),
    (KlipySize::Xs, KlipyFormat::Webp),
    (KlipySize::Md, KlipyFormat::Webp),
];
const PICTURE_TILE_CHOICES: [(KlipySize, KlipyFormat); 4] = [
    (KlipySize::Sm, KlipyFormat::Webp),
    (KlipySize::Sm, KlipyFormat::Gif),
    (KlipySize::Xs, KlipyFormat::Webp),
    (KlipySize::Md, KlipyFormat::Webp),
];
const STILL_CHOICES: [(KlipySize, KlipyFormat); 3] = [
    (KlipySize::Sm, KlipyFormat::Jpg),
    (KlipySize::Xs, KlipyFormat::Jpg),
    (KlipySize::Md, KlipyFormat::Jpg),
];
const MESSAGE_CHOICES: [(KlipySize, KlipyFormat); 4] = [
    (KlipySize::Md, KlipyFormat::Webp),
    (KlipySize::Md, KlipyFormat::Gif),
    (KlipySize::Sm, KlipyFormat::Webp),
    (KlipySize::Sm, KlipyFormat::Gif),
];

/// `[A-Za-z0-9_-]{1,64}`.
fn is_klipy_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= MAXIMUM_SLUG_LENGTH
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn parse_gif(item: &Value) -> Option<KlipyGif> {
    let slug = item
        .get("slug")?
        .as_str()
        .filter(|slug| is_klipy_slug(slug))?;
    let file = item.get("file").filter(|file| file.is_object())?;
    let web_tile = first_format(file, &TILE_CHOICES)?;
    let message = first_format(file, &MESSAGE_CHOICES)?;
    let title = item
        .get("title")
        .and_then(Value::as_str)
        .map(|title| title.trim().chars().take(MAXIMUM_TITLE_LENGTH).collect())
        .unwrap_or_default();
    Some(KlipyGif {
        slug: slug.to_owned(),
        title,
        tile: web_tile,
        picture_tile: first_format(file, &PICTURE_TILE_CHOICES),
        still: first_format(file, &STILL_CHOICES),
        message_url: message.url,
    })
}

/// Each item checked on its own: one broken item never hides the others.
#[must_use]
pub fn parse_klipy_items(items: &Value) -> Vec<KlipyItem> {
    let Some(items) = items.as_array() else {
        return Vec::new();
    };
    items
        .iter()
        .enumerate()
        .filter(|(_, item)| item.is_object())
        .filter_map(|(position, item)| match item.get("type") {
            Some(kind) if kind == "ad" => Some(KlipyItem::Ad(KlipyAd {
                position,
                width: positive_integer(item.get("width")),
                height: positive_integer(item.get("height")),
            })),
            // Absent on some answers (items by slug): a GIF endpoint answers GIFs.
            None => parse_gif(item).map(KlipyItem::Gif),
            Some(kind) if kind == "gif" => parse_gif(item).map(KlipyItem::Gif),
            Some(_) => None,
        })
        .collect()
}

/// `{ result: true, data: { data: [...], has_next } }`, or `None` for any other shape.
#[must_use]
pub fn parse_klipy_page(body: &Value) -> Option<KlipyPage> {
    if body.get("result") != Some(&Value::Bool(true)) {
        return None;
    }
    let data = body.get("data").filter(|data| data.is_object())?;
    let items = data.get("data").filter(|items| items.is_array())?;
    Some(KlipyPage {
        items: parse_klipy_items(items),
        has_next: data.get("has_next") == Some(&Value::Bool(true)),
    })
}

fn is_two_letters(code: &str) -> bool {
    code.len() == 2 && code.bytes().all(|byte| byte.is_ascii_alphabetic())
}

/// Klipy's `locale` is a country code ("fr", "us", "gb"): the region of the reader's locale when
/// it has one, else their language. Takes BCP 47 (`fr-CA`) and POSIX (`fr_FR.UTF-8`) tags.
#[must_use]
pub fn klipy_locale_of(language_tag: &str) -> String {
    let tag = language_tag.split(['.', '@']).next().unwrap_or_default();
    let mut subtags = tag.split(['-', '_']);
    let language = subtags.next().unwrap_or_default();
    let code = match subtags.next() {
        Some(region) if is_two_letters(region) => region,
        _ => language,
    };
    if is_two_letters(code) {
        code.to_ascii_lowercase()
    } else {
        DEFAULT_KLIPY_LOCALE.to_owned()
    }
}

/// `https://api.klipy.com/api/v1/{key}/gifs/{path…}?{query}`, each part percent-encoded.
fn api_url(settings: &GifSettings, path: &[&str], query: &[(&str, &str)]) -> String {
    let Ok(mut url) = Url::parse(KLIPY_API_ORIGIN) else {
        unreachable!("the constant origin parses");
    };
    if let Ok(mut segments) = url.path_segments_mut() {
        segments
            .clear()
            .extend(["api", "v1", settings.klipy_app_key.as_str(), "gifs"])
            .extend(path);
    }
    if !query.is_empty() {
        url.query_pairs_mut().extend_pairs(query);
    }
    url.to_string()
}

/// Whether `url` is one of the addresses built here (the API of Klipy, over https).
#[must_use]
pub fn is_klipy_api_url(url: &str) -> bool {
    url.starts_with(KLIPY_API_ORIGIN) && url[KLIPY_API_ORIGIN.len()..].starts_with("/api/v1/")
}

/// Trending, or a search when `query` is not blank; `page` counts from 1.
#[must_use]
pub fn klipy_list_url(settings: &GifSettings, query: &str, page: u32, locale: &str) -> String {
    let page = page.to_string();
    let per_page = KLIPY_PAGE_SIZE.to_string();
    let mut parameters = vec![
        ("page", page.as_str()),
        ("per_page", per_page.as_str()),
        ("customer_id", settings.customer_id.as_str()),
        ("locale", locale),
    ];
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return api_url(settings, &["trending"], &parameters);
    }
    parameters.push(("q", trimmed));
    api_url(settings, &["search"], &parameters)
}

/// Klipy's items endpoint takes slugs as one comma-separated parameter.
#[must_use]
pub fn klipy_items_url(settings: &GifSettings, slugs: &[String]) -> String {
    api_url(settings, &["items"], &[("slugs", &slugs.join(","))])
}

#[must_use]
pub fn klipy_share_url(settings: &GifSettings, slug: &str) -> String {
    api_url(settings, &["share", slug], &[])
}

/// The body of Klipy's share trigger: `q` is the search that found the GIF, empty from trending
/// or favourites.
#[must_use]
pub fn klipy_share_body(settings: &GifSettings, query: &str) -> Value {
    serde_json::json!({ "customer_id": settings.customer_id, "q": query.trim() })
}

/// A message shown as a GIF: its whole text is one address of Klipy's media (ADR-0042).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageGifMedia {
    pub url: String,
    pub is_video: bool,
}

/// The GIF a message is, or `None`: its trimmed text must be a single `https://static*.klipy.com/…`
/// address of a picture or clip. Anything more (words, a second link, `<…>`) keeps the link.
#[must_use]
pub fn message_gif_media_of(content: &str) -> Option<MessageGifMedia> {
    let text = content.trim();
    // Cheap first: almost no message starts so.
    if !text.starts_with("https://static") || text.chars().any(char::is_whitespace) {
        return None;
    }
    let url = klipy_media_url(text)?;
    let extension = extension_of(&url);
    if VIDEO_EXTENSIONS.contains(&extension.as_str()) {
        return Some(MessageGifMedia {
            url,
            is_video: true,
        });
    }
    PICTURE_EXTENSIONS
        .contains(&extension.as_str())
        .then_some(MessageGifMedia {
            url,
            is_video: false,
        })
}

// ---- The picker (`gif-picker.ts`) ----

/// A tile's place in the picker's sheet, in its own coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileBounds {
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GifTileKey {
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GifTileMove {
    Tile(usize),
    /// Up from the first row: back to the search field.
    Search,
}

/// Where an arrow key goes from tile `current`, among tiles laid out in justified rows in Klipy's
/// order: Left and Right step through that order, Up and Down reach the row above or below, the
/// tile whose middle is nearest; Home and End the first and last. `None` when the key does
/// nothing.
#[must_use]
pub fn next_gif_tile(
    bounds: &[TileBounds],
    current: usize,
    key: GifTileKey,
) -> Option<GifTileMove> {
    let here = bounds.get(current)?;
    let last = bounds.len() - 1;
    let is_up = match key {
        GifTileKey::Left => return (current > 0).then(|| GifTileMove::Tile(current - 1)),
        GifTileKey::Right => return (current < last).then(|| GifTileMove::Tile(current + 1)),
        GifTileKey::Home => return Some(GifTileMove::Tile(0)),
        GifTileKey::End => return Some(GifTileMove::Tile(last)),
        GifTileKey::Up => true,
        GifTileKey::Down => false,
    };
    let middle = here.left + here.width / 2.0;
    let half_row = here.height / 2.0;
    // Rows are told apart by their top; a tile is in another row when it starts half a row away.
    let other_row_tops = bounds.iter().map(|tile| tile.top).filter(|top| {
        if is_up {
            *top < here.top - half_row
        } else {
            *top > here.top + half_row
        }
    });
    let target_top = if is_up {
        other_row_tops.reduce(f32::max)
    } else {
        other_row_tops.reduce(f32::min)
    };
    let Some(target_top) = target_top else {
        return is_up.then_some(GifTileMove::Search);
    };
    bounds
        .iter()
        .enumerate()
        .filter(|(_, tile)| (tile.top - target_top).abs() <= half_row)
        .min_by(|(_, first), (_, second)| {
            let distance = |tile: &TileBounds| (tile.left + tile.width / 2.0 - middle).abs();
            distance(first).total_cmp(&distance(second))
        })
        .map(|(index, _)| GifTileMove::Tile(index))
}

/// Favourites come back from Klipy in its own order; the picker shows them newest starred first,
/// as the instance lists them. A slug Klipy no longer knows is left out.
#[must_use]
pub fn gifs_in_slug_order(items: &[KlipyItem], slugs: &[String]) -> Vec<KlipyGif> {
    slugs
        .iter()
        .filter_map(|slug| {
            items.iter().find_map(|item| match item {
                KlipyItem::Gif(gif) if gif.slug == *slug => Some(gif.clone()),
                _ => None,
            })
        })
        .collect()
}

/// The starred slugs after a star or an unstar: a new star goes first, as the instance lists them.
#[must_use]
pub fn with_favourite_set(slugs: &[String], slug: &str, is_starred: bool) -> Vec<String> {
    let others = slugs.iter().filter(|candidate| *candidate != slug).cloned();
    if is_starred {
        std::iter::once(slug.to_owned()).chain(others).collect()
    } else {
        others.collect()
    }
}

/// Wider or narrower than this, a tile is cropped rather than made a sliver or a whole row.
const MINIMUM_TILE_ASPECT_RATIO: f32 = 0.5;
const MAXIMUM_TILE_ASPECT_RATIO: f32 = 2.5;
/// Klipy's ads have no size of their own here: they take a square.
const AD_ASPECT_RATIO: f32 = 1.0;

/// Width over height of a tile in the justified rows.
#[must_use]
pub fn tile_aspect_ratio(item: &KlipyItem) -> f32 {
    let size = match item {
        KlipyItem::Gif(gif) => Some((gif.tile.width, gif.tile.height)),
        KlipyItem::Ad(ad) => ad.width.zip(ad.height),
    };
    // Sizes of a few hundred pixels: exact in f32.
    #[allow(clippy::cast_precision_loss)]
    let ratio = size.map_or(AD_ASPECT_RATIO, |(width, height)| {
        width as f32 / height as f32
    });
    ratio.clamp(MINIMUM_TILE_ASPECT_RATIO, MAXIMUM_TILE_ASPECT_RATIO)
}

/// The next page of favourites to ask Klipy for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FavouritesPage {
    pub slugs: Vec<String>,
    /// How many of the frozen list have been asked for, this page included.
    pub asked_count: usize,
    pub has_next: bool,
}

/// The next page of favourites, cut from the list as it was when the first page loaded: stars
/// changed while browsing reorder the live list, and paging over it would show a GIF twice.
#[must_use]
pub fn favourites_page(
    frozen_slugs: &[String],
    asked_count: usize,
    page_size: usize,
) -> FavouritesPage {
    let start = asked_count.min(frozen_slugs.len());
    let end = start.saturating_add(page_size).min(frozen_slugs.len());
    let slugs = frozen_slugs[start..end].to_vec();
    FavouritesPage {
        asked_count: end,
        has_next: end < frozen_slugs.len(),
        slugs,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const STATIC: &str = "https://static.klipy.com/ii/935d7ab9d8c6202580a668421940ec81/14/af";

    fn settings() -> GifSettings {
        GifSettings {
            klipy_app_key: "app key/1".to_owned(),
            customer_id: "0123456789abcdef0123456789abcdef".to_owned(),
        }
    }

    fn format(name: &str, width: u32, height: u32) -> Value {
        json!({ "url": format!("{STATIC}/{name}"), "width": width, "height": height, "size": 1000 })
    }

    /// Shaped as Klipy's documentation shows an item (docs.klipy.com, GIF Search API).
    fn klipy_item(slug: &str) -> Value {
        json!({
            "id": 8_041_071_659_142_944_u64,
            "slug": slug,
            "title": "Hello",
            "type": "gif",
            "blur_preview": "data:image/jpeg;base64,/9j//gAQTGF2YzU5==",
            "file": {
                "hd": { "gif": format("hd.gif", 498, 498), "webp": format("hd.webp", 498, 498) },
                "md": { "gif": format("md.gif", 220, 180), "webp": format("md.webp", 220, 180), "mp4": format("md.mp4", 220, 180), "jpg": format("md.jpg", 220, 180) },
                "sm": { "gif": format("sm.gif", 220, 180), "webp": format("sm.webp", 220, 180), "mp4": format("sm.mp4", 220, 180), "jpg": format("sm.jpg", 220, 180) },
                "xs": { "webp": format("xs.webp", 90, 74), "jpg": format("xs.jpg", 90, 74) },
            },
        })
    }

    fn with(mut item: Value, key: &str, value: Value) -> Value {
        if let Some(object) = item.as_object_mut() {
            if value.is_null() && key == "type" {
                object.remove(key);
            } else {
                object.insert(key.to_owned(), value);
            }
        }
        item
    }

    fn media(name: &str, is_video: bool) -> KlipyMedia {
        KlipyMedia {
            url: format!("{STATIC}/{name}"),
            width: 220,
            height: 180,
            is_video,
        }
    }

    fn slugs_of(items: &[KlipyItem]) -> Vec<&str> {
        items
            .iter()
            .map(|item| match item {
                KlipyItem::Gif(gif) => gif.slug.as_str(),
                KlipyItem::Ad(_) => "ad",
            })
            .collect()
    }

    fn gif_of(item: Option<&KlipyItem>) -> &KlipyGif {
        match item {
            Some(KlipyItem::Gif(gif)) => gif,
            other => panic!("expected a GIF, got {other:?}"),
        }
    }

    #[test]
    fn media_addresses_are_only_https_on_klipy_media_hosts() {
        assert_eq!(
            klipy_media_url(&format!("{STATIC}/a.webp")).as_deref(),
            Some(format!("{STATIC}/a.webp").as_str())
        );
        assert_eq!(
            klipy_media_url("https://static2.klipy.com/x.gif").as_deref(),
            Some("https://static2.klipy.com/x.gif")
        );
        for refused in [
            "http://static.klipy.com/x.gif",
            "https://static3.klipy.com/x.gif",
            "https://static.klipy.com.evil.example/x.gif",
            "https://evil.example/static.klipy.com/x.gif",
            "https://user:pass@static.klipy.com/x.gif",
            "https://static.klipy.com:8443/x.gif",
            "javascript:alert(1)",
            "/relative.gif",
        ] {
            assert_eq!(klipy_media_url(refused), None, "{refused}");
        }
    }

    #[test]
    fn a_page_is_read_in_klipys_order() {
        let page = parse_klipy_page(&json!({
            "result": true,
            "data": { "data": [klipy_item("hello-hi-662"), klipy_item("cat-2")], "current_page": 1, "per_page": 24, "has_next": true },
        }));
        let Some(page) = page else {
            panic!("the page parses");
        };
        assert!(page.has_next);
        assert_eq!(slugs_of(&page.items), ["hello-hi-662", "cat-2"]);
        assert_eq!(
            gif_of(page.items.first()),
            &KlipyGif {
                slug: "hello-hi-662".to_owned(),
                title: "Hello".to_owned(),
                tile: media("sm.webp", false),
                picture_tile: Some(media("sm.webp", false)),
                still: Some(media("sm.jpg", false)),
                message_url: format!("{STATIC}/md.webp"),
            }
        );
    }

    #[test]
    fn an_answer_of_another_shape_is_refused() {
        assert_eq!(parse_klipy_page(&Value::Null), None);
        assert_eq!(
            parse_klipy_page(&json!({ "result": false, "data": { "data": [] } })),
            None
        );
        assert_eq!(
            parse_klipy_page(&json!({ "result": true, "data": [] })),
            None
        );
        assert_eq!(
            parse_klipy_page(&json!({ "result": true, "data": { "data": [], "has_next": "yes" } })),
            Some(KlipyPage {
                items: Vec::new(),
                has_next: false
            })
        );
    }

    #[test]
    fn an_item_that_cannot_be_shown_is_skipped_never_the_others() {
        let items = parse_klipy_items(&json!([
            klipy_item("ok-1"),
            "not an object",
            klipy_item("bad slug!"),
            with(klipy_item("no-file"), "file", Value::Null),
            with(
                klipy_item("elsewhere"),
                "file",
                json!({ "md": { "webp": { "url": "https://evil.example/x.webp", "width": 1, "height": 1 } } })
            ),
            with(
                klipy_item("no-size"),
                "file",
                json!({ "md": { "webp": { "url": format!("{STATIC}/x.webp") } } })
            ),
            with(klipy_item("a-sticker"), "type", json!("sticker")),
            klipy_item("ok-2"),
        ]));
        assert_eq!(slugs_of(&items), ["ok-1", "ok-2"]);
    }

    #[test]
    fn an_ad_keeps_its_place_without_its_html() {
        let items = parse_klipy_items(&json!([
            klipy_item("a"),
            { "type": "ad", "content": "<script>x</script>", "width": 300, "height": 250 },
            klipy_item("b"),
        ]));
        assert_eq!(
            items.get(1),
            Some(&KlipyItem::Ad(KlipyAd {
                position: 1,
                width: Some(300),
                height: Some(250)
            }))
        );
    }

    #[test]
    fn messages_fall_back_from_the_medium_webp_to_the_medium_gif_then_a_small_one() {
        let without_medium_webp = with(
            klipy_item("a"),
            "file",
            json!({ "md": { "gif": format("md.gif", 220, 180) }, "sm": { "webp": format("sm.webp", 220, 180) } }),
        );
        let small_only = with(
            klipy_item("b"),
            "file",
            json!({ "sm": { "gif": format("sm.gif", 220, 180) } }),
        );
        let items = parse_klipy_items(&json!([without_medium_webp, small_only]));
        assert_eq!(
            gif_of(items.first()).message_url,
            format!("{STATIC}/md.gif")
        );
        let second = gif_of(items.get(1));
        assert_eq!(second.message_url, format!("{STATIC}/sm.gif"));
        assert_eq!(second.tile.url, format!("{STATIC}/sm.gif"));
    }

    #[test]
    fn a_tile_without_a_small_webp_uses_the_small_mp4_and_a_picture_for_clients_without_video() {
        let item = with(
            klipy_item("a"),
            "file",
            json!({ "md": { "webp": format("md.webp", 220, 180) }, "sm": { "mp4": format("sm.mp4", 220, 180) } }),
        );
        let items = parse_klipy_items(&json!([item]));
        let gif = gif_of(items.first());
        assert_eq!(gif.tile, media("sm.mp4", true));
        assert_eq!(gif.picture_tile, Some(media("md.webp", false)));
    }

    #[test]
    fn items_without_a_type_are_gifs_as_the_items_endpoint_sends_them() {
        let items = parse_klipy_items(&json!([with(klipy_item("a"), "type", Value::Null)]));
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn trending_or_a_search_is_asked_for() {
        let trending = Url::parse(&klipy_list_url(&settings(), "  ", 2, "fr")).ok();
        let Some(trending) = trending else {
            panic!("the address parses");
        };
        assert_eq!(
            format!(
                "{}{}",
                trending.origin().ascii_serialization(),
                trending.path()
            ),
            "https://api.klipy.com/api/v1/app%20key%2F1/gifs/trending"
        );
        let parameters: Vec<(String, String)> = trending.query_pairs().into_owned().collect();
        assert_eq!(
            parameters,
            [
                ("page".to_owned(), "2".to_owned()),
                ("per_page".to_owned(), KLIPY_PAGE_SIZE.to_string()),
                ("customer_id".to_owned(), settings().customer_id),
                ("locale".to_owned(), "fr".to_owned()),
            ]
        );
        let search = Url::parse(&klipy_list_url(&settings(), " chat & chien ", 1, "fr")).ok();
        let Some(search) = search else {
            panic!("the address parses");
        };
        assert_eq!(search.path(), "/api/v1/app%20key%2F1/gifs/search");
        assert!(
            search
                .query_pairs()
                .any(|(name, value)| name == "q" && value == "chat & chien")
        );
        assert!(is_klipy_api_url(search.as_str()));
        assert!(!is_klipy_api_url(
            "https://api.klipy.com.evil.example/api/v1/x"
        ));
    }

    #[test]
    fn items_are_asked_for_by_slugs_and_shared_by_slug() {
        let items = Url::parse(&klipy_items_url(
            &settings(),
            &["a-1".to_owned(), "b_2".to_owned()],
        ))
        .ok();
        assert!(items.is_some_and(|items| {
            items
                .query_pairs()
                .any(|(name, value)| name == "slugs" && value == "a-1,b_2")
        }));
        assert_eq!(
            klipy_share_url(&settings(), "hello-hi-662"),
            "https://api.klipy.com/api/v1/app%20key%2F1/gifs/share/hello-hi-662"
        );
        assert_eq!(
            klipy_share_body(&settings(), " hello "),
            json!({ "customer_id": settings().customer_id, "q": "hello" })
        );
    }

    #[test]
    fn klipy_gets_a_country_code_from_the_readers_locale() {
        assert_eq!(klipy_locale_of("fr-FR"), "fr");
        assert_eq!(klipy_locale_of("en-GB"), "gb");
        assert_eq!(klipy_locale_of("fr-CA"), "ca");
        assert_eq!(klipy_locale_of("fr"), "fr");
        assert_eq!(klipy_locale_of("zh-Hant-TW"), "zh");
        assert_eq!(klipy_locale_of(""), "us");
        assert_eq!(klipy_locale_of("fr_BE.UTF-8"), "be");
    }

    #[test]
    fn a_message_that_is_only_a_klipy_media_address_is_the_gif() {
        assert_eq!(
            message_gif_media_of(&format!("  {STATIC}/md.webp\n")),
            Some(MessageGifMedia {
                url: format!("{STATIC}/md.webp"),
                is_video: false
            })
        );
        assert_eq!(
            message_gif_media_of(&format!("{STATIC}/md.GIF")),
            Some(MessageGifMedia {
                url: format!("{STATIC}/md.GIF"),
                is_video: false
            })
        );
        assert_eq!(
            message_gif_media_of(&format!("{STATIC}/md.mp4")),
            Some(MessageGifMedia {
                url: format!("{STATIC}/md.mp4"),
                is_video: true
            })
        );
    }

    #[test]
    fn anything_more_or_anything_else_keeps_the_link() {
        for content in [
            format!("look {STATIC}/md.webp"),
            format!("{STATIC}/md.webp {STATIC}/sm.webp"),
            format!("<{STATIC}/md.webp>"),
            format!("{STATIC}/page.html"),
            format!("{STATIC}/no-extension"),
            "https://static.example.org/a.gif".to_owned(),
            "https://static.klipy.com.evil.example/a.gif".to_owned(),
            "http://static.klipy.com/a.gif".to_owned(),
            String::new(),
        ] {
            assert_eq!(message_gif_media_of(&content), None, "{content}");
        }
    }

    // ---- `gif-picker.test.ts` ----

    /// Two justified rows: [0 wide][1][2] above [3][4 wide].
    fn rows() -> Vec<TileBounds> {
        let tile = |left: f32, top: f32, width: f32| TileBounds {
            left,
            top,
            width,
            height: 96.0,
        };
        vec![
            tile(0.0, 0.0, 200.0),
            tile(204.0, 0.0, 100.0),
            tile(308.0, 0.0, 100.0),
            tile(0.0, 100.0, 100.0),
            tile(104.0, 100.0, 304.0),
        ]
    }

    #[test]
    fn left_and_right_step_through_klipys_order() {
        let rows = rows();
        assert_eq!(
            next_gif_tile(&rows, 2, GifTileKey::Right),
            Some(GifTileMove::Tile(3))
        );
        assert_eq!(
            next_gif_tile(&rows, 3, GifTileKey::Left),
            Some(GifTileMove::Tile(2))
        );
        assert_eq!(next_gif_tile(&rows, 0, GifTileKey::Left), None);
        assert_eq!(next_gif_tile(&rows, 4, GifTileKey::Right), None);
        assert_eq!(
            next_gif_tile(&rows, 2, GifTileKey::Home),
            Some(GifTileMove::Tile(0))
        );
        assert_eq!(
            next_gif_tile(&rows, 0, GifTileKey::End),
            Some(GifTileMove::Tile(4))
        );
    }

    #[test]
    fn up_and_down_reach_the_nearest_tile_of_the_row_above_or_below() {
        let rows = rows();
        assert_eq!(
            next_gif_tile(&rows, 0, GifTileKey::Down),
            Some(GifTileMove::Tile(3))
        );
        assert_eq!(
            next_gif_tile(&rows, 2, GifTileKey::Down),
            Some(GifTileMove::Tile(4))
        );
        assert_eq!(
            next_gif_tile(&rows, 4, GifTileKey::Up),
            Some(GifTileMove::Tile(1))
        );
        assert_eq!(
            next_gif_tile(&rows, 3, GifTileKey::Up),
            Some(GifTileMove::Tile(0))
        );
    }

    #[test]
    fn up_from_the_first_row_goes_back_to_the_search_and_down_from_the_last_nowhere() {
        let rows = rows();
        assert_eq!(
            next_gif_tile(&rows, 1, GifTileKey::Up),
            Some(GifTileMove::Search)
        );
        assert_eq!(next_gif_tile(&rows, 4, GifTileKey::Down), None);
        assert_eq!(next_gif_tile(&[], 0, GifTileKey::Down), None);
    }

    fn gif(slug: &str, width: u32, height: u32) -> KlipyItem {
        KlipyItem::Gif(KlipyGif {
            slug: slug.to_owned(),
            title: String::new(),
            tile: KlipyMedia {
                url: format!("{STATIC}/{slug}.webp"),
                width,
                height,
                is_video: false,
            },
            picture_tile: None,
            still: None,
            message_url: format!("{STATIC}/{slug}.webp"),
        })
    }

    fn owned(slugs: &[&str]) -> Vec<String> {
        slugs.iter().map(|slug| (*slug).to_owned()).collect()
    }

    #[test]
    fn favourites_show_in_the_instances_order_without_what_klipy_no_longer_knows() {
        let from_klipy = [gif("b", 1, 1), gif("a", 1, 1)];
        let ordered = gifs_in_slug_order(&from_klipy, &owned(&["a", "gone", "b"]));
        let slugs: Vec<&str> = ordered.iter().map(|gif| gif.slug.as_str()).collect();
        assert_eq!(slugs, ["a", "b"]);
    }

    #[test]
    fn a_new_star_goes_first_and_an_unstarred_one_goes_out() {
        assert_eq!(
            with_favourite_set(&owned(&["a", "b"]), "c", true),
            owned(&["c", "a", "b"])
        );
        assert_eq!(
            with_favourite_set(&owned(&["a", "b"]), "b", true),
            owned(&["b", "a"])
        );
        assert_eq!(
            with_favourite_set(&owned(&["a", "b"]), "a", false),
            owned(&["b"])
        );
        assert_eq!(with_favourite_set(&[], "a", false), Vec::<String>::new());
    }

    #[test]
    fn a_tile_follows_the_gifs_shape_within_bounds_and_an_ad_takes_a_square() {
        assert!((tile_aspect_ratio(&gif("a", 200, 100)) - 2.0).abs() < f32::EPSILON);
        assert!((tile_aspect_ratio(&gif("a", 1000, 100)) - 2.5).abs() < f32::EPSILON);
        assert!((tile_aspect_ratio(&gif("a", 10, 100)) - 0.5).abs() < f32::EPSILON);
        let ad = |width, height| {
            KlipyItem::Ad(KlipyAd {
                position: 0,
                width,
                height,
            })
        };
        assert!((tile_aspect_ratio(&ad(None, None)) - 1.0).abs() < f32::EPSILON);
        assert!((tile_aspect_ratio(&ad(Some(300), Some(250))) - 1.2).abs() < 1e-6);
    }

    #[test]
    fn favourites_page_through_the_list_frozen_at_the_first_page_whatever_is_starred_meanwhile() {
        let frozen: Vec<String> = (0..30).map(|index| format!("slug-{index}")).collect();
        let first = favourites_page(&frozen, 0, 24);
        assert_eq!(
            first,
            FavouritesPage {
                slugs: frozen[..24].to_vec(),
                asked_count: 24,
                has_next: true
            }
        );
        let live = with_favourite_set(&frozen, "slug-27", true);
        let second = favourites_page(&frozen, first.asked_count, 24);
        assert_eq!(
            second,
            FavouritesPage {
                slugs: frozen[24..].to_vec(),
                asked_count: 30,
                has_next: false
            }
        );
        let shown: Vec<&String> = first.slugs.iter().chain(&second.slugs).collect();
        let unique: std::collections::HashSet<&&String> = shown.iter().collect();
        assert_eq!(unique.len(), shown.len());
        assert_eq!(live.first().map(String::as_str), Some("slug-27"));
        assert_eq!(
            favourites_page(&frozen, 30, 24),
            FavouritesPage {
                slugs: Vec::new(),
                asked_count: 30,
                has_next: false
            }
        );
    }
}
