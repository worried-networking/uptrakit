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
//!   is kept whole across pages when it fits within
//!   [`MAX_DISCOVERIES_PER_PLUGIN`](crate::limits::MAX_DISCOVERIES_PER_PLUGIN).
//!   Plugin results that exceed this limit are **normalized** (chunked) into
//!   multiple `DiscoveryPluginResult` entries before pagination so that every
//!   individual entry respects the wire validation limit. The controller
//!   processes each entry independently, so splitting is transparent.
//! - No payload buffering on the controller: each page is processed and
//!   dropped immediately.

use serde::Serialize;
use uuid::Uuid;

use crate::envelope::ReportPagination;
use crate::limits::{MAX_DISCOVERIES_PER_PLUGIN, PAGINATION_SIZE_THRESHOLD};
use crate::messages::ServiceMessage;
use crate::payloads::{
    BatchUpdateResultPayload, DiscoveryPluginResult, DiscoveryResultsPayload, ReportHostsPayload,
    ReportPageLimits, VersionCheckResultsPayload,
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

    /// Maximum number of items allowed on a single page for this payload type.
    fn max_items_per_page(limits: &ReportPageLimits) -> usize;

    /// Normalize the payload before pagination.
    ///
    /// Called at the start of [`paginate_payload`] to ensure nested
    /// collections also respect their individual wire validation limits. The
    /// default implementation is a no-op. Override for payloads with nested
    /// vecs that may individually exceed their per-item wire limits.
    fn normalize(self) -> Self {
        self
    }
}

impl Paginatable for DiscoveryResultsPayload {
    type Item = DiscoveryPluginResult;

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

    fn max_items_per_page(limits: &ReportPageLimits) -> usize {
        limits.discovery_results as usize
    }

    /// Split any [`DiscoveryPluginResult`] whose `discoveries` vec exceeds
    /// [`MAX_DISCOVERIES_PER_PLUGIN`] into multiple entries so that every
    /// entry passes wire validation. The controller processes each entry
    /// independently, so splitting is transparent to the caller.
    fn normalize(self) -> Self {
        let results = self
            .results
            .into_iter()
            .flat_map(|result| {
                if result.discoveries.len() <= MAX_DISCOVERIES_PER_PLUGIN {
                    vec![result]
                } else {
                    let plugin_config_id = result.plugin_config_id;
                    let plugin_type = result.plugin_type.clone();
                    let error = result.error.clone();
                    result
                        .discoveries
                        .chunks(MAX_DISCOVERIES_PER_PLUGIN)
                        .map(|chunk| DiscoveryPluginResult {
                            plugin_config_id,
                            plugin_type: plugin_type.clone(),
                            discoveries: chunk.to_vec(),
                            error: error.clone(),
                        })
                        .collect::<Vec<_>>()
                }
            })
            .collect();
        Self {
            host_machine_id: self.host_machine_id,
            results,
        }
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

    fn max_items_per_page(limits: &ReportPageLimits) -> usize {
        limits.version_check_results as usize
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

    fn max_items_per_page(limits: &ReportPageLimits) -> usize {
        limits.report_hosts as usize
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

    fn max_items_per_page(limits: &ReportPageLimits) -> usize {
        limits.batch_update_results as usize
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
    limits: &ReportPageLimits,
) -> Result<Vec<PayloadPage<P>>, serde_json::Error> {
    // Normalize before pagination: split any nested collections that would
    // individually exceed their wire validation limits.
    let payload = payload.normalize();
    let max_items_per_page = P::max_items_per_page(limits);
    // Fast path: check full payload size first.
    let full_json = serde_json::to_string(&payload)?;
    if full_json.len() <= PAGINATION_SIZE_THRESHOLD && payload.items().len() <= max_items_per_page {
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

        let page_full_by_size = !current_page.is_empty() && current_size + item_cost > budget;
        let page_full_by_count = current_page.len() >= max_items_per_page;
        if page_full_by_size || page_full_by_count {
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
    use crate::limits::MAX_DISCOVERIES_PER_PLUGIN;
    use crate::payloads::{DiscoveryPluginResult, VersionCheckResult};
    use uptrakit_shared_types::{DiscoveredSoftware, PluginType};

    fn make_discovered_software(name: &str) -> DiscoveredSoftware {
        DiscoveredSoftware {
            package_identifier: format!("pkg-{name}"),
            name: name.to_string(),
            installed_version: "1.0.0".to_string(),
            qualifier: None,
            plugin_package_identifier: None,
            featured: false,
            targets: Vec::new(),
            extra: None,
        }
    }

    fn make_discovery_result(name: &str) -> DiscoveryPluginResult {
        DiscoveryPluginResult {
            plugin_config_id: Some(Uuid::new_v4()),
            plugin_type: PluginType::PackageManagerApt,
            discoveries: vec![make_discovered_software(name)],
            error: None,
        }
    }

    fn make_discovery_result_with_count(count: usize) -> DiscoveryPluginResult {
        DiscoveryPluginResult {
            plugin_config_id: Some(Uuid::new_v4()),
            plugin_type: PluginType::PackageManagerApt,
            discoveries: (0..count)
                .map(|i| make_discovered_software(&format!("pkg-{i}")))
                .collect(),
            error: None,
        }
    }

    #[test]
    fn small_payload_not_paginated() {
        let payload = DiscoveryResultsPayload {
            host_machine_id: "machine-1".to_string(),
            results: vec![make_discovery_result("small")],
        };
        let pages = paginate_payload(payload, &ReportPageLimits::default()).unwrap();
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

        let pages = paginate_payload(payload, &ReportPageLimits::default()).unwrap();
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
        let pages = paginate_payload(payload, &ReportPageLimits::default()).unwrap();
        assert_eq!(pages.len(), 1);
        assert!(pages[0].pagination.is_none());
    }

    #[test]
    fn version_check_results_paginatable() {
        let results: Vec<_> = (0..5000)
            .map(|i| VersionCheckResult {
                software_item_id: Uuid::new_v4(),
                installed_version: Some(format!("1.0.{i}")),
                installed_display_version: None,
                latest_version: Some(format!("2.0.{i}")),
                error: None,
                host_software_item_id: None,
                update_category: uptrakit_shared_types::UpdateCategory::Unknown,
            })
            .collect();
        let payload = VersionCheckResultsPayload { results };
        let pages = paginate_payload(payload, &ReportPageLimits::default()).unwrap();
        let total_items: usize = pages.iter().map(|p| p.payload.results.len()).sum();
        assert_eq!(total_items, 5000);
    }

    #[test]
    fn payload_respects_item_count_limit_even_when_under_size_threshold() {
        let payload = VersionCheckResultsPayload {
            results: (0..5)
                .map(|i| VersionCheckResult {
                    software_item_id: Uuid::new_v4(),
                    installed_version: Some(format!("1.0.{i}")),
                    installed_display_version: None,
                    latest_version: Some(format!("2.0.{i}")),
                    error: None,
                    host_software_item_id: None,
                    update_category: uptrakit_shared_types::UpdateCategory::Unknown,
                })
                .collect(),
        };
        let limits = ReportPageLimits {
            version_check_results: 2,
            ..ReportPageLimits::default()
        };

        let pages = paginate_payload(payload, &limits).unwrap();

        assert_eq!(pages.len(), 3);
        assert_eq!(pages[0].payload.results.len(), 2);
        assert_eq!(pages[1].payload.results.len(), 2);
        assert_eq!(pages[2].payload.results.len(), 1);
    }

    // normalize() tests

    #[test]
    fn normalize_noop_when_within_limit() {
        // A single plugin result with exactly MAX_DISCOVERIES_PER_PLUGIN items
        // must not be split.
        let result = make_discovery_result_with_count(MAX_DISCOVERIES_PER_PLUGIN);
        let payload = DiscoveryResultsPayload {
            host_machine_id: "machine-1".to_string(),
            results: vec![result],
        };
        let normalized = payload.normalize();
        assert_eq!(normalized.results.len(), 1);
        assert_eq!(
            normalized.results[0].discoveries.len(),
            MAX_DISCOVERIES_PER_PLUGIN
        );
    }

    #[test]
    fn normalize_splits_oversized_plugin_result() {
        // A single plugin result with MAX + 132 items (simulating the 1132-item
        // APT case that triggered the original bug) must be split into chunks.
        let total = MAX_DISCOVERIES_PER_PLUGIN + 132;
        let result = make_discovery_result_with_count(total);
        let config_id = result.plugin_config_id;
        let payload = DiscoveryResultsPayload {
            host_machine_id: "machine-1".to_string(),
            results: vec![result],
        };
        let normalized = payload.normalize();

        // Should have produced 2 chunks.
        assert_eq!(normalized.results.len(), 2);
        assert_eq!(
            normalized.results[0].discoveries.len(),
            MAX_DISCOVERIES_PER_PLUGIN
        );
        assert_eq!(normalized.results[1].discoveries.len(), 132);

        // Metadata preserved on every chunk.
        for chunk in &normalized.results {
            assert_eq!(chunk.plugin_config_id, config_id);
            assert_eq!(chunk.plugin_type, PluginType::PackageManagerApt);
            assert!(chunk.error.is_none());
        }

        // No discovery is lost.
        let total_after: usize = normalized.results.iter().map(|r| r.discoveries.len()).sum();
        assert_eq!(total_after, total);
    }

    #[test]
    fn normalize_splits_then_paginate_validates_cleanly() {
        // End-to-end: a payload that would fail wire validation without
        // normalization must pass after paginate_payload (which calls normalize).
        let total = MAX_DISCOVERIES_PER_PLUGIN + 132;
        let result = make_discovery_result_with_count(total);
        let payload = DiscoveryResultsPayload {
            host_machine_id: "machine-1".to_string(),
            results: vec![result],
        };

        let pages = paginate_payload(payload, &ReportPageLimits::default()).unwrap();

        // All resulting plugin results must respect the wire limit.
        for page in &pages {
            for r in &page.payload.results {
                assert!(
                    r.discoveries.len() <= MAX_DISCOVERIES_PER_PLUGIN,
                    "chunk has {} discoveries, exceeds limit",
                    r.discoveries.len()
                );
            }
        }

        // No discovery is lost.
        let total_after: usize = pages
            .iter()
            .flat_map(|p| p.payload.results.iter())
            .map(|r| r.discoveries.len())
            .sum();
        assert_eq!(total_after, total);
    }

    #[test]
    fn normalize_preserves_error_on_all_chunks() {
        let total = MAX_DISCOVERIES_PER_PLUGIN + 1;
        let mut result = make_discovery_result_with_count(total);
        result.error = Some("partial failure".to_string());
        let payload = DiscoveryResultsPayload {
            host_machine_id: "machine-1".to_string(),
            results: vec![result],
        };
        let normalized = payload.normalize();
        assert_eq!(normalized.results.len(), 2);
        for chunk in &normalized.results {
            assert_eq!(chunk.error.as_deref(), Some("partial failure"));
        }
    }
}
