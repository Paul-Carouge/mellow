//! Session authentication for handlers: a bearer token (native clients), or the session cookie of
//! the web client served by this instance (ADR-0019).

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderName, HeaderValue, header};
use mellow_protocol::CurrentUser;

use crate::domain::clock::now_unix_milliseconds;
use crate::http::AppState;
use crate::http::api_error::ApiError;

const BEARER_PREFIX: &str = "Bearer ";
pub(crate) const SESSION_COOKIE_NAME: &str = "mellow_session";
/// Sent by the web client to sign in: the instance then sets the session cookie instead of
/// handing the token to JavaScript. A custom header cannot be sent by another site without a CORS
/// preflight, which this server never grants, so it also blocks login CSRF.
pub(crate) const COOKIE_SESSION_REQUEST_HEADER: HeaderName =
    HeaderName::from_static("x-mellow-session-transport");
const COOKIE_SESSION_REQUEST_VALUE: &str = "cookie";
const SESSION_LIFETIME_SECONDS: u64 = 90 * 24 * 60 * 60;
const SEC_FETCH_SITE: HeaderName = HeaderName::from_static("sec-fetch-site");

/// The request's session token: the bearer header, else the session cookie, which only counts on
/// a request a page of this instance made (CSRF, cross-site WebSocket hijacking).
pub(crate) fn session_token(headers: &HeaderMap) -> Result<&str, ApiError> {
    if let Some(token) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix(BEARER_PREFIX))
    {
        return Ok(token);
    }
    match session_cookie(headers) {
        Some(token) if is_from_this_origin(headers) => Ok(token),
        _ => Err(ApiError::unauthorized()),
    }
}

pub(crate) fn session_cookie(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == SESSION_COOKIE_NAME)
        .map(|(_, token)| token)
        .filter(|token| !token.is_empty())
}

/// False when a browser says the request comes from another site, or names another origin.
/// Native clients send neither header. The `Origin` authority must equal `Host` (the reverse proxy
/// in front must keep `Host`, as Caddy and nginx's `proxy_set_header Host $host` do).
pub(crate) fn is_from_this_origin(headers: &HeaderMap) -> bool {
    let header_text = |name: HeaderName| headers.get(name).and_then(|value| value.to_str().ok());
    if let Some(site) = header_text(SEC_FETCH_SITE)
        && site != "same-origin"
        && site != "none"
    {
        return false;
    }
    match header_text(header::ORIGIN) {
        None => true,
        Some(origin) => {
            let origin_authority = origin.split_once("://").map(|(_, authority)| authority);
            origin_authority.is_some() && origin_authority == header_text(header::HOST)
        }
    }
}

pub(crate) fn wants_cookie_session(headers: &HeaderMap) -> bool {
    headers
        .get(COOKIE_SESSION_REQUEST_HEADER)
        .is_some_and(|value| value == COOKIE_SESSION_REQUEST_VALUE)
}

/// `Secure` always: browsers accept it on `http://localhost`, and an instance reached by browsers
/// elsewhere is served over TLS (built in or by its reverse proxy).
pub(crate) fn session_cookie_header(token: &str) -> Option<HeaderValue> {
    HeaderValue::from_str(&format!(
        "{SESSION_COOKIE_NAME}={token}; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age={SESSION_LIFETIME_SECONDS}"
    ))
    .ok()
}

pub(crate) fn expired_session_cookie_header() -> HeaderValue {
    HeaderValue::from_static(
        "mellow_session=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0",
    )
}

/// The signed-in user; extracting it rejects the request with `UNAUTHORIZED` otherwise.
#[derive(Debug, Clone)]
pub(crate) struct AuthenticatedUser(pub(crate) CurrentUser);

impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = session_token(&parts.headers)?;
        let user = state
            .accounts
            .authenticate(token, now_unix_milliseconds())
            .await?;
        Ok(Self(user))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(HeaderName, &str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.append(name.clone(), HeaderValue::from_str(value).unwrap());
        }
        map
    }

    #[test]
    fn the_bearer_header_wins_and_the_cookie_counts_only_from_this_origin() {
        let cookie = (header::COOKIE, "theme=dark; mellow_session=mlw_cookie");
        let host = (header::HOST, "chat.example.org");
        assert_eq!(
            session_token(&headers(&[
                (header::AUTHORIZATION, "Bearer mlw_bearer"),
                cookie.clone()
            ]))
            .unwrap(),
            "mlw_bearer"
        );
        assert_eq!(
            session_token(&headers(&[
                cookie.clone(),
                host.clone(),
                (header::ORIGIN, "https://chat.example.org"),
                (SEC_FETCH_SITE, "same-origin"),
            ]))
            .unwrap(),
            "mlw_cookie"
        );
        for refused in [
            headers(&[
                cookie.clone(),
                host.clone(),
                (header::ORIGIN, "https://evil.example"),
            ]),
            headers(&[cookie.clone(), host.clone(), (SEC_FETCH_SITE, "cross-site")]),
            headers(&[cookie.clone(), host.clone(), (SEC_FETCH_SITE, "same-site")]),
            headers(&[cookie.clone(), host.clone(), (header::ORIGIN, "null")]),
            headers(&[(header::COOKIE, "mellow_session="), host.clone()]),
            headers(&[host]),
        ] {
            assert!(session_token(&refused).is_err(), "{refused:?}");
        }
    }

    #[test]
    fn the_session_cookie_is_http_only_secure_and_strict() {
        let value = session_cookie_header("mlw_x").unwrap();
        let value = value.to_str().unwrap();
        for attribute in ["HttpOnly", "Secure", "SameSite=Strict", "Path=/"] {
            assert!(value.contains(attribute), "{value}");
        }
        assert!(session_cookie_header("bad\ntoken").is_none());
    }
}
