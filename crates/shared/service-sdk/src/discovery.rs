//! mDNS/DNS-SD zero-configuration discovery for services.
//!
//! When `--url` is omitted, services browse for `_uptrakit._tcp.local.`
//! on the local network. The discovered controller URL and CA fingerprint
//! are cached in `discovery.json` for subsequent restarts.

use std::path::Path;

use serde::{Deserialize, Serialize};

pub use uptrakit_zeroconf::SERVICE_TYPE;

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
    crate::dirs::write_secure_file_str(&path, &data)
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

/// Browse the local network for `_uptrakit._tcp.local.` via mDNS.
pub async fn browse_mdns() -> Result<Option<DiscoveryCache>, String> {
    let found = uptrakit_zeroconf::browse_first()
        .await
        .map_err(|e| format!("mDNS browse failed: {e}"))?;
    Ok(found.map(|d| DiscoveryCache {
        url: d.url,
        pki_addr: d.pki_addr,
        ca_fingerprint: d.ca_fingerprint,
    }))
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
    #![expect(
        clippy::assertions_on_result_states,
        reason = "test assertions — assert!(result.is_ok()) are idiomatic in tests"
    )]

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
}
