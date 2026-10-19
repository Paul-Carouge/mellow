//! Which client a request comes from, as a rate-limiting key.

use std::net::{IpAddr, Ipv6Addr, SocketAddr};

use axum::http::HeaderMap;

const FORWARDED_FOR_HEADER: &str = "x-forwarded-for";
/// An IPv6 end site gets at least a /64, so one client controls every address inside it.
const IPV6_CLIENT_PREFIX_BITS: u32 = 64;

/// The rate-limiting key for a request's client.
///
/// Behind a TLS reverse proxy, the client is the last `X-Forwarded-For` hop (the one the proxy
/// appended; earlier hops are client-controlled), and only when the connection comes from
/// loopback, i.e. from the proxy on the same host. A client reaching Mellow's port directly
/// cannot choose its own key that way. IPv6 clients are grouped by /64.
pub(crate) fn client_address_key(
    peer: SocketAddr,
    headers: &HeaderMap,
    is_behind_tls_reverse_proxy: bool,
) -> String {
    let address = if is_behind_tls_reverse_proxy && peer.ip().is_loopback() {
        last_forwarded_hop(headers).unwrap_or(peer.ip())
    } else {
        peer.ip()
    };
    match address {
        IpAddr::V4(address) => address.to_string(),
        IpAddr::V6(address) => {
            if let Some(mapped) = address.to_ipv4_mapped() {
                mapped.to_string()
            } else {
                let prefix_mask = u128::MAX << (128 - IPV6_CLIENT_PREFIX_BITS);
                let prefix = Ipv6Addr::from(address.to_bits() & prefix_mask);
                format!("{prefix}/{IPV6_CLIENT_PREFIX_BITS}")
            }
        }
    }
}

fn last_forwarded_hop(headers: &HeaderMap) -> Option<IpAddr> {
    headers
        .get_all(FORWARDED_FOR_HEADER)
        .iter()
        .next_back()
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.rsplit(',').next())
        .and_then(|last_hop| last_hop.trim().parse().ok())
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn forwarded(value: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(FORWARDED_FOR_HEADER, HeaderValue::from_static(value));
        headers
    }

    #[test]
    fn forwarded_header_is_trusted_only_from_a_loopback_proxy_and_only_its_last_hop() {
        let proxy: SocketAddr = "127.0.0.1:50000".parse().unwrap();
        let direct_client: SocketAddr = "198.51.100.4:50000".parse().unwrap();
        let headers = forwarded("6.6.6.6, 203.0.113.7");

        assert_eq!(client_address_key(proxy, &headers, true), "203.0.113.7");
        assert_eq!(client_address_key(proxy, &headers, false), "127.0.0.1");
        assert_eq!(
            client_address_key(direct_client, &headers, true),
            "198.51.100.4"
        );
        assert_eq!(
            client_address_key(proxy, &HeaderMap::new(), true),
            "127.0.0.1"
        );
    }

    #[test]
    fn ipv6_clients_are_grouped_by_their_64_bit_prefix() {
        let first: SocketAddr = "[2001:db8:1:2:aaaa::1]:443".parse().unwrap();
        let second: SocketAddr = "[2001:db8:1:2:bbbb::9]:443".parse().unwrap();
        let other_site: SocketAddr = "[2001:db8:1:3::1]:443".parse().unwrap();
        let mapped: SocketAddr = "[::ffff:192.0.2.1]:443".parse().unwrap();
        let none = HeaderMap::new();
        assert_eq!(client_address_key(first, &none, false), "2001:db8:1:2::/64");
        assert_eq!(
            client_address_key(first, &none, false),
            client_address_key(second, &none, false)
        );
        assert_ne!(
            client_address_key(first, &none, false),
            client_address_key(other_site, &none, false)
        );
        assert_eq!(client_address_key(mapped, &none, false), "192.0.2.1");
    }
}
