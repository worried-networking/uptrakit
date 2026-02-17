use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::services::{
    ListServicesQuery, MergeAgentRequest, MessageResponse, ServiceResponse,
};

impl UptrakitClient {
    /// List services with optional filters and pagination.
    pub async fn list_services(
        &self,
        query: &ListServicesQuery,
    ) -> Result<PaginatedResponse<ServiceResponse>> {
        self.get_with_query("/api/v1/services", query).await
    }

    /// Get a single service by ID.
    pub async fn get_service(&self, id: &str) -> Result<ServiceResponse> {
        let path = format!("/api/v1/services/{id}");
        self.get(&path).await
    }

    /// Approve a pending service.
    pub async fn approve_service(&self, id: &str) -> Result<ServiceResponse> {
        let path = format!("/api/v1/services/{id}/approve");
        self.post_empty(&path).await
    }

    /// Reject a pending service.
    pub async fn reject_service(&self, id: &str) -> Result<ServiceResponse> {
        let path = format!("/api/v1/services/{id}/reject");
        self.post_empty(&path).await
    }

    /// Deactivate (remove) a service.
    pub async fn remove_service(&self, id: &str) -> Result<MessageResponse> {
        let path = format!("/api/v1/services/{id}");
        self.delete_json(&path).await
    }

    /// Merge a pending source service into an approved target service.
    pub async fn merge_service(
        &self,
        target_id: &str,
        req: &MergeAgentRequest,
    ) -> Result<ServiceResponse> {
        let path = format!("/api/v1/services/{target_id}/merge");
        self.post_json(&path, req).await
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_web_api_types::services::{ListServicesQuery, MergeAgentRequest};

    #[test]
    fn list_services_query_serialization_with_all_fields() {
        let query = ListServicesQuery {
            r#type: Some("agent".to_string()),
            status: Some("approved".to_string()),
            page: Some(2),
            per_page: Some(50),
        };
        let qs = serde_urlencoded::to_string(&query).expect("serialize");
        assert!(qs.contains("type=agent"));
        assert!(qs.contains("status=approved"));
        assert!(qs.contains("page=2"));
        assert!(qs.contains("per_page=50"));
    }

    #[test]
    fn list_services_query_serialization_skips_none() {
        let query = ListServicesQuery {
            r#type: None,
            status: None,
            page: None,
            per_page: None,
        };
        let qs = serde_urlencoded::to_string(&query).expect("serialize");
        assert!(qs.is_empty());
    }

    #[test]
    fn merge_agent_request_serialization() {
        let req = MergeAgentRequest {
            source_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        };
        let json = serde_json::to_string(&req).expect("serialize");
        assert!(json.contains("550e8400-e29b-41d4-a716-446655440000"));
        let parsed: MergeAgentRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.source_id, req.source_id);
    }
}
