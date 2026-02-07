use serde::{Deserialize, Serialize};
use time::UtcDateTime;
use uuid::Uuid;

// Re-export provider-core types used directly in wire protocol messages.
pub use uptrakit_provider_core::{ProviderType, ReleaseAsset};

/// Enrollment status returned in the `Enrolled` message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnrollmentStatus {
    Pending,
    Approved,
}

impl std::fmt::Display for EnrollmentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => f.write_str("pending"),
            Self::Approved => f.write_str("approved"),
        }
    }
}

/// Type of service enrolling with the controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceType {
    Agent,
    Mqtt,
}

impl std::fmt::Display for ServiceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Agent => f.write_str("agent"),
            Self::Mqtt => f.write_str("mqtt"),
        }
    }
}

/// MQTT connection transport protocol.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MqttTransport {
    /// Plain TCP connection.
    #[default]
    Tcp,
    /// TLS-encrypted connection.
    Tls,
}

impl std::fmt::Display for MqttTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Tcp => f.write_str("tcp"),
            Self::Tls => f.write_str("tls"),
        }
    }
}

/// Shell type for hook execution in update payloads.
///
/// Determines which shell interpreter and fail-early settings are used.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookShell {
    /// Bash shell with `set -euo pipefail`
    #[default]
    Bash,
    /// POSIX sh with `set -eu`
    Sh,
    /// PowerShell with `$ErrorActionPreference = 'Stop'`
    #[serde(rename = "powershell")]
    PowerShell,
}

impl std::fmt::Display for HookShell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bash => f.write_str("bash"),
            Self::Sh => f.write_str("sh"),
            Self::PowerShell => f.write_str("powershell"),
        }
    }
}

/// Unix epoch timestamp in milliseconds.
pub type Timestamp = i64;

/// Returns the current time as Unix epoch milliseconds.
pub fn now_millis() -> Timestamp {
    let now = UtcDateTime::now();
    now.unix_timestamp() * 1000 + i64::from(now.millisecond())
}

/// Messages sent from a service (agent or MQTT) to the controller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServiceMessage {
    // -- Shared enrollment + lifecycle --
    Ping(PingPayload),
    Enroll(EnrollPayload),
    RequestCertificate(RequestCertificatePayload),
    RenewCertificate(RenewCertificatePayload),
    Disconnecting(DisconnectingPayload),
    // -- Agent-specific --
    ReportHostInfo(ReportHostInfoPayload),
    VersionCheckResults(VersionCheckResultsPayload),
    UpdateStarted(UpdateStartedPayload),
    UpdateOutput(UpdateOutputPayload),
    UpdateResult(UpdateResultPayload),
    // -- MQTT-specific --
    Register(MqttRegisterPayload),
    ReleaseTenants(MqttReleaseTenantsPayload),
}

/// Messages sent from the controller to a service (agent or MQTT).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ControllerMessage {
    // -- Shared --
    Pong(PongPayload),
    Enrolled(EnrolledPayload),
    Approved(ApprovedPayload),
    Rejected(RejectedPayload),
    Certificate(CertificatePayload),
    Error(ErrorPayload),
    ServiceSettings(ServiceSettingsPayload),
    CaBundleUpdated(CaBundleUpdatedPayload),
    RequestCertRenewal(RequestCertRenewalPayload),
    ServerRestarting(ServerRestartingPayload),
    // -- Agent-specific --
    CheckVersions(CheckVersionsPayload),
    ExecuteUpdate(Box<ExecuteUpdatePayload>),
    // -- MQTT-specific --
    Registered(MqttRegisteredPayload),
    TenantAssignments(MqttTenantAssignmentsPayload),
    TenantConfigUpdated(MqttTenantConfigUpdatedPayload),
    TenantRevoked(MqttTenantRevokedPayload),
}

/// Payload for ping messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PingPayload {
    /// Timestamp when the service sent the ping.
    pub service_ts: Timestamp,
}

/// Payload for pong messages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PongPayload {
    /// Original timestamp from the service's ping.
    pub service_ts: Timestamp,
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

/// Payload for service enrollment request.
///
/// Both agents and MQTT services use this payload. Agents set `host_info` to
/// report machine identity; MQTT services leave it `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollPayload {
    pub hostname: String,
    pub friendly_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enrollment_token: Option<String>,
    /// Identifies the type of service enrolling.
    pub service_type: ServiceType,
    /// Host machine information for automatic host matching.
    /// Set by agents, absent for MQTT services.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub host_info: Option<HostInfo>,
}

/// Payload for requesting a client certificate after approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestCertificatePayload {
    /// PEM-encoded Certificate Signing Request.
    pub csr_pem: String,
}

/// Payload for requesting certificate renewal (mTLS-authenticated services).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RenewCertificatePayload {
    /// PEM-encoded Certificate Signing Request with CN=service_id.
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
    pub service_id: Uuid,
    pub enrollment_secret: String,
    pub status: EnrollmentStatus,
}

/// Payload for approval notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovedPayload {
    pub service_id: Uuid,
}

/// Payload for rejection notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RejectedPayload {
    pub service_id: Uuid,
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

/// Machine-readable error code sent in `ErrorPayload`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Malformed or unexpected message from the service.
    BadRequest,
    /// Enrollment attempt failed on the controller side.
    EnrollmentFailed,
    /// Service is not approved (pending or rejected).
    NotApproved,
    /// Service is not allowed to perform this action.
    Forbidden,
    /// Certificate signing or renewal error.
    CertificateError,
    /// Unrecoverable server-side error.
    InternalError,
    /// Agent binary version is below the minimum required.
    AgentVersionTooOld,
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadRequest => f.write_str("bad_request"),
            Self::EnrollmentFailed => f.write_str("enrollment_failed"),
            Self::NotApproved => f.write_str("not_approved"),
            Self::Forbidden => f.write_str("forbidden"),
            Self::CertificateError => f.write_str("certificate_error"),
            Self::InternalError => f.write_str("internal_error"),
            Self::AgentVersionTooOld => f.write_str("agent_version_too_old"),
        }
    }
}

/// Payload for error responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub code: ErrorCode,
    pub message: String,
}

/// Payload for service runtime settings pushed by the controller.
///
/// Used for both agents and MQTT services. `shutdown_timeout_seconds` is
/// present for agents and `None` for MQTT services.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceSettingsPayload {
    pub renewal_window_hours: u16,
    #[serde(default)]
    pub ca_bundle_hash: String,
    /// Maximum time in seconds to wait for in-flight operations during shutdown.
    /// Present for agents, absent for MQTT services.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shutdown_timeout_seconds: Option<u32>,
}

/// Payload for CA bundle update notification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CaBundleUpdatedPayload {
    pub ca_bundle_pem: String,
}

/// Payload for requesting immediate certificate renewal from services.
///
/// Sent by the controller after CA rotation or backend URL change to prompt
/// all connected services to renew their certificates with the new CA.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestCertRenewalPayload {
    /// Human-readable reason for the renewal request.
    pub reason: String,
}

/// Payload for server restarting notification.
///
/// Sent by the controller during graceful shutdown to notify connected services
/// that the server is restarting. Services should expect the connection to close
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
    pub software_item_id: Uuid,
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
    pub software_item_id: Uuid,
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
    pub assets: Vec<ReleaseAsset>,
}

/// Controller -> Agent: Trigger an update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecuteUpdatePayload {
    pub update_history_id: Uuid,
    pub software_item_id: Uuid,
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
    /// Shell to use for hook execution.
    /// Default: `HookShell::Bash` when not specified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<HookShell>,
}

/// Agent -> Controller: Update is starting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateStartedPayload {
    pub update_history_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_version: Option<String>,
}

/// Agent -> Controller: Streaming output line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateOutputPayload {
    pub update_history_id: Uuid,
    pub output: String,
    #[serde(default)]
    pub stream: OutputStreamType,
}

/// Agent -> Controller: Final result of update execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateResultPayload {
    pub update_history_id: Uuid,
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

/// Reason for service disconnection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisconnectReason {
    /// SIGTERM/SIGINT - clean exit.
    Shutdown,
    /// SIGHUP - will reconnect after external restart.
    Restart,
}

/// Service -> Controller: Notification before disconnecting.
///
/// Agents send this with just a reason; MQTT services also include the list
/// of MQTT client IDs that were active at disconnection time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisconnectingPayload {
    pub reason: DisconnectReason,
    /// MQTT client IDs that were active at disconnection time (MQTT services only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_mqtt_clients: Vec<Uuid>,
}

// =============================================================================
// MQTT Service Specific Payloads
// =============================================================================

/// Payload for MQTT service instance registration (sent after mTLS connect).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttRegisterPayload {
    /// Unique instance identifier (e.g., hostname-uuid prefix).
    pub instance_id: String,
    /// Maximum tenants this instance will handle (0 = unlimited).
    #[serde(default)]
    pub max_tenants: u32,
    /// Currently active MQTT client IDs (for reconnect reconciliation).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub active_mqtt_clients: Vec<Uuid>,
}

/// Payload for registration acknowledgment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttRegisteredPayload {
    /// Echo back the instance ID for confirmation.
    pub instance_id: String,
}

/// Payload for explicitly releasing MQTT clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttReleaseTenantsPayload {
    /// MQTT client IDs to release.
    pub mqtt_client_ids: Vec<Uuid>,
}

/// Payload for tenant assignments (initial or incremental).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttTenantAssignmentsPayload {
    /// List of tenant configurations to start serving.
    pub tenants: Vec<MqttTenantConfig>,
}

/// Configuration for a single MQTT client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttTenantConfig {
    /// MQTT client UUID (primary identifier from mqtt_clients table).
    pub mqtt_client_id: Uuid,
    /// Tenant UUID (kept for context).
    pub tenant_id: Uuid,
    /// Whether MQTT is enabled for this tenant.
    pub enabled: bool,
    /// Transport protocol (tcp, tls).
    pub transport: MqttTransport,
    /// Broker hostname.
    pub host: String,
    /// Broker port.
    pub port: u16,
    /// MQTT client ID.
    pub client_id: String,
    /// Username (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Password (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Topic prefix.
    pub topic_prefix: String,
    /// Last update timestamp (for change detection).
    #[serde(with = "utc_datetime_millis")]
    pub updated_at: UtcDateTime,
}

/// Payload for single tenant config update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttTenantConfigUpdatedPayload {
    /// Updated tenant configuration.
    pub tenant: MqttTenantConfig,
}

/// Payload for MQTT client revocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttTenantRevokedPayload {
    /// MQTT client UUID being revoked.
    pub mqtt_client_id: Uuid,
    /// Reason for revocation.
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_UUID_1: Uuid = Uuid::from_bytes([
        0x55, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44, 0x00,
        0x00,
    ]);
    const TEST_UUID_2: Uuid = Uuid::from_bytes([
        0x55, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44, 0x00,
        0x01,
    ]);
    const TEST_UUID_3: Uuid = Uuid::from_bytes([
        0x66, 0x0e, 0x84, 0x00, 0xe2, 0x9b, 0x41, 0xd4, 0xa7, 0x16, 0x44, 0x66, 0x55, 0x44, 0x00,
        0x01,
    ]);
    // =========================================================================
    // ServiceMessage tests
    // =========================================================================

    #[test]
    fn ping_serialization_roundtrip() {
        let ping = ServiceMessage::Ping(PingPayload {
            service_ts: 1706400000000,
        });
        let json = serde_json::to_string(&ping).unwrap();
        assert_eq!(json, r#"{"type":"ping","service_ts":1706400000000}"#);

        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ping);
    }

    #[test]
    fn enroll_agent_serialization_roundtrip() {
        let msg = ServiceMessage::Enroll(EnrollPayload {
            hostname: "node-1".to_string(),
            friendly_name: "Node One".to_string(),
            enrollment_token: Some("tok-123".to_string()),
            service_type: ServiceType::Agent,
            host_info: Some(HostInfo {
                machine_id: "abc123".to_string(),
                os_type: Some("linux".to_string()),
                os_version: Some("Ubuntu 24.04".to_string()),
                architecture: Some("x86_64".to_string()),
            }),
        });
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
        assert!(json.contains(r#""machine_id":"abc123"#));
        assert!(json.contains(r#""service_type":"agent"#));
    }

    #[test]
    fn enroll_mqtt_serialization_roundtrip() {
        let msg = ServiceMessage::Enroll(EnrollPayload {
            hostname: "mqtt-service-1".to_string(),
            friendly_name: "MQTT Service Node 1".to_string(),
            enrollment_token: Some("tok-456".to_string()),
            service_type: ServiceType::Mqtt,
            host_info: None,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"enroll"#));
        assert!(json.contains(r#""hostname":"mqtt-service-1"#));
        assert!(json.contains(r#""service_type":"mqtt"#));
        assert!(!json.contains("host_info"));
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn enroll_without_token_serialization_roundtrip() {
        let msg = ServiceMessage::Enroll(EnrollPayload {
            hostname: "node-2".to_string(),
            friendly_name: "Node Two".to_string(),
            enrollment_token: None,
            service_type: ServiceType::Agent,
            host_info: Some(HostInfo {
                machine_id: "def456".to_string(),
                os_type: None,
                os_version: None,
                architecture: None,
            }),
        });
        let json = serde_json::to_string(&msg).unwrap();
        // enrollment_token should be omitted when None
        assert!(!json.contains("enrollment_token"));
        // Optional host_info fields should be omitted when None
        assert!(!json.contains("os_type"));
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn request_certificate_serialization_roundtrip() {
        let msg = ServiceMessage::RequestCertificate(RequestCertificatePayload {
            csr_pem:
                "-----BEGIN CERTIFICATE REQUEST-----\ntest\n-----END CERTIFICATE REQUEST-----\n"
                    .to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"request_certificate"#));
        assert!(json.contains(r#""csr_pem":"#));
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn renew_certificate_serialization_roundtrip() {
        let msg = ServiceMessage::RenewCertificate(RenewCertificatePayload {
            csr_pem:
                "-----BEGIN CERTIFICATE REQUEST-----\nrenew\n-----END CERTIFICATE REQUEST-----\n"
                    .to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"renew_certificate"#));
        assert!(json.contains(r#""csr_pem":"#));
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn report_host_info_serialization_roundtrip() {
        let msg = ServiceMessage::ReportHostInfo(ReportHostInfoPayload {
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
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn version_check_results_serialization_roundtrip() {
        let msg = ServiceMessage::VersionCheckResults(VersionCheckResultsPayload {
            results: vec![
                VersionCheckResult {
                    software_item_id: TEST_UUID_1,
                    installed_version: Some("1.2.3".to_string()),
                    error: None,
                },
                VersionCheckResult {
                    software_item_id: TEST_UUID_2,
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
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn update_started_serialization_roundtrip() {
        let msg = ServiceMessage::UpdateStarted(UpdateStartedPayload {
            update_history_id: TEST_UUID_1,
            from_version: Some("1.0.0".to_string()),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"update_started"#));
        assert!(json.contains(r#""from_version":"1.0.0"#));
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn update_started_omits_none_from_version() {
        let msg = ServiceMessage::UpdateStarted(UpdateStartedPayload {
            update_history_id: TEST_UUID_1,
            from_version: None,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("from_version"));
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn update_output_serialization_roundtrip() {
        let msg = ServiceMessage::UpdateOutput(UpdateOutputPayload {
            update_history_id: TEST_UUID_1,
            output: "Downloading package...".to_string(),
            stream: OutputStreamType::Stdout,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"update_output"#));
        assert!(json.contains(r#""stream":"stdout"#));
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
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
            let msg = ServiceMessage::UpdateOutput(UpdateOutputPayload {
                update_history_id: TEST_UUID_1,
                output: "test".to_string(),
                stream,
            });
            let json = serde_json::to_string(&msg).unwrap();
            assert!(json.contains(&format!(r#""stream":"{expected}""#)));
        }
    }

    #[test]
    fn update_output_default_stream() {
        let json = r#"{"type":"update_output","update_history_id":"550e8400-e29b-41d4-a716-446655440000","output":"test"}"#;
        let msg: ServiceMessage = serde_json::from_str(json).unwrap();
        if let ServiceMessage::UpdateOutput(payload) = msg {
            assert_eq!(payload.stream, OutputStreamType::Stdout);
        } else {
            panic!("Expected UpdateOutput");
        }
    }

    #[test]
    fn update_result_completed_serialization_roundtrip() {
        let msg = ServiceMessage::UpdateResult(UpdateResultPayload {
            update_history_id: TEST_UUID_1,
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
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn update_result_failed_serialization_roundtrip() {
        let msg = ServiceMessage::UpdateResult(UpdateResultPayload {
            update_history_id: TEST_UUID_1,
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
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn disconnecting_shutdown_serialization_roundtrip() {
        let msg = ServiceMessage::Disconnecting(DisconnectingPayload {
            reason: DisconnectReason::Shutdown,
            active_mqtt_clients: vec![],
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"disconnecting","reason":"shutdown"}"#);
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn disconnecting_restart_serialization_roundtrip() {
        let msg = ServiceMessage::Disconnecting(DisconnectingPayload {
            reason: DisconnectReason::Restart,
            active_mqtt_clients: vec![],
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(json, r#"{"type":"disconnecting","reason":"restart"}"#);
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn disconnecting_with_active_mqtt_clients() {
        let msg = ServiceMessage::Disconnecting(DisconnectingPayload {
            reason: DisconnectReason::Shutdown,
            active_mqtt_clients: vec![TEST_UUID_1],
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""reason":"shutdown"#));
        assert!(json.contains(r#""active_mqtt_clients":["550e8400-e29b-41d4-a716-446655440000"]"#));
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn disconnecting_empty_mqtt_clients_omitted() {
        let msg = ServiceMessage::Disconnecting(DisconnectingPayload {
            reason: DisconnectReason::Restart,
            active_mqtt_clients: vec![],
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("active_mqtt_clients"));
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn register_serialization_roundtrip() {
        let msg = ServiceMessage::Register(MqttRegisterPayload {
            instance_id: "mqtt-node1-01936a1e".to_string(),
            max_tenants: 10,
            active_mqtt_clients: vec![TEST_UUID_1, TEST_UUID_2],
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"register"#));
        assert!(json.contains(r#""instance_id":"mqtt-node1-01936a1e"#));
        assert!(json.contains(r#""max_tenants":10"#));
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn register_empty_active_mqtt_clients() {
        let msg = ServiceMessage::Register(MqttRegisterPayload {
            instance_id: "mqtt-node2-01936a1e".to_string(),
            max_tenants: 0,
            active_mqtt_clients: vec![],
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("active_mqtt_clients"));
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn release_tenants_serialization_roundtrip() {
        let msg = ServiceMessage::ReleaseTenants(MqttReleaseTenantsPayload {
            mqtt_client_ids: vec![TEST_UUID_1],
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"release_tenants"#));
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    // =========================================================================
    // ControllerMessage tests
    // =========================================================================

    #[test]
    fn pong_serialization_roundtrip() {
        let pong = ControllerMessage::Pong(PongPayload {
            service_ts: 1706400000000,
            controller_ts: 1706400000050,
        });
        let json = serde_json::to_string(&pong).unwrap();
        assert_eq!(
            json,
            r#"{"type":"pong","service_ts":1706400000000,"controller_ts":1706400000050}"#
        );

        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, pong);
    }

    #[test]
    fn enrolled_serialization_roundtrip() {
        let msg = ControllerMessage::Enrolled(EnrolledPayload {
            service_id: TEST_UUID_1,
            enrollment_secret: "secret-abc".to_string(),
            status: EnrollmentStatus::Pending,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"enrolled","service_id":"550e8400-e29b-41d4-a716-446655440000","enrollment_secret":"secret-abc","status":"pending"}"#
        );
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn approved_serialization_roundtrip() {
        let msg = ControllerMessage::Approved(ApprovedPayload {
            service_id: TEST_UUID_1,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"approved","service_id":"550e8400-e29b-41d4-a716-446655440000"}"#
        );
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn rejected_serialization_roundtrip() {
        let msg = ControllerMessage::Rejected(RejectedPayload {
            service_id: TEST_UUID_1,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"rejected","service_id":"550e8400-e29b-41d4-a716-446655440000"}"#
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
    fn error_serialization_roundtrip() {
        let msg = ControllerMessage::Error(ErrorPayload {
            code: ErrorCode::EnrollmentFailed,
            message: "The enrollment token is invalid".to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"error","code":"enrollment_failed","message":"The enrollment token is invalid"}"#
        );
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn service_settings_serialization_roundtrip() {
        let msg = ControllerMessage::ServiceSettings(ServiceSettingsPayload {
            renewal_window_hours: 6,
            ca_bundle_hash: "abc123".to_string(),
            shutdown_timeout_seconds: Some(120),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"service_settings","renewal_window_hours":6,"ca_bundle_hash":"abc123","shutdown_timeout_seconds":120}"#
        );
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn service_settings_without_shutdown_timeout() {
        let msg = ControllerMessage::ServiceSettings(ServiceSettingsPayload {
            renewal_window_hours: 6,
            ca_bundle_hash: "abc123def".to_string(),
            shutdown_timeout_seconds: None,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"service_settings"#));
        assert!(!json.contains("shutdown_timeout_seconds"));
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn service_settings_backward_compat_extra_fields() {
        // Future-proof: extra fields in JSON should be ignored
        let json = r#"{"type":"service_settings","renewal_window_hours":12,"ca_bundle_hash":"def456","shutdown_timeout_seconds":60,"some_future_field":"value"}"#;
        let msg: ControllerMessage = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            ControllerMessage::ServiceSettings(ServiceSettingsPayload {
                renewal_window_hours: 12,
                ca_bundle_hash: "def456".to_string(),
                shutdown_timeout_seconds: Some(60),
            })
        );
    }

    #[test]
    fn service_settings_backward_compat_missing_ca_hash() {
        // Services running older protocol without ca_bundle_hash should still parse
        let json = r#"{"type":"service_settings","renewal_window_hours":6}"#;
        let msg: ControllerMessage = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            ControllerMessage::ServiceSettings(ServiceSettingsPayload {
                renewal_window_hours: 6,
                ca_bundle_hash: String::new(),
                shutdown_timeout_seconds: None,
            })
        );
    }

    #[test]
    fn service_settings_backward_compat_missing_shutdown_timeout() {
        // Services running older protocol without shutdown_timeout_seconds should still parse
        let json = r#"{"type":"service_settings","renewal_window_hours":6,"ca_bundle_hash":"abc"}"#;
        let msg: ControllerMessage = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            ControllerMessage::ServiceSettings(ServiceSettingsPayload {
                renewal_window_hours: 6,
                ca_bundle_hash: "abc".to_string(),
                shutdown_timeout_seconds: None,
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

    #[test]
    fn check_versions_serialization_roundtrip() {
        let msg = ControllerMessage::CheckVersions(CheckVersionsPayload {
            assignments: vec![VersionCheckAssignment {
                software_item_id: TEST_UUID_1,
                name: "Test Software".to_string(),
                provider_type: ProviderType::GithubReleases,
                package_identifier: "owner/repo".to_string(),
                config: serde_json::json!({"owner": "octocat", "repo": "hello-world"}),
            }],
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"check_versions"#));
        assert!(json.contains(r#""software_item_id":"550e8400-e29b-41d4-a716-446655440000"#));
        assert!(json.contains(r#""provider_type":"github_releases"#));
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn execute_update_serialization_roundtrip() {
        let msg = ControllerMessage::ExecuteUpdate(Box::new(ExecuteUpdatePayload {
            update_history_id: Uuid::parse_str("01936a1e-7e8c-7f00-8000-000000000001").unwrap(),
            software_item_id: Uuid::parse_str("01936a1e-7e8c-7f00-8000-000000000002").unwrap(),
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
                assets: vec![ReleaseAsset {
                    name: "node-v20.10.0-linux-x64.tar.gz".to_string(),
                    download_url: "https://github.com/nodejs/node/releases/download/v20.10.0/node-v20.10.0-linux-x64.tar.gz".to_string(),
                    size: Some(25_000_000),
                    content_type: None,
                }],
            }),
            timeout_seconds: 600,
            shell: Some(HookShell::Bash),
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
            update_history_id: TEST_UUID_1,
            software_item_id: TEST_UUID_2,
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
            "update_history_id": "550e8400-e29b-41d4-a716-446655440000",
            "software_item_id": "550e8400-e29b-41d4-a716-446655440001",
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
            "update_history_id": "550e8400-e29b-41d4-a716-446655440000",
            "software_item_id": "550e8400-e29b-41d4-a716-446655440001",
            "software_item_name": "Test",
            "package_identifier": "test",
            "to_version": "1.0.0",
            "provider_type": "github_releases",
            "provider_config": {},
            "shell": "sh"
        }"#;
        let msg: ControllerMessage = serde_json::from_str(json).unwrap();
        if let ControllerMessage::ExecuteUpdate(payload) = msg {
            assert_eq!(payload.shell, Some(HookShell::Sh));
        } else {
            panic!("Expected ExecuteUpdate");
        }
    }

    #[test]
    fn execute_update_backward_compat_extra_fields() {
        let json = r#"{
            "type": "execute_update",
            "update_history_id": "550e8400-e29b-41d4-a716-446655440000",
            "software_item_id": "550e8400-e29b-41d4-a716-446655440001",
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
    fn registered_serialization_roundtrip() {
        let msg = ControllerMessage::Registered(MqttRegisteredPayload {
            instance_id: "mqtt-node1-01936a1e".to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"registered","instance_id":"mqtt-node1-01936a1e"}"#
        );
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn tenant_assignments_serialization_roundtrip() {
        let msg = ControllerMessage::TenantAssignments(MqttTenantAssignmentsPayload {
            tenants: vec![MqttTenantConfig {
                mqtt_client_id: TEST_UUID_3,
                tenant_id: TEST_UUID_1,
                enabled: true,
                transport: MqttTransport::Tls,
                host: "broker.example.com".to_string(),
                port: 8883,
                client_id: "uptrakit".to_string(),
                username: Some("user".to_string()),
                password: Some("pass".to_string()),
                topic_prefix: "home/uptrakit".to_string(),
                updated_at: UtcDateTime::from_unix_timestamp(1706400000).unwrap(),
            }],
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"tenant_assignments"#));
        assert!(json.contains(r#""transport":"tls"#));
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn tenant_config_updated_serialization_roundtrip() {
        let msg = ControllerMessage::TenantConfigUpdated(MqttTenantConfigUpdatedPayload {
            tenant: MqttTenantConfig {
                mqtt_client_id: TEST_UUID_1,
                tenant_id: TEST_UUID_2,
                enabled: true,
                transport: MqttTransport::Tcp,
                host: "broker.local".to_string(),
                port: 1883,
                client_id: "uptrakit".to_string(),
                username: None,
                password: None,
                topic_prefix: "uptrakit".to_string(),
                updated_at: UtcDateTime::from_unix_timestamp(1706400000).unwrap(),
            },
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"tenant_config_updated"#));
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn tenant_revoked_serialization_roundtrip() {
        let msg = ControllerMessage::TenantRevoked(MqttTenantRevokedPayload {
            mqtt_client_id: TEST_UUID_1,
            reason: "mqtt client disabled".to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"tenant_revoked"#));
        assert!(json.contains(r#""reason":"mqtt client disabled"#));
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    // =========================================================================
    // Shared payload and helper tests
    // =========================================================================

    #[test]
    fn now_millis_returns_reasonable_value() {
        let ts = now_millis();
        // Should be after 2024-01-01 (1704067200000)
        assert!(ts > 1704067200000);
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
    fn enrollment_status_all_variants() {
        for (status, expected) in [
            (EnrollmentStatus::Pending, "pending"),
            (EnrollmentStatus::Approved, "approved"),
        ] {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, format!(r#""{expected}""#));
            let deserialized: EnrollmentStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, status);
        }
    }

    #[test]
    fn service_type_all_variants() {
        for (svc_type, expected) in [(ServiceType::Agent, "agent"), (ServiceType::Mqtt, "mqtt")] {
            let json = serde_json::to_string(&svc_type).unwrap();
            assert_eq!(json, format!(r#""{expected}""#));
            let deserialized: ServiceType = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, svc_type);
        }
    }

    #[test]
    fn hook_shell_all_variants() {
        for (shell, expected) in [
            (HookShell::Bash, "bash"),
            (HookShell::Sh, "sh"),
            (HookShell::PowerShell, "powershell"),
        ] {
            let json = serde_json::to_string(&shell).unwrap();
            assert_eq!(json, format!(r#""{expected}""#));
            let deserialized: HookShell = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, shell);
        }
    }

    #[test]
    fn hook_shell_default_is_bash() {
        assert_eq!(HookShell::default(), HookShell::Bash);
    }

    #[test]
    fn enrollment_status_rejects_invalid() {
        let result: std::result::Result<EnrollmentStatus, _> = serde_json::from_str(r#""invalid""#);
        assert!(result.is_err());
    }

    #[test]
    fn service_type_rejects_invalid() {
        let result: std::result::Result<ServiceType, _> = serde_json::from_str(r#""invalid""#);
        assert!(result.is_err());
    }

    #[test]
    fn hook_shell_rejects_invalid() {
        let result: std::result::Result<HookShell, _> = serde_json::from_str(r#""zsh""#);
        assert!(result.is_err());
    }

    #[test]
    fn mqtt_transport_serde_roundtrip() {
        for (variant, expected_str) in [(MqttTransport::Tcp, "tcp"), (MqttTransport::Tls, "tls")] {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, format!(r#""{expected_str}""#));
            let deserialized: MqttTransport = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn mqtt_transport_rejects_invalid() {
        let result: std::result::Result<MqttTransport, _> = serde_json::from_str(r#""udp""#);
        assert!(result.is_err());
    }

    #[test]
    fn mqtt_transport_default_is_tcp() {
        assert_eq!(MqttTransport::default(), MqttTransport::Tcp);
    }

    #[test]
    fn mqtt_transport_display() {
        assert_eq!(MqttTransport::Tcp.to_string(), "tcp");
        assert_eq!(MqttTransport::Tls.to_string(), "tls");
    }

    #[test]
    fn error_code_serde_roundtrip() {
        for (variant, expected_str) in [
            (ErrorCode::BadRequest, "bad_request"),
            (ErrorCode::EnrollmentFailed, "enrollment_failed"),
            (ErrorCode::NotApproved, "not_approved"),
            (ErrorCode::Forbidden, "forbidden"),
            (ErrorCode::CertificateError, "certificate_error"),
            (ErrorCode::InternalError, "internal_error"),
            (ErrorCode::AgentVersionTooOld, "agent_version_too_old"),
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, format!(r#""{expected_str}""#));
            let deserialized: ErrorCode = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn error_code_rejects_invalid() {
        let result: std::result::Result<ErrorCode, _> = serde_json::from_str(r#""unknown_code""#);
        assert!(result.is_err());
    }

    #[test]
    fn error_code_display() {
        assert_eq!(ErrorCode::BadRequest.to_string(), "bad_request");
        assert_eq!(ErrorCode::EnrollmentFailed.to_string(), "enrollment_failed");
        assert_eq!(ErrorCode::NotApproved.to_string(), "not_approved");
        assert_eq!(ErrorCode::Forbidden.to_string(), "forbidden");
        assert_eq!(ErrorCode::CertificateError.to_string(), "certificate_error");
        assert_eq!(ErrorCode::InternalError.to_string(), "internal_error");
        assert_eq!(
            ErrorCode::AgentVersionTooOld.to_string(),
            "agent_version_too_old"
        );
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

    #[test]
    fn version_check_assignment_serialization() {
        let assignment = VersionCheckAssignment {
            software_item_id: TEST_UUID_1,
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
                ReleaseAsset {
                    name: "app.tar.gz".to_string(),
                    download_url: "https://example.com/app.tar.gz".to_string(),
                    size: Some(1024),
                    content_type: None,
                },
                ReleaseAsset {
                    name: "app.deb".to_string(),
                    download_url: "https://example.com/app.deb".to_string(),
                    size: None,
                    content_type: None,
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
    fn mqtt_tenant_config_omits_none_fields() {
        let config = MqttTenantConfig {
            mqtt_client_id: TEST_UUID_1,
            tenant_id: TEST_UUID_2,
            enabled: true,
            transport: MqttTransport::Tcp,
            host: "localhost".to_string(),
            port: 1883,
            client_id: "uptrakit".to_string(),
            username: None,
            password: None,
            topic_prefix: "uptrakit".to_string(),
            updated_at: UtcDateTime::from_unix_timestamp(1706400000).unwrap(),
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("username"));
        assert!(!json.contains("password"));
        let deserialized: MqttTenantConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, config);
    }

    #[test]
    fn enroll_requires_service_type() {
        // Enrollment without service_type should fail deserialization.
        let json = r#"{"type":"enroll","hostname":"node-old","friendly_name":"Old Node"}"#;
        let result: std::result::Result<ServiceMessage, _> = serde_json::from_str(json);
        assert!(result.is_err(), "EnrollPayload requires service_type");
    }

    #[test]
    fn enroll_host_info_is_optional() {
        // MQTT services do not send host_info.
        let json =
            r#"{"type":"enroll","hostname":"mqtt-1","friendly_name":"MQTT","service_type":"mqtt"}"#;
        let msg: ServiceMessage = serde_json::from_str(json).unwrap();
        if let ServiceMessage::Enroll(payload) = msg {
            assert!(payload.host_info.is_none());
            assert_eq!(payload.service_type, ServiceType::Mqtt);
        } else {
            panic!("Expected Enroll");
        }
    }
}
