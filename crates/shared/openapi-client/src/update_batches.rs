use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::update_batches::{
    BatchUpdateResponse, HostBatchUpdateRequest, ItemBatchUpdateRequest,
    UpdateBatchDetailResponse, UpdateBatchListQuery, UpdateBatchSummaryResponse,
};
use uuid::Uuid;

impl UptrakitClient {
    /// Trigger a host-wide batch update for all outdated software items on a host.
    pub async fn trigger_host_batch_update(
        &self,
        host_id: &Uuid,
        req: &HostBatchUpdateRequest,
    ) -> Result<BatchUpdateResponse> {
        self.post_json(&crate::paths::update_batches::host_batch_update(host_id), req)
            .await
    }

    /// Trigger an item-wide batch update to roll out a software item to hosts.
    pub async fn trigger_item_batch_update(
        &self,
        item_id: &Uuid,
        req: &ItemBatchUpdateRequest,
    ) -> Result<BatchUpdateResponse> {
        self.post_json(&crate::paths::update_batches::item_batch_update(item_id), req)
            .await
    }

    /// List update batches with optional filters and pagination.
    pub async fn list_update_batches(
        &self,
        query: &UpdateBatchListQuery,
    ) -> Result<PaginatedResponse<UpdateBatchSummaryResponse>> {
        self.get_with_query(crate::paths::update_batches::BASE, query)
            .await
    }

    /// Fetch all update batches matching the given filters across all pages.
    pub async fn list_all_update_batches(
        &self,
        query: &UpdateBatchListQuery,
    ) -> Result<Vec<UpdateBatchSummaryResponse>> {
        self.fetch_all_pages(crate::paths::update_batches::BASE, query)
            .await
    }

    /// Get a single update batch with per-item update details.
    pub async fn get_update_batch(&self, id: &Uuid) -> Result<UpdateBatchDetailResponse> {
        self.get(&crate::paths::update_batches::by_id(id)).await
    }

    /// Returns the relative path for the batch progress SSE stream.
    ///
    /// The caller should combine this with the base URL and use an
    /// SSE/EventSource client to consume it.
    pub fn batch_progress_stream_path(id: &Uuid) -> String {
        crate::paths::update_batches::stream(id)
    }
}
