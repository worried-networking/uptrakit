// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
//! Lightweight per-connection tracker for paginated reports.
//!
//! The [`ReportTracker`] lives in the WebSocket connection handler's local
//! scope and tracks which pages of each paginated report have been received.
//! It stores **no payload data** — each page is processed immediately and
//! dropped. The tracker only holds page counts, timestamps, and a small
//! accumulated notification counter.
//!
//! # Memory budget
//!
//! Each [`PendingReport`] is ~200 bytes. With a maximum of
//! [`MAX_PENDING_REPORTS_PER_CONNECTION`](crate::limits::MAX_PENDING_REPORTS_PER_CONNECTION)
//! concurrent reports, the tracker uses at most ~2 KB per connection.
use crate::generated::wire::limits::{
    MAX_PENDING_REPORTS_PER_CONNECTION, MAX_REPORT_PAGES, REPORT_IDLE_TIMEOUT,
    REPORT_TOTAL_TIMEOUT, WireValidationError,
};
use std::collections::{BTreeSet, HashMap};
use uuid::Uuid;
/// Outcome of registering a page with the [`ReportTracker`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PageOutcome {
    /// More pages are expected; the caller should process this page's payload
    /// but defer finalization.
    Pending,
    /// This was the final page; the caller should process the payload and run
    /// finalization. Contains the accumulated `discovered_count` from all
    /// prior pages plus the current page's contribution (added by the caller).
    Final {
        /// Sum of `discovered_count` values accumulated across all prior pages.
        /// The caller adds the current page's count before using it.
        accumulated_discovered_count: u32,
    },
}
/// Tracks the state of a single paginated report.
#[derive(Debug)]
struct PendingReport {
    /// Which 1-based page numbers have been received.
    pages_received: BTreeSet<u32>,
    /// Total pages expected.
    total_pages: u32,
    /// When the first page was received.
    started_at: tokio::time::Instant,
    /// When the most recent page was received.
    last_page_at: tokio::time::Instant,
    /// Accumulated discovered_count across all processed pages (discovery only).
    discovered_count: u32,
}
/// Per-connection tracker for paginated reports.
///
/// Created when the authenticated message loop starts, dropped when the
/// connection closes. No shared or global state.
#[derive(Debug)]
pub struct ReportTracker {
    pending: HashMap<Uuid, PendingReport>,
}
impl ReportTracker {
    /// Create a new empty tracker.
    pub fn new() -> Self {
        Self {
            pending: HashMap::new(),
        }
    }
    /// Register a page of a paginated report.
    ///
    /// Returns:
    /// - `Ok(PageOutcome::Pending)` if more pages are expected.
    /// - `Ok(PageOutcome::Final { .. })` if all pages have now been received.
    /// - `Err(WireValidationError)` if limits are violated (duplicate page,
    ///   too many concurrent reports, `total_pages` mismatch, etc.).
    ///
    /// The caller must process the page's payload regardless of the outcome.
    pub fn register_page(
        &mut self,
        report_id: Uuid,
        page: u32,
        total_pages: u32,
    ) -> Result<PageOutcome, WireValidationError> {
        self.evict_expired();
        if let Some(existing) = self.pending.get_mut(&report_id) {
            if existing.total_pages != total_pages {
                return Err(WireValidationError {
                    field: "pagination.total_pages",
                    message: format!(
                        "total_pages changed from {} to {total_pages} within report {report_id}",
                        existing.total_pages
                    ),
                });
            }
            if existing.pages_received.contains(&page) {
                return Err(WireValidationError {
                    field: "pagination.page",
                    message: format!("duplicate page {page} for report {report_id}"),
                });
            }
            existing.pages_received.insert(page);
            existing.last_page_at = tokio::time::Instant::now();
            if existing.pages_received.len() == existing.total_pages as usize {
                let report = self
                    .pending
                    .remove(&report_id)
                    .expect("report was just accessed");
                Ok(PageOutcome::Final {
                    accumulated_discovered_count: report.discovered_count,
                })
            } else {
                Ok(PageOutcome::Pending)
            }
        } else {
            if self.pending.len() >= MAX_PENDING_REPORTS_PER_CONNECTION {
                return Err(WireValidationError {
                    field: "pagination.report_id",
                    message: format!(
                        "too many concurrent paginated reports (max {MAX_PENDING_REPORTS_PER_CONNECTION})"
                    ),
                });
            }
            if total_pages > MAX_REPORT_PAGES {
                return Err(WireValidationError {
                    field: "pagination.total_pages",
                    message: format!("total_pages is {total_pages}, max {MAX_REPORT_PAGES}"),
                });
            }
            let now = tokio::time::Instant::now();
            let mut pages_received = BTreeSet::new();
            pages_received.insert(page);
            if total_pages == 1 {
                return Ok(PageOutcome::Final {
                    accumulated_discovered_count: 0,
                });
            }
            self.pending.insert(
                report_id,
                PendingReport {
                    pages_received,
                    total_pages,
                    started_at: now,
                    last_page_at: now,
                    discovered_count: 0,
                },
            );
            Ok(PageOutcome::Pending)
        }
    }
    /// Add to the accumulated `discovered_count` for an in-progress report.
    ///
    /// No-op if the report has already been finalized or does not exist.
    pub fn add_discovered_count(&mut self, report_id: Uuid, count: u32) {
        if let Some(report) = self.pending.get_mut(&report_id) {
            report.discovered_count = report.discovered_count.saturating_add(count);
        }
    }
    /// Evict reports that have exceeded the total or idle timeout.
    ///
    /// Called automatically by [`register_page`](Self::register_page). Can also
    /// be called periodically from a background task.
    pub fn evict_expired(&mut self) {
        let now = tokio::time::Instant::now();
        self.pending.retain(|id, report| {
            let total_elapsed = now.duration_since(report.started_at);
            let idle_elapsed = now.duration_since(report.last_page_at);
            if total_elapsed >= REPORT_TOTAL_TIMEOUT {
                tracing::warn!(
                    report_id = % id, pages_received = report.pages_received.len(),
                    total_pages = report.total_pages,
                    "paginated report timed out (total timeout {}s exceeded)",
                    REPORT_TOTAL_TIMEOUT.as_secs()
                );
                return false;
            }
            if idle_elapsed >= REPORT_IDLE_TIMEOUT {
                tracing::warn!(
                    report_id = % id, pages_received = report.pages_received.len(),
                    total_pages = report.total_pages,
                    "paginated report timed out (idle timeout {}s exceeded)",
                    REPORT_IDLE_TIMEOUT.as_secs()
                );
                return false;
            }
            true
        });
    }
    /// Returns the number of currently pending reports.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}
impl Default for ReportTracker {
    fn default() -> Self {
        Self::new()
    }
}
