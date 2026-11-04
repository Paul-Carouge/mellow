//! Fetching a page or a picture for a link preview, safely (ADR-0041): public addresses only,
//! redirects checked one by one, bounded time and size.

use std::sync::Arc;
use std::time::Duration;

use reqwest::header::{ACCEPT, CONTENT_TYPE, LOCATION};
use url::{Host, Url};

use super::page::{PageDetails, page_details};
use super::public_addresses::{AddressPolicy, PublicAddressResolver};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// The whole fetch of one page or picture, redirects included.
const FETCH_TIMEOUT: Duration = Duration::from_secs(8);
const MAXIMUM_REDIRECTS: usize = 3;
/// The head of a page is enough; a longer page is read up to here and parsed as it is.
const MAXIMUM_PAGE_BYTES: usize = 1024 * 1024;
pub(crate) const MAXIMUM_PICTURE_BYTES: usize = 8 * 1024 * 1024;
const PAGE_CONTENT_TYPES: [&str; 2] = ["text/html", "application/xhtml+xml"];

#[derive(Debug, thiserror::Error)]
pub(crate) enum FetchError {
    /// A link or redirect the fetcher refuses: scheme, port, credentials or a non-public address.
    #[error("link not allowed")]
    NotAllowed,
    #[error("network error")]
    Network(#[from] reqwest::Error),
    #[error("too many redirects")]
    TooManyRedirects,
    #[error("unexpected status")]
    Status,
    #[error("unsupported content")]
    Unsupported,
    #[error("too large")]
    TooLarge,
    #[error("timed out")]
    TimedOut,
    #[error("cannot build the HTTP client")]
    Client,
}

/// What a link leads to.
#[derive(Debug)]
pub(crate) enum Fetched {
    Page(PageDetails),
    /// The link is a picture itself: its bytes, not yet checked as a picture.
    Picture(Vec<u8>),
}

#[derive(Debug, Clone)]
pub(crate) struct LinkFetcher {
    client: reqwest::Client,
    policy: AddressPolicy,
}

/// Whether the fetcher may request `url` (before any name resolution, which the resolver
/// checks): `http` or `https`, default ports, no credentials, and a public address when the
/// host is one.
fn is_allowed(url: &Url, policy: AddressPolicy) -> bool {
    let is_allowed_host = match url.host() {
        Some(Host::Domain(_)) => true,
        Some(Host::Ipv4(address)) => policy.allows(address.into()),
        Some(Host::Ipv6(address)) => policy.allows(address.into()),
        None => false,
    };
    matches!(url.scheme(), "http" | "https")
        && is_allowed_host
        && url.username().is_empty()
        && url.password().is_none()
        && policy.allows_port(url.port())
}

fn content_type_of(response: &reqwest::Response) -> String {
    response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

impl LinkFetcher {
    /// # Errors
    /// `Client` if the TLS roots cannot be loaded.
    pub(crate) fn new(user_agent: &str) -> Result<Self, FetchError> {
        Self::with_policy(user_agent, AddressPolicy::PublicOnly)
    }

    /// Reaches this machine too, on any port, for tests serving pages locally.
    #[cfg(test)]
    pub(crate) fn allowing_loopback_for_tests() -> Self {
        Self::with_policy("Mellow test", AddressPolicy::AlsoLoopbackOnAnyPort).unwrap()
    }

    fn with_policy(user_agent: &str, policy: AddressPolicy) -> Result<Self, FetchError> {
        let roots = webpki_root_certs::TLS_SERVER_ROOT_CERTS
            .iter()
            .map(|certificate| reqwest::Certificate::from_der(certificate))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| FetchError::Client)?;
        let client = reqwest::Client::builder()
            .user_agent(user_agent)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .referer(false)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(FETCH_TIMEOUT)
            .dns_resolver(Arc::new(PublicAddressResolver { policy }))
            .tls_certs_only(roots)
            .build()
            .map_err(|_| FetchError::Client)?;
        Ok(Self { client, policy })
    }

    /// The page's details, or the picture the link points at.
    ///
    /// # Errors
    /// Any refusal, network error, status other than 2xx, or content that is neither.
    pub(crate) async fn fetch_page_or_picture(&self, url: Url) -> Result<Fetched, FetchError> {
        let deadline = tokio::time::Instant::now() + FETCH_TIMEOUT;
        let (final_url, response) = self
            .get_following_redirects(
                url,
                "text/html,application/xhtml+xml,image/*;q=0.8",
                deadline,
            )
            .await?;
        let content_type = content_type_of(&response);
        if PAGE_CONTENT_TYPES.contains(&content_type.as_str()) {
            let body = read_at_most(
                response,
                MAXIMUM_PAGE_BYTES,
                BodyOverflow::Truncate,
                deadline,
            )
            .await?;
            // Parsing a hostile megabyte is bounded but not free: off the async threads.
            let details = tokio::task::spawn_blocking(move || {
                page_details(&String::from_utf8_lossy(&body), &final_url)
            })
            .await
            .map_err(|_| FetchError::Unsupported)?;
            return Ok(Fetched::Page(details));
        }
        if content_type.starts_with("image/") {
            let body = read_at_most(
                response,
                MAXIMUM_PICTURE_BYTES,
                BodyOverflow::Fail,
                deadline,
            )
            .await?;
            return Ok(Fetched::Picture(body));
        }
        Err(FetchError::Unsupported)
    }

    /// The picture's bytes, not yet checked as a picture.
    ///
    /// # Errors
    /// Any refusal, network error, status other than 2xx, or a body over the limit.
    pub(crate) async fn fetch_picture(&self, url: Url) -> Result<Vec<u8>, FetchError> {
        let deadline = tokio::time::Instant::now() + FETCH_TIMEOUT;
        let (_, response) = self
            .get_following_redirects(url, "image/*", deadline)
            .await?;
        if !content_type_of(&response).starts_with("image/") {
            return Err(FetchError::Unsupported);
        }
        read_at_most(
            response,
            MAXIMUM_PICTURE_BYTES,
            BodyOverflow::Fail,
            deadline,
        )
        .await
    }

    async fn get_following_redirects(
        &self,
        mut url: Url,
        accept: &str,
        deadline: tokio::time::Instant,
    ) -> Result<(Url, reqwest::Response), FetchError> {
        for _ in 0..=MAXIMUM_REDIRECTS {
            if !is_allowed(&url, self.policy) {
                return Err(FetchError::NotAllowed);
            }
            let response = tokio::time::timeout_at(
                deadline,
                self.client.get(url.clone()).header(ACCEPT, accept).send(),
            )
            .await
            .map_err(|_| FetchError::TimedOut)??;
            let status = response.status();
            if status.is_redirection() {
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or(FetchError::Status)?;
                url = url.join(location).map_err(|_| FetchError::NotAllowed)?;
                continue;
            }
            if !status.is_success() {
                return Err(FetchError::Status);
            }
            return Ok((url, response));
        }
        Err(FetchError::TooManyRedirects)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BodyOverflow {
    /// Keep the first bytes (a page's head).
    Truncate,
    /// A partial body is useless (a picture).
    Fail,
}

/// The body, within `deadline` too: the whole fetch, redirects and body, has one time budget.
async fn read_at_most(
    response: reqwest::Response,
    maximum: usize,
    overflow: BodyOverflow,
    deadline: tokio::time::Instant,
) -> Result<Vec<u8>, FetchError> {
    tokio::time::timeout_at(deadline, read_body(response, maximum, overflow))
        .await
        .map_err(|_| FetchError::TimedOut)?
}

async fn read_body(
    mut response: reqwest::Response,
    maximum: usize,
    overflow: BodyOverflow,
) -> Result<Vec<u8>, FetchError> {
    if overflow == BodyOverflow::Fail
        && response
            .content_length()
            .is_some_and(|length| length > maximum as u64)
    {
        return Err(FetchError::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        let room = maximum - body.len();
        if chunk.len() > room {
            if overflow == BodyOverflow::Fail {
                return Err(FetchError::TooLarge);
            }
            body.extend_from_slice(&chunk[..room]);
            break;
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_public_default_port_web_links_without_credentials_are_allowed() {
        let allowed =
            |link: &str| is_allowed(&Url::parse(link).unwrap(), AddressPolicy::PublicOnly);
        assert!(allowed("https://example.org/a"));
        assert!(allowed("http://93.184.215.14/"));
        assert!(!allowed("http://127.0.0.1/"));
        assert!(!allowed("http://[::1]/"));
        assert!(!allowed("http://192.168.1.18/"));
        assert!(!allowed("http://169.254.169.254/latest/meta-data/"));
        assert!(!allowed("https://example.org:8443/"));
        assert!(!allowed("https://user@example.org/"));
        assert!(!allowed("file:///etc/passwd"));
        assert!(!allowed("gopher://example.org/"));
    }

    #[tokio::test]
    async fn names_of_private_addresses_are_refused_before_connecting() {
        let fetcher = LinkFetcher::new("Mellow test").unwrap();
        let refused = fetcher
            .fetch_page_or_picture(Url::parse("http://localhost/").unwrap())
            .await;
        assert!(
            matches!(refused, Err(FetchError::Network(_))),
            "{refused:?}"
        );
    }
}
