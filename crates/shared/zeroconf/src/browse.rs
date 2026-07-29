//! Browse primitives: locate controllers advertising [`SERVICE_TYPE`](crate::SERVICE_TYPE).

use std::net::IpAddr;
use std::time::Duration;

use rootcause::prelude::*;

use crate::{DiscoveredController, Result, SERVICE_TYPE, parse_txt};

/// Browse timeout for the first-match policy, in seconds.
const BROWSE_TIMEOUT_SECS: u64 = 10;

/// Termination policy for the shared receive loop.
enum BrowsePolicy {
    /// Stop at the first resolved controller (service enrollment behavior).
    First,
    /// Collect distinct controllers: full `window` while nothing has been
    /// found, then a short `settle` grace after the first sighting.
    All { window: Duration, settle: Duration },
}

/// Browse for the first controller advertising the Uptrakit service type.
///
/// Waits up to 10 s for a resolved advertisement; `Ok(None)` when nothing
/// answers in time.
///
/// # Errors
///
/// Returns [`ZeroconfError::Daemon`](crate::ZeroconfError::Daemon) when the
/// mDNS daemon cannot be started or the browse request fails.
pub async fn browse_first() -> Result<Option<DiscoveredController>> {
    let mut found = browse_collect(BrowsePolicy::First).await?;
    Ok(found.pop())
}

/// Browse and collect all distinct controllers with an adaptive settle window.
///
/// Browses until `window` elapses while nothing has been found; once the
/// first controller is seen, collection continues only for a further `settle`
/// grace (catching near-simultaneous responders), then returns. Entries
/// carrying a CA fingerprint are deduplicated on the fingerprint (first-seen
/// URL wins, so a dual-stack controller advertising a consistent `ca_fp`
/// yields one entry); fingerprint-less entries are deduplicated on URL.
/// First-seen order is preserved.
///
/// # Errors
///
/// Returns [`ZeroconfError::Daemon`](crate::ZeroconfError::Daemon) when the
/// mDNS daemon cannot be started or the browse request fails.
pub async fn browse_all(window: Duration, settle: Duration) -> Result<Vec<DiscoveredController>> {
    browse_collect(BrowsePolicy::All { window, settle }).await
}

async fn browse_collect(policy: BrowsePolicy) -> Result<Vec<DiscoveredController>> {
    use mdns_sd::ServiceDaemon;

    let mdns = ServiceDaemon::new().context_to()?;
    let receiver = mdns.browse(SERVICE_TYPE).context_to()?;

    let window = match &policy {
        BrowsePolicy::First => Duration::from_secs(BROWSE_TIMEOUT_SECS),
        BrowsePolicy::All { window, .. } => *window,
    };
    let hard_deadline = tokio::time::Instant::now() + window;
    let mut settle_deadline: Option<tokio::time::Instant> = None;
    let mut found: Vec<DiscoveredController> = Vec::new();

    loop {
        let deadline = settle_deadline.unwrap_or(hard_deadline);
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
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
                    let addresses: Vec<IpAddr> = info
                        .get_addresses()
                        .iter()
                        .map(|ip| ip.to_ip_addr())
                        .collect();
                    let port = info.get_port();
                    let properties: Vec<(&str, &str)> = info
                        .get_properties()
                        .iter()
                        .map(|p| (p.key(), p.val_str()))
                        .collect();

                    if let Some(controller) = parse_txt(&addresses, port, &properties) {
                        found.push(controller);
                        match &policy {
                            BrowsePolicy::First => break,
                            BrowsePolicy::All { settle, .. } => {
                                if settle_deadline.is_none() {
                                    settle_deadline = Some(tokio::time::Instant::now() + *settle);
                                }
                            }
                        }
                    }
                }
            }
            Ok(Ok(Err(_))) => {
                // recv_timeout tick elapsed or channel closed; re-check deadlines.
                continue;
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "mDNS browse task panicked");
                break;
            }
            Err(_) => {
                // Outer deadline elapsed.
                break;
            }
        }
    }

    // Best-effort shutdown
    #[expect(
        clippy::let_underscore_must_use,
        reason = "stop_browse and shutdown failures are not actionable; the result is already stored"
    )]
    let _ = mdns.stop_browse(SERVICE_TYPE);
    #[expect(
        clippy::let_underscore_must_use,
        reason = "stop_browse and shutdown failures are not actionable; the result is already stored"
    )]
    let _ = mdns.shutdown();

    Ok(dedup_discovered(found))
}

/// Collapse duplicate sightings of the same controller.
///
/// Same `ca_fingerprint` (case-insensitive) means the same controller —
/// dual-stack advertisements collapse to the first-seen URL. Entries without
/// a fingerprint collapse on URL. First-seen order is preserved.
fn dedup_discovered(found: Vec<DiscoveredController>) -> Vec<DiscoveredController> {
    use std::collections::HashSet;

    let mut seen_fps: HashSet<String> = HashSet::new();
    let mut seen_urls: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for entry in found {
        let fresh = match &entry.ca_fingerprint {
            Some(fp) => seen_fps.insert(fp.to_ascii_lowercase()),
            None => seen_urls.insert(entry.url.clone()),
        };
        if fresh {
            out.push(entry);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dc(url: &str, fp: Option<&str>) -> DiscoveredController {
        DiscoveredController::new(url.to_string(), None, fp.map(String::from))
    }

    #[test]
    fn dedup_collapses_same_fingerprint_dual_stack() {
        let out = dedup_discovered(vec![
            dc("https://192.168.1.10:8443", Some("abcd")),
            dc("https://[fe80::1]:8443", Some("ABCD")),
        ]);
        assert_eq!(out.len(), 1);
        assert_eq!(out.first().unwrap().url, "https://192.168.1.10:8443");
    }

    #[test]
    fn dedup_fingerprint_less_collapses_by_url() {
        let out = dedup_discovered(vec![
            dc("https://192.168.1.10:8443", None),
            dc("https://192.168.1.10:8443", None),
            dc("https://192.168.1.11:8443", None),
        ]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn dedup_does_not_cross_collapse_buckets() {
        // A fingerprinted entry and a fingerprint-less entry with the same URL
        // are kept apart (split-TXT double-listing: rare, cosmetic, accepted).
        let out = dedup_discovered(vec![
            dc("https://192.168.1.10:8443", Some("abcd")),
            dc("https://192.168.1.10:8443", None),
        ]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn dedup_preserves_first_seen_order() {
        let out = dedup_discovered(vec![
            dc("https://b.local:8443", Some("bbbb")),
            dc("https://a.local:8443", Some("aaaa")),
            dc("https://b2.local:8443", Some("bbbb")),
        ]);
        let urls: Vec<&str> = out.iter().map(|c| c.url.as_str()).collect();
        assert_eq!(urls, vec!["https://b.local:8443", "https://a.local:8443"]);
    }
}
