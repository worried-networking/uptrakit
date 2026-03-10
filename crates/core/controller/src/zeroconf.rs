//! mDNS/DNS-SD zero-configuration advertiser.
//!
//! Registers `_uptrakit._tcp.local.` on the local network so that services
//! can discover the controller without explicit `--url` configuration.
//!
//! TXT records:
//! - `ca_fp=<SHA-256 hex>` — active CA fingerprint for TOFU verification
//! - `url=<https://...>` — optional, override URL for reverse proxy deployments
//! - `pki_addr=<url>` — optional, PKI endpoint address

use std::net::SocketAddr;

use mdns_sd::{ServiceDaemon, ServiceInfo};
use tokio_util::sync::CancellationToken;

use uptrakit_web_api::ca_snapshot::CaPublicSnapshot;
use uptrakit_web_api::settings::ZeroconfSnapshot;

/// mDNS service type for Uptrakit controller discovery.
pub const SERVICE_TYPE: &str = "_uptrakit._tcp.local.";

/// Build TXT record properties from the current CA snapshot and zeroconf settings.
pub fn build_txt_properties(
    ca_snapshot: &CaPublicSnapshot,
    zeroconf: &ZeroconfSnapshot,
) -> Vec<(&'static str, String)> {
    let mut properties = vec![("ca_fp", ca_snapshot.active_fingerprint.clone())];

    if let Some(ref url) = zeroconf.url {
        properties.push(("url", url.clone()));
    }

    if let Some(ref pki_addr) = zeroconf.pki_addr {
        properties.push(("pki_addr", pki_addr.clone()));
    }

    properties
}

/// Run the mDNS advertiser until the cancellation token is triggered.
///
/// Errors during startup are logged as warnings and do not crash the controller.
pub async fn run_advertiser(
    cancel: CancellationToken,
    https_addr: SocketAddr,
    ca_snapshot: CaPublicSnapshot,
    zeroconf: ZeroconfSnapshot,
) {
    let instance_name = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "uptrakit".to_string());

    let port = https_addr.port();

    let properties = build_txt_properties(&ca_snapshot, &zeroconf);
    let txt_props: Vec<(&str, &str)> = properties.iter().map(|(k, v)| (*k, v.as_str())).collect();

    let service_info = match ServiceInfo::new(
        SERVICE_TYPE,
        &instance_name,
        &instance_name,
        (),
        port,
        &txt_props[..],
    ) {
        Ok(info) => info,
        Err(e) => {
            tracing::warn!(error = %e, "failed to create mDNS service info, zeroconf advertising disabled");
            return;
        }
    };

    let mdns = match ServiceDaemon::new() {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(error = %e, "failed to start mDNS daemon, zeroconf advertising disabled");
            return;
        }
    };

    let fullname = service_info.get_fullname().to_string();

    if let Err(e) = mdns.register(service_info) {
        tracing::warn!(error = %e, "failed to register mDNS service, zeroconf advertising disabled");
        return;
    }

    tracing::info!(
        port = port,
        fingerprint = %ca_snapshot.active_fingerprint,
        "mDNS advertising enabled. CA fingerprint: SHA256:{}",
        ca_snapshot.active_fingerprint
    );
    tracing::info!("Verify this fingerprint on connecting services to prevent MITM.");

    // Wait for shutdown signal
    cancel.cancelled().await;

    tracing::info!("shutting down mDNS advertiser");
    if let Err(e) = mdns.unregister(&fullname) {
        tracing::warn!(error = %e, "failed to unregister mDNS service");
    }
    if let Err(e) = mdns.shutdown() {
        tracing::warn!(error = %e, "failed to shut down mDNS daemon");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ca_snapshot() -> CaPublicSnapshot {
        CaPublicSnapshot {
            active_cert_pem: String::new(),
            active_fingerprint: "abcd1234".to_string(),
            previous_cert_pem: None,
            previous_fingerprint: None,
            trusted_cas: vec![],
            trusted_ca_cns: vec![],
            bundle_pem: String::new(),
            bundle_hash: String::new(),
            managed: true,
            active_not_after: time::OffsetDateTime::now_utc(),
            pki_addr: None,
        }
    }

    #[test]
    fn txt_record_basic() {
        let ca = test_ca_snapshot();
        let zeroconf = ZeroconfSnapshot {
            enabled: true,
            url: None,
            pki_addr: None,
        };
        let props = build_txt_properties(&ca, &zeroconf);
        assert_eq!(props.len(), 1);
        assert_eq!(props[0], ("ca_fp", "abcd1234".to_string()));
    }

    #[test]
    fn txt_record_with_url_override() {
        let ca = test_ca_snapshot();
        let zeroconf = ZeroconfSnapshot {
            enabled: true,
            url: Some("https://proxy.example.com:443".to_string()),
            pki_addr: None,
        };
        let props = build_txt_properties(&ca, &zeroconf);
        assert_eq!(props.len(), 2);
        assert_eq!(
            props[1],
            ("url", "https://proxy.example.com:443".to_string())
        );
    }

    #[test]
    fn txt_record_with_all_overrides() {
        let ca = test_ca_snapshot();
        let zeroconf = ZeroconfSnapshot {
            enabled: true,
            url: Some("https://proxy.example.com:443".to_string()),
            pki_addr: Some("http://pki.local:8080".to_string()),
        };
        let props = build_txt_properties(&ca, &zeroconf);
        assert_eq!(props.len(), 3);
        assert_eq!(props[0], ("ca_fp", "abcd1234".to_string()));
        assert_eq!(
            props[1],
            ("url", "https://proxy.example.com:443".to_string())
        );
        assert_eq!(props[2], ("pki_addr", "http://pki.local:8080".to_string()));
    }
}
