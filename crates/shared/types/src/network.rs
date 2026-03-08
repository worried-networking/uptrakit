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
pub fn is_private_ip(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => {
            let octets = v4.octets();
            v4.is_private()
                || v4.is_loopback()
                || v4.is_unspecified()
                // Link-local: 169.254.0.0/16
                || (octets[0] == 169 && octets[1] == 254)
                // CGNAT (Carrier-Grade NAT): 100.64.0.0/10
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
        }
        IpAddr::V6(v6) => {
            let segments = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                // ULA (Unique Local Address): fc00::/7
                || (segments[0] >> 8) & 0xFE == 0xFC
                // Link-local: fe80::/10
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── DNS names ───────────────────────────────────────────────────────

    #[test]
    fn blocks_localhost() {
        assert!(is_private_host("localhost"));
        assert!(is_private_host("LOCALHOST"));
    }

    #[test]
    fn blocks_dot_local() {
        assert!(is_private_host("myhost.local"));
        assert!(is_private_host("MyHost.Local"));
    }

    #[test]
    fn blocks_dot_internal() {
        assert!(is_private_host("service.internal"));
    }

    #[test]
    fn blocks_dot_localhost() {
        assert!(is_private_host("something.localhost"));
    }

    #[test]
    fn allows_public_hostnames() {
        assert!(!is_private_host("example.com"));
        assert!(!is_private_host("api.github.com"));
        assert!(!is_private_host("gitlab.com"));
    }

    // ── IPv4 private (RFC 1918) ─────────────────────────────────────────

    #[test]
    fn blocks_ipv4_10_range() {
        assert!(is_private_host("10.0.0.1"));
        assert!(is_private_host("10.255.255.255"));
    }

    #[test]
    fn blocks_ipv4_172_16_range() {
        assert!(is_private_host("172.16.0.1"));
        assert!(is_private_host("172.31.255.255"));
    }

    #[test]
    fn blocks_ipv4_192_168_range() {
        assert!(is_private_host("192.168.0.1"));
        assert!(is_private_host("192.168.255.255"));
    }

    // ── IPv4 loopback ───────────────────────────────────────────────────

    #[test]
    fn blocks_ipv4_loopback() {
        assert!(is_private_host("127.0.0.1"));
        assert!(is_private_host("127.255.255.254"));
    }

    // ── IPv4 unspecified ────────────────────────────────────────────────

    #[test]
    fn blocks_ipv4_unspecified() {
        assert!(is_private_host("0.0.0.0"));
    }

    // ── IPv4 link-local ─────────────────────────────────────────────────

    #[test]
    fn blocks_ipv4_link_local() {
        assert!(is_private_host("169.254.0.1"));
        assert!(is_private_host("169.254.255.255"));
    }

    // ── IPv4 CGNAT ──────────────────────────────────────────────────────

    #[test]
    fn blocks_ipv4_cgnat() {
        assert!(is_private_host("100.64.0.1"));
        assert!(is_private_host("100.100.100.100"));
        assert!(is_private_host("100.127.255.255"));
    }

    #[test]
    fn allows_ipv4_just_outside_cgnat() {
        // 100.63.x.x is below CGNAT range
        assert!(!is_private_host("100.63.255.255"));
        // 100.128.x.x is above CGNAT range
        assert!(!is_private_host("100.128.0.0"));
    }

    // ── IPv4 public ─────────────────────────────────────────────────────

    #[test]
    fn allows_public_ipv4() {
        assert!(!is_private_host("8.8.8.8"));
        assert!(!is_private_host("1.1.1.1"));
        assert!(!is_private_host("93.184.216.34"));
    }

    // ── IPv6 loopback ───────────────────────────────────────────────────

    #[test]
    fn blocks_ipv6_loopback() {
        assert!(is_private_host("::1"));
    }

    // ── IPv6 unspecified ────────────────────────────────────────────────

    #[test]
    fn blocks_ipv6_unspecified() {
        assert!(is_private_host("::"));
    }

    // ── IPv6 ULA (fc00::/7) ─────────────────────────────────────────────

    #[test]
    fn blocks_ipv6_ula() {
        assert!(is_private_host("fc00::1"));
        assert!(is_private_host("fd00::1"));
        assert!(is_private_host("fdab:cdef:1234::1"));
    }

    // ── IPv6 link-local (fe80::/10) ─────────────────────────────────────

    #[test]
    fn blocks_ipv6_link_local() {
        assert!(is_private_host("fe80::1"));
        assert!(is_private_host("fe80::abcd:ef01:2345:6789"));
        // febf is still within fe80::/10
        assert!(is_private_host("febf::1"));
    }

    #[test]
    fn allows_ipv6_just_outside_link_local() {
        // fec0:: is outside fe80::/10
        assert!(!is_private_host("fec0::1"));
    }

    // ── IPv6 public ─────────────────────────────────────────────────────

    #[test]
    fn allows_public_ipv6() {
        assert!(!is_private_host("2001:4860:4860::8888"));
        assert!(!is_private_host("2606:4700:4700::1111"));
    }

    // ── Non-parseable hostnames ─────────────────────────────────────────

    #[test]
    fn non_parseable_hostname_returns_false() {
        assert!(!is_private_host("not-an-ip-or-blocked-domain.org"));
    }

    // ── is_private_ip direct tests ────────────────────────────────────────

    #[test]
    fn is_private_ip_ipv4_private() {
        assert!(is_private_ip("10.0.0.1".parse().unwrap()));
        assert!(is_private_ip("172.16.0.1".parse().unwrap()));
        assert!(is_private_ip("192.168.1.1".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_ipv4_loopback() {
        assert!(is_private_ip("127.0.0.1".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_ipv4_link_local() {
        assert!(is_private_ip("169.254.1.1".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_ipv4_cgnat() {
        assert!(is_private_ip("100.64.0.1".parse().unwrap()));
        assert!(is_private_ip("100.127.255.255".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_ipv4_unspecified() {
        assert!(is_private_ip("0.0.0.0".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_ipv4_public() {
        assert!(!is_private_ip("8.8.8.8".parse().unwrap()));
        assert!(!is_private_ip("1.1.1.1".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_ipv6_loopback() {
        assert!(is_private_ip("::1".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_ipv6_ula() {
        assert!(is_private_ip("fd00::1".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_ipv6_link_local() {
        assert!(is_private_ip("fe80::1".parse().unwrap()));
    }

    #[test]
    fn is_private_ip_ipv6_public() {
        assert!(!is_private_ip("2001:4860:4860::8888".parse().unwrap()));
    }
}
