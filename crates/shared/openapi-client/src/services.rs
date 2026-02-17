use crate::Result;
use crate::UptrakitClient;
use serde::Serialize;
use uptrakit_shared_types::ServiceType;
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::services::{
    EnrollmentTokenResponse, EnrollmentTokenStatusResponse, ListServicesQuery, MergeAgentRequest,
    MessageResponse, ServiceResponse,
};
use uuid::Uuid;

/// Query parameter for enrollment token endpoints.
#[derive(Serialize)]
struct EnrollmentTokenQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    r#type: Option<String>,
}

impl EnrollmentTokenQuery {
    fn from_service_type(service_type: Option<ServiceType>) -> Self {
        Self {
            r#type: service_type.map(|t| t.as_str().to_string()),
        }
    }
}

impl UptrakitClient {
    /// List services with optional filters and pagination.
    pub async fn list_services(
        &self,
        query: &ListServicesQuery,
    ) -> Result<PaginatedResponse<ServiceResponse>> {
        self.get_with_query("/api/v1/services", query).await
    }

    /// Get a single service by ID.
    pub async fn get_service(&self, id: &Uuid) -> Result<ServiceResponse> {
        let path = format!("/api/v1/services/{id}");
        self.get(&path).await
    }

    /// Approve a pending service.
    pub async fn approve_service(&self, id: &Uuid) -> Result<ServiceResponse> {
        let path = format!("/api/v1/services/{id}/approve");
        self.post_empty(&path).await
    }

    /// Reject a pending service.
    pub async fn reject_service(&self, id: &Uuid) -> Result<ServiceResponse> {
        let path = format!("/api/v1/services/{id}/reject");
        self.post_empty(&path).await
    }

    /// Deactivate (remove) a service.
    pub async fn remove_service(&self, id: &Uuid) -> Result<MessageResponse> {
        let path = format!("/api/v1/services/{id}");
        self.delete_json(&path).await
    }

    /// Merge a pending source service into an approved target service.
    pub async fn merge_service(
        &self,
        target_id: &Uuid,
        req: &MergeAgentRequest,
    ) -> Result<ServiceResponse> {
        let path = format!("/api/v1/services/{target_id}/merge");
        self.post_json(&path, req).await
    }

    /// Create an enrollment token for a service type.
    pub async fn create_enrollment_token(
        &self,
        service_type: Option<ServiceType>,
    ) -> Result<EnrollmentTokenResponse> {
        let query = EnrollmentTokenQuery::from_service_type(service_type);
        self.post_empty_with_query("/api/v1/services/enrollment-token", &query)
            .await
    }

    /// Revoke the enrollment token for a service type.
    pub async fn revoke_enrollment_token(&self, service_type: Option<ServiceType>) -> Result<()> {
        let query = EnrollmentTokenQuery::from_service_type(service_type);
        self.delete_with_query("/api/v1/services/enrollment-token", &query)
            .await
    }

    /// Check if an enrollment token is configured for a service type.
    pub async fn enrollment_token_status(
        &self,
        service_type: Option<ServiceType>,
    ) -> Result<EnrollmentTokenStatusResponse> {
        let query = EnrollmentTokenQuery::from_service_type(service_type);
        self.get_with_query("/api/v1/services/enrollment-token/status", &query)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::EnrollmentTokenQuery;
    use uptrakit_shared_types::ServiceType;
    use uptrakit_web_api_types::services::{ListServicesQuery, MergeAgentRequest};
    use uuid::Uuid;

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
    fn enrollment_token_query_with_agent_type() {
        let query = EnrollmentTokenQuery::from_service_type(Some(ServiceType::Agent));
        let qs = serde_urlencoded::to_string(&query).expect("serialize");
        assert_eq!(qs, "type=agent");
    }

    #[test]
    fn enrollment_token_query_with_mqtt_type() {
        let query = EnrollmentTokenQuery::from_service_type(Some(ServiceType::Mqtt));
        let qs = serde_urlencoded::to_string(&query).expect("serialize");
        assert_eq!(qs, "type=mqtt");
    }

    #[test]
    fn enrollment_token_query_with_ssh_agent_type() {
        let query = EnrollmentTokenQuery::from_service_type(Some(ServiceType::SshAgent));
        let qs = serde_urlencoded::to_string(&query).expect("serialize");
        assert_eq!(qs, "type=ssh_agent");
    }

    #[test]
    fn enrollment_token_query_without_type() {
        let query = EnrollmentTokenQuery::from_service_type(None);
        let qs = serde_urlencoded::to_string(&query).expect("serialize");
        assert!(qs.is_empty());
    }
}
