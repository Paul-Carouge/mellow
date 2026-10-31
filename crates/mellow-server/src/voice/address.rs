//! The address the server announces as its only ICE candidate (ADR-0030): `[voice]
//! public_address` when set, else the host the client reached the gateway at. Host values come
//! from clients: they are checked before any lookup and never logged.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

/// DNS names are at most 253 characters.
const MAXIMUM_HOST_NAME_LENGTH: usize = 253;
/// A slow resolver must not hold a join, and the gateway loop behind it, for long.
const HOST_LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

/// A DNS name or an IPv4 address, as written in `public_address` or in a `Host` header without
/// its port: letters, digits, hyphens and dots only.
pub(crate) fn is_valid_host(host: &str) -> bool {
    (1..=MAXIMUM_HOST_NAME_LENGTH).contains(&host.len())
        && host
            .split('.')
            .all(|label| !label.is_empty() && !label.starts_with('-') && !label.ends_with('-'))
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.')
}

/// The host of a `Host` header, its port removed, if it is a valid host. An IPv6 literal is
/// refused: the media socket listens on IPv4 only.
pub(crate) fn host_from_header(host_header: &str) -> Option<&str> {
    let host = match host_header.rsplit_once(':') {
        Some((host, port))
            if !port.is_empty() && port.bytes().all(|byte| byte.is_ascii_digit()) =>
        {
            host
        }
        _ => host_header,
    };
    is_valid_host(host).then_some(host)
}

/// The IPv4 address of `host` (a literal or a name, resolved now since home addresses change),
/// with `port`. `None` when it cannot be resolved to IPv4 in time.
pub(crate) async fn resolve_candidate_address(host: &str, port: u16) -> Option<SocketAddr> {
    if let Ok(address) = host.parse::<Ipv4Addr>() {
        return Some(SocketAddr::new(IpAddr::V4(address), port));
    }
    if !is_valid_host(host) {
        return None;
    }
    let addresses =
        tokio::time::timeout(HOST_LOOKUP_TIMEOUT, tokio::net::lookup_host((host, port)))
            .await
            .ok()?
            .ok()?;
    addresses.into_iter().find(SocketAddr::is_ipv4)
}

/// Browsers do not send media to a loopback candidate from their network interfaces, so an
/// instance reached as `localhost` announces the machine's own network address instead. No
/// packet is sent: connecting a UDP socket only picks the outgoing interface. Kept as is when the
/// machine has no route (offline), where loopback is all there is.
pub(crate) fn announceable_from_browsers(address: SocketAddr) -> SocketAddr {
    if !address.ip().is_loopback() {
        return address;
    }
    let local_network_address = std::net::UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .and_then(|probe| {
            probe.connect(ROUTE_PROBE_DESTINATION)?;
            probe.local_addr()
        })
        .ok()
        .map(|probe_address| probe_address.ip())
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified());
    local_network_address.map_or(address, |ip| SocketAddr::new(ip, address.port()))
}

/// A documentation address (RFC 5737): only used to ask the routing table for an interface.
const ROUTE_PROBE_DESTINATION: (Ipv4Addr, u16) = (Ipv4Addr::new(192, 0, 2, 1), 9);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosts_are_names_or_ipv4_and_lose_their_port() {
        assert_eq!(
            host_from_header("chat.example.org"),
            Some("chat.example.org")
        );
        assert_eq!(
            host_from_header("chat.example.org:443"),
            Some("chat.example.org")
        );
        assert_eq!(host_from_header("192.0.2.10:8080"), Some("192.0.2.10"));
        assert_eq!(host_from_header("localhost"), Some("localhost"));
        for refused in [
            "",
            "[2001:db8::1]:443",
            "chat example.org",
            "chat.example.org:",
            "-bad.example.org",
            "a..b",
            "évil.example",
            "a/b",
        ] {
            assert_eq!(host_from_header(refused), None, "{refused:?}");
        }
        assert!(!is_valid_host(&"a".repeat(254)));
    }

    #[test]
    fn loopback_becomes_a_network_address_and_others_are_kept() {
        let public: SocketAddr = "192.0.2.10:50000".parse().unwrap();
        assert_eq!(announceable_from_browsers(public), public);
        let announced = announceable_from_browsers("127.0.0.1:50000".parse().unwrap());
        assert_eq!(announced.port(), 50000);
        // Offline machines keep loopback; any other answer must be a real interface address.
        assert!(announced.ip() == Ipv4Addr::LOCALHOST || !announced.ip().is_loopback());
    }

    #[tokio::test]
    async fn literals_resolve_without_a_lookup_and_invalid_names_do_not_resolve() {
        assert_eq!(
            resolve_candidate_address("192.0.2.10", 50000).await,
            Some("192.0.2.10:50000".parse().unwrap())
        );
        assert_eq!(resolve_candidate_address("bad name", 50000).await, None);
        assert_eq!(
            resolve_candidate_address("localhost", 50000)
                .await
                .map(|address| address.port()),
            Some(50000)
        );
    }
}
