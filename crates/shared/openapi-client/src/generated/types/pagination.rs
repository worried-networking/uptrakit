// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
use serde::{Deserialize, Serialize};
pub const DEFAULT_PER_PAGE: u64 = 20;
pub const MAX_PER_PAGE: u64 = 1000;
/// Raw pagination query parameters (both optional).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationParams {
    /// Page number (1-indexed). Defaults to 1.
    pub page: Option<u64>,
    /// Items per page. Defaults to 20, max 1000.
    pub per_page: Option<u64>,
}
/// Resolved (validated, clamped) pagination values.
#[derive(Debug, Clone, Copy)]
pub struct ResolvedPagination {
    pub page: u64,
    pub per_page: u64,
}
impl PaginationParams {
    pub fn resolve(&self) -> ResolvedPagination {
        let page = self.page.unwrap_or(1).max(1);
        let per_page = self
            .per_page
            .unwrap_or(DEFAULT_PER_PAGE)
            .clamp(1, MAX_PER_PAGE);
        ResolvedPagination { page, per_page }
    }
}
impl ResolvedPagination {
    pub fn offset(&self) -> u64 {
        (self.page - 1) * self.per_page
    }
    pub fn total_pages(&self, total: u64) -> u64 {
        total.div_ceil(self.per_page)
    }
}
/// Paginated response wrapper.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginatedResponse<T> {
    pub items: Vec<T>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
    pub total_pages: u64,
}
impl<T> PaginatedResponse<T> {
    pub fn new(items: Vec<T>, total: u64, pagination: ResolvedPagination) -> Self {
        Self {
            items,
            total,
            page: pagination.page,
            per_page: pagination.per_page,
            total_pages: pagination.total_pages(total),
        }
    }
}
