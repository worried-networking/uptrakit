//! mDNS/DNS-SD zero-configuration discovery for services.
//!
//! When `--url` is omitted, services browse for `_uptrakit._tcp.local.`
//! on the local network. The discovered controller URL and CA fingerprint
//! are cached in `discovery.json` for subsequent restarts.

use std::net::IpAddr;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// mDNS service type to browse for.
pub const SERVICE_TYPE: &str = "_uptrakit._tcp.local.";

/// Browse timeout in seconds.
const BROWSE_TIMEOUT_SECS: u64 = 10;

/// Cached discovery result, persisted to `discovery.json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscoveryCache {
    /// The discovered (or override) HTTPS URL of the controller.
    pub url: String,
    /// Optional PKI endpoint address from the TXT record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pki_addr: Option<String>,
    /// CA fingerprint from the TXT record (for TOFU verification).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_fingerprint: Option<String>,
}

/// Result of the discovery process.
#[derive(Debug)]
pub enum DiscoveryResult {
    /// Loaded from the on-disk cache.
    Cached(DiscoveryCache),
    /// Freshly discovered via mDNS and saved to cache.
    Discovered(DiscoveryCache),
    /// No controller found.
    NotFound,
}

const CACHE_FILENAME: &str = "discovery.json";

/// Load the cached discovery result from `<state_dir>/discovery.json`.
pub fn load_cache(state_dir: &Path) -> Result<Option<DiscoveryCache>, String> {
    let path = state_dir.join(CACHE_FILENAME);
    if !path.exists() {
        return Ok(None);
    }
    let data = std::fs::read_to_string(&path)
        .map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    let cache: DiscoveryCache = serde_json::from_str(&data)
        .map_err(|e| format!("failed to parse {}: {e}", path.display()))?;
    Ok(Some(cache))
}

/// Save the discovery cache to `<state_dir>/discovery.json` with 0o600 permissions.
pub async fn save_cache(state_dir: &Path, cache: &DiscoveryCache) -> Result<(), String> {
    let path = state_dir.join(CACHE_FILENAME);
    let data = serde_json::to_string_pretty(cache)
        .map_err(|e| format!("failed to serialize discovery cache: {e}"))?;
    uptrakit_directories::write_secure_file_str(&path, &data)
        .await
        .map_err(|e| format!("failed to write {}: {e}", path.display()))?;
    Ok(())
}

/// Remove the cached discovery result.
pub fn clear_cache(state_dir: &Path) -> Result<(), String> {
    let path = state_dir.join(CACHE_FILENAME);
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|e| format!("failed to remove {}: {e}", path.display()))?;
    }
    Ok(())
}

/// Extract a named property value from mDNS TXT record properties.
fn get_txt_property<'a>(properties: &'a [(&str, &str)], key: &str) -> Option<&'a str> {
    properties.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// Build a `DiscoveryCache` from mDNS service info.
fn cache_from_mdns(
    addresses: &[IpAddr],
    port: u16,
    properties: &[(&str, &str)],
) -> Option<DiscoveryCache> {
    let ca_fingerprint = get_txt_property(properties, "ca_fp").map(String::from);
    let pki_addr = get_txt_property(properties, "pki_addr").map(String::from);

    // If a URL override is in the TXT record, use it directly (reverse proxy mode)
    let url = if let Some(url_override) = get_txt_property(properties, "url") {
        url_override.to_string()
    } else {
        // Construct URL from the mDNS-resolved address
        let ip = addresses.iter().find(|ip| !ip.is_loopback())?;
        match ip {
            IpAddr::V4(v4) => format!("https://{v4}:{port}"),
            IpAddr::V6(v6) => format!("https://[{v6}]:{port}"),
        }
    };

    Some(DiscoveryCache {
        url,
        pki_addr,
        ca_fingerprint,
    })
}

/// Browse the local network for `_uptrakit._tcp.local.` via mDNS.
pub async fn browse_mdns() -> Result<Option<DiscoveryCache>, String> {
    use mdns_sd::ServiceDaemon;

    let mdns = ServiceDaemon::new().map_err(|e| format!("failed to start mDNS browser: {e}"))?;

    let receiver = mdns
        .browse(SERVICE_TYPE)
        .map_err(|e| format!("failed to browse mDNS: {e}"))?;

    let timeout = Duration::from_secs(BROWSE_TIMEOUT_SECS);
    let deadline = tokio::time::Instant::now() + timeout;

    let result = loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break None;
        }

        match tokio::time::timeout(
            remaining,
            tokio::task::spawn_blocking({
                let receiver = receiver.clone();
                move || receiver.recv_timeout(Duration::from_secs(1))
            }),
        )
        .await
        {
            Ok(Ok(Ok(event))) => {
                if let mdns_sd::ServiceEvent::ServiceResolved(info) = event {
                    let addresses: Vec<IpAddr> = info.get_addresses().iter().copied().collect();
                    let port = info.get_port();
                    let properties: Vec<(&str, &str)> = info
                        .get_properties()
                        .iter()
                        .map(|p| (p.key(), p.val_str()))
                        .collect();

                    if let Some(cache) = cache_from_mdns(&addresses, port, &properties) {
                        break Some(cache);
                    }
                }
            }
            Ok(Ok(Err(_))) => {
                // recv_timeout returned an error (channel closed or timeout)
                continue;
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "mDNS browse task panicked");
                break None;
            }
            Err(_) => {
                // Tokio timeout elapsed
                break None;
            }
        }
    };

    // Best-effort shutdown
    let _ = mdns.stop_browse(SERVICE_TYPE);
    let _ = mdns.shutdown();

    Ok(result)
}

/// Run the full discovery process: try cache, then mDNS browse.
pub async fn discover(state_dir: &Path) -> Result<DiscoveryResult, String> {
    // 1. Try cache
    if let Some(cache) = load_cache(state_dir)? {
        tracing::info!(url = %cache.url, "using cached controller discovery result");
        return Ok(DiscoveryResult::Cached(cache));
    }

    // 2. Browse mDNS
    tracing::info!("browsing for controller via mDNS ({SERVICE_TYPE})...");
    if let Some(cache) = browse_mdns().await? {
        tracing::warn!(
            url = %cache.url,
            "Discovered controller via mDNS at {}",
            cache.url
        );
        if let Some(ref fp) = cache.ca_fingerprint {
            tracing::warn!("CA fingerprint: SHA256:{fp}");
            tracing::warn!("Verify this matches the controller's fingerprint to prevent MITM.");
            tracing::warn!("Use --tofu-fingerprint to enforce verification automatically.");
        }

        save_cache(state_dir, &cache).await?;
        return Ok(DiscoveryResult::Discovered(cache));
    }

    Ok(DiscoveryResult::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = DiscoveryCache {
            url: "https://192.168.1.100:8443".to_string(),
            pki_addr: Some("http://192.168.1.100:8080".to_string()),
            ca_fingerprint: Some("abcd1234".to_string()),
        };

        // Save synchronously for test
        let path = dir.path().join(CACHE_FILENAME);
        let data = serde_json::to_string_pretty(&cache).unwrap();
        std::fs::write(&path, data).unwrap();

        let loaded = load_cache(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.url, "https://192.168.1.100:8443");
        assert_eq!(
            loaded.pki_addr.as_deref(),
            Some("http://192.168.1.100:8080")
        );
        assert_eq!(loaded.ca_fingerprint.as_deref(), Some("abcd1234"));
    }

    #[test]
    fn cache_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let loaded = load_cache(dir.path()).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn cache_backward_compat() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(CACHE_FILENAME);
        // Old format without optional fields
        std::fs::write(&path, r#"{"url":"https://192.168.1.100:8443"}"#).unwrap();

        let loaded = load_cache(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.url, "https://192.168.1.100:8443");
        assert!(loaded.pki_addr.is_none());
        assert!(loaded.ca_fingerprint.is_none());
    }

    #[test]
    fn clear_cache_removes_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(CACHE_FILENAME);
        std::fs::write(&path, "{}").unwrap();
        assert!(path.exists());

        clear_cache(dir.path()).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn clear_cache_missing_file_ok() {
        let dir = tempfile::tempdir().expect("tempdir");
        let result = clear_cache(dir.path());
        assert!(result.is_ok());
    }

    #[test]
    fn url_from_txt_override() {
        let addresses = vec![IpAddr::from([192, 168, 1, 100])];
        let properties = vec![
            ("ca_fp", "abcd1234"),
            ("url", "https://proxy.example.com:443"),
        ];
        let cache = cache_from_mdns(&addresses, 8443, &properties).unwrap();
        assert_eq!(cache.url, "https://proxy.example.com:443");
        assert_eq!(cache.ca_fingerprint.as_deref(), Some("abcd1234"));
    }

    #[test]
    fn url_from_mdns_ip_port() {
        let addresses = vec![IpAddr::from([192, 168, 1, 100])];
        let properties = vec![("ca_fp", "abcd1234")];
        let cache = cache_from_mdns(&addresses, 8443, &properties).unwrap();
        assert_eq!(cache.url, "https://192.168.1.100:8443");
    }

    #[test]
    fn url_from_mdns_ipv6() {
        let addresses = vec![IpAddr::from([0xfe80, 0, 0, 0, 0, 0, 0, 1])];
        let properties = vec![("ca_fp", "abcd1234")];
        let cache = cache_from_mdns(&addresses, 8443, &properties).unwrap();
        assert_eq!(cache.url, "https://[fe80::1]:8443");
    }

    #[test]
    fn url_skips_loopback() {
        let addresses = vec![
            IpAddr::from([127, 0, 0, 1]),
            IpAddr::from([192, 168, 1, 100]),
        ];
        let properties = vec![];
        let cache = cache_from_mdns(&addresses, 8443, &properties).unwrap();
        assert_eq!(cache.url, "https://192.168.1.100:8443");
    }

    #[test]
    fn url_only_loopback_returns_none() {
        let addresses = vec![IpAddr::from([127, 0, 0, 1])];
        let properties = vec![];
        let cache = cache_from_mdns(&addresses, 8443, &properties);
        assert!(cache.is_none());
    }

    #[test]
    fn pki_addr_from_txt() {
        let addresses = vec![IpAddr::from([192, 168, 1, 100])];
        let properties = vec![("pki_addr", "http://192.168.1.100:8080")];
        let cache = cache_from_mdns(&addresses, 8443, &properties).unwrap();
        assert_eq!(cache.pki_addr.as_deref(), Some("http://192.168.1.100:8080"));
    }

    #[test]
    fn get_txt_property_finds_key() {
        let props = vec![("key1", "val1"), ("key2", "val2")];
        assert_eq!(get_txt_property(&props, "key1"), Some("val1"));
        assert_eq!(get_txt_property(&props, "key2"), Some("val2"));
        assert_eq!(get_txt_property(&props, "missing"), None);
    }
}
