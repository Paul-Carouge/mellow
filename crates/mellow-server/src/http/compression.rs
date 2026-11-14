//! Gzip for the large JSON answers of the API (history, members, guilds, search), which shrink
//! seven to eleven times. Only the routes listed here are compressed: none of them returns a
//! session token, a two-factor secret or recovery code, an invite code, a key or an email.
//! Compressing a secret next to text an attacker chooses (a message, a name) lets them guess it
//! from the answer's length (BREACH), so a new route stays uncompressed until it is listed.

use std::io::Write as _;

use axum::body::Body;
use axum::extract::{MatchedPath, Request};
use axum::http::{HeaderMap, HeaderValue, Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse as _, Response};
use flate2::Compression;
use flate2::write::GzEncoder;
use http_body::Body as _;

/// Methods and route templates (as routed, `MatchedPath`) whose answers are free of secrets.
const COMPRESSED_ROUTES: &[(Method, &str)] = &[
    (Method::GET, "/api/v1/channels/{channel_id}/messages"),
    (Method::GET, "/api/v1/channels/{channel_id}/pins"),
    (
        Method::GET,
        "/api/v1/channels/{channel_id}/messages/{message_id}/reactions/{emoji}",
    ),
    (
        Method::GET,
        "/api/v1/channels/{channel_id}/messages/{message_id}/poll/answers/{answer_id}/voters",
    ),
    (Method::POST, "/api/v1/guilds/{guild_id}/messages/search"),
    (Method::POST, "/api/v1/users/@me/messages/search"),
    (Method::GET, "/api/v1/users/@me/read-states"),
    (Method::GET, "/api/v1/users/@me/guilds"),
    (Method::GET, "/api/v1/users/@me/relationships"),
    (Method::GET, "/api/v1/guilds/{guild_id}"),
    (Method::GET, "/api/v1/guilds/{guild_id}/members"),
    (Method::GET, "/api/v1/guilds/{guild_id}/audit-log"),
    (Method::GET, "/api/v1/guilds/{guild_id}/bans"),
    (Method::GET, "/api/v1/guilds/{guild_id}/emojis"),
    (Method::GET, "/api/v1/guilds/{guild_id}/sounds"),
];

/// Below this, gzip's own 20 bytes and the CPU are not worth it.
const MINIMUM_COMPRESSED_BODY_BYTES: u64 = 1024;
/// Larger answers go as they are.
const MAXIMUM_COMPRESSED_BODY_BYTES: u64 = 4 * 1024 * 1024;
/// Up to this, compressing takes well under a millisecond (a 71 KB member list: about 50 µs) and
/// runs on the runtime worker; larger answers are compressed on the blocking pool, so a worker is
/// never held for milliseconds.
const MAXIMUM_INLINE_COMPRESSED_BODY_BYTES: usize = 256 * 1024;

/// Compresses the listed routes' JSON answers for clients that accept gzip.
pub(super) async fn compress_listed_json_answers(request: Request, next: Next) -> Response {
    let is_listed = request
        .extensions()
        .get::<MatchedPath>()
        .is_some_and(|path| is_listed_for_compression(request.method(), path.as_str()));
    if !is_listed {
        return next.run(request).await;
    }
    let accepts_gzip = accepts_gzip(request.headers());
    let mut response = next.run(request).await;
    response
        .headers_mut()
        .append(header::VARY, HeaderValue::from_static("Accept-Encoding"));
    let is_json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .is_some_and(|value| value.as_bytes().starts_with(b"application/json"));
    let is_worth_compressing = response.body().size_hint().exact().is_some_and(|bytes| {
        (MINIMUM_COMPRESSED_BODY_BYTES..=MAXIMUM_COMPRESSED_BODY_BYTES).contains(&bytes)
    });
    if !accepts_gzip
        || !is_json
        || !is_worth_compressing
        || response.headers().contains_key(header::CONTENT_ENCODING)
    {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    // The size is exact and bounded above, so this reads a body already in memory.
    let plain = match axum::body::to_bytes(body, usize::MAX).await {
        Ok(plain) => plain,
        Err(error) => {
            return crate::http::api_error::ApiError::internal(
                "cannot read an answer to compress it",
                &error,
            )
            .into_response();
        }
    };
    match gzip_off_the_worker_if_large(plain.clone()).await {
        Ok(compressed) => {
            parts
                .headers
                .insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
            parts.headers.remove(header::CONTENT_LENGTH);
            Response::from_parts(parts, Body::from(compressed))
        }
        Err(error) => {
            tracing::warn!(%error, "cannot compress an answer: sent as it is");
            Response::from_parts(parts, Body::from(plain))
        }
    }
}

/// `gzip`, on the blocking pool past `MAXIMUM_INLINE_COMPRESSED_BODY_BYTES`.
async fn gzip_off_the_worker_if_large(plain: axum::body::Bytes) -> std::io::Result<Vec<u8>> {
    if plain.len() <= MAXIMUM_INLINE_COMPRESSED_BODY_BYTES {
        return gzip(&plain);
    }
    tokio::task::spawn_blocking(move || gzip(&plain))
        .await
        .unwrap_or_else(|error| Err(std::io::Error::other(error)))
}

fn is_listed_for_compression(method: &Method, route: &str) -> bool {
    COMPRESSED_ROUTES
        .iter()
        .any(|(listed_method, listed_route)| listed_method == method && *listed_route == route)
}

/// Level 1: a history page goes from 16 KB to 3.2 KB, against 2.4 KB at level 6 for several
/// times the CPU.
fn gzip(plain: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::with_capacity(plain.len() / 4), Compression::fast());
    encoder.write_all(plain)?;
    encoder.finish()
}

/// Whether `Accept-Encoding` lists gzip without refusing it (`q=0`).
fn accepts_gzip(headers: &HeaderMap) -> bool {
    headers
        .get_all(header::ACCEPT_ENCODING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|coding| {
            let mut coding_parts = coding.split(';');
            let name = coding_parts.next().unwrap_or_default().trim();
            let is_refused = coding_parts.any(|parameter| {
                parameter.split_once('=').is_some_and(|(key, quality)| {
                    key.trim().eq_ignore_ascii_case("q")
                        && quality.trim().parse::<f32>().is_ok_and(|q| q <= 0.0)
                })
            });
            name.eq_ignore_ascii_case("gzip") && !is_refused
        })
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use axum::http::StatusCode;
    use flate2::read::GzDecoder;

    use super::*;
    use crate::domain::account_rules::RegistrationMode;
    use crate::domain::accounts::tests::migrated_database;
    use crate::http::test_support::{
        TEST_PEER, authorized, guild_with_guest, owner_and_general_channel, send_from, test_state,
    };

    /// Routes whose answers carry a session token, a two-factor secret or recovery codes, an
    /// invite code, a key or the reader's email and friend code: never compressed (BREACH).
    const ROUTES_RETURNING_SECRETS: &[&str] = &[
        "/api/v1/auth/register",
        "/api/v1/auth/login",
        "/api/v1/auth/login/two-factor",
        "/api/v1/users/@me",
        "/api/v1/users/@me/password",
        "/api/v1/users/@me/two-factor",
        "/api/v1/users/@me/two-factor/enrolment",
        "/api/v1/users/@me/two-factor/enable",
        "/api/v1/users/@me/two-factor/disable",
        "/api/v1/users/@me/two-factor/recovery-codes",
        "/api/v1/guilds/{guild_id}/invites",
        "/api/v1/invites/{code}",
        "/api/v1/gifs/settings",
    ];

    fn accepting_gzip(mut request: Request<Body>) -> Request<Body> {
        request.headers_mut().insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip, deflate, br, zstd"),
        );
        request
    }

    fn gunzip(compressed: &[u8]) -> Vec<u8> {
        let mut plain = Vec::new();
        GzDecoder::new(compressed).read_to_end(&mut plain).unwrap();
        plain
    }

    #[test]
    fn no_route_returning_a_secret_is_listed_for_compression() {
        for route in ROUTES_RETURNING_SECRETS {
            assert!(
                COMPRESSED_ROUTES
                    .iter()
                    .all(|(_, listed_route)| listed_route != route),
                "{route}"
            );
        }
    }

    #[test]
    fn only_the_listed_method_of_a_route_is_compressed() {
        let history = "/api/v1/channels/{channel_id}/messages";
        assert!(is_listed_for_compression(&Method::GET, history));
        assert!(!is_listed_for_compression(&Method::POST, history));
    }

    #[tokio::test]
    async fn small_and_large_answers_decode_to_the_same_bytes() {
        for repeats in [100, MAXIMUM_INLINE_COMPRESSED_BODY_BYTES / 10] {
            let plain = axum::body::Bytes::from("{\"id\":\"1\"},".repeat(repeats));
            let compressed = gzip_off_the_worker_if_large(plain.clone()).await.unwrap();
            assert_eq!(gunzip(&compressed), plain);
        }
    }

    #[test]
    fn gzip_counts_as_accepted_only_when_listed_and_not_refused() {
        let accepts = |value: &'static str| {
            let mut headers = HeaderMap::new();
            headers.insert(header::ACCEPT_ENCODING, HeaderValue::from_static(value));
            accepts_gzip(&headers)
        };
        assert!(accepts("gzip"));
        assert!(accepts("br;q=1.0, GZIP;q=0.5"));
        assert!(!accepts("gzip;q=0"));
        assert!(!accepts("gzip;Q=0"));
        assert!(!accepts("gzip; q = 0.000"));
        assert!(!accepts("identity, br"));
        assert!(!accepts_gzip(&HeaderMap::new()));
    }

    #[tokio::test]
    async fn a_history_page_is_gzipped_for_clients_that_accept_it_and_decodes_to_the_same_json() {
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (owner, general) = owner_and_general_channel(&state).await;
        let path = format!("/api/v1/channels/{general}/messages");
        for index in 0..3 {
            let ((status, _), _) = send_from(
                &state,
                TEST_PEER,
                authorized(
                    "POST",
                    &path,
                    &owner,
                    Some(serde_json::json!({
                        "content": "a long enough message ".repeat(30),
                        "nonce": format!("nonce-{index}"),
                    })),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED);
        }

        let ((_, plain), plain_headers) =
            send_from(&state, TEST_PEER, authorized("GET", &path, &owner, None)).await;
        let ((status, compressed), headers) = send_from(
            &state,
            TEST_PEER,
            accepting_gzip(authorized("GET", &path, &owner, None)),
        )
        .await;
        database.close().await;

        assert_eq!(status, StatusCode::OK);
        assert!(plain_headers.get(header::CONTENT_ENCODING).is_none());
        assert!(
            plain_headers
                .get_all(header::VARY)
                .iter()
                .any(|value| value == "Accept-Encoding")
        );
        assert_eq!(headers.get(header::CONTENT_ENCODING).unwrap(), "gzip");
        assert!(compressed.len() < plain.len() / 3, "{}", compressed.len());
        assert_eq!(gunzip(&compressed), plain);
    }

    #[tokio::test]
    async fn invite_codes_stay_uncompressed_even_when_large_and_accepted() {
        let (_directory, database) = migrated_database().await;
        let state = test_state(database.clone(), RegistrationMode::Open);
        let (owner, _, guild_id, _) = guild_with_guest(&state).await;
        let path = format!("/api/v1/guilds/{guild_id}/invites");
        for _ in 0..8 {
            send_from(
                &state,
                TEST_PEER,
                authorized("POST", &path, &owner, Some(serde_json::json!({}))),
            )
            .await;
        }
        let ((status, body), headers) = send_from(
            &state,
            TEST_PEER,
            accepting_gzip(authorized("GET", &path, &owner, None)),
        )
        .await;
        database.close().await;

        assert_eq!(status, StatusCode::OK);
        assert!(
            body.len() as u64 > MINIMUM_COMPRESSED_BODY_BYTES,
            "the list must be large enough to be compressed if it were listed"
        );
        assert!(headers.get(header::CONTENT_ENCODING).is_none());
    }
}
