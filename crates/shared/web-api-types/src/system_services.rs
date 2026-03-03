use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::validation::{Validate, ValidationError};

// Canonical types from shared-types with feature-gated OpenAPI derives.
pub use uptrakit_shared_types::{ParseServiceStatusError, ServiceStatus};

/// Unified response for a tenant-agnostic system service (MQTT bridge, scheduler).
#[derive(Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SystemServiceResponse {
    pub id: Uuid,
    pub capabilities: Vec<String>,
    pub hostname: String,
    pub friendly_name: String,
    pub ip_address: Option<String>,
    pub status: ServiceStatus,
    pub client_version: Option<String>,
    #[serde(with = "time::serde::rfc3339::option")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = Option<String>, format = DateTime)
    )]
    pub last_seen_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    #[cfg_attr(
        feature = "openapi",
        schema(value_type = String, format = DateTime)
    )]
    pub updated_at: OffsetDateTime,
    /// Custom ping interval override in seconds. `None` means the global
    /// default is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ping_interval_seconds: Option<u32>,
    /// Per-service certificate lifetime override in hours. `None` means the
    /// global default is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert_lifetime_hours: Option<u32>,
}

/// Query parameters for listing system services.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::IntoParams))]
pub struct ListSystemServicesQuery {
    /// Filter by capability.
    pub capability: Option<String>,
    /// Filter by status: `pending`, `approved`, `rejected`, `deactivated`.
    pub status: Option<ServiceStatus>,
    /// Page number (1-indexed). Defaults to 1.
    pub page: Option<u64>,
    /// Items per page. Defaults to 20, max 1000.
    pub per_page: Option<u64>,
}

impl ListSystemServicesQuery {
    pub fn pagination(&self) -> crate::pagination::PaginationParams {
        crate::pagination::PaginationParams {
            page: self.page,
            per_page: self.per_page,
        }
    }
}

/// Request to update a system service's configurable settings.
#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateSystemServiceRequest {
    /// Custom ping interval in seconds.
    /// Omit to keep current value. Set to `0` to clear the override and
    /// revert to the global default. Set to a positive value to override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ping_interval_seconds: Option<u32>,
    /// Per-service certificate lifetime in hours.
    /// Omit to keep current value. Set to `0` to clear the override and revert
    /// to the global default. Set to a positive value (1–17520) to override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert_lifetime_hours: Option<u32>,
}

impl Validate for UpdateSystemServiceRequest {
    fn validate(&self) -> Result<(), ValidationError> {
        if let Some(interval) = self.ping_interval_seconds {
            // 0 is a sentinel meaning "clear the override"; any positive value
            // must be at least 5 seconds to avoid excessive polling.
            if interval != 0 && interval < 5 {
                return Err(ValidationError {
                    field: "ping_interval_seconds",
                    message: "ping_interval_seconds must be 0 (to clear) or at least 5"
                        .to_string(),
                });
            }
        }
        if let Some(hours) = self.cert_lifetime_hours
            && hours != 0
            && !(1..=17_520u32).contains(&hours)
        {
            return Err(ValidationError {
                field: "cert_lifetime_hours",
                message: "cert_lifetime_hours must be 0 (to clear) or between 1 and 17520"
                    .to_string(),
            });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn sample_uuid() -> Uuid {
        Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6")
            .expect("hard-coded UUID should be valid")
    }

    // ── SystemServiceResponse ─────────────────────────────────────────

    #[test]
    fn system_service_response_round_trip_all_fields() {
        let resp = SystemServiceResponse {
            id: sample_uuid(),
            capabilities: vec!["mqtt_bridge".into(), "graceful_shutdown".into()],
            hostname: "mqtt-host.local".to_string(),
            friendly_name: "MQTT Bridge".to_string(),
            ip_address: Some("10.0.0.2".to_string()),
            status: ServiceStatus::Approved,
            client_version: Some("2.0.0".to_string()),
            last_seen_at: Some(datetime!(2025-06-01 12:00:00 UTC)),
            created_at: datetime!(2025-01-01 0:00:00 UTC),
            updated_at: datetime!(2025-06-01 12:00:00 UTC),
            ping_interval_seconds: Some(30),
            cert_lifetime_hours: None,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: SystemServiceResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.id, sample_uuid());
        assert_eq!(
            deserialized.capabilities,
            vec!["mqtt_bridge", "graceful_shutdown"]
        );
        assert_eq!(deserialized.hostname, "mqtt-host.local");
        assert_eq!(deserialized.friendly_name, "MQTT Bridge");
        assert_eq!(deserialized.ip_address.as_deref(), Some("10.0.0.2"));
        assert_eq!(deserialized.status, ServiceStatus::Approved);
        assert_eq!(deserialized.client_version.as_deref(), Some("2.0.0"));
        assert!(deserialized.last_seen_at.is_some());
        assert_eq!(deserialized.ping_interval_seconds, Some(30));
    }

    #[test]
    fn system_service_response_round_trip_none_fields() {
        let resp = SystemServiceResponse {
            id: sample_uuid(),
            capabilities: vec!["scheduler".into()],
            hostname: "scheduler-host".to_string(),
            friendly_name: "System Scheduler".to_string(),
            ip_address: None,
            status: ServiceStatus::Pending,
            client_version: None,
            last_seen_at: None,
            created_at: datetime!(2025-01-01 0:00:00 UTC),
            updated_at: datetime!(2025-01-01 0:00:00 UTC),
            ping_interval_seconds: None,
            cert_lifetime_hours: None,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: SystemServiceResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(deserialized.ip_address.is_none());
        assert!(deserialized.client_version.is_none());
        assert!(deserialized.last_seen_at.is_none());
        assert_eq!(deserialized.status, ServiceStatus::Pending);
        assert!(deserialized.ping_interval_seconds.is_none());
    }

    #[test]
    fn system_service_response_status_deactivated() {
        let resp = SystemServiceResponse {
            id: sample_uuid(),
            capabilities: vec!["mqtt_bridge".into()],
            hostname: "old-broker".to_string(),
            friendly_name: "Deactivated MQTT".to_string(),
            ip_address: None,
            status: ServiceStatus::Deactivated,
            client_version: None,
            last_seen_at: None,
            created_at: datetime!(2025-01-01 0:00:00 UTC),
            updated_at: datetime!(2025-01-01 0:00:00 UTC),
            ping_interval_seconds: None,
            cert_lifetime_hours: None,
        };
        let json_value =
            serde_json::to_value(&resp).expect("serialization to Value should succeed");
        assert_eq!(
            json_value.get("status").and_then(|v| v.as_str()),
            Some("deactivated")
        );
    }

    // ── ListSystemServicesQuery ───────────────────────────────────────

    #[test]
    fn list_system_services_query_round_trip_all_fields() {
        let query = ListSystemServicesQuery {
            capability: Some("mqtt_bridge".into()),
            status: Some(ServiceStatus::Approved),
            page: Some(2),
            per_page: Some(50),
        };
        let json = serde_json::to_string(&query).expect("serialization should succeed");
        let deserialized: ListSystemServicesQuery =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.capability.as_deref(), Some("mqtt_bridge"));
        assert_eq!(deserialized.status, Some(ServiceStatus::Approved));
        assert_eq!(deserialized.page, Some(2));
        assert_eq!(deserialized.per_page, Some(50));
    }

    #[test]
    fn list_system_services_query_round_trip_none_fields() {
        let query = ListSystemServicesQuery {
            capability: None,
            status: None,
            page: None,
            per_page: None,
        };
        let json = serde_json::to_string(&query).expect("serialization should succeed");
        let deserialized: ListSystemServicesQuery =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(deserialized.capability.is_none());
        assert!(deserialized.status.is_none());
        assert!(deserialized.page.is_none());
        assert!(deserialized.per_page.is_none());
    }

    // ── ListSystemServicesQuery::pagination() ─────────────────────────

    #[test]
    fn pagination_returns_page_and_per_page() {
        let query = ListSystemServicesQuery {
            capability: None,
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
        let query = ListSystemServicesQuery {
            capability: None,
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
        let query = ListSystemServicesQuery {
            capability: None,
            status: None,
            page: None,
            per_page: None,
        };
        let resolved = query.pagination().resolve();
        assert_eq!(resolved.page, 1);
        assert_eq!(resolved.per_page, crate::pagination::DEFAULT_PER_PAGE);
    }

    // ── UpdateSystemServiceRequest ────────────────────────────────────

    #[test]
    fn update_system_service_request_with_ping_interval() {
        let req = UpdateSystemServiceRequest {
            ping_interval_seconds: Some(60),
            cert_lifetime_hours: None,
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        assert!(json.contains(r#""ping_interval_seconds":60"#));
        let parsed: UpdateSystemServiceRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(parsed.ping_interval_seconds, Some(60));
    }

    #[test]
    fn update_system_service_request_without_ping_interval() {
        let req = UpdateSystemServiceRequest {
            ping_interval_seconds: None,
            cert_lifetime_hours: None,
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        assert!(!json.contains("ping_interval_seconds"));
    }

    #[test]
    fn update_system_service_request_clear_with_zero() {
        let json = r#"{"ping_interval_seconds":0}"#;
        let parsed: UpdateSystemServiceRequest =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert_eq!(parsed.ping_interval_seconds, Some(0));
    }

    // ── Validate ──────────────────────────────────────────────────────

    #[test]
    fn validate_accepts_none_interval() {
        let req = UpdateSystemServiceRequest {
            ping_interval_seconds: None,
            cert_lifetime_hours: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_accepts_zero_interval_as_clear_sentinel() {
        let req = UpdateSystemServiceRequest {
            ping_interval_seconds: Some(0),
            cert_lifetime_hours: None,
        };
        assert!(req.validate().is_ok());
    }

    #[test]
    fn validate_accepts_interval_of_five_or_more() {
        for v in [5u32, 10, 60, 3600] {
            let req = UpdateSystemServiceRequest {
                ping_interval_seconds: Some(v),
                cert_lifetime_hours: None,
            };
            assert!(req.validate().is_ok(), "expected ok for {v}");
        }
    }

    #[test]
    fn validate_rejects_interval_below_five() {
        for v in [1u32, 2, 3, 4] {
            let req = UpdateSystemServiceRequest {
                ping_interval_seconds: Some(v),
                cert_lifetime_hours: None,
            };
            let err = req.validate().unwrap_err();
            assert_eq!(err.field, "ping_interval_seconds", "field mismatch for {v}");
        }
    }

    // ── cert_lifetime_hours ───────────────────────────────────────────

    #[test]
    fn system_service_response_includes_cert_lifetime_hours() {
        let resp = SystemServiceResponse {
            id: sample_uuid(),
            capabilities: vec!["mqtt_bridge".into()],
            hostname: "host".to_string(),
            friendly_name: "H".to_string(),
            ip_address: None,
            status: ServiceStatus::Approved,
            client_version: None,
            last_seen_at: None,
            created_at: datetime!(2025-01-01 0:00:00 UTC),
            updated_at: datetime!(2025-01-01 0:00:00 UTC),
            ping_interval_seconds: None,
            cert_lifetime_hours: Some(48),
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        assert!(json.contains(r#""cert_lifetime_hours":48"#));
        let de: SystemServiceResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(de.cert_lifetime_hours, Some(48));
    }

    #[test]
    fn system_service_response_omits_cert_lifetime_hours_when_none() {
        let resp = SystemServiceResponse {
            id: sample_uuid(),
            capabilities: vec!["mqtt_bridge".into()],
            hostname: "host".to_string(),
            friendly_name: "H".to_string(),
            ip_address: None,
            status: ServiceStatus::Approved,
            client_version: None,
            last_seen_at: None,
            created_at: datetime!(2025-01-01 0:00:00 UTC),
            updated_at: datetime!(2025-01-01 0:00:00 UTC),
            ping_interval_seconds: None,
            cert_lifetime_hours: None,
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        assert!(!json.contains("cert_lifetime_hours"));
    }

    #[test]
    fn update_system_service_request_with_cert_lifetime_hours() {
        let req = UpdateSystemServiceRequest {
            ping_interval_seconds: None,
            cert_lifetime_hours: Some(48),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        assert!(json.contains(r#""cert_lifetime_hours":48"#));
        let parsed: UpdateSystemServiceRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(parsed.cert_lifetime_hours, Some(48));
    }

    #[test]
    fn update_system_service_request_clear_cert_lifetime_with_zero() {
        let json = r#"{"cert_lifetime_hours":0}"#;
        let parsed: UpdateSystemServiceRequest =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert_eq!(parsed.cert_lifetime_hours, Some(0));
        assert!(parsed.validate().is_ok());
    }

    #[test]
    fn validate_accepts_cert_lifetime_hours_in_range() {
        for v in [1u32, 12, 48, 168, 17_520] {
            let req = UpdateSystemServiceRequest {
                ping_interval_seconds: None,
                cert_lifetime_hours: Some(v),
            };
            assert!(req.validate().is_ok(), "expected ok for {v}");
        }
    }

    #[test]
    fn validate_rejects_cert_lifetime_hours_above_max() {
        let req = UpdateSystemServiceRequest {
            ping_interval_seconds: None,
            cert_lifetime_hours: Some(17_521),
        };
        let err = req.validate().unwrap_err();
        assert_eq!(err.field, "cert_lifetime_hours");
    }
}
