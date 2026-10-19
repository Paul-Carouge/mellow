//! Which client a request comes from, as a rate-limiting key.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use axum::http::HeaderMap;
use ipnet::{IpNet, Ipv4Net, Ipv6Net};

const FORWARDED_FOR_HEADER: &str = "x-forwarded-for";
/// An IPv6 end site gets at least a /64, so one client controls every address inside it.
const IPV6_CLIENT_PREFIX_BITS: u32 = 64;

/// The rate-limiting key for a request's client.
///
/// Behind a TLS reverse proxy, the client is the last `X-Forwarded-For` hop (the one the proxy
/// appended; earlier hops are client-controlled), and only when the connection comes from a
/// trusted proxy network (see [`build_trusted_proxy_networks`]). A client reaching Mellow's port
/// directly cannot choose its own key that way. IPv6 clients are grouped by /64.
pub(crate) fn client_address_key(
    peer: SocketAddr,
    headers: &HeaderMap,
    trusted_proxy_networks: &[IpNet],
) -> String {
    let peer_address = peer.ip().to_canonical();
    let is_peer_trusted_proxy = trusted_proxy_networks
        .iter()
        .any(|network| network.contains(&peer_address));
    let address = if is_peer_trusted_proxy {
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

/// Loopback (a proxy on this host) plus the configured networks, or nothing without a proxy.
pub(crate) fn build_trusted_proxy_networks(
    is_behind_tls_reverse_proxy: bool,
    configured_networks: &[IpNet],
) -> Vec<IpNet> {
    if !is_behind_tls_reverse_proxy {
        return Vec::new();
    }
    let loopback_networks = [
        IpNet::V4(Ipv4Net::new_assert(Ipv4Addr::new(127, 0, 0, 0), 8)),
        IpNet::V6(Ipv6Net::new_assert(Ipv6Addr::LOCALHOST, 128)),
    ];
    loopback_networks
        .into_iter()
        .chain(configured_networks.iter().copied())
        .collect()
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
        let behind_proxy = build_trusted_proxy_networks(true, &[]);
        let without_proxy = build_trusted_proxy_networks(false, &[]);
        let proxy: SocketAddr = "127.0.0.1:50000".parse().unwrap();
        let direct_client: SocketAddr = "198.51.100.4:50000".parse().unwrap();
        let headers = forwarded("6.6.6.6, 203.0.113.7");

        assert_eq!(
            client_address_key(proxy, &headers, &behind_proxy),
            "203.0.113.7"
        );
        assert_eq!(
            client_address_key(proxy, &headers, &without_proxy),
            "127.0.0.1"
        );
        assert_eq!(
            client_address_key(direct_client, &headers, &behind_proxy),
            "198.51.100.4"
        );
        assert_eq!(
            client_address_key(proxy, &HeaderMap::new(), &behind_proxy),
            "127.0.0.1"
        );
    }

    #[test]
    fn forwarded_header_is_trusted_from_a_configured_proxy_network_only() {
        let docker_network: IpNet = "172.18.0.0/16".parse().unwrap();
        let behind_docker_proxy = build_trusted_proxy_networks(true, &[docker_network]);
        let docker_proxy: SocketAddr = "172.18.0.4:40000".parse().unwrap();
        let other_container: SocketAddr = "172.19.0.4:40000".parse().unwrap();
        let mapped_docker_proxy: SocketAddr = "[::ffff:172.18.0.4]:40000".parse().unwrap();
        let headers = forwarded("203.0.113.7");

        assert_eq!(
            client_address_key(docker_proxy, &headers, &behind_docker_proxy),
            "203.0.113.7"
        );
        assert_eq!(
            client_address_key(mapped_docker_proxy, &headers, &behind_docker_proxy),
            "203.0.113.7"
        );
        assert_eq!(
            client_address_key(other_container, &headers, &behind_docker_proxy),
            "172.19.0.4"
        );
        assert_eq!(
            build_trusted_proxy_networks(false, &[docker_network]),
            Vec::<IpNet>::new()
        );
    }

    #[test]
    fn ipv6_clients_are_grouped_by_their_64_bit_prefix() {
        let first: SocketAddr = "[2001:db8:1:2:aaaa::1]:443".parse().unwrap();
        let second: SocketAddr = "[2001:db8:1:2:bbbb::9]:443".parse().unwrap();
        let other_site: SocketAddr = "[2001:db8:1:3::1]:443".parse().unwrap();
        let mapped: SocketAddr = "[::ffff:192.0.2.1]:443".parse().unwrap();
        let none = HeaderMap::new();
        assert_eq!(client_address_key(first, &none, &[]), "2001:db8:1:2::/64");
        assert_eq!(
            client_address_key(first, &none, &[]),
            client_address_key(second, &none, &[])
        );
        assert_ne!(
            client_address_key(first, &none, &[]),
            client_address_key(other_site, &none, &[])
        );
        assert_eq!(client_address_key(mapped, &none, &[]), "192.0.2.1");
    }
}
