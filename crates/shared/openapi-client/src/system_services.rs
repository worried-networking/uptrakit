use crate::Result;
use crate::UptrakitClient;
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::settings_system_services::{
    SystemServicesSettingsResponse, UpdateSystemServicesSettingsRequest,
};
use uptrakit_web_api_types::system_services::{
    ListSystemServicesQuery, SystemServiceResponse, UpdateSystemServiceRequest,
};
use uuid::Uuid;

impl UptrakitClient {
    /// List system services with optional filters and pagination.
    pub async fn list_system_services(
        &self,
        query: &ListSystemServicesQuery,
    ) -> Result<PaginatedResponse<SystemServiceResponse>> {
        self.get_with_query(crate::paths::system_services::BASE, query)
            .await
    }

    /// Fetch all system services matching the given filters across all pages.
    ///
    /// Automatically iterates through every page at [`MAX_PER_PAGE`] items per
    /// request. The `page` and `per_page` fields of `query` are ignored; use
    /// [`list_system_services`] for manual pagination control.
    ///
    /// [`MAX_PER_PAGE`]: uptrakit_web_api_types::pagination::MAX_PER_PAGE
    /// [`list_system_services`]: Self::list_system_services
    pub async fn list_all_system_services(
        &self,
        query: &ListSystemServicesQuery,
    ) -> Result<Vec<SystemServiceResponse>> {
        self.fetch_all_pages(crate::paths::system_services::BASE, query)
            .await
    }

    /// Get a single system service by ID.
    pub async fn get_system_service(&self, id: &Uuid) -> Result<SystemServiceResponse> {
        self.get(&crate::paths::system_services::by_id(id)).await
    }

    /// Approve a pending system service.
    pub async fn approve_system_service(&self, id: &Uuid) -> Result<SystemServiceResponse> {
        self.post_empty(&crate::paths::system_services::approve(id))
            .await
    }

    /// Reject a pending system service.
    pub async fn reject_system_service(&self, id: &Uuid) -> Result<SystemServiceResponse> {
        self.post_empty(&crate::paths::system_services::reject(id))
            .await
    }

    /// Update a system service's configurable settings (e.g. ping interval).
    pub async fn update_system_service(
        &self,
        id: &Uuid,
        req: &UpdateSystemServiceRequest,
    ) -> Result<SystemServiceResponse> {
        self.put_json(&crate::paths::system_services::by_id(id), req)
            .await
    }

    /// Deactivate (remove) a system service.
    pub async fn remove_system_service(&self, id: &Uuid) -> Result<()> {
        self.delete(&crate::paths::system_services::by_id(id))
            .await
    }

    /// Get the global system services settings (enrollment token).
    pub async fn get_system_services_settings(&self) -> Result<SystemServicesSettingsResponse> {
        self.get(crate::paths::settings_system_services::BASE).await
    }

    /// Update the global system services settings.
    pub async fn update_system_services_settings(
        &self,
        req: &UpdateSystemServicesSettingsRequest,
    ) -> Result<SystemServicesSettingsResponse> {
        self.put_json(crate::paths::settings_system_services::BASE, req)
            .await
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_shared_types::ServiceStatus;
    use uptrakit_web_api_types::system_services::{
        ListSystemServicesQuery, UpdateSystemServiceRequest,
    };

    #[test]
    fn list_system_services_query_serialization_with_all_fields() {
        let query = ListSystemServicesQuery {
            capability: Some("mqtt_bridge".to_string()),
            status: Some(ServiceStatus::Approved),
            page: Some(2),
            per_page: Some(50),
        };
        let qs = serde_urlencoded::to_string(&query).expect("serialize");
        assert!(qs.contains("capability=mqtt_bridge"));
        assert!(qs.contains("status=approved"));
        assert!(qs.contains("page=2"));
        assert!(qs.contains("per_page=50"));
    }

    #[test]
    fn list_system_services_query_serialization_skips_none() {
        let query = ListSystemServicesQuery {
            capability: None,
            status: None,
            page: None,
            per_page: None,
        };
        let qs = serde_urlencoded::to_string(&query).expect("serialize");
        assert!(qs.is_empty());
    }

    #[test]
    fn update_system_service_request_cert_lifetime_hours_round_trip() {
        let req = UpdateSystemServiceRequest {
            ping_interval_seconds: None,
            cert_lifetime_hours: Some(48),
        };
        let json = serde_json::to_string(&req).expect("serialize");
        assert!(json.contains(r#""cert_lifetime_hours":48"#));
        let parsed: UpdateSystemServiceRequest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.cert_lifetime_hours, Some(48));
    }

    #[test]
    fn update_system_service_request_omits_none_fields() {
        let req = UpdateSystemServiceRequest {
            ping_interval_seconds: None,
            cert_lifetime_hours: None,
        };
        let json = serde_json::to_string(&req).expect("serialize");
        assert!(!json.contains("ping_interval_seconds"));
        assert!(!json.contains("cert_lifetime_hours"));
    }
}
