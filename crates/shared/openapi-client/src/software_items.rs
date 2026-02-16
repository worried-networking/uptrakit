use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::pagination::{PaginatedResponse, PaginationParams};
use uptrakit_web_api_types::software_items::{
    SoftwareItemDetailResponse, SoftwareItemResponse, TriggerUpdateRequest, TriggerUpdateResponse,
    TriggerVersionCheckResponse,
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
        let path =
            format!("/api/v1/software-items/{item_id}/hosts/{host_id}/check-versions");
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
    use uptrakit_web_api_types::software_items::{ReleaseInfoRequest, TriggerUpdateRequest};

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
