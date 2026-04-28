use crate::Result;
use crate::UptrakitClient;
use crate::generated::types::agents::MessageResponse;
use crate::generated::types::batch_actions::{BatchActionRequest, BatchActionResponse};
use crate::generated::types::pagination::PaginatedResponse;
use crate::generated::types::services::{
    ListServicesQuery, MergeAgentRequest, ServiceResponse, SetUpdateFreezeRequest,
    UpdateServiceRequest,
};
use uuid::Uuid;

impl UptrakitClient {
    /// List services with optional filters and pagination.
    pub async fn list_services(
        &self,
        query: &ListServicesQuery,
    ) -> Result<PaginatedResponse<ServiceResponse>> {
        self.get_with_query(crate::paths::services::BASE, query)
            .await
    }

    /// Fetch all services matching the given filters across all pages.
    ///
    /// Automatically iterates through every page at [`MAX_PER_PAGE`] items per
    /// request. The `page` and `per_page` fields of `query` are ignored; use
    /// [`list_services`] for manual pagination control.
    ///
    /// [`MAX_PER_PAGE`]: uptrakit_web_api_types::pagination::MAX_PER_PAGE
    /// [`list_services`]: Self::list_services
    pub async fn list_all_services(
        &self,
        query: &ListServicesQuery,
    ) -> Result<Vec<ServiceResponse>> {
        self.fetch_all_pages(crate::paths::services::BASE, query)
            .await
    }

    /// Get a single service by ID.
    pub async fn get_service(&self, id: &Uuid) -> Result<ServiceResponse> {
        self.get(&crate::paths::services::by_id(id)).await
    }

    /// Approve a pending service.
    pub async fn approve_service(&self, id: &Uuid) -> Result<ServiceResponse> {
        self.post_empty(&crate::paths::services::approve(id)).await
    }

    /// Reject a pending service.
    pub async fn reject_service(&self, id: &Uuid) -> Result<ServiceResponse> {
        self.post_empty(&crate::paths::services::reject(id)).await
    }

    /// Update a service's configurable settings (e.g. ping interval).
    pub async fn update_service(
        &self,
        id: &Uuid,
        req: &UpdateServiceRequest,
    ) -> Result<ServiceResponse> {
        self.put_json(&crate::paths::services::by_id(id), req).await
    }

    /// Deactivate (remove) a service.
    pub async fn remove_service(&self, id: &Uuid) -> Result<()> {
        self.delete(&crate::paths::services::by_id(id)).await
    }

    /// Enable or disable the update freeze on a connected service.
    pub async fn set_update_freeze(
        &self,
        id: &Uuid,
        req: &SetUpdateFreezeRequest,
    ) -> Result<MessageResponse> {
        self.post_json(&crate::paths::services::update_freeze(id), req)
            .await
    }

    /// Perform a batch action on multiple services.
    ///
    /// Supported actions: `approve`, `reject`, `deactivate`.
    pub async fn batch_services(&self, req: &BatchActionRequest) -> Result<BatchActionResponse> {
        self.post_json(crate::paths::services::BATCH, req).await
    }

    /// Merge a pending source service into an approved target service.
    pub async fn merge_service(
        &self,
        target_id: &Uuid,
        req: &MergeAgentRequest,
    ) -> Result<ServiceResponse> {
        self.post_json(&crate::paths::services::merge(target_id), req)
            .await
    }
}

#[cfg(test)]
mod tests {
    use crate::generated::shared_types::ServiceStatus;
    use crate::generated::types::services::{ListServicesQuery, MergeAgentRequest};
    use uuid::Uuid;

    #[test]
    fn list_services_query_serialization_with_all_fields() {
        let query = ListServicesQuery {
            capability: Some("software_discovery".to_string()),
            status: Some(ServiceStatus::Approved),
            page: Some(2),
            per_page: Some(50),
        };
        let qs = serde_urlencoded::to_string(&query).expect("serialize");
        assert!(qs.contains("capability=software_discovery"));
        assert!(qs.contains("status=approved"));
        assert!(qs.contains("page=2"));
        assert!(qs.contains("per_page=50"));
    }

    #[test]
    fn list_services_query_serialization_skips_none() {
        let query = ListServicesQuery {
            capability: None,
            status: None,
            page: None,
            per_page: None,
        };
        let qs = serde_urlencoded::to_string(&query).expect("serialize");
        assert!(qs.is_empty());
    }

    #[test]
    fn merge_agent_request_serialization() {
        let source_uuid =
            Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").expect("valid uuid");
        let req = MergeAgentRequest {
            source_id: source_uuid,
        };
        let json = serde_json::to_string(&req).expect("serialize");
        assert!(json.contains("550e8400-e29b-41d4-a716-446655440000"));
        let parsed: MergeAgentRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.source_id, req.source_id);
    }

    #[test]
    fn update_service_request_cert_lifetime_hours_round_trip() {
        use crate::generated::types::services::UpdateServiceRequest;

        let req = UpdateServiceRequest {
            ping_interval_seconds: None,
            cert_lifetime_hours: Some(48),
        };
        let json = serde_json::to_string(&req).expect("serialize");
        assert!(json.contains(r#""cert_lifetime_hours":48"#));
        let parsed: UpdateServiceRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.cert_lifetime_hours, Some(48));
    }
}
