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
// YouTube sends some servers through its consent page and back, four hops for a Shorts link.
const MAXIMUM_REDIRECTS: usize = 5;
/// The head of a page is enough; a longer page is read up to here and parsed as it is.
const MAXIMUM_PAGE_BYTES: usize = 1024 * 1024;
pub(crate) const MAXIMUM_PICTURE_BYTES: usize = 8 * 1024 * 1024;
/// A provider's API or oEmbed answer (ADR-0044): a few kilobytes; a longer one is refused, since
/// a truncated document cannot be read.
const MAXIMUM_JSON_BYTES: usize = 64 * 1024;
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
    /// A status that is neither a success nor a redirect, or a redirect without a location.
    #[error("unexpected status {0}")]
    Status(reqwest::StatusCode),
    #[error("unsupported content")]
    Unsupported,
    #[error("too large")]
    TooLarge,
    #[error("timed out")]
    TimedOut,
    #[error("cannot build the HTTP client")]
    Client,
}

impl FetchError {
    /// The site may answer if asked again soon: it did not answer in time, the network failed,
    /// or it said it is busy or broken (429, 5xx). Any other error is the site's answer.
    pub(crate) fn is_transient(&self) -> bool {
        match self {
            Self::TimedOut | Self::Network(_) => true,
            Self::Status(status) => {
                *status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
            }
            Self::NotAllowed
            | Self::TooManyRedirects
            | Self::Unsupported
            | Self::TooLarge
            | Self::Client => false,
        }
    }
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
    /// The local site standing in for every host; only tests set it
    /// (`stand_in_for_every_host_for_tests`).
    stand_in_site: Option<Url>,
}

/// Whether the fetcher may request `url` (before any name resolution, which the resolver
/// checks): `http` or `https`, default ports, no credentials, and a public address when the
/// host is one.
/// Klipy's pages and media are never fetched by the server, whatever leads there (a link, a
/// redirect, a page's picture): its terms require members' browsers to load them (ADR-0042).
pub(crate) fn is_klipy_host(url: &Url) -> bool {
    url.host_str().is_some_and(|host| {
        let host = host.trim_end_matches('.');
        host.eq_ignore_ascii_case("klipy.com") || host.to_ascii_lowercase().ends_with(".klipy.com")
    })
}

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

    /// Sends every request to `site` instead, the real host as the path's first segment
    /// (`https://www.youtube.com/oembed?url=…` asks `{site}/www.youtube.com/oembed?url=…`), so
    /// tests stand in for providers' fixed endpoints without touching the network.
    #[cfg(test)]
    pub(crate) fn stand_in_for_every_host_for_tests(site: &str) -> Self {
        Self {
            stand_in_site: Some(Url::parse(site).unwrap()),
            ..Self::allowing_loopback_for_tests()
        }
    }

    fn request_url(&self, url: &Url) -> Url {
        let Some(site) = &self.stand_in_site else {
            return url.clone();
        };
        let mut request_url = site.clone();
        request_url.set_path(&format!(
            "{}{}",
            url.host_str().unwrap_or_default(),
            url.path()
        ));
        request_url.set_query(url.query());
        request_url
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
        Ok(Self {
            client,
            policy,
            stand_in_site: None,
        })
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

    /// A provider's JSON answer (ADR-0044), read into `T`, under the same guard as pages.
    ///
    /// # Errors
    /// Any refusal, network error, status other than 2xx, a type other than JSON, a body over
    /// 64 KiB, or JSON that is not a `T`.
    pub(crate) async fn fetch_json<T: serde::de::DeserializeOwned>(
        &self,
        url: Url,
    ) -> Result<T, FetchError> {
        let deadline = tokio::time::Instant::now() + FETCH_TIMEOUT;
        let (_, response) = self
            .get_following_redirects(url, "application/json", deadline)
            .await?;
        let content_type = content_type_of(&response);
        // `application/json`, oEmbed's `application/json+oembed`, or a `+json` type.
        if !content_type.starts_with("application/json") && !content_type.ends_with("+json") {
            return Err(FetchError::Unsupported);
        }
        let body = read_at_most(response, MAXIMUM_JSON_BYTES, BodyOverflow::Fail, deadline).await?;
        // Parsed in place: serde_json is linear and the body is 16 times smaller than a page.
        serde_json::from_slice(&body).map_err(|_| FetchError::Unsupported)
    }

    async fn get_following_redirects(
        &self,
        mut url: Url,
        accept: &str,
        deadline: tokio::time::Instant,
    ) -> Result<(Url, reqwest::Response), FetchError> {
        for _ in 0..=MAXIMUM_REDIRECTS {
            if !is_allowed(&url, self.policy) || is_klipy_host(&url) {
                return Err(FetchError::NotAllowed);
            }
            let response = tokio::time::timeout_at(
                deadline,
                self.client
                    .get(self.request_url(&url))
                    .header(ACCEPT, accept)
                    .send(),
            )
            .await
            .map_err(|_| FetchError::TimedOut)??;
            let status = response.status();
            if status.is_redirection() {
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or(FetchError::Status(status))?;
                url = url.join(location).map_err(|_| FetchError::NotAllowed)?;
                continue;
            }
            if !status.is_success() {
                return Err(FetchError::Status(status));
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
    use axum::response::IntoResponse;

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
        for klipy in [
            "https://static.klipy.com/ii/x.webp",
            "https://static.klipy.com./ii/x.webp",
            "https://KLIPY.com/gifs/x",
        ] {
            assert!(is_klipy_host(&Url::parse(klipy).unwrap()), "{klipy}");
        }
        assert!(!is_klipy_host(
            &Url::parse("https://klipy.com.evil.example/").unwrap()
        ));
        assert!(!is_klipy_host(
            &Url::parse("https://notklipy.com/").unwrap()
        ));
    }

    #[tokio::test]
    async fn a_redirect_to_klipy_is_never_followed() {
        let site = axum::Router::new().route(
            "/gif",
            axum::routing::get(|| async {
                (
                    axum::http::StatusCode::FOUND,
                    [(
                        axum::http::header::LOCATION,
                        "https://static.klipy.com./ii/x.webp",
                    )],
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, site).await });
        let fetcher = LinkFetcher::allowing_loopback_for_tests();
        let page = fetcher
            .fetch_page_or_picture(Url::parse(&format!("http://{address}/gif")).unwrap())
            .await;
        let picture = fetcher
            .fetch_picture(Url::parse(&format!("http://{address}/gif")).unwrap())
            .await;
        assert!(matches!(page, Err(FetchError::NotAllowed)), "{page:?}");
        assert!(
            matches!(picture, Err(FetchError::NotAllowed)),
            "a page's picture too"
        );
    }

    /// A site answering `/hop/<n>` with a redirect to `/hop/<n - 1>`, and a page at `/hop/0`.
    async fn serve_redirect_chain() -> std::net::SocketAddr {
        let site = axum::Router::new().route(
            "/hop/{remaining}",
            axum::routing::get(
                |axum::extract::Path(remaining): axum::extract::Path<usize>| async move {
                    if remaining == 0 {
                        return axum::response::Html(
                            "<html><head><title>Arrived</title></head></html>",
                        )
                        .into_response();
                    }
                    axum::response::Redirect::to(&format!("/hop/{}", remaining - 1)).into_response()
                },
            ),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, site).await });
        address
    }

    #[tokio::test]
    async fn a_chain_of_four_redirects_is_followed() {
        // YouTube's Shorts links go through a consent page and back: four hops from some servers.
        let address = serve_redirect_chain().await;
        let fetcher = LinkFetcher::allowing_loopback_for_tests();
        let outcome = fetcher
            .fetch_page_or_picture(Url::parse(&format!("http://{address}/hop/4")).unwrap())
            .await;
        let Ok(Fetched::Page(page)) = outcome else {
            panic!("{outcome:?}");
        };
        assert_eq!(page.title.as_deref(), Some("Arrived"));
    }

    #[tokio::test]
    async fn a_chain_longer_than_the_redirect_limit_is_refused() {
        let address = serve_redirect_chain().await;
        let fetcher = LinkFetcher::allowing_loopback_for_tests();
        let outcome = fetcher
            .fetch_page_or_picture(
                Url::parse(&format!("http://{address}/hop/{}", MAXIMUM_REDIRECTS + 1)).unwrap(),
            )
            .await;
        assert!(
            matches!(outcome, Err(FetchError::TooManyRedirects)),
            "{outcome:?}"
        );
    }

    #[derive(Debug, serde::Deserialize)]
    struct OEmbed {
        title: String,
    }

    #[tokio::test]
    async fn json_up_to_64_kib_is_read_and_longer_or_other_answers_are_refused() {
        fn json_of_length(length: usize) -> String {
            let padding = length - r#"{"title":"Talk","padding":""}"#.len();
            format!(r#"{{"title":"Talk","padding":"{}"}}"#, "x".repeat(padding))
        }
        fn json(body: String) -> impl axum::response::IntoResponse {
            (
                [(axum::http::header::CONTENT_TYPE, "application/json")],
                body,
            )
        }
        let site = axum::Router::new()
            .route(
                "/fits",
                axum::routing::get(move || async move { json(json_of_length(MAXIMUM_JSON_BYTES)) }),
            )
            .route(
                "/oembed",
                axum::routing::get(|| async {
                    (
                        [(
                            axum::http::header::CONTENT_TYPE,
                            "application/json+oembed; charset=utf-8",
                        )],
                        r#"{"title":"Talk"}"#,
                    )
                }),
            )
            .route(
                "/too-long",
                axum::routing::get(
                    move || async move { json(json_of_length(MAXIMUM_JSON_BYTES + 1)) },
                ),
            )
            .route(
                // No length announced: the cap holds while reading.
                "/too-long-streamed",
                axum::routing::get(|| async {
                    let chunks = (0..=MAXIMUM_JSON_BYTES / 1024).map(|_| {
                        Ok::<_, std::convert::Infallible>(axum::body::Bytes::from(vec![b' '; 1024]))
                    });
                    (
                        [(axum::http::header::CONTENT_TYPE, "application/json")],
                        axum::body::Body::from_stream(futures_util::stream::iter(chunks)),
                    )
                }),
            )
            .route(
                "/page",
                axum::routing::get(|| async { axum::response::Html(r#"{"title":"Talk"}"#) }),
            )
            .route(
                "/not-json",
                axum::routing::get(|| async { json("<html>".into()) }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, site).await });
        let fetcher = LinkFetcher::allowing_loopback_for_tests();
        let fetch = |path: &str| {
            fetcher.fetch_json::<OEmbed>(Url::parse(&format!("http://{address}{path}")).unwrap())
        };

        assert_eq!(fetch("/fits").await.unwrap().title, "Talk");
        assert_eq!(fetch("/oembed").await.unwrap().title, "Talk");
        let too_long = fetch("/too-long").await;
        assert!(
            matches!(too_long, Err(FetchError::TooLarge)),
            "{too_long:?}"
        );
        let streamed = fetch("/too-long-streamed").await;
        assert!(
            matches!(streamed, Err(FetchError::TooLarge)),
            "{streamed:?}"
        );
        let page = fetch("/page").await;
        assert!(matches!(page, Err(FetchError::Unsupported)), "{page:?}");
        let not_json = fetch("/not-json").await;
        assert!(
            matches!(not_json, Err(FetchError::Unsupported)),
            "{not_json:?}"
        );
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
