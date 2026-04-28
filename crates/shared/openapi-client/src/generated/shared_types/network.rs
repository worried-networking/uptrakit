//! Network-related utility functions.
//!
//! Provides host validation helpers used across the codebase to prevent
//! server-side request forgery (SSRF) and other network-based attacks.
use std::net::IpAddr;
/// Check whether an IP address is private, loopback, link-local, or
/// otherwise non-public.
///
/// Returns `true` for:
/// - IPv4: private (RFC 1918), loopback, link-local (`169.254.0.0/16`),
///   CGNAT (`100.64.0.0/10`), unspecified (`0.0.0.0`)
/// - IPv6: loopback (`::1`), unspecified (`::`), ULA (`fc00::/7`),
///   link-local (`fe80::/10`)
#[must_use]
pub fn is_private_ip(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_unspecified()
                || (octets[0] == 169 && octets[1] == 254)
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || (segments[0] >> 8) & 0xFE == 0xFC
                || (segments[0] & 0xFFC0) == 0xFE80
        }
    }
}
/// Check whether a hostname or IP address refers to a private, loopback,
/// link-local, or otherwise non-public network destination.
///
/// Returns `true` for:
/// - DNS names: `localhost`, `*.local`, `*.internal`, `*.localhost`
/// - IP addresses: delegates to [`is_private_ip`]
///
/// Non-parseable hostnames that don't match the blocked DNS patterns
/// return `false`.
#[must_use]
pub fn is_private_host(host: &str) -> bool {
    let lower = host.to_lowercase();
    if lower == "localhost"
        || lower.ends_with(".local")
        || lower.ends_with(".internal")
        || lower.ends_with(".localhost")
    {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(addr) => is_private_ip(addr),
        Err(_) => false,
    }
}
