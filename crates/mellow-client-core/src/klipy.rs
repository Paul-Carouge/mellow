//! Klipy, called by this app itself and never through the instance (ADR-0042: Klipy's terms forbid
//! a relay), as the web client's browser does (`lib/klipy.ts`). Its own HTTP client, separate from
//! the instance's: no Mellow token, no cookie, no referrer, `https` only, redirects only to Klipy's
//! own hosts, timeouts, and bodies read up to a bound.

use std::time::Duration;

use mellow_protocol::GifSettings;
use reqwest::{StatusCode, redirect};
use url::Url;

use crate::error::ClientError;
use crate::rest::RestClient;
use crate::shared_logic::klipy::{
    KlipyPage, is_klipy_api_url, klipy_media_url, klipy_share_body, klipy_share_url,
    parse_klipy_page,
};

/// The web's `KLIPY_REQUEST_TIMEOUT_MILLISECONDS`.
const KLIPY_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const KLIPY_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// A page of 24 items is a few dozen kilobytes; far above that is a broken answer.
const MAXIMUM_KLIPY_PAGE_BYTES: usize = 2 * 1024 * 1024;
/// Klipy's medium WebP weighs about 636 KB at the median; its largest GIFs a few megabytes.
const MAXIMUM_KLIPY_MEDIA_BYTES: usize = 24 * 1024 * 1024;
/// A CDN may hop once or twice between Klipy's hosts; never further.
const MAXIMUM_KLIPY_REDIRECTS: usize = 3;
const KLIPY_API_HOST: &str = "api.klipy.com";

/// Why Klipy gave nothing (the web's `KlipyFailure`, plus what only a download can meet).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KlipyError {
    /// No answer: the network or Klipy is down, or the answer stalled.
    #[error("KLIPY could not be reached")]
    Unreachable,
    /// Klipy's 429: a test key allows about 100 requests an hour for the whole instance.
    #[error("KLIPY is rate limiting this instance")]
    RateLimited,
    /// 401 or 403: the instance's key is refused.
    #[error("KLIPY refused the instance's key")]
    Refused,
    /// Any other status, an answer of another shape, or an address that is not Klipy's.
    #[error("KLIPY's answer was unusable")]
    Failed,
    #[error("KLIPY's file is too large")]
    TooLarge,
}

fn failure_of_status(status: StatusCode) -> KlipyError {
    match status.as_u16() {
        429 => KlipyError::RateLimited,
        401 | 403 => KlipyError::Refused,
        _ => KlipyError::Failed,
    }
}

impl From<reqwest::Error> for KlipyError {
    fn from(error: reqwest::Error) -> Self {
        if error.is_timeout() || error.is_connect() || error.is_request() {
            Self::Unreachable
        } else {
            Self::Failed
        }
    }
}

/// Whether a redirect stays on Klipy: `https` on its API or media hosts.
fn is_klipy_address(url: &Url) -> bool {
    url.host_str() == Some(KLIPY_API_HOST) && url.scheme() == "https"
        || klipy_media_url(url.as_str()).is_some()
}

#[derive(Debug, Clone)]
pub struct KlipyClient {
    http: reqwest::Client,
}

impl KlipyClient {
    /// # Errors
    /// Fails only if the TLS stack cannot initialise.
    pub fn new() -> Result<Self, ClientError> {
        let http = reqwest::Client::builder()
            .https_only(true)
            .referer(false)
            .user_agent(concat!("mellow-client/", env!("CARGO_PKG_VERSION")))
            .timeout(KLIPY_REQUEST_TIMEOUT)
            .connect_timeout(KLIPY_CONNECT_TIMEOUT)
            .redirect(redirect::Policy::custom(|attempt| {
                if attempt.previous().len() < MAXIMUM_KLIPY_REDIRECTS
                    && is_klipy_address(attempt.url())
                {
                    attempt.follow()
                } else {
                    attempt.stop()
                }
            }))
            .build()
            .map_err(ClientError::Network)?;
        Ok(Self { http })
    }

    async fn body(
        response: reqwest::Response,
        maximum_bytes: usize,
    ) -> Result<Vec<u8>, KlipyError> {
        if !response.status().is_success() {
            return Err(failure_of_status(response.status()));
        }
        match RestClient::body_within(response, maximum_bytes, KLIPY_REQUEST_TIMEOUT).await {
            Ok(Some(body)) => Ok(body),
            Ok(None) => Err(KlipyError::TooLarge),
            Err(_) => Err(KlipyError::Unreachable),
        }
    }

    /// One page of trending, a search or items, from an address built by `shared_logic::klipy`.
    ///
    /// # Errors
    /// [`KlipyError`]; `Failed` for an address that is not Klipy's API or an answer of another
    /// shape.
    pub async fn page(&self, url: &str) -> Result<KlipyPage, KlipyError> {
        if !is_klipy_api_url(url) {
            return Err(KlipyError::Failed);
        }
        let response = self.http.get(url).send().await?;
        let body = Self::body(response, MAXIMUM_KLIPY_PAGE_BYTES)
            .await
            .map_err(|error| match error {
                KlipyError::TooLarge => KlipyError::Failed,
                error => error,
            })?;
        serde_json::from_slice(&body)
            .ok()
            .and_then(|body| parse_klipy_page(&body))
            .ok_or(KlipyError::Failed)
    }

    /// Tells Klipy a GIF was sent, as their API asks (`query` is the search that found it, empty
    /// from trending or favourites).
    ///
    /// # Errors
    /// [`KlipyError`]; a failure changes nothing for the sender.
    pub async fn trigger_share(
        &self,
        settings: &GifSettings,
        slug: &str,
        query: &str,
    ) -> Result<(), KlipyError> {
        let response = self
            .http
            .post(klipy_share_url(settings, slug))
            .json(&klipy_share_body(settings, query))
            .send()
            .await?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(failure_of_status(response.status()))
        }
    }

    /// The bytes of a picture on Klipy's media hosts, loaded from the address Klipy returned.
    ///
    /// # Errors
    /// [`KlipyError`]; `Failed` for an address that is not Klipy's media, `TooLarge` past the
    /// bound.
    pub async fn media(&self, url: &str) -> Result<Vec<u8>, KlipyError> {
        let Some(url) = klipy_media_url(url) else {
            return Err(KlipyError::Failed);
        };
        let response = self.http.get(url).send().await?;
        Self::body(response, MAXIMUM_KLIPY_MEDIA_BYTES).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn klipys_statuses_say_why_it_did_not_answer() {
        assert_eq!(
            failure_of_status(StatusCode::TOO_MANY_REQUESTS),
            KlipyError::RateLimited
        );
        assert_eq!(
            failure_of_status(StatusCode::FORBIDDEN),
            KlipyError::Refused
        );
        assert_eq!(
            failure_of_status(StatusCode::UNAUTHORIZED),
            KlipyError::Refused
        );
        assert_eq!(
            failure_of_status(StatusCode::INTERNAL_SERVER_ERROR),
            KlipyError::Failed
        );
    }

    #[test]
    fn redirects_stay_on_klipys_hosts_over_https() {
        let address = |text: &str| Url::parse(text).ok();
        for kept in [
            "https://api.klipy.com/api/v1/key/gifs/trending",
            "https://static1.klipy.com/ii/a.webp",
        ] {
            assert!(
                address(kept).is_some_and(|url| is_klipy_address(&url)),
                "{kept}"
            );
        }
        for refused in [
            "http://api.klipy.com/api/v1/key/gifs/trending",
            "https://evil.example/a.webp",
            "https://static.klipy.com.evil.example/a.webp",
        ] {
            assert!(
                !address(refused).is_some_and(|url| is_klipy_address(&url)),
                "{refused}"
            );
        }
    }

    #[tokio::test]
    async fn nothing_but_klipys_own_addresses_is_ever_asked_for() {
        let Ok(client) = KlipyClient::new() else {
            panic!("the TLS stack initialises");
        };
        assert_eq!(
            client
                .media("https://instance.example/api/v1/attachments/1")
                .await,
            Err(KlipyError::Failed)
        );
        assert_eq!(
            client
                .page("https://instance.example/api/v1/gifs/settings")
                .await,
            Err(KlipyError::Failed)
        );
    }
}
