//! Address checks for the egress proxy.
//!
//! A name is resolved once and every address it resolves to must pass
//! [`forbidden`] before the first is dialed. A route's target is a host
//! service and is dialed without these checks, except an injecting route's
//! `https` target, which is checked like an allowlisted host.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    let Some(first) = addresses.first().copied() else {
        return Err(ResolveError::Unresolved);
    };
    for address in &addresses {
        if let Some(reason) = forbidden(address.ip()) {
            return Err(ResolveError::Forbidden(reason));
        }
    }
    Ok(first)
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
    // Connecting to the unspecified address reaches the local host.
    if ip.is_unspecified() {
        return Some("unspecified");
    }
    if ip.is_private() {
        return Some("private");
    }
    if ip.is_link_local() {
        return Some("link-local");
    }
    let octets = ip.octets();
    // 100.64.0.0/10.
    if octets[0] == 100 && (64..=127).contains(&octets[1]) {
        return Some("cgnat");
    }
    // 198.18.0.0/15.
    if octets[0] == 198 && (octets[1] == 18 || octets[1] == 19) {
        return Some("benchmark");
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
    // An IPv4-mapped address dials the IPv4 address it embeds.
    if let Some(ip) = ip.to_ipv4_mapped() {
        return forbidden_v4(ip);
    }
    // fc00::/7.
    if ip.segments()[0] & 0xfe00 == 0xfc00 {
        return Some("private");
    }
    // fe80::/10.
    if ip.segments()[0] & 0xffc0 == 0xfe80 {
        return Some("link-local");
    }
    None
}

/// Split an authority into its host and port, handling a bracketed IPv6
/// host. `default` is the port when it names none.
pub fn authority_host(authority: &str, default: u16) -> Option<(&str, u16)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, rest) = rest.split_once(']')?;
        if host.is_empty() {
            return None;
        }
        let port = match rest {
            "" => default,
            rest => {
                let port = rest.strip_prefix(':')?;
                if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
                    return None;
                }
                port.parse().ok()?
            }
        };
        return Some((host, port));
    }
    match authority.rsplit_once(':') {
        Some((host, port)) => {
            if host.is_empty() || port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit())
            {
                return None;
            }
            Some((host, port.parse().ok()?))
        }
        None => Some((authority, default)),
    }
}
