use serde::{Deserialize, Serialize};
use time::UtcDateTime;

// Re-export ProviderType from provider-core for use in wire protocol messages.
pub use uptrakit_provider_core::ProviderType;

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
    VersionCheckResults(VersionCheckResultsPayload),
    UpdateStarted(UpdateStartedPayload),
    UpdateOutput(UpdateOutputPayload),
    UpdateResult(UpdateResultPayload),
    Disconnecting(DisconnectingPayload),
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
    CheckVersions(CheckVersionsPayload),
    ExecuteUpdate(Box<ExecuteUpdatePayload>),
    ServerRestarting(ServerRestartingPayload),
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
    /// Agent binary version (e.g., "0.0.1").
    pub agent_version: String,
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

/// Default shutdown timeout in seconds for graceful shutdown.
fn default_shutdown_timeout_seconds() -> u32 {
    120
}

/// Payload for agent runtime settings pushed by the controller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSettingsPayload {
    pub renewal_window_hours: u16,
    #[serde(default)]
    pub ca_bundle_hash: String,
    /// Maximum time in seconds to wait for in-flight updates during shutdown.
    #[serde(default = "default_shutdown_timeout_seconds")]
    pub shutdown_timeout_seconds: u32,
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

/// Payload for server restarting notification.
///
/// Sent by the controller during graceful shutdown to notify connected agents
/// that the server is restarting. Agents should expect the connection to close
/// and reconnect automatically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerRestartingPayload {
    /// Human-readable reason for the restart.
    pub reason: String,
}

/// Payload for requesting version checks from the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckVersionsPayload {
    /// List of software items to check.
    pub assignments: Vec<VersionCheckAssignment>,
}

/// A single software item to check for installed version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionCheckAssignment {
    /// Software item ID.
    pub software_item_id: String,
    /// Human-readable name for logging.
    pub name: String,
    /// Provider type.
    pub provider_type: ProviderType,
    /// Package identifier for the provider.
    pub package_identifier: String,
    /// Provider-specific configuration.
    pub config: serde_json::Value,
}

/// Payload for version check results from the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionCheckResultsPayload {
    /// Results for each checked software item.
    pub results: Vec<VersionCheckResult>,
}

/// Result of a single version check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionCheckResult {
    /// Software item ID.
    pub software_item_id: String,
    /// Detected installed version, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
    /// Error message if detection failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// --- Update execution messages ---

/// Output stream source for UpdateOutput messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputStreamType {
    #[default]
    Stdout,
    Stderr,
    PreHook,
    PostHook,
    System,
}

/// Final status of an update execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateFinalStatus {
    Completed,
    Failed,
}

/// Default timeout for update execution in seconds.
fn default_update_timeout() -> u32 {
    300
}

/// Simplified release info sent to agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub tag: String,
    pub release_url: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<ReleaseAssetInfo>,
}

/// Information about a release asset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseAssetInfo {
    pub name: String,
    pub download_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

/// Controller -> Agent: Trigger an update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecuteUpdatePayload {
    pub update_history_id: String,
    pub software_item_id: String,
    pub software_item_name: String,
    pub package_identifier: String,
    pub to_version: String,
    pub provider_type: ProviderType,
    /// Merged provider config (base + override).
    pub provider_config: serde_json::Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_update_commands: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_update_commands: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_info: Option<ReleaseInfo>,
    #[serde(default = "default_update_timeout")]
    pub timeout_seconds: u32,
    /// Shell to use for hook execution ("bash", "sh", or future "powershell").
    /// Default: "bash" when not specified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<String>,
}

/// Agent -> Controller: Update is starting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateStartedPayload {
    pub update_history_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_version: Option<String>,
}

/// Agent -> Controller: Streaming output line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateOutputPayload {
    pub update_history_id: String,
    pub output: String,
    #[serde(default)]
    pub stream: OutputStreamType,
}

/// Agent -> Controller: Final result of update execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateResultPayload {
    pub update_history_id: String,
    pub status: UpdateFinalStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_version: Option<String>,
    pub output: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// --- Graceful shutdown messages ---

/// Reason for agent disconnection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisconnectReason {
    /// SIGTERM/SIGINT - clean exit.
    Shutdown,
    /// SIGHUP - will reconnect after external restart.
    Restart,
}

/// Agent -> Controller: Notification before disconnecting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisconnectingPayload {
    pub reason: DisconnectReason,
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
    }

    #[test]
    fn enroll_without_token_serialization_roundtrip() {
        let msg = AgentMessage::Enroll(EnrollPayload {
            client_id: "01936a1e-7e8c-7f00-8000-000000000002".to_string(),
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
            shutdown_timeout_seconds: 120,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"agent_settings","renewal_window_hours":6,"ca_bundle_hash":"abc123","shutdown_timeout_seconds":120}"#
        );
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn agent_settings_backward_compat_extra_fields() {
        // Future-proof: extra fields in JSON should be ignored
        let json = r#"{"type":"agent_settings","renewal_window_hours":12,"ca_bundle_hash":"def456","shutdown_timeout_seconds":60,"some_future_field":"value"}"#;
        let msg: ControllerMessage = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            ControllerMessage::AgentSettings(AgentSettingsPayload {
                renewal_window_hours: 12,
                ca_bundle_hash: "def456".to_string(),
                shutdown_timeout_seconds: 60,
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
                shutdown_timeout_seconds: 120, // default
            })
        );
    }

    #[test]
    fn agent_settings_backward_compat_missing_shutdown_timeout() {
        // Agents running older protocol without shutdown_timeout_seconds should still parse
        let json = r#"{"type":"agent_settings","renewal_window_hours":6,"ca_bundle_hash":"abc"}"#;
        let msg: ControllerMessage = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            ControllerMessage::AgentSettings(AgentSettingsPayload {
                renewal_window_hours: 6,
                ca_bundle_hash: "abc".to_string(),
                shutdown_timeout_seconds: 120, // default
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
            agent_version: "0.0.1".to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"report_host_info"#));
        assert!(json.contains(r#""agent_version":"0.0.1"#));
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
        // Older agents may not send client_id/host_info — this should fail
        // deserialization since these are required. This test documents the breaking change.
        let json = r#"{"type":"enroll","hostname":"node-old","friendly_name":"Old Node"}"#;
        let result: std::result::Result<AgentMessage, _> = serde_json::from_str(json);
        assert!(
            result.is_err(),
            "EnrollPayload requires client_id and host_info"
        );
    }

    #[test]
    fn check_versions_serialization_roundtrip() {
        let msg = ControllerMessage::CheckVersions(CheckVersionsPayload {
            assignments: vec![VersionCheckAssignment {
                software_item_id: "item-1".to_string(),
                name: "Test Software".to_string(),
                provider_type: ProviderType::GithubReleases,
                package_identifier: "owner/repo".to_string(),
                config: serde_json::json!({"owner": "octocat", "repo": "hello-world"}),
            }],
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"check_versions"#));
        assert!(json.contains(r#""software_item_id":"item-1"#));
        assert!(json.contains(r#""provider_type":"github_releases"#));
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn version_check_results_serialization_roundtrip() {
        let msg = AgentMessage::VersionCheckResults(VersionCheckResultsPayload {
            results: vec![
                VersionCheckResult {
                    software_item_id: "item-1".to_string(),
                    installed_version: Some("1.2.3".to_string()),
                    error: None,
                },
                VersionCheckResult {
                    software_item_id: "item-2".to_string(),
                    installed_version: None,
                    error: Some("detection failed".to_string()),
                },
            ],
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"version_check_results"#));
        assert!(json.contains(r#""installed_version":"1.2.3"#));
        // installed_version should be omitted when None
        assert!(!json.contains(r#""installed_version":null"#));
        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn version_check_assignment_serialization() {
        let assignment = VersionCheckAssignment {
            software_item_id: "uuid-123".to_string(),
            name: "Docker Image".to_string(),
            provider_type: ProviderType::DockerRegistry,
            package_identifier: "nginx:latest".to_string(),
            config: serde_json::json!({}),
        };
        let json = serde_json::to_string(&assignment).unwrap();
        assert!(json.contains(r#""provider_type":"docker_registry""#));
        let deserialized: VersionCheckAssignment = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, assignment);
    }

    // --- Update message tests ---

    #[test]
    fn execute_update_serialization_roundtrip() {
        let msg = ControllerMessage::ExecuteUpdate(Box::new(ExecuteUpdatePayload {
            update_history_id: "01936a1e-7e8c-7f00-8000-000000000001".to_string(),
            software_item_id: "01936a1e-7e8c-7f00-8000-000000000002".to_string(),
            software_item_name: "Node.js".to_string(),
            package_identifier: "nodejs".to_string(),
            to_version: "20.10.0".to_string(),
            provider_type: ProviderType::GithubReleases,
            provider_config: serde_json::json!({"owner": "nodejs", "repo": "node"}),
            pre_update_commands: vec!["systemctl stop myapp".to_string()],
            post_update_commands: vec!["systemctl start myapp".to_string()],
            release_info: Some(ReleaseInfo {
                tag: "v20.10.0".to_string(),
                release_url: "https://github.com/nodejs/node/releases/tag/v20.10.0".to_string(),
                assets: vec![ReleaseAssetInfo {
                    name: "node-v20.10.0-linux-x64.tar.gz".to_string(),
                    download_url: "https://github.com/nodejs/node/releases/download/v20.10.0/node-v20.10.0-linux-x64.tar.gz".to_string(),
                    size: Some(25_000_000),
                }],
            }),
            timeout_seconds: 600,
            shell: Some("bash".to_string()),
        }));
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"execute_update"#));
        assert!(json.contains(r#""provider_type":"github_releases"#));
        assert!(json.contains(r#""shell":"bash"#));
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn execute_update_minimal_serialization() {
        let msg = ControllerMessage::ExecuteUpdate(Box::new(ExecuteUpdatePayload {
            update_history_id: "id-1".to_string(),
            software_item_id: "id-2".to_string(),
            software_item_name: "Redis".to_string(),
            package_identifier: "redis-server".to_string(),
            to_version: "7.2.0".to_string(),
            provider_type: ProviderType::ProxmoxHelperScripts,
            provider_config: serde_json::json!({}),
            pre_update_commands: vec![],
            post_update_commands: vec![],
            release_info: None,
            timeout_seconds: 300,
            shell: None,
        }));
        let json = serde_json::to_string(&msg).unwrap();
        // Empty vectors should be omitted
        assert!(!json.contains("pre_update_commands"));
        assert!(!json.contains("post_update_commands"));
        assert!(!json.contains("release_info"));
        assert!(!json.contains("shell"));
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn execute_update_backward_compat_default_timeout() {
        let json = r#"{
            "type": "execute_update",
            "update_history_id": "id-1",
            "software_item_id": "id-2",
            "software_item_name": "Test",
            "package_identifier": "test",
            "to_version": "1.0.0",
            "provider_type": "github_releases",
            "provider_config": {}
        }"#;
        let msg: ControllerMessage = serde_json::from_str(json).unwrap();
        if let ControllerMessage::ExecuteUpdate(payload) = msg {
            assert_eq!(payload.timeout_seconds, 300);
            assert!(payload.pre_update_commands.is_empty());
            assert!(payload.post_update_commands.is_empty());
            assert!(payload.shell.is_none());
        } else {
            panic!("Expected ExecuteUpdate");
        }
    }

    #[test]
    fn execute_update_with_shell_field() {
        let json = r#"{
            "type": "execute_update",
            "update_history_id": "id-1",
            "software_item_id": "id-2",
            "software_item_name": "Test",
            "package_identifier": "test",
            "to_version": "1.0.0",
            "provider_type": "github_releases",
            "provider_config": {},
            "shell": "sh"
        }"#;
        let msg: ControllerMessage = serde_json::from_str(json).unwrap();
        if let ControllerMessage::ExecuteUpdate(payload) = msg {
            assert_eq!(payload.shell, Some("sh".to_string()));
        } else {
            panic!("Expected ExecuteUpdate");
        }
    }

    #[test]
    fn update_started_serialization_roundtrip() {
        let msg = AgentMessage::UpdateStarted(UpdateStartedPayload {
            update_history_id: "id-1".to_string(),
            from_version: Some("1.0.0".to_string()),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"update_started"#));
        assert!(json.contains(r#""from_version":"1.0.0"#));
        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn update_started_omits_none_from_version() {
        let msg = AgentMessage::UpdateStarted(UpdateStartedPayload {
            update_history_id: "id-1".to_string(),
            from_version: None,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("from_version"));
        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn update_output_serialization_roundtrip() {
        let msg = AgentMessage::UpdateOutput(UpdateOutputPayload {
            update_history_id: "id-1".to_string(),
            output: "Downloading package...".to_string(),
            stream: OutputStreamType::Stdout,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"update_output"#));
        assert!(json.contains(r#""stream":"stdout"#));
        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn update_output_all_stream_types() {
        for (stream, expected) in [
            (OutputStreamType::Stdout, "stdout"),
            (OutputStreamType::Stderr, "stderr"),
            (OutputStreamType::PreHook, "pre_hook"),
            (OutputStreamType::PostHook, "post_hook"),
            (OutputStreamType::System, "system"),
        ] {
            let msg = AgentMessage::UpdateOutput(UpdateOutputPayload {
                update_history_id: "id-1".to_string(),
                output: "test".to_string(),
                stream,
            });
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains(&format!(r#""stream":"{expected}""#)));
        }
    }

    #[test]
    fn update_output_default_stream() {
        let json = r#"{"type":"update_output","update_history_id":"id-1","output":"test"}"#;
        let msg: AgentMessage = serde_json::from_str(json).unwrap();
        if let AgentMessage::UpdateOutput(payload) = msg {
            assert_eq!(payload.stream, OutputStreamType::Stdout);
        } else {
            panic!("Expected UpdateOutput");
        }
    }

    #[test]
    fn update_result_completed_serialization_roundtrip() {
        let msg = AgentMessage::UpdateResult(UpdateResultPayload {
            update_history_id: "id-1".to_string(),
            status: UpdateFinalStatus::Completed,
            from_version: Some("1.0.0".to_string()),
            to_version: Some("2.0.0".to_string()),
            output: "Update completed successfully".to_string(),
            error: None,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"update_result"#));
        assert!(json.contains(r#""status":"completed"#));
        assert!(!json.contains("error"));
        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn update_result_failed_serialization_roundtrip() {
        let msg = AgentMessage::UpdateResult(UpdateResultPayload {
            update_history_id: "id-1".to_string(),
            status: UpdateFinalStatus::Failed,
            from_version: None,
            to_version: None,
            output: "Error output".to_string(),
            error: Some("Package not found".to_string()),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"update_result"#));
        assert!(json.contains(r#""status":"failed"#));
        assert!(json.contains(r#""error":"Package not found"#));
        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn provider_type_all_variants() {
        for (provider, expected) in [
            (ProviderType::GithubReleases, "github_releases"),
            (ProviderType::ProxmoxHelperScripts, "proxmox_helper_scripts"),
            (ProviderType::DockerRegistry, "docker_registry"),
        ] {
            let json = serde_json::to_string(&provider).unwrap();
            assert_eq!(json, format!(r#""{expected}""#));
            let deserialized: ProviderType = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, provider);
        }
    }

    #[test]
    fn release_info_serialization_roundtrip() {
        let info = ReleaseInfo {
            tag: "v1.0.0".to_string(),
            release_url: "https://example.com/release".to_string(),
            assets: vec![
                ReleaseAssetInfo {
                    name: "app.tar.gz".to_string(),
                    download_url: "https://example.com/app.tar.gz".to_string(),
                    size: Some(1024),
                },
                ReleaseAssetInfo {
                    name: "app.deb".to_string(),
                    download_url: "https://example.com/app.deb".to_string(),
                    size: None,
                },
            ],
        };
        let json = serde_json::to_string(&info).unwrap();
        let deserialized: ReleaseInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, info);
    }

    #[test]
    fn release_info_empty_assets_omitted() {
        let info = ReleaseInfo {
            tag: "v1.0.0".to_string(),
            release_url: "https://example.com/release".to_string(),
            assets: vec![],
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(!json.contains("assets"));
        let deserialized: ReleaseInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, info);
    }

    #[test]
    fn execute_update_backward_compat_extra_fields() {
        let json = r#"{
            "type": "execute_update",
            "update_history_id": "id-1",
            "software_item_id": "id-2",
            "software_item_name": "Test",
            "package_identifier": "test",
            "to_version": "1.0.0",
            "provider_type": "github_releases",
            "provider_config": {},
            "unknown_field": "ignored"
        }"#;
        let msg: ControllerMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ControllerMessage::ExecuteUpdate(_)));
    }

    #[test]
    fn server_restarting_serialization_roundtrip() {
        let msg = ControllerMessage::ServerRestarting(ServerRestartingPayload {
            reason: "controller restarting for upgrade".to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"server_restarting"#));
        assert!(json.contains(r#""reason":"controller restarting for upgrade"#));
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn server_restarting_backward_compat_extra_fields() {
        let json = r#"{"type":"server_restarting","reason":"restart","unknown_field":"ignored"}"#;
        let msg: ControllerMessage = serde_json::from_str(json).unwrap();
        assert!(matches!(msg, ControllerMessage::ServerRestarting(_)));
    }

    // --- Graceful shutdown message tests ---

    #[test]
    fn disconnecting_shutdown_serialization_roundtrip() {
        let msg = AgentMessage::Disconnecting(DisconnectingPayload {
            reason: DisconnectReason::Shutdown,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"disconnecting","reason":"shutdown"}"#);
        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn disconnecting_restart_serialization_roundtrip() {
        let msg = AgentMessage::Disconnecting(DisconnectingPayload {
            reason: DisconnectReason::Restart,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"disconnecting","reason":"restart"}"#);
        let deserialized: AgentMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn disconnect_reason_all_variants() {
        for (reason, expected) in [
            (DisconnectReason::Shutdown, "shutdown"),
            (DisconnectReason::Restart, "restart"),
        ] {
            let json = serde_json::to_string(&reason).unwrap();
            assert_eq!(json, format!(r#""{expected}""#));
            let deserialized: DisconnectReason = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, reason);
        }
    }
}
