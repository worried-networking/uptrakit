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
use crate::generated::wire::envelope::ReportPagination;
use crate::generated::wire::limits::{MAX_DISCOVERIES_PER_PLUGIN, PAGINATION_SIZE_THRESHOLD};
use crate::generated::wire::messages::ServiceMessage;
use crate::generated::wire::payloads::{
    BatchUpdateResultPayload, DiscoveryPluginResult, DiscoveryResultsPayload, ReportHostsPayload,
    ReportPageLimits, VersionCheckResultsPayload,
};
use serde::Serialize;
use uuid::Uuid;
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
    type Item = crate::generated::wire::payloads::VersionCheckResult;
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
    type Item = crate::generated::wire::payloads::HostInfo;
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
    type Item = crate::generated::wire::payloads::BatchUpdateItemResult;
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
    let payload = payload.normalize();
    let max_items_per_page = P::max_items_per_page(limits);
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
    let empty_payload = payload.with_items(Vec::new());
    let empty_json_len = serde_json::to_string(&empty_payload)?.len();
    let envelope_overhead = empty_json_len + 150;
    let budget = PAGINATION_SIZE_THRESHOLD.saturating_sub(envelope_overhead);
    let mut pages: Vec<Vec<P::Item>> = Vec::new();
    let mut current_page: Vec<P::Item> = Vec::new();
    let mut current_size: usize = 0;
    for item in items {
        let item_json_len = serde_json::to_string(&item)?.len();
        let item_cost = item_json_len + 1;
        let page_full_by_size = !current_page.is_empty() && current_size + item_cost > budget;
        let page_full_by_count = current_page.len() >= max_items_per_page;
        if page_full_by_size || page_full_by_count {
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
