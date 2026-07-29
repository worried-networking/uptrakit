//! Uptrakit zeroconf contract — the single home for the mDNS/DNS-SD service
//! type, the TXT record keys, and their build/parse logic, plus the browse
//! primitives used by services and the CLI to locate a controller.
//!
//! The advertised contract (`SERVICE_TYPE` + TXT keys) lives here and nowhere
//! else: the controller-runtime advertiser builds its TXT records through
//! [`build_txt_properties`] and every browser parses through [`parse_txt`].

use std::net::IpAddr;

use rootcause::prelude::*;
use thiserror::Error;
use uptrakit_shared_macros::impl_report_conversion;

/// mDNS service type advertised by the controller and browsed by clients.
pub const SERVICE_TYPE: &str = "_uptrakit._tcp.local.";

/// TXT record key: SHA-256 fingerprint of the controller's active CA certificate.
pub const TXT_KEY_CA_FP: &str = "ca_fp";
/// TXT record key: optional HTTPS URL override (reverse proxy deployments).
pub const TXT_KEY_URL: &str = "url";
/// TXT record key: optional PKI endpoint address.
pub const TXT_KEY_PKI_ADDR: &str = "pki_addr";

/// Errors produced by the zeroconf browse primitives.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ZeroconfError {
    /// The mDNS daemon could not be started, browsed, or shut down.
    #[error("mDNS daemon error: {0}")]
    Daemon(mdns_sd::Error),
}

/// Boundary result alias covering all fallible functions in this crate.
pub type Result<T> = std::result::Result<T, Report<ZeroconfError>>;

impl_report_conversion!(mdns_sd::Error => ZeroconfError::Daemon);

/// A controller discovered via mDNS.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredController {
    /// The discovered (or TXT-override) HTTPS URL of the controller.
    pub url: String,
    /// Optional PKI endpoint address from the TXT record.
    pub pki_addr: Option<String>,
    /// CA fingerprint from the TXT record (for TOFU verification).
    pub ca_fingerprint: Option<String>,
}

impl DiscoveredController {
    /// Constructor for external callers (`#[non_exhaustive]` blocks struct literals).
    pub fn new(url: String, pki_addr: Option<String>, ca_fingerprint: Option<String>) -> Self {
        Self {
            url,
            pki_addr,
            ca_fingerprint,
        }
    }
}

/// Extract a named property value from mDNS TXT record properties.
fn get_txt_property<'a>(properties: &'a [(&str, &str)], key: &str) -> Option<&'a str> {
    properties.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// Build a [`DiscoveredController`] from resolved mDNS service info.
///
/// A TXT `url` override wins (reverse proxy mode); otherwise the URL is built
/// from the first non-loopback address and the advertised port. Loopback-only
/// address sets yield `None`.
pub fn parse_txt(
    addresses: &[IpAddr],
    port: u16,
    properties: &[(&str, &str)],
) -> Option<DiscoveredController> {
    let ca_fingerprint = get_txt_property(properties, TXT_KEY_CA_FP).map(String::from);
    let pki_addr = get_txt_property(properties, TXT_KEY_PKI_ADDR).map(String::from);

    // If a URL override is in the TXT record, use it directly (reverse proxy mode)
    let url = if let Some(url_override) = get_txt_property(properties, TXT_KEY_URL) {
        url_override.to_string()
    } else {
        // Construct URL from the mDNS-resolved address
        let ip = addresses.iter().find(|ip| !ip.is_loopback())?;
        match ip {
            IpAddr::V4(v4) => format!("https://{v4}:{port}"),
            IpAddr::V6(v6) => format!("https://[{v6}]:{port}"),
        }
    };

    Some(DiscoveredController {
        url,
        pki_addr,
        ca_fingerprint,
    })
}

/// Build the TXT record property list the controller advertises.
///
/// Order is part of the contract: `ca_fp` first, then optional `url`, then
/// optional `pki_addr`.
pub fn build_txt_properties(
    ca_fingerprint: &str,
    url: Option<&str>,
    pki_addr: Option<&str>,
) -> Vec<(&'static str, String)> {
    let mut properties = vec![(TXT_KEY_CA_FP, ca_fingerprint.to_string())];

    if let Some(url) = url {
        properties.push((TXT_KEY_URL, url.to_string()));
    }

    if let Some(pki_addr) = pki_addr {
        properties.push((TXT_KEY_PKI_ADDR, pki_addr.to_string()));
    }

    properties
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_from_txt_override() {
        let addresses = vec![IpAddr::from([192, 168, 1, 100])];
        let properties = vec![
            ("ca_fp", "abcd1234"),
            ("url", "https://proxy.example.com:443"),
        ];
        let controller = parse_txt(&addresses, 8443, &properties).unwrap();
        assert_eq!(controller.url, "https://proxy.example.com:443");
        assert_eq!(controller.ca_fingerprint.as_deref(), Some("abcd1234"));
    }

    #[test]
    fn url_from_mdns_ip_port() {
        let addresses = vec![IpAddr::from([192, 168, 1, 100])];
        let properties = vec![("ca_fp", "abcd1234")];
        let controller = parse_txt(&addresses, 8443, &properties).unwrap();
        assert_eq!(controller.url, "https://192.168.1.100:8443");
    }

    #[test]
    fn url_from_mdns_ipv6() {
        let addresses = vec![IpAddr::from([0xfe80, 0, 0, 0, 0, 0, 0, 1])];
        let properties = vec![("ca_fp", "abcd1234")];
        let controller = parse_txt(&addresses, 8443, &properties).unwrap();
        assert_eq!(controller.url, "https://[fe80::1]:8443");
    }

    #[test]
    fn url_skips_loopback() {
        let addresses = vec![
            IpAddr::from([127, 0, 0, 1]),
            IpAddr::from([192, 168, 1, 100]),
        ];
        let properties = vec![];
        let controller = parse_txt(&addresses, 8443, &properties).unwrap();
        assert_eq!(controller.url, "https://192.168.1.100:8443");
    }

    #[test]
    fn url_only_loopback_returns_none() {
        let addresses = vec![IpAddr::from([127, 0, 0, 1])];
        let properties = vec![];
        assert!(parse_txt(&addresses, 8443, &properties).is_none());
    }

    #[test]
    fn pki_addr_from_txt() {
        let addresses = vec![IpAddr::from([192, 168, 1, 100])];
        let properties = vec![("pki_addr", "http://192.168.1.100:8080")];
        let controller = parse_txt(&addresses, 8443, &properties).unwrap();
        assert_eq!(
            controller.pki_addr.as_deref(),
            Some("http://192.168.1.100:8080")
        );
    }

    #[test]
    fn get_txt_property_finds_key() {
        let props = vec![("key1", "val1"), ("key2", "val2")];
        assert_eq!(get_txt_property(&props, "key1"), Some("val1"));
        assert_eq!(get_txt_property(&props, "key2"), Some("val2"));
        assert_eq!(get_txt_property(&props, "missing"), None);
    }

    #[test]
    fn txt_properties_basic() {
        let props = build_txt_properties("abcd1234", None, None);
        assert_eq!(props, vec![("ca_fp", "abcd1234".to_string())]);
    }

    #[test]
    fn txt_properties_with_url_override() {
        let props = build_txt_properties("abcd1234", Some("https://proxy.example.com:443"), None);
        assert_eq!(
            props,
            vec![
                ("ca_fp", "abcd1234".to_string()),
                ("url", "https://proxy.example.com:443".to_string()),
            ]
        );
    }

    #[test]
    fn txt_properties_with_all_overrides() {
        let props = build_txt_properties(
            "abcd1234",
            Some("https://proxy.example.com:443"),
            Some("http://pki.local:8080"),
        );
        assert_eq!(
            props,
            vec![
                ("ca_fp", "abcd1234".to_string()),
                ("url", "https://proxy.example.com:443".to_string()),
                ("pki_addr", "http://pki.local:8080".to_string()),
            ]
        );
    }
}
