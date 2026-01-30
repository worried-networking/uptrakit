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

/// Payload for agent enrollment request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollPayload {
    pub hostname: String,
    pub friendly_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enrollment_token: Option<String>,
}

/// Payload for requesting a client certificate after approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestCertificatePayload {}

/// Payload for requesting certificate renewal (mTLS-authenticated agents).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenewCertificatePayload {}

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
    pub key_pem: String,
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
            hostname: "node-1".to_string(),
            friendly_name: "Node One".to_string(),
            enrollment_token: Some("tok-123".to_string()),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"enroll","hostname":"node-1","friendly_name":"Node One","enrollment_token":"tok-123"}"#
        );
        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn enroll_without_token_serialization_roundtrip() {
        let msg = AgentMessage::Enroll(EnrollPayload {
            hostname: "node-2".to_string(),
            friendly_name: "Node Two".to_string(),
            enrollment_token: None,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"enroll","hostname":"node-2","friendly_name":"Node Two"}"#
        );
        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn request_certificate_serialization_roundtrip() {
        let msg = AgentMessage::RequestCertificate(RequestCertificatePayload {});
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"request_certificate"}"#);
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
            key_pem: "-----BEGIN PRIVATE KEY-----\nMIIE...\n-----END PRIVATE KEY-----\n"
                .to_string(),
            not_after: UtcDateTime::from_unix_timestamp(1_706_400_000).unwrap(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn renew_certificate_serialization_roundtrip() {
        let msg = AgentMessage::RenewCertificate(RenewCertificatePayload {});
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"renew_certificate"}"#);
        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn agent_settings_serialization_roundtrip() {
        let msg = ControllerMessage::AgentSettings(AgentSettingsPayload {
            renewal_window_hours: 6,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"agent_settings","renewal_window_hours":6}"#
        );
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn agent_settings_backward_compat_extra_fields() {
        // Future-proof: extra fields in JSON should be ignored
        let json =
            r#"{"type":"agent_settings","renewal_window_hours":12,"some_future_field":"value"}"#;
        let msg: ControllerMessage = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            ControllerMessage::AgentSettings(AgentSettingsPayload {
                renewal_window_hours: 12,
            })
        );
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
}
