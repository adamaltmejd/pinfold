//! Address checks for the egress proxy.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};

use crate::core::plan::Target;

/// The address a host names when it is an IP literal. The bracketed IPv6
/// form counts too.
pub fn literal(host: &str) -> Option<IpAddr> {
    let host = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    host.parse().ok()
}

/// Why a name could not be dialed.
pub enum ResolveError {
    /// The name did not resolve.
    Unresolved,
    /// It resolved to an address the proxy must not dial.
    Forbidden(&'static str),
}

/// Resolve `host:port` once and return the first address. Every address the
/// name resolves to is checked, so a name that resolves to a forbidden
/// address is refused even when another address would be allowed.
pub fn resolve(host: &str, port: u16) -> Result<SocketAddr, ResolveError> {
    let addresses: Vec<SocketAddr> = (host, port)
        .to_socket_addrs()
        .map_err(|_| ResolveError::Unresolved)?
        .collect();
    for address in &addresses {
        if let Some(reason) = forbidden(address.ip()) {
            return Err(ResolveError::Forbidden(reason));
        }
    }
    addresses.first().copied().ok_or(ResolveError::Unresolved)
}

/// Whether an address must not be dialed, and the log reason.
pub fn forbidden(ip: IpAddr) -> Option<&'static str> {
    match ip {
        IpAddr::V4(ip) => forbidden_v4(ip),
        IpAddr::V6(ip) => forbidden_v6(ip),
    }
}

fn forbidden_v4(ip: Ipv4Addr) -> Option<&'static str> {
    if ip.is_loopback() {
        return Some("loopback");
    }
    let octets = ip.octets();
    // 0.0.0.0/8; connecting to the unspecified address reaches the local
    // host.
    if octets[0] == 0 {
        return Some("unspecified");
    }
    if ip.is_private() {
        return Some("private");
    }
    if ip.is_link_local() {
        return Some("link-local");
    }
    // 100.64.0.0/10.
    if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        return Some("cgnat");
    }
    // 198.18.0.0/15.
    if octets[0] == 198 && (octets[1] == 18 || octets[1] == 19) {
        return Some("benchmark");
    }
    // 192.0.0.0/24.
    if octets[0] == 192 && octets[1] == 0 && octets[2] == 0 {
        return Some("ietf-protocol");
    }
    // 224.0.0.0/4.
    if ip.is_multicast() {
        return Some("multicast");
    }
    // 240.0.0.0/4, which includes the broadcast address.
    if octets[0] >= 240 {
        return Some("reserved");
    }
    None
}

fn forbidden_v6(ip: Ipv6Addr) -> Option<&'static str> {
    if ip.is_loopback() {
        return Some("loopback");
    }
    // Connecting to the unspecified address reaches the local host.
    if ip.is_unspecified() {
        return Some("unspecified");
    }
    // IPv4-mapped ::ffff:0:0/96 and IPv4-compatible ::/96 dial the IPv4
    // address in the last 32 bits.
    if let Some(v4) = ip.to_ipv4() {
        return forbidden_v4(v4);
    }
    let octets = ip.octets();
    // NAT64 well-known prefix 64:ff9b::/96 dials the IPv4 address in the
    // last 32 bits.
    if octets[..12] == [0x00, 0x64, 0xff, 0x9b, 0, 0, 0, 0, 0, 0, 0, 0] {
        return forbidden_v4(Ipv4Addr::new(
            octets[12], octets[13], octets[14], octets[15],
        ));
    }
    // 6to4 2002::/16 dials the IPv4 address in bits 16-47.
    if octets[0] == 0x20 && octets[1] == 0x02 {
        return forbidden_v4(Ipv4Addr::new(octets[2], octets[3], octets[4], octets[5]));
    }
    // NAT64 local-use 64:ff9b:1::/48. RFC 6052 puts the embedded IPv4 at a
    // position set by the chosen prefix length, so it is refused whole.
    if octets[..6] == [0x00, 0x64, 0xff, 0x9b, 0x00, 0x01] {
        return Some("nat64 local-use");
    }
    if ip.is_unique_local() {
        return Some("private");
    }
    if ip.is_unicast_link_local() {
        return Some("link-local");
    }
    // fec0::/10.
    if ip.segments()[0] & 0xffc0 == 0xfec0 {
        return Some("site-local");
    }
    if ip.is_multicast() {
        return Some("multicast");
    }
    None
}

/// Split an `http://` or `https://` absolute URL into its origin and the
/// rest, from the path on. The port defaults to the scheme's. An empty
/// authority or one with userinfo, which would make it ambiguous, is none.
pub fn absolute_url(url: &str) -> Option<(Target, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    let https = if scheme.eq_ignore_ascii_case("https") {
        true
    } else if scheme.eq_ignore_ascii_case("http") {
        false
    } else {
        return None;
    };
    let (authority, path) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    if authority.is_empty() || authority.contains('@') {
        return None;
    }
    let (host, port) = authority_host(authority, if https { 443 } else { 80 })?;
    let target = Target {
        https,
        host: host.to_string(),
        port,
        authority: authority.to_string(),
    };
    Some((target, path))
}

/// Split an authority into its host and port, handling a bracketed IPv6
/// host. `default` is the port when it names none.
pub fn authority_host(authority: &str, default: u16) -> Option<(&str, u16)> {
    let (host, port) = match authority.strip_prefix('[') {
        Some(rest) => match rest.split_once(']')? {
            (host, "") => (host, None),
            (host, port) => (host, Some(port.strip_prefix(':')?)),
        },
        None => match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => return Some((authority, default)),
        },
    };
    let port = match port {
        None => default,
        Some(port) if port.bytes().all(|byte| byte.is_ascii_digit()) => port.parse().ok()?,
        Some(_) => return None,
    };
    (!host.is_empty()).then_some((host, port))
}
