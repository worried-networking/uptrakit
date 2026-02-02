use serde::{Deserialize, Serialize};
use time::UtcDateTime;

/// Unix epoch timestamp in milliseconds.
pub type Timestamp = i64;

/// Returns the current time as Unix epoch milliseconds.
pub fn now_millis() -> Timestamp {
    let now = UtcDateTime::now();
    now.unix_timestamp() * 1000 + i64::from(now.millisecond())
}

/// Messages sent from the agent to the controller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMessage {
    Ping(PingPayload),
    Enroll(EnrollPayload),
    RequestCertificate(RequestCertificatePayload),
    RenewCertificate(RenewCertificatePayload),
    ReportHostInfo(ReportHostInfoPayload),
}

/// Messages sent from the controller to the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControllerMessage {
    Pong(PongPayload),
    Enrolled(EnrolledPayload),
    Approved(ApprovedPayload),
    Rejected(RejectedPayload),
    Certificate(CertificatePayload),
    Error(ErrorPayload),
    AgentSettings(AgentSettingsPayload),
    CaBundleUpdated(CaBundleUpdatedPayload),
    RequestCertRenewal(RequestCertRenewalPayload),
}

/// Payload for ping messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PingPayload {
    /// Timestamp when the agent sent the ping.
    pub agent_ts: Timestamp,
}

/// Payload for pong messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PongPayload {
    /// Original timestamp from the agent's ping.
    pub agent_ts: Timestamp,
    /// Timestamp when the controller processed the ping.
    pub controller_ts: Timestamp,
}

/// Information about the host machine running the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostInfo {
    /// Persistent machine identifier (e.g. `/etc/machine-id` on Linux, `IOPlatformUUID` on macOS).
    pub machine_id: String,
    /// Operating system type (e.g. "linux", "macos").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_type: Option<String>,
    /// Operating system version (e.g. "Ubuntu 24.04 LTS").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    /// CPU architecture (e.g. "x86_64", "aarch64").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub architecture: Option<String>,
}

/// Payload for agent enrollment request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollPayload {
    /// Agent-generated UUIDv7 client identifier.
    pub client_id: String,
    /// PEM-encoded Certificate Signing Request with CN=client_id.
    pub csr_pem: String,
    pub hostname: String,
    pub friendly_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enrollment_token: Option<String>,
    /// Host machine information for automatic host matching.
    pub host_info: HostInfo,
}

/// Payload for requesting a client certificate after approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestCertificatePayload {
    /// PEM-encoded Certificate Signing Request.
    pub csr_pem: String,
}

/// Payload for requesting certificate renewal (mTLS-authenticated agents).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenewCertificatePayload {
    /// PEM-encoded Certificate Signing Request with CN=client_id.
    pub csr_pem: String,
}

/// Payload for reporting host information (sent by authenticated agents on connect).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportHostInfoPayload {
    /// Host machine information.
    pub host_info: HostInfo,
}

/// Payload for enrollment confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrolledPayload {
    pub agent_id: String,
    pub enrollment_secret: String,
    pub status: String,
}

/// Payload for approval notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovedPayload {
    pub agent_id: String,
}

/// Payload for rejection notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedPayload {
    pub agent_id: String,
}

/// Payload for issued certificate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificatePayload {
    pub cert_pem: String,
    /// Certificate "not valid after" timestamp.
    #[serde(with = "utc_datetime_millis")]
    pub not_after: UtcDateTime,
}

/// Serde helper: serialize/deserialize `UtcDateTime` as Unix epoch milliseconds.
mod utc_datetime_millis {
    use serde::{self, Deserialize, Deserializer, Serializer};
    use time::UtcDateTime;

    pub fn serialize<S: Serializer>(dt: &UtcDateTime, serializer: S) -> Result<S::Ok, S::Error> {
        let millis = dt.unix_timestamp_nanos() / 1_000_000;
        serializer.serialize_i64(millis as i64)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<UtcDateTime, D::Error> {
        let millis = i64::deserialize(deserializer)?;
        let nanos = i128::from(millis) * 1_000_000;
        UtcDateTime::from_unix_timestamp_nanos(nanos).map_err(serde::de::Error::custom)
    }
}

/// Payload for error responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub code: String,
    pub message: String,
}

/// Payload for agent runtime settings pushed by the controller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSettingsPayload {
    pub renewal_window_hours: u16,
    #[serde(default)]
    pub ca_bundle_hash: String,
}

/// Payload for CA bundle update notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaBundleUpdatedPayload {
    pub ca_bundle_pem: String,
}

/// Payload for requesting immediate certificate renewal from agents.
///
/// Sent by the controller after CA rotation or backend URL change to prompt
/// all connected agents to renew their certificates with the new CA.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestCertRenewalPayload {
    /// Human-readable reason for the renewal request.
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_serialization_roundtrip() {
        let ping = AgentMessage::Ping(PingPayload {
            agent_ts: 1706400000000,
        });
        let json = serde_json::to_string(&ping).unwrap();
        assert_eq!(json, r#"{"type":"ping","agent_ts":1706400000000}"#);

        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ping);
    }

    #[test]
    fn pong_serialization_roundtrip() {
        let pong = ControllerMessage::Pong(PongPayload {
            agent_ts: 1706400000000,
            controller_ts: 1706400000050,
        });
        let json = serde_json::to_string(&pong).unwrap();
        assert_eq!(
            json,
            r#"{"type":"pong","agent_ts":1706400000000,"controller_ts":1706400000050}"#
        );

        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, pong);
    }

    #[test]
    fn now_millis_returns_reasonable_value() {
        let ts = now_millis();
        // Should be after 2024-01-01 (1704067200000)
        assert!(ts > 1704067200000);
    }

    #[test]
    fn enroll_serialization_roundtrip() {
        let msg = AgentMessage::Enroll(EnrollPayload {
            client_id: "01936a1e-7e8c-7f00-8000-000000000001".to_string(),
            csr_pem:
                "-----BEGIN CERTIFICATE REQUEST-----\ntest\n-----END CERTIFICATE REQUEST-----\n"
                    .to_string(),
            hostname: "node-1".to_string(),
            friendly_name: "Node One".to_string(),
            enrollment_token: Some("tok-123".to_string()),
            host_info: HostInfo {
                machine_id: "abc123".to_string(),
                os_type: Some("linux".to_string()),
                os_version: Some("Ubuntu 24.04".to_string()),
                architecture: Some("x86_64".to_string()),
            },
        });
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
        assert!(json.contains(r#""machine_id":"abc123"#));
        assert!(json.contains(r#""client_id":"01936a1e"#));
        assert!(json.contains(r#""csr_pem":"#));
    }

    #[test]
    fn enroll_without_token_serialization_roundtrip() {
        let msg = AgentMessage::Enroll(EnrollPayload {
            client_id: "01936a1e-7e8c-7f00-8000-000000000002".to_string(),
            csr_pem:
                "-----BEGIN CERTIFICATE REQUEST-----\ntest\n-----END CERTIFICATE REQUEST-----\n"
                    .to_string(),
            hostname: "node-2".to_string(),
            friendly_name: "Node Two".to_string(),
            enrollment_token: None,
            host_info: HostInfo {
                machine_id: "def456".to_string(),
                os_type: None,
                os_version: None,
                architecture: None,
            },
        });
        let json = serde_json::to_string(&msg).unwrap();
        // enrollment_token should be omitted when None
        assert!(!json.contains("enrollment_token"));
        // Optional host_info fields should be omitted when None
        assert!(!json.contains("os_type"));
        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn request_certificate_serialization_roundtrip() {
        let msg = AgentMessage::RequestCertificate(RequestCertificatePayload {
            csr_pem:
                "-----BEGIN CERTIFICATE REQUEST-----\ntest\n-----END CERTIFICATE REQUEST-----\n"
                    .to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"request_certificate"#));
        assert!(json.contains(r#""csr_pem":"#));
        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn enrolled_serialization_roundtrip() {
        let msg = ControllerMessage::Enrolled(EnrolledPayload {
            agent_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            enrollment_secret: "secret-abc".to_string(),
            status: "pending".to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"enrolled","agent_id":"550e8400-e29b-41d4-a716-446655440000","enrollment_secret":"secret-abc","status":"pending"}"#
        );
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn approved_serialization_roundtrip() {
        let msg = ControllerMessage::Approved(ApprovedPayload {
            agent_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"approved","agent_id":"550e8400-e29b-41d4-a716-446655440000"}"#
        );
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn rejected_serialization_roundtrip() {
        let msg = ControllerMessage::Rejected(RejectedPayload {
            agent_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"rejected","agent_id":"550e8400-e29b-41d4-a716-446655440000"}"#
        );
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn certificate_serialization_roundtrip() {
        let msg = ControllerMessage::Certificate(CertificatePayload {
            cert_pem: "-----BEGIN CERTIFICATE-----\nMIIB...\n-----END CERTIFICATE-----\n"
                .to_string(),
            not_after: UtcDateTime::from_unix_timestamp(1_706_400_000).unwrap(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("key_pem"));
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn renew_certificate_serialization_roundtrip() {
        let msg = AgentMessage::RenewCertificate(RenewCertificatePayload {
            csr_pem:
                "-----BEGIN CERTIFICATE REQUEST-----\nrenew\n-----END CERTIFICATE REQUEST-----\n"
                    .to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"renew_certificate"#));
        assert!(json.contains(r#""csr_pem":"#));
        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn agent_settings_serialization_roundtrip() {
        let msg = ControllerMessage::AgentSettings(AgentSettingsPayload {
            renewal_window_hours: 6,
            ca_bundle_hash: "abc123".to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"agent_settings","renewal_window_hours":6,"ca_bundle_hash":"abc123"}"#
        );
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn agent_settings_backward_compat_extra_fields() {
        // Future-proof: extra fields in JSON should be ignored
        let json = r#"{"type":"agent_settings","renewal_window_hours":12,"ca_bundle_hash":"def456","some_future_field":"value"}"#;
        let msg: ControllerMessage = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            ControllerMessage::AgentSettings(AgentSettingsPayload {
                renewal_window_hours: 12,
                ca_bundle_hash: "def456".to_string(),
            })
        );
    }

    #[test]
    fn agent_settings_backward_compat_missing_ca_hash() {
        // Agents running older protocol without ca_bundle_hash should still parse
        let json = r#"{"type":"agent_settings","renewal_window_hours":6}"#;
        let msg: ControllerMessage = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            ControllerMessage::AgentSettings(AgentSettingsPayload {
                renewal_window_hours: 6,
                ca_bundle_hash: String::new(),
            })
        );
    }

    #[test]
    fn ca_bundle_updated_serialization_roundtrip() {
        let msg = ControllerMessage::CaBundleUpdated(CaBundleUpdatedPayload {
            ca_bundle_pem: "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n"
                .to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn error_serialization_roundtrip() {
        let msg = ControllerMessage::Error(ErrorPayload {
            code: "invalid_token".to_string(),
            message: "The enrollment token is invalid".to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"error","code":"invalid_token","message":"The enrollment token is invalid"}"#
        );
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn host_info_serialization_roundtrip() {
        let info = HostInfo {
            machine_id: "abc-123-def".to_string(),
            os_type: Some("linux".to_string()),
            os_version: Some("Debian GNU/Linux 12 (bookworm)".to_string()),
            architecture: Some("aarch64".to_string()),
        };
        let json = serde_json::to_string(&info).unwrap();
        let deserialized: HostInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, info);
    }

    #[test]
    fn host_info_minimal_serialization_roundtrip() {
        let info = HostInfo {
            machine_id: "unknown".to_string(),
            os_type: None,
            os_version: None,
            architecture: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert_eq!(json, r#"{"machine_id":"unknown"}"#);
        let deserialized: HostInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, info);
    }

    #[test]
    fn report_host_info_serialization_roundtrip() {
        let msg = AgentMessage::ReportHostInfo(ReportHostInfoPayload {
            host_info: HostInfo {
                machine_id: "machine-42".to_string(),
                os_type: Some("linux".to_string()),
                os_version: Some("Ubuntu 24.04 LTS".to_string()),
                architecture: Some("x86_64".to_string()),
            },
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"report_host_info"#));
        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn request_cert_renewal_serialization_roundtrip() {
        let msg = ControllerMessage::RequestCertRenewal(RequestCertRenewalPayload {
            reason: "CA rotation after backend URL change".to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"request_cert_renewal"#));
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn enroll_backward_compat_without_required_fields() {
        // Older agents may not send client_id/csr_pem/host_info — this should fail
        // deserialization since these are required. This test documents the breaking change.
        let json = r#"{"type":"enroll","hostname":"node-old","friendly_name":"Old Node"}"#;
        let result: std::result::Result<AgentMessage, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "EnrollPayload requires client_id, csr_pem, and host_info"
        );
    }
}
