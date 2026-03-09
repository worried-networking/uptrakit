//! Payload pagination for wire protocol messages.
//!
//! When a service-to-controller report exceeds the
//! [`PAGINATION_SIZE_THRESHOLD`](crate::limits::PAGINATION_SIZE_THRESHOLD),
//! the sender splits it into multiple pages. Each page is a complete,
//! independently processable message; the controller tracks page arrival
//! via [`ReportTracker`](crate::report_tracker::ReportTracker) and defers
//! only lightweight finalization until the last page.
//!
//! # Design constraints
//!
//! - Each [`DiscoveryPluginResult`](crate::payloads::DiscoveryPluginResult)
//!   must stay whole (never split across pages) to preserve per-plugin
//!   snapshot semantics.
//! - No payload buffering on the controller: each page is processed and
//!   dropped immediately.

use serde::Serialize;
use uuid::Uuid;

use crate::envelope::ReportPagination;
use crate::limits::PAGINATION_SIZE_THRESHOLD;
use crate::messages::ServiceMessage;
use crate::payloads::{
    BatchUpdateResultPayload, DiscoveryResultsPayload, ReportHostsPayload,
    VersionCheckResultsPayload,
};

/// A trait for wire payloads whose primary `Vec` field can be split across
/// pages when the serialized size exceeds the pagination threshold.
///
/// Implementors must keep non-vec fields (e.g. `host_machine_id`,
/// `batch_id`, `agent_version`) identical across all pages.
pub trait Paginatable: Serialize + Sized {
    /// Type of items in the splittable vec.
    type Item: Serialize + Clone;

    /// Borrow the items that may be distributed across pages.
    fn items(&self) -> &[Self::Item];

    /// Reconstruct the payload with a subset of items.
    fn with_items(&self, items: Vec<Self::Item>) -> Self;

    /// Wrap the payload into a [`ServiceMessage`].
    fn into_message(self) -> ServiceMessage;
}

impl Paginatable for DiscoveryResultsPayload {
    type Item = crate::payloads::DiscoveryPluginResult;

    fn items(&self) -> &[Self::Item] {
        &self.results
    }

    fn with_items(&self, items: Vec<Self::Item>) -> Self {
        Self {
            host_machine_id: self.host_machine_id.clone(),
            results: items,
        }
    }

    fn into_message(self) -> ServiceMessage {
        ServiceMessage::DiscoveryResults(self)
    }
}

impl Paginatable for VersionCheckResultsPayload {
    type Item = crate::payloads::VersionCheckResult;

    fn items(&self) -> &[Self::Item] {
        &self.results
    }

    fn with_items(&self, items: Vec<Self::Item>) -> Self {
        Self { results: items }
    }

    fn into_message(self) -> ServiceMessage {
        ServiceMessage::VersionCheckResults(self)
    }
}

impl Paginatable for ReportHostsPayload {
    type Item = crate::payloads::HostInfo;

    fn items(&self) -> &[Self::Item] {
        &self.hosts
    }

    fn with_items(&self, items: Vec<Self::Item>) -> Self {
        Self {
            hosts: items,
            agent_version: self.agent_version.clone(),
            capabilities: self.capabilities.clone(),
        }
    }

    fn into_message(self) -> ServiceMessage {
        ServiceMessage::ReportHosts(self)
    }
}

impl Paginatable for BatchUpdateResultPayload {
    type Item = crate::payloads::BatchUpdateItemResult;

    fn items(&self) -> &[Self::Item] {
        &self.results
    }

    fn with_items(&self, items: Vec<Self::Item>) -> Self {
        Self {
            batch_id: self.batch_id,
            results: items,
        }
    }

    fn into_message(self) -> ServiceMessage {
        ServiceMessage::BatchUpdateResult(self)
    }
}

/// A single page produced by [`paginate_payload`].
#[derive(Debug)]
pub struct PayloadPage<P> {
    /// The page's payload (subset of items from the original).
    pub payload: P,
    /// Pagination metadata, `None` if the payload fit in a single message.
    pub pagination: Option<ReportPagination>,
}

/// Split a [`Paginatable`] payload into pages that each serialize under the
/// [`PAGINATION_SIZE_THRESHOLD`].
///
/// If the full payload is already under the threshold, returns a single page
/// with `pagination: None` (no overhead).
///
/// Each item is kept whole — never split across pages. If a single item
/// exceeds the threshold on its own, it becomes the sole item on its page
/// (the 1 MB WebSocket frame limit is the hard cap).
///
/// # Errors
///
/// Returns `Err` if serialization fails.
pub fn paginate_payload<P: Paginatable>(
    payload: P,
) -> Result<Vec<PayloadPage<P>>, serde_json::Error> {
    // Fast path: check full payload size first.
    let full_json = serde_json::to_string(&payload)?;
    if full_json.len() <= PAGINATION_SIZE_THRESHOLD {
        return Ok(vec![PayloadPage {
            payload,
            pagination: None,
        }]);
    }

    let items = payload.items().to_vec();
    if items.is_empty() {
        return Ok(vec![PayloadPage {
            payload,
            pagination: None,
        }]);
    }

    // Build pages by accumulating items until adding the next item would
    // exceed the threshold. We estimate per-item overhead by measuring the
    // empty payload size and subtracting it from the threshold.
    let empty_payload = payload.with_items(Vec::new());
    let empty_json_len = serde_json::to_string(&empty_payload)?.len();
    // Account for the pagination envelope fields (~120 bytes for
    // report_id UUID + page + total_pages JSON fields).
    let envelope_overhead = empty_json_len + 150;
    let budget = PAGINATION_SIZE_THRESHOLD.saturating_sub(envelope_overhead);

    let mut pages: Vec<Vec<P::Item>> = Vec::new();
    let mut current_page: Vec<P::Item> = Vec::new();
    let mut current_size: usize = 0;

    for item in items {
        let item_json_len = serde_json::to_string(&item)?.len();
        // +1 for the comma separator in the JSON array.
        let item_cost = item_json_len + 1;

        if !current_page.is_empty() && current_size + item_cost > budget {
            // Current page is full, start a new one.
            pages.push(std::mem::take(&mut current_page));
            current_size = 0;
        }

        current_size += item_cost;
        current_page.push(item);
    }

    if !current_page.is_empty() {
        pages.push(current_page);
    }

    let total_pages = pages.len() as u32;
    let report_id = Uuid::new_v4();

    let result = pages
        .into_iter()
        .enumerate()
        .map(|(i, items)| PayloadPage {
            payload: payload.with_items(items),
            pagination: Some(ReportPagination {
                report_id,
                page: (i as u32) + 1,
                total_pages,
            }),
        })
        .collect();

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payloads::{DiscoveryPluginResult, VersionCheckResult};
    use uptrakit_shared_types::PluginType;

    fn make_discovery_result(name: &str) -> DiscoveryPluginResult {
        use uptrakit_shared_types::DiscoveredSoftware;
        DiscoveryPluginResult {
            plugin_config_id: Some(Uuid::new_v4()),
            plugin_type: PluginType::PackageManagerApt,
            discoveries: vec![DiscoveredSoftware {
                package_identifier: format!("pkg-{name}"),
                name: name.to_string(),
                installed_version: "1.0.0".to_string(),
                qualifier: None,
                plugin_package_identifier: None,
                featured: false,
                targets: Vec::new(),
                extra: None,
            }],
            error: None,
        }
    }

    #[test]
    fn small_payload_not_paginated() {
        let payload = DiscoveryResultsPayload {
            host_machine_id: "machine-1".to_string(),
            results: vec![make_discovery_result("small")],
        };
        let pages = paginate_payload(payload).unwrap();
        assert_eq!(pages.len(), 1);
        assert!(pages[0].pagination.is_none());
    }

    #[test]
    fn large_payload_paginated() {
        // Create a payload with many results to exceed the threshold.
        let results: Vec<_> = (0..5000)
            .map(|i| make_discovery_result(&format!("pkg-{i}")))
            .collect();
        let payload = DiscoveryResultsPayload {
            host_machine_id: "machine-1".to_string(),
            results,
        };

        let full_size = serde_json::to_string(&payload).unwrap().len();
        assert!(
            full_size > PAGINATION_SIZE_THRESHOLD,
            "test payload should exceed threshold, was {full_size}"
        );

        let pages = paginate_payload(payload).unwrap();
        assert!(pages.len() > 1, "should have multiple pages");

        // All pages should have the same report_id.
        let report_id = pages[0].pagination.as_ref().unwrap().report_id;
        for (i, page) in pages.iter().enumerate() {
            let p = page.pagination.as_ref().unwrap();
            assert_eq!(p.report_id, report_id);
            assert_eq!(p.page, (i as u32) + 1);
            assert_eq!(p.total_pages, pages.len() as u32);
        }

        // All items should be accounted for.
        let total_items: usize = pages.iter().map(|p| p.payload.results.len()).sum();
        assert_eq!(total_items, 5000);

        // Each page should have the same host_machine_id.
        for page in &pages {
            assert_eq!(page.payload.host_machine_id, "machine-1");
        }
    }

    #[test]
    fn empty_payload_not_paginated() {
        let payload = VersionCheckResultsPayload {
            results: Vec::new(),
        };
        let pages = paginate_payload(payload).unwrap();
        assert_eq!(pages.len(), 1);
        assert!(pages[0].pagination.is_none());
    }

    #[test]
    fn version_check_results_paginatable() {
        let results: Vec<_> = (0..5000)
            .map(|i| VersionCheckResult {
                software_item_id: Uuid::new_v4(),
                installed_version: Some(format!("1.0.{i}")),
                latest_version: Some(format!("2.0.{i}")),
                error: None,
                host_software_item_id: None,
                update_category: uptrakit_shared_types::UpdateCategory::Unknown,
            })
            .collect();
        let payload = VersionCheckResultsPayload { results };
        let pages = paginate_payload(payload).unwrap();
        let total_items: usize = pages.iter().map(|p| p.payload.results.len()).sum();
        assert_eq!(total_items, 5000);
    }
}
