//! Where an instance lives, and the rule that tokens never travel unencrypted.

use url::{Host, Url};

use crate::error::ClientError;

/// The base URL of one Mellow instance, e.g. `https://chat.example.org`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceAddress {
    base: Url,
}

impl InstanceAddress {
    /// HTTPS only, except plain HTTP on the loopback interface for local development: session
    /// tokens must never cross a network unencrypted. A bare host (`chat.example.org`) means HTTPS.
    ///
    /// # Errors
    /// `InvalidInstanceAddress` for anything else (other schemes, credentials, query strings).
    pub fn parse(text: &str) -> Result<Self, ClientError> {
        let invalid = |reason: &str| ClientError::InvalidInstanceAddress(reason.to_owned());
        let text = text.trim();
        let with_scheme = if text.contains("://") {
            text.to_owned()
        } else {
            format!("https://{text}")
        };
        let mut base = Url::parse(&with_scheme).map_err(|_| invalid("not a valid address"))?;
        let is_loopback = match base.host() {
            Some(Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
            Some(Host::Ipv4(address)) => address.is_loopback(),
            Some(Host::Ipv6(address)) => address.is_loopback(),
            None => return Err(invalid("missing host")),
        };
        match base.scheme() {
            "https" => {}
            "http" if is_loopback => {}
            "http" => {
                return Err(invalid(
                    "plain http is only allowed for localhost; use https",
                ));
            }
            _ => return Err(invalid("the address must start with https://")),
        }
        if !base.username().is_empty() || base.password().is_some() {
            return Err(invalid("credentials do not belong in the address"));
        }
        if base.query().is_some() || base.fragment().is_some() {
            return Err(invalid("the address must not have a query or fragment"));
        }
        if !base.path().ends_with('/') {
            let path = format!("{}/", base.path());
            base.set_path(&path);
        }
        Ok(Self { base })
    }

    /// `path` is relative, e.g. `api/v1/users/@me`.
    pub(crate) fn api_url(&self, path: &str) -> Result<Url, ClientError> {
        self.base
            .join(path)
            .map_err(|_| ClientError::InvalidInstanceAddress(format!("cannot build {path}")))
    }

    /// The gateway URL, `wss://` (or `ws://` on loopback).
    pub(crate) fn gateway_url(&self) -> Result<Url, ClientError> {
        let mut url = self.api_url(&format!("gateway?v={}", mellow_protocol::GATEWAY_VERSION))?;
        let scheme = if self.base.scheme() == "https" {
            "wss"
        } else {
            "ws"
        };
        url.set_scheme(scheme).map_err(|()| {
            ClientError::InvalidInstanceAddress("cannot build the gateway URL".into())
        })?;
        Ok(url)
    }

    /// The host name shown to people, e.g. `chat.example.org`.
    #[must_use]
    pub fn host(&self) -> &str {
        self.base.host_str().unwrap_or_default()
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        self.base.as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_anywhere_and_http_only_on_loopback() {
        let instance = InstanceAddress::parse("https://chat.example.org").unwrap();
        assert_eq!(
            instance.api_url("api/v1/health").unwrap().as_str(),
            "https://chat.example.org/api/v1/health"
        );
        assert_eq!(
            instance.gateway_url().unwrap().as_str(),
            "wss://chat.example.org/gateway?v=1"
        );

        let local = InstanceAddress::parse("http://127.0.0.1:8080").unwrap();
        assert_eq!(local.gateway_url().unwrap().scheme(), "ws");
        assert!(InstanceAddress::parse("http://localhost:8080/").is_ok());
        assert!(InstanceAddress::parse("http://[::1]:8080").is_ok());

        let bare = InstanceAddress::parse("chat.example.org").unwrap();
        assert_eq!(bare.as_str(), "https://chat.example.org/");
        assert_eq!(bare.host(), "chat.example.org");

        let under_path = InstanceAddress::parse("https://example.org/mellow").unwrap();
        assert_eq!(
            under_path.api_url("api/v1/health").unwrap().as_str(),
            "https://example.org/mellow/api/v1/health"
        );

        for invalid in [
            "http://chat.example.org",
            "ftp://chat.example.org",
            "https://user:pass@chat.example.org",
            "https://chat.example.org/?x=1",
            "",
        ] {
            assert!(InstanceAddress::parse(invalid).is_err(), "{invalid}");
        }
    }
}
