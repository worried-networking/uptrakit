use crate::Result;
use crate::UptrakitClient;
use crate::types_impl::batch_actions::{BatchActionRequest, BatchActionResponse};
use crate::types_impl::hosts::{HostResponse, UpdateHostRequest};
use crate::types_impl::pagination::{PaginatedResponse, PaginationParams};
use uuid::Uuid;

impl UptrakitClient {
    /// List hosts with pagination.
    pub async fn list_hosts(
        &self,
        params: &PaginationParams,
    ) -> Result<PaginatedResponse<HostResponse>> {
        self.get_with_query(crate::paths::hosts::BASE, params).await
    }

    /// Fetch all hosts across all pages.
    ///
    /// Automatically iterates through every page at [`MAX_PER_PAGE`] items per
    /// request and returns the concatenated list. Use [`list_hosts`] when
    /// manual pagination control is needed.
    ///
    /// [`MAX_PER_PAGE`]: uptrakit_web_api_types::pagination::MAX_PER_PAGE
    /// [`list_hosts`]: Self::list_hosts
    pub async fn list_all_hosts(&self) -> Result<Vec<HostResponse>> {
        let base = PaginationParams {
            page: None,
            per_page: None,
        };
        self.fetch_all_pages(crate::paths::hosts::BASE, &base).await
    }

    /// Get a single host by ID.
    pub async fn get_host(&self, id: &Uuid) -> Result<HostResponse> {
        self.get(&crate::paths::hosts::by_id(id)).await
    }

    /// Update a host (e.g. change its friendly name).
    pub async fn update_host(&self, id: &Uuid, req: &UpdateHostRequest) -> Result<HostResponse> {
        self.put_json(&crate::paths::hosts::by_id(id), req).await
    }

    /// Deactivate (remove) a host.
    pub async fn deactivate_host(&self, id: &Uuid) -> Result<()> {
        self.delete(&crate::paths::hosts::by_id(id)).await
    }

    /// Perform a batch action on multiple hosts.
    ///
    /// Supported actions: `deactivate`.
    pub async fn batch_hosts(&self, req: &BatchActionRequest) -> Result<BatchActionResponse> {
        self.post_json(crate::paths::hosts::BATCH, req).await
    }
}

#[cfg(test)]
mod tests {
    use crate::types_impl::hosts::UpdateHostRequest;
    use crate::types_impl::pagination::PaginationParams;

    #[test]
    fn pagination_params_serialization_with_values() {
        let params = PaginationParams {
            page: Some(2),
            per_page: Some(50),
        };
        let qs = serde_urlencoded::to_string(&params).expect("serialize");
        assert!(qs.contains("page=2"));
        assert!(qs.contains("per_page=50"));
    }

    #[test]
    fn pagination_params_serialization_skips_none() {
        let params = PaginationParams {
            page: None,
            per_page: None,
        };
        let qs = serde_urlencoded::to_string(&params).expect("serialize");
        assert!(qs.is_empty());
    }

    #[test]
    fn update_host_request_serialization() {
        let req = UpdateHostRequest {
            friendly_name: Some("Production Server".to_string()),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["friendly_name"], "Production Server");
    }

    #[test]
    fn update_host_request_serialization_none() {
        let req = UpdateHostRequest {
            friendly_name: None,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert!(json["friendly_name"].is_null());
    }
}
