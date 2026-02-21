use crate::services::ServiceStatus;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub use super::agents::MessageResponse as HostMessageResponse;

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HostResponse {
    pub id: Uuid,
    pub machine_id: String,
    pub hostname: String,
    pub friendly_name: String,
    pub os_type: Option<String>,
    pub os_version: Option<String>,
    pub architecture: Option<String>,
    pub ip_address: Option<String>,
    pub last_seen_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub agents: Vec<HostAgentSummary>,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct HostAgentSummary {
    pub id: Uuid,
    pub friendly_name: String,
    pub status: ServiceStatus,
}

#[derive(Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UpdateHostRequest {
    pub friendly_name: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_uuid() -> Uuid {
        Uuid::parse_str("a1a2a3a4-b1b2-c1c2-d1d2-e1e2e3e4e5e6")
            .expect("hard-coded UUID should be valid")
    }

    fn sample_agent_uuid() -> Uuid {
        Uuid::parse_str("b1b2b3b4-c1c2-d1d2-e1e2-f1f2f3f4f5f6")
            .expect("hard-coded UUID should be valid")
    }

    // ── HostAgentSummary ─────────────────────────────────────────────

    #[test]
    fn host_agent_summary_round_trip() {
        let summary = HostAgentSummary {
            id: sample_agent_uuid(),
            friendly_name: "agent-1".to_string(),
            status: ServiceStatus::Approved,
        };
        let json = serde_json::to_string(&summary).expect("serialization should succeed");
        let deserialized: HostAgentSummary =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.id, sample_agent_uuid());
        assert_eq!(deserialized.friendly_name, "agent-1");
        assert_eq!(deserialized.status, ServiceStatus::Approved);
    }

    #[test]
    fn host_agent_summary_pending_status() {
        let summary = HostAgentSummary {
            id: sample_agent_uuid(),
            friendly_name: "new-agent".to_string(),
            status: ServiceStatus::Pending,
        };
        let json_value =
            serde_json::to_value(&summary).expect("serialization to Value should succeed");
        assert_eq!(
            json_value.get("status").and_then(|v| v.as_str()),
            Some("pending")
        );
    }

    // ── HostResponse ─────────────────────────────────────────────────

    #[test]
    fn host_response_round_trip_all_fields() {
        let resp = HostResponse {
            id: sample_uuid(),
            machine_id: "machine-001".to_string(),
            hostname: "server-1.local".to_string(),
            friendly_name: "Production Server".to_string(),
            os_type: Some("linux".to_string()),
            os_version: Some("Ubuntu 22.04".to_string()),
            architecture: Some("x86_64".to_string()),
            ip_address: Some("192.168.1.100".to_string()),
            last_seen_at: Some("2025-06-01T12:00:00Z".to_string()),
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-06-01T12:00:00Z".to_string(),
            agents: vec![HostAgentSummary {
                id: sample_agent_uuid(),
                friendly_name: "agent-1".to_string(),
                status: ServiceStatus::Approved,
            }],
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: HostResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.id, sample_uuid());
        assert_eq!(deserialized.machine_id, "machine-001");
        assert_eq!(deserialized.hostname, "server-1.local");
        assert_eq!(deserialized.friendly_name, "Production Server");
        assert_eq!(deserialized.os_type.as_deref(), Some("linux"));
        assert_eq!(deserialized.os_version.as_deref(), Some("Ubuntu 22.04"));
        assert_eq!(deserialized.architecture.as_deref(), Some("x86_64"));
        assert_eq!(deserialized.ip_address.as_deref(), Some("192.168.1.100"));
        assert_eq!(
            deserialized.last_seen_at.as_deref(),
            Some("2025-06-01T12:00:00Z")
        );
        assert_eq!(deserialized.agents.len(), 1);
        assert_eq!(deserialized.agents[0].status, ServiceStatus::Approved);
    }

    #[test]
    fn host_response_round_trip_none_fields() {
        let resp = HostResponse {
            id: sample_uuid(),
            machine_id: "machine-002".to_string(),
            hostname: "unknown-host".to_string(),
            friendly_name: "New Host".to_string(),
            os_type: None,
            os_version: None,
            architecture: None,
            ip_address: None,
            last_seen_at: None,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
            agents: vec![],
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: HostResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(deserialized.os_type.is_none());
        assert!(deserialized.os_version.is_none());
        assert!(deserialized.architecture.is_none());
        assert!(deserialized.ip_address.is_none());
        assert!(deserialized.last_seen_at.is_none());
        assert!(deserialized.agents.is_empty());
    }

    #[test]
    fn host_response_none_fields_serialize_as_null() {
        let resp = HostResponse {
            id: sample_uuid(),
            machine_id: "m".to_string(),
            hostname: "h".to_string(),
            friendly_name: "f".to_string(),
            os_type: None,
            os_version: None,
            architecture: None,
            ip_address: None,
            last_seen_at: None,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
            agents: vec![],
        };
        let json_value =
            serde_json::to_value(&resp).expect("serialization to Value should succeed");
        let obj = json_value
            .as_object()
            .expect("top-level value should be an object");
        for field in [
            "os_type",
            "os_version",
            "architecture",
            "ip_address",
            "last_seen_at",
        ] {
            assert!(
                obj.get(field).expect("field should be present").is_null(),
                "{field} should serialize as null when None"
            );
        }
    }

    #[test]
    fn host_response_multiple_agents_different_statuses() {
        let resp = HostResponse {
            id: sample_uuid(),
            machine_id: "m".to_string(),
            hostname: "h".to_string(),
            friendly_name: "f".to_string(),
            os_type: None,
            os_version: None,
            architecture: None,
            ip_address: None,
            last_seen_at: None,
            created_at: "2025-01-01T00:00:00Z".to_string(),
            updated_at: "2025-01-01T00:00:00Z".to_string(),
            agents: vec![
                HostAgentSummary {
                    id: sample_agent_uuid(),
                    friendly_name: "approved-agent".to_string(),
                    status: ServiceStatus::Approved,
                },
                HostAgentSummary {
                    id: sample_uuid(),
                    friendly_name: "pending-agent".to_string(),
                    status: ServiceStatus::Pending,
                },
            ],
        };
        let json = serde_json::to_string(&resp).expect("serialization should succeed");
        let deserialized: HostResponse =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.agents.len(), 2);
        assert_eq!(deserialized.agents[0].status, ServiceStatus::Approved);
        assert_eq!(deserialized.agents[1].status, ServiceStatus::Pending);
    }

    // ── UpdateHostRequest ────────────────────────────────────────────

    #[test]
    fn update_host_request_round_trip_with_name() {
        let req = UpdateHostRequest {
            friendly_name: Some("New Name".to_string()),
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: UpdateHostRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert_eq!(deserialized.friendly_name.as_deref(), Some("New Name"));
    }

    #[test]
    fn update_host_request_round_trip_none() {
        let req = UpdateHostRequest {
            friendly_name: None,
        };
        let json = serde_json::to_string(&req).expect("serialization should succeed");
        let deserialized: UpdateHostRequest =
            serde_json::from_str(&json).expect("deserialization should succeed");
        assert!(deserialized.friendly_name.is_none());
    }

    #[test]
    fn update_host_request_from_empty_json_object() {
        let json = r#"{}"#;
        let req: UpdateHostRequest =
            serde_json::from_str(json).expect("deserialization should succeed");
        assert!(req.friendly_name.is_none());
    }
}
