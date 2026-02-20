use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Canonical types from shared-types with feature-gated OpenAPI derives.
pub use uptrakit_shared_types::{
    ParseServiceStatusError, ParseServiceTypeError, ServiceStatus, ServiceType,
};

/// Unified response for any service (agent or MQTT).
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ServiceResponse {
    pub id: Uuid,
    pub service_type: ServiceType,
    pub hostname: String,
    pub friendly_name: String,
    pub ip_address: Option<String>,
    pub status: ServiceStatus,
    pub client_version: Option<String>,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Query parameters for listing services.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
pub struct ListServicesQuery {
    /// Filter by service type: `agent`, `mqtt`, or `ssh_agent`.
    pub r#type: Option<ServiceType>,
    /// Filter by status: `pending`, `approved`, `rejected`, `deactivated`.
    pub status: Option<ServiceStatus>,
    /// Page number (1-indexed). Defaults to 1.
    pub page: Option<u64>,
    /// Items per page. Defaults to 20, max 1000.
    pub per_page: Option<u64>,
}

impl ListServicesQuery {
    pub fn pagination(&self) -> crate::pagination::PaginationParams {
        crate::pagination::PaginationParams {
            page: self.page,
            per_page: self.per_page,
        }
    }
}

// Re-export generic types that are shared across service operations.
pub use super::agents::{
    EnrollmentTokenResponse, EnrollmentTokenStatusResponse, MergeAgentRequest, MessageResponse,
};

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_uuid() -> Uuid {
        Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6")
            .expect("hard-coded UUID should be valid")
    }

    // ── ServiceResponse ──────────────────────────────────────────────

    #[test]
    fn service_response_round_trip_all_fields() {
        let resp = ServiceResponse {
            id: sample_uuid(),
            service_type: ServiceType::Agent,
            hostname: "host-1.local".to_string(),
            friendly_name: "My Agent".to_string(),
            ip_address: Some("10.0.0.1".to_string()),
            status: ServiceStatus::Approved,
            client_version: Some("1.2.3".to_string()),
            last_seen_at: Some("2025-06-01T12:00:00Z".to_string()),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-06-01T12:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: ServiceResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.id, sample_uuid());
        assert_eq!(deserialized.service_type, ServiceType::Agent);
        assert_eq!(deserialized.hostname, "host-1.local");
        assert_eq!(deserialized.friendly_name, "My Agent");
        assert_eq!(deserialized.ip_address.as_deref(), Some("10.0.0.1"));
        assert_eq!(deserialized.status, ServiceStatus::Approved);
        assert_eq!(deserialized.client_version.as_deref(), Some("1.2.3"));
        assert_eq!(
            deserialized.last_seen_at.as_deref(),
            Some("2025-06-01T12:00:00Z")
        );
    }

    #[test]
    fn service_response_round_trip_none_fields() {
        let resp = ServiceResponse {
            id: sample_uuid(),
            service_type: ServiceType::Mqtt,
            hostname: "mqtt-broker".to_string(),
            friendly_name: "MQTT Service".to_string(),
            ip_address: None,
            status: ServiceStatus::Pending,
            client_version: None,
            last_seen_at: None,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: ServiceResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(deserialized.ip_address.is_none());
        assert!(deserialized.client_version.is_none());
        assert!(deserialized.last_seen_at.is_none());
        assert_eq!(deserialized.status, ServiceStatus::Pending);
    }

    #[test]
    fn service_response_ssh_agent_type() {
        let resp = ServiceResponse {
            id: sample_uuid(),
            service_type: ServiceType::SshAgent,
            hostname: "ssh-host".to_string(),
            friendly_name: "SSH Agent".to_string(),
            ip_address: None,
            status: ServiceStatus::Deactivated,
            client_version: None,
            last_seen_at: None,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
        };
        let json_value =
            serde_json::to_value(&resp).expect("serialization to Value should succeed");
        assert_eq!(
            json_value.get("service_type").and_then(|v| v.as_str()),
            Some("ssh_agent")
        );
        assert_eq!(
            json_value.get("status").and_then(|v| v.as_str()),
            Some("deactivated")
        );
    }

    // ── ListServicesQuery ────────────────────────────────────────────

    #[test]
    fn list_services_query_round_trip_all_fields() {
        let query = ListServicesQuery {
            r#type: Some(ServiceType::Agent),
            status: Some(ServiceStatus::Approved),
            page: Some(2),
            per_page: Some(50),
        };
        let json = serde_json::to_string(&query).expect("serialization should succeed");
        let deserialized: ListServicesQuery =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.r#type, Some(ServiceType::Agent));
        assert_eq!(deserialized.status, Some(ServiceStatus::Approved));
        assert_eq!(deserialized.page, Some(2));
        assert_eq!(deserialized.per_page, Some(50));
    }

    #[test]
    fn list_services_query_round_trip_none_fields() {
        let query = ListServicesQuery {
            r#type: None,
            status: None,
            page: None,
            per_page: None,
        };
        let json = serde_json::to_string(&query).expect("serialization should succeed");
        let deserialized: ListServicesQuery =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(deserialized.r#type.is_none());
        assert!(deserialized.status.is_none());
        assert!(deserialized.page.is_none());
        assert!(deserialized.per_page.is_none());
    }

    // ── ListServicesQuery::pagination() ──────────────────────────────

    #[test]
    fn pagination_returns_page_and_per_page() {
        let query = ListServicesQuery {
            r#type: None,
            status: None,
            page: Some(3),
            per_page: Some(25),
        };
        let params = query.pagination();
        assert_eq!(params.page, Some(3));
        assert_eq!(params.per_page, Some(25));
    }

    #[test]
    fn pagination_returns_none_when_not_set() {
        let query = ListServicesQuery {
            r#type: None,
            status: None,
            page: None,
            per_page: None,
        };
        let params = query.pagination();
        assert!(params.page.is_none());
        assert!(params.per_page.is_none());
    }

    #[test]
    fn pagination_resolve_applies_defaults() {
        let query = ListServicesQuery {
            r#type: None,
            status: None,
            page: None,
            per_page: None,
        };
        let resolved = query.pagination().resolve();
        assert_eq!(resolved.page, 1);
        assert_eq!(resolved.per_page, crate::pagination::DEFAULT_PER_PAGE);
    }
}
