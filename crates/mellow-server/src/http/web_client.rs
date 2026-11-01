//! Serves the web client embedded at build time (ADR-0019): its files, and `index.html` for any
//! other page path, since the client routes in the browser. API and gateway paths never fall
//! through to it. The build ships Brotli and gzip siblings (`name.br`, `name.gz`) of compressible
//! files, chosen here by `Accept-Encoding`, so the server never compresses at runtime.

use axum::body::Body;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

use crate::http::api_error::ApiError;

include!(concat!(env!("OUT_DIR"), "/web_client_assets.rs"));

const INDEX_PATH: &str = "index.html";
/// Vite names these after their content hash, so they never change under the same name.
const HASHED_ASSETS_PREFIX: &str = "assets/";
const CACHE_HASHED_ASSET: &str = "public, max-age=31536000, immutable";
const CACHE_REVALIDATE: &str = "no-cache";
/// Scripts and styles from this origin only; connections to this origin only (REST and the
/// gateway). Inline styles are allowed for Svelte transitions; inline scripts never are. `blob:`
/// images are the composer's previews of files about to be uploaded. The manifest makes the client
/// installable as an app; there is no service worker (Chrome installs without one), and
/// `worker-src 'none'` keeps a script served from this origin, such as an upload, from becoming one.
/// `'wasm-unsafe-eval'` lets the voice noise suppressor compile its WebAssembly (ADR-0031); it
/// allows no JavaScript `eval`.
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; \
     style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; \
     connect-src 'self'; manifest-src 'self'; worker-src 'none'; object-src 'none'; \
     base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ContentEncoding {
    Brotli,
    Gzip,
    Identity,
}

impl ContentEncoding {
    /// Most preferred first, at equal quality values.
    const PRECOMPRESSED_BY_PREFERENCE: [Self; 2] = [Self::Brotli, Self::Gzip];

    fn token(self) -> &'static str {
        match self {
            Self::Brotli => "br",
            Self::Gzip => "gzip",
            Self::Identity => "identity",
        }
    }

    fn sibling_suffix(self) -> &'static str {
        match self {
            Self::Brotli => ".br",
            Self::Gzip => ".gz",
            Self::Identity => "",
        }
    }
}

pub(crate) async fn serve(uri: Uri, request_headers: HeaderMap) -> Response {
    web_client_response(uri.path(), &request_headers, embedded_asset)
}

fn embedded_asset(name: &str) -> Option<&'static [u8]> {
    WEB_CLIENT_ASSETS
        .iter()
        .find(|(asset_name, _)| *asset_name == name)
        .map(|(_, bytes)| *bytes)
}

fn web_client_response(
    path: &str,
    request_headers: &HeaderMap,
    lookup: impl Fn(&str) -> Option<&'static [u8]>,
) -> Response {
    if path.starts_with("/api/") || path == "/api" || path.starts_with("/gateway") {
        return ApiError::not_found().into_response();
    }
    let relative = match path.trim_start_matches('/') {
        "" => INDEX_PATH,
        relative => relative,
    };
    // Siblings are reached only through `Accept-Encoding`: by name they would carry the wrong type.
    let is_precompressed_sibling_name = ContentEncoding::PRECOMPRESSED_BY_PREFERENCE
        .iter()
        .any(|encoding| relative.ends_with(encoding.sibling_suffix()));
    if is_precompressed_sibling_name {
        return ApiError::not_found().into_response();
    }
    let name = if lookup(relative).is_some() {
        relative
    } else {
        // A page path (no file extension) is a client route: the client decides what it shows.
        let is_page_path = !relative
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .contains('.');
        if !is_page_path {
            return ApiError::not_found().into_response();
        }
        INDEX_PATH
    };
    let accept_encoding = joined_header_values(request_headers, header::ACCEPT_ENCODING);
    let Some((encoding, bytes)) = select_encoded_variant(&accept_encoding, name, lookup) else {
        return ApiError::not_found().into_response();
    };
    let if_none_match = joined_header_values(request_headers, header::IF_NONE_MATCH);
    asset_response(name, encoding, bytes, &if_none_match)
}

/// A header sent on several lines means the same as one comma-separated line.
fn joined_header_values(headers: &HeaderMap, name: HeaderName) -> String {
    headers
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect::<Vec<_>>()
        .join(",")
}

/// Picks the precompressed sibling of `name` the client accepts with the highest quality value
/// (Brotli first on a tie), else the original file. `None` when `name` itself does not exist.
fn select_encoded_variant(
    accept_encoding: &str,
    name: &str,
    lookup: impl Fn(&str) -> Option<&'static [u8]>,
) -> Option<(ContentEncoding, &'static [u8])> {
    let original = lookup(name)?;
    let mut best: Option<(f32, ContentEncoding, &'static [u8])> = None;
    for encoding in ContentEncoding::PRECOMPRESSED_BY_PREFERENCE {
        let quality = accepted_quality(accept_encoding, encoding.token());
        let is_better =
            quality > 0.0 && best.is_none_or(|(best_quality, _, _)| quality > best_quality);
        if !is_better {
            continue;
        }
        if let Some(bytes) = lookup(&format!("{name}{}", encoding.sibling_suffix())) {
            best = Some((quality, encoding, bytes));
        }
    }
    // The original is sent even to a client that refuses identity, rather than a bare 406.
    Some(best.map_or(
        (ContentEncoding::Identity, original),
        |(_, encoding, bytes)| (encoding, bytes),
    ))
}

/// The quality value (RFC 9110 §12.5.3) the client gives `coding`: its own entry, else `*`, else 0.
/// An unparsable or out-of-range quality counts as 0, so a malformed header gets the original.
fn accepted_quality(accept_encoding: &str, coding: &str) -> f32 {
    let mut wildcard_quality = None;
    for entry in accept_encoding.split(',') {
        let mut parameters = entry.split(';');
        let entry_coding = parameters.next().unwrap_or_default().trim();
        let quality = parameters
            .filter_map(|parameter| parameter.split_once('='))
            .find(|(key, _)| key.trim().eq_ignore_ascii_case("q"))
            .map_or(Some(1.0), |(_, value)| value.trim().parse::<f32>().ok())
            .filter(|quality| (0.0..=1.0).contains(quality))
            .unwrap_or(0.0);
        if entry_coding.eq_ignore_ascii_case(coding) {
            return quality;
        }
        if entry_coding == "*" {
            wildcard_quality = Some(quality);
        }
    }
    wildcard_quality.unwrap_or(0.0)
}

/// One per embedded build, distinct per coding: each compressed file is its own representation.
fn entity_tag(encoding: ContentEncoding) -> String {
    match encoding {
        ContentEncoding::Identity => format!("\"{WEB_CLIENT_BUILD_TAG}\""),
        compressed => format!("\"{WEB_CLIENT_BUILD_TAG}-{}\"", compressed.token()),
    }
}

/// `If-None-Match` uses the weak comparison (RFC 9110 §13.1.2).
fn is_entity_tag_listed(if_none_match: &str, entity_tag: &str) -> bool {
    if_none_match.split(',').map(str::trim).any(|candidate| {
        candidate == "*" || candidate.strip_prefix("W/").unwrap_or(candidate) == entity_tag
    })
}

fn asset_response(
    name: &str,
    encoding: ContentEncoding,
    bytes: &'static [u8],
    if_none_match: &str,
) -> Response {
    let is_hashed_asset = name.starts_with(HASHED_ASSETS_PREFIX);
    let cache_control = if is_hashed_asset {
        CACHE_HASHED_ASSET
    } else {
        CACHE_REVALIDATE
    };
    let mut response = (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static(content_type(name)),
            ),
            // Every response varies, identity included, so a cache never hands a compressed body
            // to a client that cannot read it, or the reverse.
            (header::VARY, HeaderValue::from_static("Accept-Encoding")),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static(cache_control),
            ),
            (
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static(CONTENT_SECURITY_POLICY),
            ),
            (
                header::X_CONTENT_TYPE_OPTIONS,
                HeaderValue::from_static("nosniff"),
            ),
            (
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
            ),
            (header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY")),
        ],
        bytes,
    )
        .into_response();
    let headers = response.headers_mut();
    // RFC 9110 §8.4.1: `identity` is never sent as a `Content-Encoding`.
    if encoding != ContentEncoding::Identity {
        headers.insert(
            header::CONTENT_ENCODING,
            HeaderValue::from_static(encoding.token()),
        );
    }
    // Hashed assets never change under their name, so only the others are revalidated.
    if !is_hashed_asset {
        let entity_tag = entity_tag(encoding);
        let is_not_modified = is_entity_tag_listed(if_none_match, &entity_tag);
        // The build tag is a version and a hex hash, always a valid header value.
        if let Ok(value) = HeaderValue::from_str(&entity_tag) {
            headers.insert(header::ETAG, value);
        }
        if is_not_modified {
            *response.status_mut() = StatusCode::NOT_MODIFIED;
            *response.body_mut() = Body::empty();
        }
    }
    response
}

fn content_type(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, extension)| extension) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("json") => "application/json",
        Some("webmanifest") => "application/manifest+json",
        Some("txt") => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unit tests build without the `web-client` feature, so they serve this table instead.
    const FAKE_ASSETS: &[(&str, &[u8])] = &[
        ("index.html", b"<!doctype html>"),
        ("index.html.br", b"br index"),
        ("index.html.gz", b"gz index"),
        ("assets/index-3f2a.js", b"console.log(1)"),
        ("assets/index-3f2a.js.br", b"br js"),
        ("assets/index-3f2a.js.gz", b"gz js"),
        ("assets/only-gzip.css", b"body{}"),
        ("assets/only-gzip.css.gz", b"gz css"),
        ("icons/mellow-192.png", b"png"),
    ];

    fn fake_asset(name: &str) -> Option<&'static [u8]> {
        FAKE_ASSETS
            .iter()
            .find(|(asset_name, _)| *asset_name == name)
            .map(|(_, bytes)| *bytes)
    }

    fn selected(accept_encoding: &str, name: &str) -> Option<(ContentEncoding, &'static [u8])> {
        select_encoded_variant(accept_encoding, name, fake_asset)
    }

    fn request_headers(pairs: &[(HeaderName, &str)]) -> HeaderMap {
        pairs
            .iter()
            .map(|(name, value)| (name.clone(), HeaderValue::from_str(value).unwrap()))
            .collect()
    }

    fn fake_response(path: &str, pairs: &[(HeaderName, &str)]) -> Response {
        web_client_response(path, &request_headers(pairs), fake_asset)
    }

    async fn body_bytes(response: Response) -> Vec<u8> {
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec()
    }

    #[test]
    fn brotli_is_preferred_then_gzip_then_the_original() {
        let js = "assets/index-3f2a.js";
        assert_eq!(
            selected("gzip, deflate, br, zstd", js),
            Some((ContentEncoding::Brotli, b"br js".as_slice()))
        );
        assert_eq!(
            selected("gzip", js),
            Some((ContentEncoding::Gzip, b"gz js".as_slice()))
        );
        assert_eq!(
            selected("", js),
            Some((ContentEncoding::Identity, b"console.log(1)".as_slice()))
        );
        assert_eq!(
            selected("br, gzip", "assets/only-gzip.css"),
            Some((ContentEncoding::Gzip, b"gz css".as_slice())),
            "a missing sibling falls back to the next accepted coding"
        );
        assert_eq!(
            selected("br, gzip", "icons/mellow-192.png"),
            Some((ContentEncoding::Identity, b"png".as_slice()))
        );
        assert_eq!(selected("br", "missing.js"), None);
    }

    #[test]
    fn quality_values_rank_codings_and_zero_refuses_one() {
        let js = "assets/index-3f2a.js";
        assert_eq!(
            selected("br;q=0.5, gzip", js).unwrap().0,
            ContentEncoding::Gzip
        );
        assert_eq!(
            selected("br;q=0, gzip;q=0.1", js).unwrap().0,
            ContentEncoding::Gzip
        );
        assert_eq!(
            selected("BR ; Q=0.9, GZIP;q=0.8", js).unwrap().0,
            ContentEncoding::Brotli
        );
        assert_eq!(
            selected("br;q=0, gzip;q=0", js).unwrap().0,
            ContentEncoding::Identity
        );
        assert_eq!(selected("*", js).unwrap().0, ContentEncoding::Brotli);
        assert_eq!(
            selected("*;q=0.5, br;q=0", js).unwrap().0,
            ContentEncoding::Gzip
        );
        assert_eq!(
            selected("br;q=nonsense, gzip;q=2", js).unwrap().0,
            ContentEncoding::Identity,
            "malformed quality values never earn a compressed answer"
        );
    }

    #[tokio::test]
    async fn a_compressed_answer_keeps_the_original_type_and_every_answer_varies() {
        let response = fake_response(
            "/assets/index-3f2a.js",
            &[(header::ACCEPT_ENCODING, "gzip, br")],
        );
        let headers = response.headers();
        assert_eq!(headers[header::CONTENT_ENCODING], "br");
        assert_eq!(
            headers[header::CONTENT_TYPE],
            "text/javascript; charset=utf-8"
        );
        assert_eq!(headers[header::VARY], "Accept-Encoding");
        assert_eq!(headers[header::CACHE_CONTROL], CACHE_HASHED_ASSET);
        assert!(
            headers.get(header::ETAG).is_none(),
            "hashed assets need no validator"
        );
        assert_eq!(body_bytes(response).await, b"br js");

        let identity = fake_response("/assets/index-3f2a.js", &[]);
        assert!(identity.headers().get(header::CONTENT_ENCODING).is_none());
        assert_eq!(identity.headers()[header::VARY], "Accept-Encoding");
        assert_eq!(body_bytes(identity).await, b"console.log(1)");
    }

    #[tokio::test]
    async fn page_paths_get_the_negotiated_index_with_its_validator() {
        let response = fake_response("/channels/42", &[(header::ACCEPT_ENCODING, "gzip")]);
        let headers = response.headers();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(headers[header::CONTENT_TYPE], "text/html; charset=utf-8");
        assert_eq!(headers[header::CONTENT_ENCODING], "gzip");
        assert_eq!(headers[header::VARY], "Accept-Encoding");
        assert_eq!(headers[header::CACHE_CONTROL], CACHE_REVALIDATE);
        assert_eq!(
            headers[header::ETAG].to_str().unwrap(),
            format!("\"{WEB_CLIENT_BUILD_TAG}-gzip\"")
        );
        assert_eq!(body_bytes(response).await, b"gz index");
    }

    #[tokio::test]
    async fn a_matching_entity_tag_answers_not_modified_without_a_body() {
        let first = fake_response("/", &[]);
        let entity_tag = first.headers()[header::ETAG].to_str().unwrap().to_owned();
        assert_eq!(entity_tag, format!("\"{WEB_CLIENT_BUILD_TAG}\""));

        let entity_tag = entity_tag.as_str();
        let repeated = fake_response("/", &[(header::IF_NONE_MATCH, entity_tag)]);
        assert_eq!(repeated.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(repeated.headers()[header::ETAG], entity_tag);
        assert_eq!(repeated.headers()[header::VARY], "Accept-Encoding");
        assert_eq!(body_bytes(repeated).await, b"");

        let other_coding = fake_response(
            "/",
            &[
                (header::IF_NONE_MATCH, entity_tag),
                (header::ACCEPT_ENCODING, "br"),
            ],
        );
        assert_eq!(
            other_coding.status(),
            StatusCode::OK,
            "the Brotli body is another representation with its own tag"
        );
        let stale = fake_response("/", &[(header::IF_NONE_MATCH, "\"0.1.0-0\"")]);
        assert_eq!(stale.status(), StatusCode::OK);
        assert!(is_entity_tag_listed("\"a\", W/\"b\"", "\"b\""));
        assert!(is_entity_tag_listed("*", "\"b\""));
    }

    #[test]
    fn precompressed_siblings_are_never_served_by_name() {
        for path in [
            "/assets/index-3f2a.js.br",
            "/assets/index-3f2a.js.gz",
            "/index.html.br",
        ] {
            let response = fake_response(path, &[(header::ACCEPT_ENCODING, "br, gzip")]);
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }
        let missing_file = fake_response("/assets/missing.js", &[]);
        assert_eq!(missing_file.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn known_extensions_get_their_type_and_others_are_never_sniffed() {
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(
            content_type("assets/index-3f2a.js"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(content_type("assets/Figtree.ttf"), "font/ttf");
        assert_eq!(content_type("README"), "application/octet-stream");
    }

    #[test]
    fn the_manifest_and_app_icons_are_served_with_their_type_and_the_policy() {
        let manifest = asset_response("manifest.webmanifest", ContentEncoding::Identity, b"{}", "");
        let headers = manifest.headers();
        assert_eq!(headers[header::CONTENT_TYPE], "application/manifest+json");
        assert_eq!(headers[header::CACHE_CONTROL], CACHE_REVALIDATE);
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        let policy = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
        assert!(policy.contains("manifest-src 'self'"), "{policy}");
        assert!(policy.contains("worker-src 'none'"), "{policy}");
        assert!(
            policy.contains("script-src 'self' 'wasm-unsafe-eval';")
                && !policy.contains("'unsafe-eval'"),
            "WebAssembly only, never JavaScript eval: {policy}"
        );

        let icon = asset_response(
            "icons/mellow-maskable-512.png",
            ContentEncoding::Identity,
            b"",
            "",
        );
        assert_eq!(icon.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(icon.headers()[header::CACHE_CONTROL], CACHE_REVALIDATE);
    }

    #[tokio::test]
    async fn api_and_gateway_paths_never_reach_the_client() {
        for path in ["/api/v1/nothing", "/api", "/gateway/x"] {
            let response = serve(path.parse().unwrap(), HeaderMap::new()).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            let content_type = response.headers().get(header::CONTENT_TYPE).unwrap();
            assert_eq!(content_type, "application/json");
        }
    }
}
