use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::batch_actions::{BatchActionRequest, BatchActionResponse};
use uptrakit_web_api_types::host_tags::{
    CreateHostTagRequest, HostTagResponse, HostTagSummary, ListHostTagsQuery, SetHostTagsRequest,
    UpdateHostTagRequest,
};
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uuid::Uuid;

impl UptrakitClient {
    /// List host tags with pagination and optional search.
    pub async fn list_host_tags(
        &self,
        params: &ListHostTagsQuery,
    ) -> Result<PaginatedResponse<HostTagResponse>> {
        self.get_with_query(crate::paths::host_tags::BASE, params)
            .await
    }

    /// Fetch all host tags across all pages.
    pub async fn list_all_host_tags(&self) -> Result<Vec<HostTagResponse>> {
        use uptrakit_web_api_types::pagination::PaginationParams;
        let base = PaginationParams {
            page: None,
            per_page: None,
        };
        self.fetch_all_pages(crate::paths::host_tags::BASE, &base)
            .await
    }

    /// Get a single host tag by ID.
    pub async fn get_host_tag(&self, id: &Uuid) -> Result<HostTagResponse> {
        self.get(&crate::paths::host_tags::by_id(id)).await
    }

    /// Create a new host tag.
    pub async fn create_host_tag(&self, req: &CreateHostTagRequest) -> Result<HostTagResponse> {
        self.post_json(crate::paths::host_tags::BASE, req).await
    }

    /// Update an existing host tag.
    pub async fn update_host_tag(
        &self,
        id: &Uuid,
        req: &UpdateHostTagRequest,
    ) -> Result<HostTagResponse> {
        self.put_json(&crate::paths::host_tags::by_id(id), req)
            .await
    }

    /// Delete a host tag (soft-delete).
    pub async fn delete_host_tag(&self, id: &Uuid) -> Result<()> {
        self.delete(&crate::paths::host_tags::by_id(id)).await
    }

    /// Perform a batch action on multiple host tags.
    ///
    /// Supported actions: `delete`.
    pub async fn batch_host_tags(&self, req: &BatchActionRequest) -> Result<BatchActionResponse> {
        self.post_json(crate::paths::host_tags::BATCH, req).await
    }

    /// Set (replace-all) tags on a host.
    pub async fn set_host_tags(
        &self,
        host_id: &Uuid,
        req: &SetHostTagsRequest,
    ) -> Result<Vec<HostTagSummary>> {
        self.put_json(&crate::paths::host_tags::host_tags(host_id), req)
            .await
    }
}
