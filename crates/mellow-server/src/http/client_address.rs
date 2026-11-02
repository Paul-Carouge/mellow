//! Which client a request comes from, as a rate-limiting key.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use axum::http::HeaderMap;
use ipnet::{IpNet, Ipv4Net, Ipv6Net};

const FORWARDED_FOR_HEADER: &str = "x-forwarded-for";
/// An IPv6 end site gets at least a /64, so one client controls every address inside it.
const IPV6_CLIENT_PREFIX_BITS: u32 = 64;
/// The largest common end-site allocation: a free tunnel-broker /48 holds 65 536 /64s.
const IPV6_SITE_PREFIX_BITS: u32 = 48;

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
    match client_address(peer, headers, trusted_proxy_networks) {
        IpAddr::V4(address) => address.to_string(),
        IpAddr::V6(address) => address.to_ipv4_mapped().map_or_else(
            || ipv6_prefix_key(address, IPV6_CLIENT_PREFIX_BITS),
            |mapped| mapped.to_string(),
        ),
    }
}

/// The rate-limiting key for the IPv6 /48 a request's client sits in, the client found as in
/// [`client_address_key`]; `None` for IPv4 clients, whose address key already is their site.
pub(crate) fn client_ipv6_site_key(
    peer: SocketAddr,
    headers: &HeaderMap,
    trusted_proxy_networks: &[IpNet],
) -> Option<String> {
    match client_address(peer, headers, trusted_proxy_networks) {
        IpAddr::V6(address) if address.to_ipv4_mapped().is_none() => {
            Some(ipv6_prefix_key(address, IPV6_SITE_PREFIX_BITS))
        }
        IpAddr::V4(_) | IpAddr::V6(_) => None,
    }
}

fn ipv6_prefix_key(address: Ipv6Addr, prefix_bits: u32) -> String {
    let prefix_mask = u128::MAX << (128 - prefix_bits);
    let prefix = Ipv6Addr::from(address.to_bits() & prefix_mask);
    format!("{prefix}/{prefix_bits}")
}

fn client_address(
    peer: SocketAddr,
    headers: &HeaderMap,
    trusted_proxy_networks: &[IpNet],
) -> IpAddr {
    let peer_address = peer.ip().to_canonical();
    let is_peer_trusted_proxy = trusted_proxy_networks
        .iter()
        .any(|network| network.contains(&peer_address));
    if is_peer_trusted_proxy {
        last_forwarded_hop(headers).unwrap_or(peer.ip())
    } else {
        peer.ip()
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

    #[test]
    fn ipv6_clients_also_have_a_48_bit_site_key_and_ipv4_clients_none() {
        let first: SocketAddr = "[2001:db8:1:2:aaaa::1]:443".parse().unwrap();
        let same_site: SocketAddr = "[2001:db8:1:ffff::9]:443".parse().unwrap();
        let mapped: SocketAddr = "[::ffff:192.0.2.1]:443".parse().unwrap();
        let ipv4: SocketAddr = "192.0.2.1:443".parse().unwrap();
        let none = HeaderMap::new();
        assert_eq!(
            client_ipv6_site_key(first, &none, &[]).as_deref(),
            Some("2001:db8:1::/48")
        );
        assert_eq!(
            client_ipv6_site_key(first, &none, &[]),
            client_ipv6_site_key(same_site, &none, &[])
        );
        assert_eq!(client_ipv6_site_key(mapped, &none, &[]), None);
        assert_eq!(client_ipv6_site_key(ipv4, &none, &[]), None);
    }
}
