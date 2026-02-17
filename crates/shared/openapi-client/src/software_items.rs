use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
use uptrakit_web_api_types::software_items::{
    AssignHostsRequest, CreateSoftwareItemRequest, SoftwareItemDetailResponse,
    SoftwareItemResponse, TriggerUpdateRequest, TriggerUpdateResponse,
    TriggerVersionCheckResponse, UpdateSoftwareItemRequest,
};

impl UptrakitClient {
    /// List software items with pagination.
    pub async fn list_software_items(
        &self,
        params: &PaginationParams,
    ) -> Result<PaginatedResponse<SoftwareItemResponse>> {
        self.get_with_query("/api/v1/software-items", params).await
    }

    /// Get a single software item by ID (detailed view with host info).
    pub async fn get_software_item(&self, id: &str) -> Result<SoftwareItemDetailResponse> {
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
        id: &str,
        req: &UpdateSoftwareItemRequest,
    ) -> Result<SoftwareItemResponse> {
        let path = format!("/api/v1/software-items/{id}");
        self.put_json(&path, req).await
    }

    /// Delete a software item.
    pub async fn delete_software_item(&self, id: &str) -> Result<()> {
        let path = format!("/api/v1/software-items/{id}");
        self.delete(&path).await
    }

    /// Assign hosts to a software item.
    pub async fn assign_hosts(
        &self,
        id: &str,
        req: &AssignHostsRequest,
    ) -> Result<SoftwareItemDetailResponse> {
        let path = format!("/api/v1/software-items/{id}/hosts");
        self.post_json(&path, req).await
    }

    /// Unassign a host from a software item.
    pub async fn unassign_host(&self, item_id: &str, host_id: &str) -> Result<()> {
        let path = format!("/api/v1/software-items/{item_id}/hosts/{host_id}");
        self.delete(&path).await
    }

    /// Trigger a version check for a software item across all assigned hosts.
    pub async fn check_versions(&self, item_id: &str) -> Result<TriggerVersionCheckResponse> {
        let path = format!("/api/v1/software-items/{item_id}/check-versions");
        self.post_empty(&path).await
    }

    /// Trigger a version check for a software item on a specific host.
    pub async fn check_versions_host(
        &self,
        item_id: &str,
        host_id: &str,
    ) -> Result<TriggerVersionCheckResponse> {
        let path = format!("/api/v1/software-items/{item_id}/hosts/{host_id}/check-versions");
        self.post_empty(&path).await
    }

    /// Trigger an update for a software item on a specific host.
    pub async fn trigger_update(
        &self,
        item_id: &str,
        host_id: &str,
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

    #[test]
    fn create_software_item_request_serialization() {
        let req = CreateSoftwareItemRequest {
            name: "Node.js".to_string(),
            provider_config_id: Some("config-uuid-1".to_string()),
            provider_config: None,
            package_identifier: Some("nodejs/node".to_string()),
            config_override: None,
            enabled: true,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["name"], "Node.js");
        assert_eq!(json["provider_config_id"], "config-uuid-1");
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
        let req = AssignHostsRequest {
            host_ids: vec!["host-1".to_string(), "host-2".to_string()],
        };
        let json = serde_json::to_value(&req).expect("serialize");
        let ids = json["host_ids"].as_array().expect("array");
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], "host-1");
        assert_eq!(ids[1], "host-2");
    }

    #[test]
    fn trigger_update_request_without_release_info() {
        let req = TriggerUpdateRequest {
            to_version: "2.0.0".to_string(),
            release_info: None,
        };
        let json = serde_json::to_value(&req).expect("serialize");
        assert_eq!(json["to_version"], "2.0.0");
        assert!(json.get("release_info").is_none());
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
