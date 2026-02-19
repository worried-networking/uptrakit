use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
use uptrakit_web_api_types::software_items::{
    AssignHostsRequest, CreateSoftwareItemRequest, SoftwareItemDetailResponse,
    SoftwareItemResponse, TriggerUpdateRequest, TriggerUpdateResponse, TriggerVersionCheckResponse,
    UpdateSoftwareItemRequest,
};
use uuid::Uuid;

impl UptrakitClient {
    /// List software items with pagination.
    pub async fn list_software_items(
        &self,
        params: &PaginationParams,
    ) -> Result<PaginatedResponse<SoftwareItemResponse>> {
        self.get_with_query("/api/v1/software-items", params).await
    }

    /// Get a single software item by ID (detailed view with host info).
    pub async fn get_software_item(&self, id: &Uuid) -> Result<SoftwareItemDetailResponse> {
        let path = format!("/api/v1/software-items/{id}");
        self.get(&path).await
    }

    /// Create a new software item.
    pub async fn create_software_item(
        &self,
        req: &CreateSoftwareItemRequest,
    ) -> Result<SoftwareItemResponse> {
        self.post_json("/api/v1/software-items", req).await
    }

    /// Update an existing software item.
    pub async fn update_software_item(
        &self,
        id: &Uuid,
        req: &UpdateSoftwareItemRequest,
    ) -> Result<SoftwareItemResponse> {
        let path = format!("/api/v1/software-items/{id}");
        self.put_json(&path, req).await
    }

    /// Delete a software item.
    pub async fn delete_software_item(&self, id: &Uuid) -> Result<()> {
        let path = format!("/api/v1/software-items/{id}");
        self.delete(&path).await
    }

    /// Assign hosts to a software item.
    pub async fn assign_hosts(
        &self,
        id: &Uuid,
        req: &AssignHostsRequest,
    ) -> Result<SoftwareItemDetailResponse> {
        let path = format!("/api/v1/software-items/{id}/hosts");
        self.post_json(&path, req).await
    }

    /// Unassign a host from a software item.
    pub async fn unassign_host(&self, item_id: &Uuid, host_id: &Uuid) -> Result<()> {
        let path = format!("/api/v1/software-items/{item_id}/hosts/{host_id}");
        self.delete(&path).await
    }

    /// Trigger a version check for a software item across all assigned hosts.
    pub async fn check_versions(&self, item_id: &Uuid) -> Result<TriggerVersionCheckResponse> {
        let path = format!("/api/v1/software-items/{item_id}/check-versions");
        self.post_empty(&path).await
    }

    /// Trigger a version check for a software item on a specific host.
    pub async fn check_versions_host(
        &self,
        item_id: &Uuid,
        host_id: &Uuid,
    ) -> Result<TriggerVersionCheckResponse> {
        let path = format!("/api/v1/software-items/{item_id}/hosts/{host_id}/check-versions");
        self.post_empty(&path).await
    }

    /// Trigger an update for a software item on a specific host.
    pub async fn trigger_update(
        &self,
        item_id: &Uuid,
        host_id: &Uuid,
        req: &TriggerUpdateRequest,
    ) -> Result<TriggerUpdateResponse> {
        let path = format!("/api/v1/software-items/{item_id}/hosts/{host_id}/update");
        self.post_json(&path, req).await
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_web_api_types::software_items::{
        AssignHostsRequest, CreateSoftwareItemRequest, ReleaseInfoRequest, TriggerUpdateRequest,
        UpdateSoftwareItemRequest,
    };
    use uuid::Uuid;

    #[test]
    fn create_software_item_request_serialization() {
        let config_id =
            Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6").expect("valid uuid");
        let req = CreateSoftwareItemRequest {
            name: "Node.js".to_string(),
            provider_config_id: Some(config_id),
            provider_config: None,
            package_identifier: Some("nodejs/node".to_string()),
            config_override: None,
            enabled: true,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["name"], "Node.js");
        assert_eq!(
            json["provider_config_id"],
            "a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6"
        );
        assert_eq!(json["package_identifier"], "nodejs/node");
        assert_eq!(json["enabled"], true);
    }

    #[test]
    fn update_software_item_request_serialization() {
        let req = UpdateSoftwareItemRequest {
            name: Some("Node.js LTS".to_string()),
            package_identifier: None,
            config_override: None,
            enabled: Some(false),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["name"], "Node.js LTS");
        assert_eq!(json["enabled"], false);
    }

    #[test]
    fn assign_hosts_request_serialization() {
        let host1 = Uuid::parse_str("11111111-1111-1111-1111-111111111111").expect("valid uuid");
        let host2 = Uuid::parse_str("22222222-2222-2222-2222-222222222222").expect("valid uuid");
        let req = AssignHostsRequest {
            host_ids: vec![host1, host2],
        };
        let json = serde_json::to_value(&req).expect("serialize");
        let ids = json["host_ids"].as_array().expect("array");
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], "11111111-1111-1111-1111-111111111111");
        assert_eq!(ids[1], "22222222-2222-2222-2222-222222222222");
    }

    #[test]
    fn trigger_update_request_without_release_info() {
        let req = TriggerUpdateRequest {
            to_version: "2.0.0".to_string(),
            release_info: None,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["to_version"], "2.0.0");
        assert!(json["release_info"].is_null());
    }

    #[test]
    fn trigger_update_request_with_release_info() {
        let req = TriggerUpdateRequest {
            to_version: "2.0.0".to_string(),
            release_info: Some(ReleaseInfoRequest {
                tag: "v2.0.0".to_string(),
                release_url: "https://example.com/releases/v2.0.0".to_string(),
                assets: vec![],
            }),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["to_version"], "2.0.0");
        assert_eq!(json["release_info"]["tag"], "v2.0.0");
    }
}
