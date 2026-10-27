//! Serves the web client embedded at build time (ADR-0019): its files, and `index.html` for any
//! other page path, since the client routes in the browser. API and gateway paths never fall
//! through to it.

use axum::http::{HeaderValue, StatusCode, Uri, header};
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
const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; \
     style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; font-src 'self'; \
     connect-src 'self'; manifest-src 'self'; worker-src 'none'; object-src 'none'; \
     base-uri 'none'; form-action 'self'; frame-ancestors 'none'";

pub(crate) async fn serve(uri: Uri) -> Response {
    let path = uri.path();
    if path.starts_with("/api/") || path == "/api" || path.starts_with("/gateway") {
        return ApiError::not_found().into_response();
    }
    let relative = path.trim_start_matches('/');
    let asset = find_asset(relative).or_else(|| {
        // A page path (no file extension) is a client route: the client decides what it shows.
        let is_page_path = !relative
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .contains('.');
        is_page_path.then(|| find_asset(INDEX_PATH)).flatten()
    });
    let Some((name, bytes)) = asset else {
        return ApiError::not_found().into_response();
    };
    asset_response(name, bytes)
}

fn asset_response(name: &str, bytes: &'static [u8]) -> Response {
    let cache_control = if name.starts_with(HASHED_ASSETS_PREFIX) {
        CACHE_HASHED_ASSET
    } else {
        CACHE_REVALIDATE
    };
    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static(content_type(name)),
            ),
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
        .into_response()
}

fn find_asset(relative: &str) -> Option<(&'static str, &'static [u8])> {
    let relative = if relative.is_empty() {
        INDEX_PATH
    } else {
        relative
    };
    WEB_CLIENT_ASSETS
        .iter()
        .find(|(name, _)| *name == relative)
        .copied()
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
        let manifest = asset_response("manifest.webmanifest", b"{}");
        let headers = manifest.headers();
        assert_eq!(headers[header::CONTENT_TYPE], "application/manifest+json");
        assert_eq!(headers[header::CACHE_CONTROL], CACHE_REVALIDATE);
        assert_eq!(headers[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        let policy = headers[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
        assert!(policy.contains("manifest-src 'self'"), "{policy}");
        assert!(policy.contains("worker-src 'none'"), "{policy}");

        let icon = asset_response("icons/mellow-maskable-512.png", b"");
        assert_eq!(icon.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(icon.headers()[header::CACHE_CONTROL], CACHE_REVALIDATE);
    }

    #[tokio::test]
    async fn api_and_gateway_paths_never_reach_the_client() {
        for path in ["/api/v1/nothing", "/api", "/gateway/x"] {
            let response = serve(path.parse().unwrap()).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND);
            let content_type = response.headers().get(header::CONTENT_TYPE).unwrap();
            assert_eq!(content_type, "application/json");
        }
    }
}
