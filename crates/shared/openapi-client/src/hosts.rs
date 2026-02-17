use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::hosts::{HostMessageResponse, HostResponse, UpdateHostRequest};
use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};

impl UptrakitClient {
    /// List hosts with pagination.
    pub async fn list_hosts(
        &self,
        params: &PaginationParams,
    ) -> Result<PaginatedResponse<HostResponse>> {
        self.get_with_query("/api/v1/hosts", params).await
    }

    /// Get a single host by ID.
    pub async fn get_host(&self, id: &str) -> Result<HostResponse> {
        let path = format!("/api/v1/hosts/{id}");
        self.get(&path).await
    }

    /// Update a host (e.g. change its friendly name).
    pub async fn update_host(&self, id: &str, req: &UpdateHostRequest) -> Result<HostResponse> {
        let path = format!("/api/v1/hosts/{id}");
        self.put_json(&path, req).await
    }

    /// Deactivate (remove) a host.
    pub async fn deactivate_host(&self, id: &str) -> Result<HostMessageResponse> {
        let path = format!("/api/v1/hosts/{id}");
        self.delete_json(&path).await
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_web_api_types::hosts::UpdateHostRequest;
    use uptrakit_web_api_types::pagination::PaginationParams;

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
