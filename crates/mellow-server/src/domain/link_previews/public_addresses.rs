//! Which addresses the link preview fetcher may connect to (ADR-0041): public unicast ones only,
//! so a link cannot make the instance reach its own machine or network.

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::pin::Pin;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

/// Whether `address` is a public unicast address, IPv4 inside IPv6 judged as IPv4.
pub(crate) fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_public_ipv4(address),
        IpAddr::V6(address) => is_public_ipv6(address),
    }
}

fn is_public_ipv4(address: Ipv4Addr) -> bool {
    let [first, second, third, _] = address.octets();
    let is_in_special_range = first == 0 // "this network"
        || first == 10
        || first == 127
        || (first == 100 && (64..128).contains(&second)) // carrier-grade NAT
        || (first == 169 && second == 254)
        || (first == 172 && (16..32).contains(&second))
        || (first == 192 && second == 0 && third == 0) // IETF protocol assignments
        || (first == 192 && second == 0 && third == 2) // documentation
        || (first == 192 && second == 88 && third == 99) // 6to4 relay
        || (first == 192 && second == 168)
        || (first == 198 && (18..20).contains(&second)) // benchmarking
        || (first == 198 && second == 51 && third == 100) // documentation
        || (first == 203 && second == 0 && third == 113) // documentation
        || first >= 224; // multicast, reserved and broadcast
    !is_in_special_range
}

fn is_public_ipv6(address: Ipv6Addr) -> bool {
    if let Some(ipv4) = embedded_ipv4(address) {
        return is_public_ipv4(ipv4);
    }
    let segments = address.segments();
    let is_in_special_range = address.is_unspecified()
        || address.is_loopback()
        || (segments[0] & 0xfe00) == 0xfc00 // unique local
        || (segments[0] & 0xffc0) == 0xfe80 // link-local
        || (segments[0] & 0xffc0) == 0xfec0 // site-local (deprecated)
        || (segments[0] & 0xff00) == 0xff00 // multicast
        || (segments[0] == 0x2001 && segments[1] == 0x0db8) // documentation
        || (segments[0] == 0x3fff && segments[1] < 0x1000) // documentation (RFC 9637)
        || (segments[0] == 0x0100 && segments[1..4] == [0, 0, 0]) // discard
        || (segments[0] == 0x2001 && segments[1] < 0x0200); // IETF protocol assignments
    // Only global unicast (2000::/3) is reachable on the Internet.
    !is_in_special_range && (segments[0] & 0xe000) == 0x2000
}

/// The IPv4 address an IPv6 address carries: mapped (`::ffff:a.b.c.d`), compatible
/// (`::a.b.c.d`), NAT64 (`64:ff9b::a.b.c.d`) or 6to4 (`2002:aabb:ccdd::`).
fn embedded_ipv4(address: Ipv6Addr) -> Option<Ipv4Addr> {
    let segments = address.segments();
    let octets = address.octets();
    let last_four = Ipv4Addr::new(octets[12], octets[13], octets[14], octets[15]);
    if segments[..5] == [0; 5] && (segments[5] == 0xffff || segments[5] == 0) {
        // `::` and `::1` are handled as IPv6.
        return (segments[5] == 0xffff || !(address.is_unspecified() || address.is_loopback()))
            .then_some(last_four);
    }
    if segments[0] == 0x0064 && segments[1] == 0xff9b && segments[2..6] == [0; 4] {
        return Some(last_four);
    }
    if segments[0] == 0x2002 {
        return Some(Ipv4Addr::new(octets[2], octets[3], octets[4], octets[5]));
    }
    None
}

/// Which addresses the fetcher may reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AddressPolicy {
    /// Public unicast addresses, on ports 80 and 443: the only policy outside tests.
    PublicOnly,
    /// Also this machine, on any port, so tests can serve pages locally; every other
    /// non-public address stays refused.
    #[cfg(test)]
    AlsoLoopbackOnAnyPort,
}

impl AddressPolicy {
    pub(crate) fn allows(self, address: IpAddr) -> bool {
        match self {
            Self::PublicOnly => is_public_address(address),
            #[cfg(test)]
            Self::AlsoLoopbackOnAnyPort => address.is_loopback() || is_public_address(address),
        }
    }

    pub(crate) fn allows_port(self, port: Option<u16>) -> bool {
        match self {
            Self::PublicOnly => port.is_none_or(|port| [80, 443].contains(&port)),
            #[cfg(test)]
            Self::AlsoLoopbackOnAnyPort => true,
        }
    }
}

/// Resolves names with the system resolver and refuses a name if any of its addresses is not
/// allowed, so the connection only ever goes to an address checked here.
#[derive(Debug)]
pub(crate) struct PublicAddressResolver {
    pub(crate) policy: AddressPolicy,
}

#[derive(Debug, thiserror::Error)]
#[error("the name resolves to an address that is not public")]
struct NotPublic;

impl Resolve for PublicAddressResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let host = name.as_str().to_owned();
        let policy = self.policy;
        let resolving: Pin<Box<dyn Future<Output = _> + Send>> = Box::pin(async move {
            let addresses: Vec<SocketAddr> =
                tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            if addresses.is_empty() || !addresses.iter().all(|address| policy.allows(address.ip()))
            {
                return Err(Box::new(NotPublic) as Box<dyn std::error::Error + Send + Sync>);
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        });
        resolving
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_public(text: &str) -> bool {
        is_public_address(text.parse().unwrap())
    }

    #[test]
    fn private_loopback_and_special_ipv4_ranges_are_refused() {
        for address in [
            "0.0.0.0",
            "10.1.2.3",
            "100.64.0.1",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.8",
            "192.0.2.1",
            "192.168.1.18",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.9",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
        ] {
            assert!(!is_public(address), "{address}");
        }
        for address in [
            "1.1.1.1",
            "8.8.8.8",
            "100.63.255.255",
            "172.32.0.1",
            "198.20.0.1",
        ] {
            assert!(is_public(address), "{address}");
        }
    }

    #[test]
    fn ipv6_special_ranges_and_embedded_private_ipv4_are_refused() {
        for address in [
            "::",
            "::1",
            "fc00::1",
            "fd12:3456::1",
            "fe80::1",
            "fec0::1",
            "ff02::1",
            "2001:db8::1",
            "100::1",
            "2001::1",
            "::ffff:127.0.0.1",
            "::ffff:192.168.1.18",
            "::10.0.0.1",
            "64:ff9b::10.0.0.1",
            "2002:c0a8:0112::1",
            "3fff::1",
        ] {
            assert!(!is_public(address), "{address}");
        }
        for address in [
            "2606:4700:4700::1111",
            "2a00:1450:4007:80e::200e",
            "::ffff:1.1.1.1",
            "64:ff9b::808:808",
        ] {
            assert!(is_public(address), "{address}");
        }
    }
}
