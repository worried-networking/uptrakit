pub mod close_reason;
pub use close_reason::CloseReason;

pub mod service_profile;
pub use service_profile::{ServiceProfile, parse_capabilities, serialize_capabilities};

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use time::UtcDateTime;
use uuid::Uuid;

// Re-export shared types used directly in wire protocol messages.
pub use uptrakit_shared_types::{
    DiscoveredSoftware, DiscoveryTarget, HookShell, MqttClientConnectionStatus, MqttTransport,
    OutputStreamType, PluginRole, PluginType, ReleaseAsset, ReleaseInfo,
};
// Re-export `SecretString` for callers that need it for secret fields.
pub use uptrakit_shared_types::SecretString;

/// A protocol capability advertised by a service or controller during connection setup.
///
/// Both sides announce their capability sets at the start of each authenticated
/// connection. Each side independently computes the agreed set as the intersection
/// of typed variants only — [`Other`](Self::Other) is excluded from intersection.
///
/// ## Wire format
///
/// Capabilities are serialized as plain strings (snake_case). Unknown strings from
/// a newer peer become `Other(String)` for forward compatibility.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub enum Capability {
    /// Service participates in the graceful-shutdown protocol: sends
    /// `Disconnecting` before clean exit and honours
    /// `shutdown_timeout_seconds` from `ServiceSettings`.
    ///
    /// Wire string: `graceful_shutdown`.
    GracefulShutdown,
    /// Service is an MQTT bridge: handles `Register`, `TenantAssignments`,
    /// `ReleaseTenants`, `MqttClientStatus`, etc.
    ///
    /// Identifies an MQTT bridge service. The controller uses this capability
    /// to gate MQTT-specific message handling and lease coordination.
    ///
    /// Wire string: `mqtt_bridge`.
    MqttBridge,
    /// Service supports `DiscoverSoftware` → `DiscoveryResults` flow.
    ///
    /// The controller gates autodiscovery requests on this capability.
    ///
    /// Wire string: `software_discovery`.
    SoftwareDiscovery,
    /// Service manages remote hosts over SSH, rather than running locally.
    ///
    /// Identifies an SSH-backed agent. Combined with `SoftwareDiscovery`,
    /// uniquely identifies an SSH agent (vs. a local agent).
    ///
    /// Wire string: `ssh_remote`.
    SshRemote,
    /// Service supports pre-/post-update hook commands (`HookCommand` in
    /// `ExecuteUpdatePayload`). The controller omits hooks when absent.
    ///
    /// Wire string: `update_hooks`.
    UpdateHooks,
    /// Marker: service is an external task scheduler.
    ///
    /// Identifies a service that runs scheduled tasks (version checks, cert
    /// checks, auth cleanup, etc.) externally. The controller uses this to
    /// detect scheduler presence and disable the embedded scheduler.
    ///
    /// Wire string: `scheduler`.
    Scheduler,
    /// Service requires direct database access. The controller will include
    /// `db_url` in [`ServiceCredentialsPayload`].
    ///
    /// Wire string: `database_access`.
    DatabaseAccess,
    /// Service requires NATS access. The controller will include `nats_url`
    /// in [`ServiceCredentialsPayload`] (if NATS is configured).
    ///
    /// Wire string: `nats_access`.
    NatsAccess,
    /// Service requires the master encryption key. The controller will include
    /// `master_key_hex` in [`ServiceCredentialsPayload`] (if encryption is enabled).
    ///
    /// Wire string: `master_key_access`.
    MasterKeyAccess,
    /// Service can request CA certificate rotation via [`RequestCaRotationPayload`].
    /// The controller will accept `RequestCaRotation` messages from services
    /// with this capability (via NATS or local delivery).
    ///
    /// Wire string: `ca_management`.
    CaManagement,
    /// Unknown capability from a newer peer; never participates in intersection.
    ///
    /// Provides forward compatibility: a newer peer may advertise capabilities
    /// that an older build does not yet recognise. These are preserved on receipt
    /// but never emitted by the current codebase.
    Other(String),
}

impl Capability {
    /// Returns the snake_case wire string for this capability.
    pub fn as_str(&self) -> &str {
        match self {
            Self::SoftwareDiscovery => "software_discovery",
            Self::UpdateHooks => "update_hooks",
            Self::GracefulShutdown => "graceful_shutdown",
            Self::MqttBridge => "mqtt_bridge",
            Self::SshRemote => "ssh_remote",
            Self::Scheduler => "scheduler",
            Self::DatabaseAccess => "database_access",
            Self::NatsAccess => "nats_access",
            Self::MasterKeyAccess => "master_key_access",
            Self::CaManagement => "ca_management",
            Self::Other(s) => s.as_str(),
        }
    }

    /// Returns `true` for typed variants; `Other` returns `false`.
    ///
    /// Only typed variants participate in capability intersection. `Other` values
    /// are forwarded-compatibility markers and must not gate behaviour.
    pub fn is_known(&self) -> bool {
        !matches!(self, Self::Other(_))
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when parsing a capability string fails.
///
/// This error is never actually returned because [`Capability::Other`]
/// catches all unrecognized strings, but the type satisfies the
/// [`FromStr`] trait contract.
#[derive(Debug)]
pub struct ParseCapabilityError(std::convert::Infallible);

impl fmt::Display for ParseCapabilityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid capability")
    }
}

impl std::error::Error for ParseCapabilityError {}

impl FromStr for Capability {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "software_discovery" => Self::SoftwareDiscovery,
            "update_hooks" => Self::UpdateHooks,
            "graceful_shutdown" => Self::GracefulShutdown,
            "mqtt_bridge" => Self::MqttBridge,
            "ssh_remote" => Self::SshRemote,
            "scheduler" => Self::Scheduler,
            "database_access" => Self::DatabaseAccess,
            "nats_access" => Self::NatsAccess,
            "master_key_access" => Self::MasterKeyAccess,
            "ca_management" => Self::CaManagement,
            other => Self::Other(other.to_string()),
        })
    }
}

impl Serialize for Capability {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Capability {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(s.parse().unwrap_or(Capability::Other(s)))
    }
}

/// Enrollment status returned in the `Enrolled` message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, strum::Display)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum EnrollmentStatus {
    Pending,
    Approved,
}

/// A single hook command to execute on the agent.
///
/// Predefined hooks use the `Exec` variant which avoids shell interpretation.
/// Custom commands use the `Shell` variant which runs through a shell.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookCommand {
    /// Execute a command string through a shell interpreter.
    Shell {
        command: String,
        #[serde(default)]
        shell: HookShell,
    },
    /// Execute a program directly with arguments (no shell interpretation).
    Exec {
        program: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        working_dir: Option<String>,
    },
}

/// Human-readable formatting for logging only. Not intended for round-trip
/// serialization — use serde for machine-readable encoding.
impl fmt::Display for HookCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Shell { command, .. } => write!(f, "{command}"),
            Self::Exec {
                program,
                args,
                working_dir,
            } => {
                if let Some(dir) = working_dir {
                    write!(f, "(in {dir}) ")?;
                }
                write!(f, "{program}")?;
                for arg in args {
                    write!(f, " {arg}")?;
                }
                Ok(())
            }
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
#[non_exhaustive]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServiceMessage {
    // -- Shared enrollment + lifecycle --
    Ping(PingPayload),
    Enroll(EnrollPayload),
    RequestCertificate(RequestCertificatePayload),
    RenewCertificate(RenewCertificatePayload),
    Disconnecting(DisconnectingPayload),
    // -- Agent-specific --
    ReportHosts(ReportHostsPayload),
    VersionCheckResults(VersionCheckResultsPayload),
    UpdateStarted(UpdateStartedPayload),
    UpdateOutput(UpdateOutputPayload),
    UpdateResult(UpdateResultPayload),
    DiscoveryResults(DiscoveryResultsPayload),
    // -- MQTT-specific --
    Register(MqttRegisterPayload),
    ReleaseTenants(MqttReleaseTenantsPayload),
    MqttClientStatus(MqttClientStatusPayload),
    MqttTriggerUpdate(MqttUpdateTriggerPayload),
}

/// Messages sent from the controller to a service (agent or MQTT).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
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
    DiscoverSoftware(DiscoverSoftwarePayload),
    // -- MQTT-specific --
    Registered(MqttRegisteredPayload),
    TenantAssignments(MqttTenantAssignmentsPayload),
    TenantConfigUpdated(MqttTenantConfigUpdatedPayload),
    TenantRevoked(MqttTenantRevokedPayload),
    MqttClientCreated(MqttClientCreatedPayload),
    SoftwareStates(MqttSoftwareStatesPayload),
    // -- Infrastructure credential delivery --
    /// Infrastructure credentials for services that advertise credential
    /// capabilities. Fields are populated based on the service's capability set:
    ///   - `database_access` → `db_url` is set
    ///   - `nats_access` → `nats_url` is set (if controller has NATS)
    ///   - `master_key_access` → `master_key_hex` is set (if encryption enabled)
    ///
    /// **Security**: NEVER published to NATS. Delivered locally via WebSocket only,
    /// following the same pattern as MQTT credential messages.
    ServiceCredentials(ServiceCredentialsPayload),
    /// Request from an external component (e.g. scheduler) for the controller to
    /// perform CA certificate rotation. Published via NATS to the controller subject;
    /// handled by triggering `ca_rotation_trigger.notify_one()`.
    RequestCaRotation(RequestCaRotationPayload),
}

impl ControllerMessage {
    /// Returns `true` if this message may be published to NATS JetStream.
    ///
    /// Credential-bearing variants (`ServiceCredentials`, `TenantAssignments`,
    /// `TenantConfigUpdated`, `TenantRevoked`) must **never** be published to
    /// NATS — they are delivered exclusively over authenticated WebSocket
    /// connections.  All other variants are safe to broadcast via NATS.
    ///
    /// This is the authoritative gate used by [`NatsConnection::publish`].
    pub fn is_nats_publishable(&self) -> bool {
        !matches!(
            self,
            ControllerMessage::ServiceCredentials(_)
                | ControllerMessage::TenantAssignments(_)
                | ControllerMessage::TenantConfigUpdated(_)
                | ControllerMessage::TenantRevoked(_)
        )
    }
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
    /// Hostname reported by the agent/host machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    /// Network address of the host (SSH target address for SSH agent hosts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip_address: Option<String>,
}

/// Payload for service enrollment request.
///
/// Used by both agents and MQTT services. Host information is reported
/// separately via [`ReportHostsPayload`] after authentication.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollPayload {
    pub hostname: String,
    pub friendly_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enrollment_token: Option<SecretString>,
    /// Capabilities this service supports.
    ///
    /// The controller persists these in the `services.capabilities` column and
    /// derives behavioral defaults from the resulting [`ServiceProfile`].
    pub capabilities: BTreeSet<Capability>,
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
///
/// Supports multiple hosts per message, enabling a single service instance
/// (e.g. a future SSH-backed agent) to manage several remote hosts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportHostsPayload {
    /// One or more host machines managed by this service.
    pub hosts: Vec<HostInfo>,
    /// Agent binary version (e.g., "0.0.1").
    pub agent_version: String,
    /// Capabilities advertised by this service.
    ///
    /// The controller computes the agreed set as the intersection of this set
    /// with its own capabilities, considering only typed (known) variants.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub capabilities: BTreeSet<Capability>,
}

/// Payload for enrollment confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrolledPayload {
    pub service_id: Uuid,
    pub enrollment_secret: SecretString,
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
        let millis_i64 = i64::try_from(millis).map_err(serde::ser::Error::custom)?;
        serializer.serialize_i64(millis_i64)
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, strum::Display)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
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
    /// Message sequence number mismatch (replay protection).
    SequenceError,
}

/// Payload for error responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub code: ErrorCode,
    pub message: String,
}

/// Serde helper: serialize/deserialize `std::time::Duration` as whole seconds (`u32`).
///
/// Uses `u32` consistently across wire, HTTP API, and CLI representations.
/// Maximum representable interval: ~136 years — more than sufficient for ping intervals.
pub mod duration_seconds {
    use serde::{self, Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        let secs = u32::try_from(d.as_secs()).map_err(serde::ser::Error::custom)?;
        serializer.serialize_u32(secs)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        let secs = u32::deserialize(deserializer)?;
        Ok(Duration::from_secs(u64::from(secs)))
    }
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
    /// Capabilities advertised by the controller.
    ///
    /// The service computes the agreed set as the intersection of this set
    /// with its own capabilities, considering only typed (known) variants.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub capabilities: BTreeSet<Capability>,
    /// Maximum time in seconds to wait for in-flight operations during shutdown.
    /// Present for agents, absent for MQTT services.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shutdown_timeout_seconds: Option<u32>,
    /// How often the service should send ping messages.
    /// Controller-managed; derived from per-service DB override or service-type default.
    #[serde(with = "duration_seconds")]
    pub ping_interval: std::time::Duration,
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
    /// The machine_id of the host to check versions on.
    ///
    /// For the regular agent (one service = one host), the agent validates that
    /// this matches its own machine_id as a defensive sanity check.
    /// For the SSH agent (one service = N remote hosts), the agent uses this
    /// field to look up the correct SSH credentials and route the operation to
    /// the right remote host.
    pub host_machine_id: String,
    /// List of software items to check.
    pub assignments: Vec<VersionCheckAssignment>,
}

/// A plugin assignment for a specific role in a version check or update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginAssignment {
    /// The plugin type (e.g. github_releases, apt, homebrew).
    pub plugin_type: PluginType,
    /// Package identifier for this role's plugin.
    pub package_identifier: String,
    /// Merged plugin config (base + override).
    pub config: serde_json::Value,
}

/// A single software item to check for installed version and/or latest version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionCheckAssignment {
    /// Software item ID.
    pub software_item_id: Uuid,
    /// Human-readable name for logging.
    pub name: String,
    /// Plugin for the detect_version role.
    /// None if no detect_version plugin is configured for this host-software pair.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detect_version: Option<PluginAssignment>,
    /// Plugin for the fetch_releases role — only included for agent-side plugins
    /// (i.e., plugins without ControllerSideFetchReleases or with execution_site = agent).
    /// Controller-side fetch_releases is handled by the scheduler, not sent to the agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch_releases: Option<PluginAssignment>,
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
    /// Latest available version from the package index, if resolved locally
    /// by the agent (e.g., Homebrew). Absent for plugins whose latest
    /// version is resolved on the controller side.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    /// Error message if detection failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// --- Update execution messages ---

/// Final status of an update execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum UpdateFinalStatus {
    Completed,
    Failed,
}

/// Default timeout for update execution, in seconds (2 hours).
pub const DEFAULT_UPDATE_TIMEOUT_SECS: u32 = 7200;

/// Default timeout for update execution in seconds.
fn default_update_timeout() -> u32 {
    DEFAULT_UPDATE_TIMEOUT_SECS
}

/// Controller -> Agent: Trigger an update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecuteUpdatePayload {
    /// The machine_id of the host to run the update on.
    ///
    /// For the regular agent (one service = one host), the agent validates that
    /// this matches its own machine_id as a defensive sanity check.
    /// For the SSH agent (one service = N remote hosts), the agent uses this
    /// field to look up the correct SSH credentials and route the operation to
    /// the right remote host.
    pub host_machine_id: String,
    pub update_history_id: Uuid,
    pub software_item_id: Uuid,
    pub software_item_name: String,
    pub to_version: String,
    /// Plugin for the detect_version role (for before/after installed-version detection).
    /// Absent when no detect_version plugin is configured for this assignment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detect_version_plugin: Option<PluginAssignment>,
    /// Plugin for the execute_update role.
    pub execute_update_plugin: PluginAssignment,
    /// Pre-update hook commands to execute before the update.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_update_hooks: Vec<HookCommand>,
    /// Post-update hook commands to execute after the update.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_update_hooks: Vec<HookCommand>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_info: Option<ReleaseInfo>,
    #[serde(default = "default_update_timeout")]
    pub timeout_seconds: u32,
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
#[non_exhaustive]
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

impl DisconnectingPayload {
    /// Create a `DisconnectingPayload` for non-MQTT services (agents).
    pub fn new(reason: DisconnectReason) -> Self {
        Self {
            reason,
            active_mqtt_clients: Vec::new(),
        }
    }
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
    /// Capabilities advertised by this MQTT service.
    ///
    /// The controller computes the agreed set as the intersection of this set
    /// with its own capabilities, considering only typed (known) variants.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub capabilities: BTreeSet<Capability>,
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

/// Payload for MQTT client connection status updates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttClientStatusPayload {
    /// MQTT client UUID (primary identifier from mqtt_clients table).
    pub mqtt_client_id: Uuid,
    /// Current connection status.
    pub status: MqttClientConnectionStatus,
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
    pub username: Option<SecretString>,
    /// Password (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<SecretString>,
    /// Custom CA certificate in PEM format (optional, for private brokers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_pem: Option<SecretString>,
    /// Topic prefix.
    pub topic_prefix: String,
    /// Whether to publish Home Assistant MQTT discovery topics.
    #[serde(default)]
    pub ha_discovery: bool,
    /// Prefix for Home Assistant MQTT discovery topics.
    #[serde(default = "default_ha_discovery_prefix")]
    pub ha_discovery_prefix: String,
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

/// Payload for controller outbox events when a new MQTT client is created.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttClientCreatedPayload {
    /// MQTT client UUID to lease.
    pub mqtt_client_id: Uuid,
}

// =============================================================================
// Infrastructure Credential Payloads
// =============================================================================

/// Infrastructure credentials for services that advertise credential capabilities.
///
/// Fields are populated based on the service's capability set:
///   - `database_access` → `db_url` is set
///   - `nats_access` → `nats_url` is set (if controller has NATS)
///   - `master_key_access` → `master_key_hex` is set (if encryption enabled)
///
/// **Security**: This payload contains highly sensitive credentials. It must
/// NEVER be published to NATS or any external transport. It is delivered
/// exclusively over the authenticated WebSocket connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceCredentialsPayload {
    /// Database connection URL. Present when the service has `database_access`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub db_url: Option<SecretString>,
    /// Master encryption key as 64-char hex. Present when the service has
    /// `master_key_access` and encryption is enabled on the controller.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub master_key_hex: Option<SecretString>,
    /// NATS server URL. Present when the service has `nats_access` and
    /// NATS is configured on the controller.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nats_url: Option<String>,
}

/// Request from an external component (e.g. scheduler) for the controller to
/// perform CA certificate rotation.
///
/// Published via NATS to `uptrakit.events.controller` subject. Handled by
/// triggering `ca_rotation_trigger.notify_one()` on the receiving controller.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestCaRotationPayload {
    /// Human-readable reason for the rotation request.
    pub reason: String,
}

fn default_ha_discovery_prefix() -> String {
    "homeassistant".to_string()
}

/// Controller -> MQTT service: current software version state for a tenant.
///
/// Sent after tenant assignment and after any version check or update result.
/// Safe to write to the outbox (contains no credentials).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttSoftwareStatesPayload {
    /// Tenant this state belongs to.
    pub tenant_id: Uuid,
    /// All active software items for the tenant with per-host version data.
    pub items: Vec<MqttSoftwareStateItem>,
}

/// A single software item entry in [`MqttSoftwareStatesPayload`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttSoftwareStateItem {
    /// Software item UUID.
    pub software_item_id: Uuid,
    /// Human-readable software item name.
    pub name: String,
    /// Per-host version data for this software item.
    pub hosts: Vec<MqttSoftwareStateHostEntry>,
}

/// Per-host version data for a software item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttSoftwareStateHostEntry {
    /// Host UUID.
    pub host_id: Uuid,
    /// Human-readable hostname.
    pub hostname: String,
    /// Currently installed version, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
    /// Latest available version, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    /// Whether an update is available (`latest_version > installed_version`).
    pub update_available: bool,
}

/// MQTT service -> Controller: request to trigger a software update.
///
/// Sent when a Home Assistant user presses "Install" on an update entity.
/// The controller validates and dispatches the update to the appropriate agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MqttUpdateTriggerPayload {
    /// Tenant UUID (for validation).
    pub tenant_id: Uuid,
    /// Software item to update.
    pub software_item_id: Uuid,
    /// Host to update on.
    pub host_id: Uuid,
    /// Target version to install.
    pub to_version: String,
    /// MQTT client UUID that initiated the trigger (used as actor_id).
    pub mqtt_client_id: Uuid,
}

// =============================================================================
// Software Autodiscovery Payloads
// =============================================================================

/// Controller -> Agent: Run software discovery on the given host.
///
/// The `plugins` list contains one entry per plugin that should be used.
/// When `plugin_config_id` is `None`, the assignment uses a default (empty)
/// config — the controller will auto-create a `PluginConfig` record once
/// results arrive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoverSoftwarePayload {
    /// Machine ID of the host to discover software on.
    ///
    /// For the regular agent this is validated to match its own machine_id.
    /// For the SSH agent it identifies which remote host to connect to.
    pub host_machine_id: String,
    /// Per-plugin discovery assignments.
    pub plugins: Vec<DiscoveryPluginAssignment>,
}

/// A single plugin assignment inside a [`DiscoverSoftwarePayload`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryPluginAssignment {
    /// Pre-existing plugin config ID, or `None` for a default/auto run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_config_id: Option<Uuid>,
    /// Plugin type to use for discovery.
    pub plugin_type: PluginType,
    /// Plugin-specific configuration (`{}` for default assignments).
    pub config: serde_json::Value,
}

/// Agent -> Controller: Results of a software discovery run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryResultsPayload {
    /// Machine ID of the host that was scanned (echoed from the assignment).
    pub host_machine_id: String,
    /// Per-plugin results.
    pub results: Vec<DiscoveryPluginResult>,
}

/// Result for a single plugin inside a [`DiscoveryResultsPayload`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryPluginResult {
    /// Echoed from [`DiscoveryPluginAssignment`] so the controller can route
    /// results to the correct `PluginConfig` row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_config_id: Option<Uuid>,
    /// Plugin type that produced these results.
    pub plugin_type: PluginType,
    /// Discovered software items (empty on error).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub discoveries: Vec<DiscoveredSoftware>,
    /// Plugin-level error message, if discovery failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// =============================================================================
// Envelope types for application-level replay protection
// =============================================================================

/// The current wire protocol version stamped on every envelope.
///
/// Increment this constant whenever a breaking change is introduced to the
/// wire protocol (e.g. a required field is added, a variant renamed, or
/// capability-negotiation semantics change). Peers that receive a
/// `protocol_version` value they do not recognise must close the connection
/// with [`CloseReason::ProtocolError`].
pub const CURRENT_PROTOCOL_VERSION: u32 = 1;

/// Envelope wrapping a [`ServiceMessage`] with a monotonically increasing
/// sequence number for replay protection and the current protocol version.
///
/// JSON on the wire: `{"protocol_version":1,"seq":1,"type":"ping","service_ts":123}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceEnvelope {
    pub protocol_version: u32,
    pub seq: u64,
    #[serde(flatten)]
    pub message: ServiceMessage,
}

/// Envelope wrapping a [`ControllerMessage`] with a monotonically increasing
/// sequence number for replay protection and the current protocol version.
///
/// JSON on the wire: `{"protocol_version":1,"seq":1,"type":"pong","service_ts":123,"controller_ts":456}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControllerEnvelope {
    pub protocol_version: u32,
    pub seq: u64,
    #[serde(flatten)]
    pub message: ControllerMessage,
}

/// Tracks outgoing sequence numbers for a single direction of a WebSocket
/// connection. Assigns monotonically increasing numbers starting at 1.
#[derive(Debug)]
pub struct OutgoingSeq {
    next: u64,
}

impl OutgoingSeq {
    /// Create a new outgoing sequence counter (first message gets seq 1).
    pub fn new() -> Self {
        Self { next: 1 }
    }

    /// Wrap a [`ServiceMessage`] in a [`ServiceEnvelope`], assigning the next
    /// sequence number and stamping [`CURRENT_PROTOCOL_VERSION`].
    pub fn wrap_service(&mut self, message: ServiceMessage) -> ServiceEnvelope {
        let seq = self.next;
        self.next += 1;
        ServiceEnvelope {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            seq,
            message,
        }
    }

    /// Wrap a [`ControllerMessage`] in a [`ControllerEnvelope`], assigning the
    /// next sequence number and stamping [`CURRENT_PROTOCOL_VERSION`].
    pub fn wrap_controller(&mut self, message: ControllerMessage) -> ControllerEnvelope {
        let seq = self.next;
        self.next += 1;
        ControllerEnvelope {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            seq,
            message,
        }
    }
}

impl Default for OutgoingSeq {
    fn default() -> Self {
        Self::new()
    }
}

/// Validates incoming sequence numbers for a single direction of a WebSocket
/// connection. Expects messages to arrive as 1, 2, 3, ...
#[derive(Debug)]
pub struct IncomingSeq {
    expected: u64,
}

impl IncomingSeq {
    /// Create a new incoming sequence validator (first expected seq is 1).
    pub fn new() -> Self {
        Self { expected: 1 }
    }

    /// Validate that the received sequence number matches the expected value.
    ///
    /// On success, advances the expected counter. On failure, returns a
    /// [`SeqError`] describing the mismatch.
    pub fn validate(&mut self, received: u64) -> Result<(), SeqError> {
        if received != self.expected {
            return Err(SeqError {
                expected: self.expected,
                received,
            });
        }
        self.expected += 1;
        Ok(())
    }
}

impl Default for IncomingSeq {
    fn default() -> Self {
        Self::new()
    }
}

/// Error returned when a received sequence number does not match the expected
/// value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("sequence error: expected {expected}, received {received}")]
pub struct SeqError {
    pub expected: u64,
    pub received: u64,
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

    fn agent_capabilities() -> BTreeSet<Capability> {
        [
            Capability::GracefulShutdown,
            Capability::SoftwareDiscovery,
            Capability::UpdateHooks,
        ]
        .into_iter()
        .collect()
    }

    fn mqtt_capabilities() -> BTreeSet<Capability> {
        [Capability::GracefulShutdown, Capability::MqttBridge]
            .into_iter()
            .collect()
    }

    fn ssh_agent_capabilities() -> BTreeSet<Capability> {
        [
            Capability::GracefulShutdown,
            Capability::SoftwareDiscovery,
            Capability::SshRemote,
            Capability::UpdateHooks,
        ]
        .into_iter()
        .collect()
    }

    #[test]
    fn enroll_agent_serialization_roundtrip() {
        let msg = ServiceMessage::Enroll(EnrollPayload {
            hostname: "node-1".to_string(),
            friendly_name: "Node One".to_string(),
            enrollment_token: Some(SecretString::new("tok-123".into())),
            capabilities: agent_capabilities(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
        assert!(json.contains(r#""capabilities""#));
        assert!(json.contains(r#""software_discovery""#));
    }

    #[test]
    fn enroll_mqtt_serialization_roundtrip() {
        let msg = ServiceMessage::Enroll(EnrollPayload {
            hostname: "mqtt-service-1".to_string(),
            friendly_name: "MQTT Service Node 1".to_string(),
            enrollment_token: Some(SecretString::new("tok-456".into())),
            capabilities: mqtt_capabilities(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"enroll"#));
        assert!(json.contains(r#""hostname":"mqtt-service-1"#));
        assert!(json.contains(r#""mqtt_bridge""#));
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn enroll_ssh_agent_serialization_roundtrip() {
        let msg = ServiceMessage::Enroll(EnrollPayload {
            hostname: "ssh-agent-1".to_string(),
            friendly_name: "SSH Agent Node 1".to_string(),
            enrollment_token: Some(SecretString::new("tok-789".into())),
            capabilities: ssh_agent_capabilities(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"enroll"#));
        assert!(json.contains(r#""hostname":"ssh-agent-1"#));
        assert!(json.contains(r#""ssh_remote""#));
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn enroll_without_token_serialization_roundtrip() {
        let msg = ServiceMessage::Enroll(EnrollPayload {
            hostname: "node-2".to_string(),
            friendly_name: "Node Two".to_string(),
            enrollment_token: None,
            capabilities: agent_capabilities(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        // enrollment_token should be omitted when None
        assert!(!json.contains("enrollment_token"));
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
    fn report_hosts_serialization_roundtrip() {
        let msg = ServiceMessage::ReportHosts(ReportHostsPayload {
            hosts: vec![HostInfo {
                machine_id: "machine-42".to_string(),
                os_type: Some("linux".to_string()),
                os_version: Some("Ubuntu 24.04 LTS".to_string()),
                architecture: Some("x86_64".to_string()),
                hostname: None,
                ip_address: None,
            }],
            agent_version: "0.0.1".to_string(),
            capabilities: BTreeSet::new(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"report_hosts"#));
        assert!(json.contains(r#""agent_version":"0.0.1"#));
        assert!(json.contains(r#""hosts":[{"machine_id":"machine-42""#));
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn report_hosts_multiple_hosts() {
        let msg = ServiceMessage::ReportHosts(ReportHostsPayload {
            hosts: vec![
                HostInfo {
                    machine_id: "host-a".to_string(),
                    os_type: Some("linux".to_string()),
                    os_version: None,
                    architecture: None,
                    hostname: None,
                    ip_address: None,
                },
                HostInfo {
                    machine_id: "host-b".to_string(),
                    os_type: Some("linux".to_string()),
                    os_version: Some("Debian 12".to_string()),
                    architecture: Some("aarch64".to_string()),
                    hostname: None,
                    ip_address: None,
                },
            ],
            agent_version: "0.0.1".to_string(),
            capabilities: BTreeSet::new(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""host-a"#));
        assert!(json.contains(r#""host-b"#));
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
                    latest_version: None,
                    error: None,
                },
                VersionCheckResult {
                    software_item_id: TEST_UUID_2,
                    installed_version: None,
                    latest_version: None,
                    error: Some("detection failed".to_string()),
                },
            ],
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"version_check_results"#));
        assert!(json.contains(r#""installed_version":"1.2.3"#));
        // installed_version should be omitted when None
        assert!(!json.contains(r#""installed_version":null"#));
        // latest_version should be omitted when None
        assert!(!json.contains(r#""latest_version"#));
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn version_check_result_with_latest_version() {
        let msg = ServiceMessage::VersionCheckResults(VersionCheckResultsPayload {
            results: vec![VersionCheckResult {
                software_item_id: TEST_UUID_1,
                installed_version: Some("1.24.4".to_string()),
                latest_version: Some("1.24.5".to_string()),
                error: None,
            }],
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""latest_version":"1.24.5"#));
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn version_check_result_backward_compat_no_latest_version() {
        // Messages from older agents that don't include latest_version
        // should still deserialize correctly.
        let json = serde_json::json!({
            "type": "version_check_results",
            "results": [{
                "software_item_id": TEST_UUID_1.to_string(),
                "installed_version": "1.0.0"
            }]
        });
        let msg: ServiceMessage = serde_json::from_value(json).unwrap();
        if let ServiceMessage::VersionCheckResults(payload) = msg {
            assert_eq!(
                payload.results[0].installed_version,
                Some("1.0.0".to_string())
            );
            assert_eq!(payload.results[0].latest_version, None);
        } else {
            panic!("expected VersionCheckResults");
        }
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
            capabilities: BTreeSet::new(),
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
            capabilities: BTreeSet::new(),
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

    #[test]
    fn mqtt_client_status_serialization_roundtrip() {
        let msg = ServiceMessage::MqttClientStatus(MqttClientStatusPayload {
            mqtt_client_id: TEST_UUID_1,
            status: MqttClientConnectionStatus::Connecting,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"mqtt_client_status"#));
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
            enrollment_secret: SecretString::new("secret-abc".into()),
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
            capabilities: BTreeSet::new(),
            shutdown_timeout_seconds: Some(120),
            ping_interval: std::time::Duration::from_secs(300),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert_eq!(
            json,
            r#"{"type":"service_settings","renewal_window_hours":6,"ca_bundle_hash":"abc123","shutdown_timeout_seconds":120,"ping_interval":300}"#
        );
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn service_settings_without_shutdown_timeout() {
        let msg = ControllerMessage::ServiceSettings(ServiceSettingsPayload {
            renewal_window_hours: 6,
            ca_bundle_hash: "abc123def".to_string(),
            capabilities: BTreeSet::new(),
            shutdown_timeout_seconds: None,
            ping_interval: std::time::Duration::from_secs(15),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"service_settings"#));
        assert!(!json.contains("shutdown_timeout_seconds"));
        assert!(json.contains(r#""ping_interval":15"#));
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn service_settings_backward_compat_extra_fields() {
        // Future-proof: extra fields in JSON should be ignored
        let json = r#"{"type":"service_settings","renewal_window_hours":12,"ca_bundle_hash":"def456","shutdown_timeout_seconds":60,"ping_interval":300,"some_future_field":"value"}"#;
        let msg: ControllerMessage = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            ControllerMessage::ServiceSettings(ServiceSettingsPayload {
                renewal_window_hours: 12,
                ca_bundle_hash: "def456".to_string(),
                capabilities: BTreeSet::new(),
                shutdown_timeout_seconds: Some(60),
                ping_interval: std::time::Duration::from_secs(300),
            })
        );
    }

    #[test]
    fn service_settings_backward_compat_missing_shutdown_timeout() {
        // Services running older protocol without shutdown_timeout_seconds should still parse
        let json = r#"{"type":"service_settings","renewal_window_hours":6,"ca_bundle_hash":"abc","ping_interval":300}"#;
        let msg: ControllerMessage = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            ControllerMessage::ServiceSettings(ServiceSettingsPayload {
                renewal_window_hours: 6,
                ca_bundle_hash: "abc".to_string(),
                capabilities: BTreeSet::new(),
                shutdown_timeout_seconds: None,
                ping_interval: std::time::Duration::from_secs(300),
            })
        );
    }

    #[test]
    fn duration_seconds_roundtrip() {
        let payload = ServiceSettingsPayload {
            renewal_window_hours: 6,
            ca_bundle_hash: String::new(),
            capabilities: BTreeSet::new(),
            shutdown_timeout_seconds: None,
            ping_interval: std::time::Duration::from_secs(42),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["ping_interval"], 42);
        let deserialized: ServiceSettingsPayload = serde_json::from_value(json).unwrap();
        assert_eq!(
            deserialized.ping_interval,
            std::time::Duration::from_secs(42)
        );
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
            host_machine_id: "test-machine-id".to_string(),
            assignments: vec![VersionCheckAssignment {
                software_item_id: TEST_UUID_1,
                name: "Test Software".to_string(),
                detect_version: Some(PluginAssignment {
                    plugin_type: PluginType::ReleasesGithub,
                    package_identifier: "octocat/hello-world".to_string(),
                    config: serde_json::json!({}),
                }),
                fetch_releases: None,
            }],
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"check_versions"#));
        assert!(json.contains(r#""software_item_id":"550e8400-e29b-41d4-a716-446655440000"#));
        assert!(json.contains(r#""plugin_type":"releases_github"#));
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn execute_update_serialization_roundtrip() {
        let msg = ControllerMessage::ExecuteUpdate(Box::new(ExecuteUpdatePayload {
            host_machine_id: "test-machine-id".to_string(),
            update_history_id: Uuid::parse_str("01936a1e-7e8c-7f00-8000-000000000001").unwrap(),
            software_item_id: Uuid::parse_str("01936a1e-7e8c-7f00-8000-000000000002").unwrap(),
            software_item_name: "Node.js".to_string(),
            to_version: "20.10.0".to_string(),
            detect_version_plugin: Some(PluginAssignment {
                plugin_type: PluginType::ReleasesGithub,
                package_identifier: "nodejs/node".to_string(),
                config: serde_json::json!({}),
            }),
            execute_update_plugin: PluginAssignment {
                plugin_type: PluginType::ReleasesGithub,
                package_identifier: "nodejs/node".to_string(),
                config: serde_json::json!({}),
            },
            pre_update_hooks: vec![HookCommand::Exec {
                program: "systemctl".to_string(),
                args: vec!["stop".to_string(), "myapp".to_string()],
                working_dir: None,
            }],
            post_update_hooks: vec![HookCommand::Exec {
                program: "systemctl".to_string(),
                args: vec!["start".to_string(), "myapp".to_string()],
                working_dir: None,
            }],
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
        }));
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"execute_update"#));
        assert!(json.contains(r#""plugin_type":"releases_github"#));
        assert!(json.contains(r#""exec"#));
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn execute_update_minimal_serialization() {
        let msg = ControllerMessage::ExecuteUpdate(Box::new(ExecuteUpdatePayload {
            host_machine_id: "test-machine-id".to_string(),
            update_history_id: TEST_UUID_1,
            software_item_id: TEST_UUID_2,
            software_item_name: "Redis".to_string(),
            to_version: "7.2.0".to_string(),
            detect_version_plugin: None,
            execute_update_plugin: PluginAssignment {
                plugin_type: PluginType::DiscoveryProxmoxHelperScripts,
                package_identifier: "redis-server".to_string(),
                config: serde_json::json!({}),
            },
            pre_update_hooks: vec![],
            post_update_hooks: vec![],
            release_info: None,
            timeout_seconds: DEFAULT_UPDATE_TIMEOUT_SECS,
        }));
        let json = serde_json::to_string(&msg).unwrap();
        // Empty vectors should be omitted
        assert!(!json.contains("pre_update_hooks"));
        assert!(!json.contains("post_update_hooks"));
        assert!(!json.contains("release_info"));
        // detect_version_plugin should be omitted when None
        assert!(!json.contains("detect_version_plugin"));
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn execute_update_default_timeout() {
        let json = r#"{
            "type": "execute_update",
            "host_machine_id": "test-machine-id",
            "update_history_id": "550e8400-e29b-41d4-a716-446655440000",
            "software_item_id": "550e8400-e29b-41d4-a716-446655440001",
            "software_item_name": "Test",
            "to_version": "1.0.0",
            "execute_update_plugin": {
                "plugin_type": "releases_github",
                "package_identifier": "test",
                "config": {}
            }
        }"#;
        let msg: ControllerMessage = serde_json::from_str(json).unwrap();
        if let ControllerMessage::ExecuteUpdate(payload) = msg {
            assert_eq!(payload.timeout_seconds, DEFAULT_UPDATE_TIMEOUT_SECS);
            assert!(payload.pre_update_hooks.is_empty());
            assert!(payload.post_update_hooks.is_empty());
        } else {
            panic!("Expected ExecuteUpdate");
        }
    }

    #[test]
    fn execute_update_with_shell_hook_command() {
        let msg = ControllerMessage::ExecuteUpdate(Box::new(ExecuteUpdatePayload {
            host_machine_id: "test-machine-id".to_string(),
            update_history_id: TEST_UUID_1,
            software_item_id: TEST_UUID_2,
            software_item_name: "Test".to_string(),
            to_version: "1.0.0".to_string(),
            detect_version_plugin: None,
            execute_update_plugin: PluginAssignment {
                plugin_type: PluginType::ReleasesGithub,
                package_identifier: "test".to_string(),
                config: serde_json::json!({}),
            },
            pre_update_hooks: vec![HookCommand::Shell {
                command: "echo hello".to_string(),
                shell: HookShell::Sh,
            }],
            post_update_hooks: vec![],
            release_info: None,
            timeout_seconds: DEFAULT_UPDATE_TIMEOUT_SECS,
        }));
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""shell"#));
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn hook_command_display() {
        let shell = HookCommand::Shell {
            command: "echo hello".to_string(),
            shell: HookShell::Bash,
        };
        assert_eq!(shell.to_string(), "echo hello");

        let exec = HookCommand::Exec {
            program: "systemctl".to_string(),
            args: vec!["restart".to_string(), "nginx".to_string()],
            working_dir: Some("/opt".to_string()),
        };
        assert_eq!(exec.to_string(), "(in /opt) systemctl restart nginx");
    }

    #[test]
    fn execute_update_backward_compat_extra_fields() {
        let json = r#"{
            "type": "execute_update",
            "host_machine_id": "test-machine-id",
            "update_history_id": "550e8400-e29b-41d4-a716-446655440000",
            "software_item_id": "550e8400-e29b-41d4-a716-446655440001",
            "software_item_name": "Test",
            "to_version": "1.0.0",
            "execute_update_plugin": {
                "plugin_type": "releases_github",
                "package_identifier": "test",
                "config": {}
            },
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
                username: Some(SecretString::new("user".into())),
                password: Some(SecretString::new("pass".into())),
                ca_pem: None,
                topic_prefix: "home/uptrakit".to_string(),
                ha_discovery: false,
                ha_discovery_prefix: "homeassistant".to_string(),
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
                ca_pem: None,
                topic_prefix: "uptrakit".to_string(),
                ha_discovery: false,
                ha_discovery_prefix: "homeassistant".to_string(),
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

    #[test]
    fn mqtt_client_created_serialization_roundtrip() {
        let msg = ControllerMessage::MqttClientCreated(MqttClientCreatedPayload {
            mqtt_client_id: TEST_UUID_2,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"mqtt_client_created"#));
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
    fn host_info_minimal_serialization_roundtrip() {
        let info = HostInfo {
            machine_id: "unknown".to_string(),
            os_type: None,
            os_version: None,
            architecture: None,
            hostname: None,
            ip_address: None,
        };
        let json = serde_json::to_string(&info).unwrap();
        assert_eq!(json, r#"{"machine_id":"unknown"}"#);
        let deserialized: HostInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, info);
    }

    #[test]
    fn host_info_with_hostname_and_ip() {
        let info = HostInfo {
            machine_id: "abc-123".to_string(),
            os_type: Some("linux".to_string()),
            os_version: None,
            architecture: None,
            hostname: Some("web-01.example.com".to_string()),
            ip_address: Some("10.0.0.5".to_string()),
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains(r#""hostname":"web-01.example.com"#));
        assert!(json.contains(r#""ip_address":"10.0.0.5"#));
        let deserialized: HostInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, info);
    }

    #[test]
    fn host_info_deserializes_without_new_fields() {
        // Ensures backward compatibility: old agents that don't send hostname/ip_address
        // still deserialize correctly (fields default to None).
        let json = r#"{"machine_id":"legacy","os_type":"linux"}"#;
        let info: HostInfo = serde_json::from_str(json).unwrap();
        assert_eq!(info.machine_id, "legacy");
        assert_eq!(info.hostname, None);
        assert_eq!(info.ip_address, None);
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
    fn enrollment_status_display_matches_serde() {
        for status in [EnrollmentStatus::Pending, EnrollmentStatus::Approved] {
            let serde_str = serde_json::to_value(status).unwrap();
            assert_eq!(
                status.to_string(),
                serde_str.as_str().unwrap(),
                "Display must match serde for {status:?}"
            );
        }
    }

    #[test]
    fn error_code_display_matches_serde() {
        for code in [
            ErrorCode::BadRequest,
            ErrorCode::EnrollmentFailed,
            ErrorCode::NotApproved,
            ErrorCode::Forbidden,
            ErrorCode::CertificateError,
            ErrorCode::InternalError,
            ErrorCode::SequenceError,
        ] {
            let serde_str = serde_json::to_value(code).unwrap();
            assert_eq!(
                code.to_string(),
                serde_str.as_str().unwrap(),
                "Display must match serde for {code:?}"
            );
        }
    }

    #[test]
    fn enrollment_status_rejects_invalid() {
        let result: std::result::Result<EnrollmentStatus, _> = serde_json::from_str(r#""invalid""#);
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
    fn mqtt_client_status_serde_roundtrip() {
        for (variant, expected_str) in [
            (MqttClientConnectionStatus::Online, "online"),
            (MqttClientConnectionStatus::Offline, "offline"),
            (MqttClientConnectionStatus::Connecting, "connecting"),
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, format!(r#""{expected_str}""#));
            let deserialized: MqttClientConnectionStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant);
        }
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
            (ErrorCode::SequenceError, "sequence_error"),
        ] {
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, format!(r#""{expected_str}""#));
            let deserialized: ErrorCode = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, variant);
        }
    }

    #[test]
    fn error_code_sequence_error_serde() {
        let json = serde_json::to_string(&ErrorCode::SequenceError).unwrap();
        assert_eq!(json, r#""sequence_error""#);
        let deserialized: ErrorCode = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ErrorCode::SequenceError);
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
        assert_eq!(ErrorCode::SequenceError.to_string(), "sequence_error");
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
            detect_version: Some(PluginAssignment {
                plugin_type: PluginType::ReleasesDocker,
                package_identifier: "nginx:latest".to_string(),
                config: serde_json::json!({}),
            }),
            fetch_releases: None,
        };
        let json = serde_json::to_string(&assignment).unwrap();
        assert!(json.contains(r#""plugin_type":"releases_docker""#));
        let deserialized: VersionCheckAssignment = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, assignment);
    }

    #[test]
    fn plugin_type_all_variants() {
        for (plugin, expected) in [
            (PluginType::ReleasesGithub, "releases_github"),
            (
                PluginType::DiscoveryProxmoxHelperScripts,
                "discovery_proxmox_helper_scripts",
            ),
            (PluginType::ReleasesDocker, "releases_docker"),
        ] {
            let json = serde_json::to_string(&plugin).unwrap();
            assert_eq!(json, format!(r#""{expected}""#));
            let deserialized: PluginType = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized, plugin);
        }
    }

    /// A `VersionCheckAssignment` carrying an unknown plugin type from a
    /// newer server must deserialize without error.  The entire message is
    /// preserved so the agent can log the skip reason instead of crashing.
    #[test]
    fn version_check_assignment_with_unknown_plugin_type_deserializes() {
        let json = serde_json::json!({
            "software_item_id": "00000000-0000-0000-0000-000000000001",
            "name": "My App",
            "detect_version": {
                "plugin_type": "winget",
                "package_identifier": "my-app",
                "config": {}
            }
        });
        let assignment: VersionCheckAssignment =
            serde_json::from_value(json).expect("should deserialize");
        assert_eq!(
            assignment.detect_version.as_ref().unwrap().plugin_type,
            PluginType::Other("winget".to_string())
        );
    }

    /// `"package_manager_apt"` deserializes to the known `PackageManagerApt` variant in `VersionCheckAssignment`.
    #[test]
    fn version_check_assignment_apt_plugin_type_deserializes() {
        let json = serde_json::json!({
            "software_item_id": "00000000-0000-0000-0000-000000000001",
            "name": "nginx",
            "detect_version": {
                "plugin_type": "package_manager_apt",
                "package_identifier": "nginx",
                "config": {}
            }
        });
        let assignment: VersionCheckAssignment =
            serde_json::from_value(json).expect("should deserialize");
        assert_eq!(
            assignment.detect_version.as_ref().unwrap().plugin_type,
            PluginType::PackageManagerApt
        );
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
            ca_pem: None,
            topic_prefix: "uptrakit".to_string(),
            ha_discovery: false,
            ha_discovery_prefix: "homeassistant".to_string(),
            updated_at: UtcDateTime::from_unix_timestamp(1706400000).unwrap(),
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("username"));
        assert!(!json.contains("password"));
        assert!(!json.contains("ca_pem"));
        let deserialized: MqttTenantConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, config);
    }

    #[test]
    fn enroll_requires_capabilities() {
        // Enrollment without capabilities should fail deserialization.
        let json = r#"{"type":"enroll","hostname":"node-old","friendly_name":"Old Node"}"#;
        let result: std::result::Result<ServiceMessage, _> = serde_json::from_str(json);
        assert!(result.is_err(), "EnrollPayload requires capabilities");
    }

    // =========================================================================
    // Envelope and sequence number tests
    // =========================================================================

    #[test]
    fn service_envelope_serde_roundtrip() {
        let envelope = ServiceEnvelope {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            seq: 1,
            message: ServiceMessage::Ping(PingPayload {
                service_ts: 1706400000000,
            }),
        };
        let json = serde_json::to_string(&envelope).unwrap();
        assert_eq!(
            json,
            r#"{"protocol_version":1,"seq":1,"type":"ping","service_ts":1706400000000}"#
        );
        let deserialized: ServiceEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, envelope);
    }

    #[test]
    fn controller_envelope_serde_roundtrip() {
        let envelope = ControllerEnvelope {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            seq: 42,
            message: ControllerMessage::Pong(PongPayload {
                service_ts: 1706400000000,
                controller_ts: 1706400000050,
            }),
        };
        let json = serde_json::to_string(&envelope).unwrap();
        assert_eq!(
            json,
            r#"{"protocol_version":1,"seq":42,"type":"pong","service_ts":1706400000000,"controller_ts":1706400000050}"#
        );
        let deserialized: ControllerEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, envelope);
    }

    #[test]
    fn service_envelope_missing_protocol_version_fails() {
        // Old-format envelope without protocol_version must fail deserialization.
        let json = r#"{"seq":1,"type":"ping","service_ts":1706400000000}"#;
        assert!(serde_json::from_str::<ServiceEnvelope>(json).is_err());
    }

    #[test]
    fn controller_envelope_missing_protocol_version_fails() {
        // Old-format envelope without protocol_version must fail deserialization.
        let json = r#"{"seq":42,"type":"pong","service_ts":1706400000000,"controller_ts":1706400000050}"#;
        assert!(serde_json::from_str::<ControllerEnvelope>(json).is_err());
    }

    #[test]
    fn service_envelope_complex_message() {
        let envelope = ServiceEnvelope {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            seq: 3,
            message: ServiceMessage::Enroll(EnrollPayload {
                hostname: "test-host".to_string(),
                friendly_name: "Test".to_string(),
                enrollment_token: None,
                capabilities: agent_capabilities(),
            }),
        };
        let json = serde_json::to_string(&envelope).unwrap();
        assert!(json.contains(r#""protocol_version":1"#));
        assert!(json.contains(r#""seq":3"#));
        assert!(json.contains(r#""type":"enroll"#));
        let deserialized: ServiceEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, envelope);
    }

    #[test]
    fn controller_envelope_error_message() {
        let envelope = ControllerEnvelope {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            seq: 5,
            message: ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::SequenceError,
                message: "sequence error: expected 3, received 5".to_string(),
            }),
        };
        let json = serde_json::to_string(&envelope).unwrap();
        assert!(json.contains(r#""protocol_version":1"#));
        assert!(json.contains(r#""seq":5"#));
        assert!(json.contains(r#""code":"sequence_error""#));
        let deserialized: ControllerEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, envelope);
    }

    #[test]
    fn outgoing_seq_increments() {
        let mut seq = OutgoingSeq::new();
        let e1 = seq.wrap_service(ServiceMessage::Ping(PingPayload { service_ts: 1 }));
        let e2 = seq.wrap_service(ServiceMessage::Ping(PingPayload { service_ts: 2 }));
        let e3 = seq.wrap_service(ServiceMessage::Ping(PingPayload { service_ts: 3 }));
        assert_eq!(e1.protocol_version, CURRENT_PROTOCOL_VERSION);
        assert_eq!(e1.seq, 1);
        assert_eq!(e2.seq, 2);
        assert_eq!(e3.seq, 3);
    }

    #[test]
    fn outgoing_seq_wrap_controller() {
        let mut seq = OutgoingSeq::new();
        let e1 = seq.wrap_controller(ControllerMessage::Pong(PongPayload {
            service_ts: 1,
            controller_ts: 2,
        }));
        let e2 = seq.wrap_controller(ControllerMessage::Pong(PongPayload {
            service_ts: 3,
            controller_ts: 4,
        }));
        assert_eq!(e1.protocol_version, CURRENT_PROTOCOL_VERSION);
        assert_eq!(e1.seq, 1);
        assert_eq!(e2.seq, 2);
    }

    #[test]
    fn incoming_seq_accepts_sequential() {
        let mut seq = IncomingSeq::new();
        assert!(seq.validate(1).is_ok());
        assert!(seq.validate(2).is_ok());
        assert!(seq.validate(3).is_ok());
    }

    #[test]
    fn incoming_seq_rejects_replay() {
        let mut seq = IncomingSeq::new();
        assert!(seq.validate(1).is_ok());
        let err = seq.validate(1).unwrap_err();
        assert_eq!(err.expected, 2);
        assert_eq!(err.received, 1);
    }

    #[test]
    fn incoming_seq_rejects_skip() {
        let mut seq = IncomingSeq::new();
        let err = seq.validate(2).unwrap_err();
        assert_eq!(err.expected, 1);
        assert_eq!(err.received, 2);
    }

    #[test]
    fn incoming_seq_rejects_zero() {
        let mut seq = IncomingSeq::new();
        let err = seq.validate(0).unwrap_err();
        assert_eq!(err.expected, 1);
        assert_eq!(err.received, 0);
    }

    #[test]
    fn seq_error_display() {
        let err = SeqError {
            expected: 3,
            received: 5,
        };
        assert_eq!(err.to_string(), "sequence error: expected 3, received 5");
    }

    #[test]
    fn outgoing_seq_default() {
        let mut seq = OutgoingSeq::default();
        let e = seq.wrap_service(ServiceMessage::Ping(PingPayload { service_ts: 1 }));
        assert_eq!(e.protocol_version, CURRENT_PROTOCOL_VERSION);
        assert_eq!(e.seq, 1);
    }

    #[test]
    fn incoming_seq_default() {
        let mut seq = IncomingSeq::default();
        assert!(seq.validate(1).is_ok());
    }

    // =========================================================================
    // Timestamp serialization safety tests
    // =========================================================================

    #[test]
    fn utc_datetime_millis_roundtrip_practical_range() {
        // Verify roundtrip for a practical timestamp (2024-01-28)
        let dt = UtcDateTime::from_unix_timestamp(1_706_400_000).unwrap();
        let payload = CertificatePayload {
            cert_pem: "test".to_string(),
            not_after: dt,
        };
        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: CertificatePayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.not_after, dt);
    }

    #[test]
    fn utc_datetime_millis_roundtrip_epoch() {
        // Verify roundtrip for Unix epoch
        let dt = UtcDateTime::from_unix_timestamp(0).unwrap();
        let payload = CertificatePayload {
            cert_pem: "test".to_string(),
            not_after: dt,
        };
        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: CertificatePayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.not_after, dt);
    }

    #[test]
    fn utc_datetime_millis_roundtrip_far_future() {
        // Verify roundtrip for a far future date (year 9999)
        let dt = UtcDateTime::from_unix_timestamp(253_402_300_799).unwrap();
        let payload = CertificatePayload {
            cert_pem: "test".to_string(),
            not_after: dt,
        };
        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: CertificatePayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.not_after, dt);
    }

    #[test]
    fn utc_datetime_millis_roundtrip_negative_timestamp() {
        // Verify roundtrip for a negative timestamp (before Unix epoch)
        let dt = UtcDateTime::from_unix_timestamp(-1_000_000).unwrap();
        let payload = CertificatePayload {
            cert_pem: "test".to_string(),
            not_after: dt,
        };
        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: CertificatePayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.not_after, dt);
    }

    // =========================================================================
    // AsyncAPI spec-conformance tests
    //
    // Validate that serialized messages conform to the asyncapi.yaml schema.
    // The spec is the source of truth for the wire protocol; these tests
    // ensure Rust serde annotations stay in sync with it.
    // =========================================================================

    /// Minimal AsyncAPI schema validator for wire protocol tests.
    struct AsyncApiSpec {
        schemas: serde_json::Map<String, serde_json::Value>,
    }

    impl AsyncApiSpec {
        fn load() -> Self {
            let yaml_str = include_str!("../asyncapi.yaml");
            let doc: serde_json::Value =
                serde_yaml_ng::from_str(yaml_str).expect("asyncapi.yaml should parse");
            let schemas = doc["components"]["schemas"]
                .as_object()
                .expect("components.schemas should be an object")
                .clone();
            Self { schemas }
        }

        /// Validate that a serialized JSON value conforms to the named schema.
        ///
        /// Checks:
        /// 1. Type discriminator (`const` field) matches
        /// 2. All required fields are present
        /// 3. Enum fields serialize to values in the spec's `enum` array
        fn validate(&self, schema_name: &str, json: &serde_json::Value) {
            let schema = self
                .schemas
                .get(schema_name)
                .unwrap_or_else(|| panic!("schema '{schema_name}' not found in asyncapi.yaml"));

            let obj = json
                .as_object()
                .unwrap_or_else(|| panic!("expected JSON object for schema '{schema_name}'"));

            // Check required fields
            if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
                for field in required {
                    let field_name = field.as_str().unwrap();
                    assert!(
                        obj.contains_key(field_name),
                        "schema '{schema_name}': required field '{field_name}' missing from \
                         serialized JSON.\nJSON: {json}"
                    );
                }
            }

            // Check const and enum constraints on properties
            if let Some(properties) = schema.get("properties").and_then(|p| p.as_object()) {
                for (prop_name, prop_schema) in properties {
                    if let Some(json_val) = obj.get(prop_name) {
                        // Check const
                        if let Some(const_val) = prop_schema.get("const") {
                            assert_eq!(
                                json_val, const_val,
                                "schema '{schema_name}': field '{prop_name}' should be \
                                 const {const_val}, got {json_val}"
                            );
                        }

                        // Check enum
                        if let Some(enum_vals) = prop_schema.get("enum").and_then(|e| e.as_array())
                        {
                            assert!(
                                enum_vals.contains(json_val),
                                "schema '{schema_name}': field '{prop_name}' value {json_val} \
                                 not in enum {enum_vals:?}"
                            );
                        }

                        // Check $ref to enum schemas
                        if let Some(ref_val) = prop_schema.get("$ref").and_then(|r| r.as_str()) {
                            let ref_schema_name = ref_val
                                .strip_prefix("#/components/schemas/")
                                .unwrap_or(ref_val);
                            if let Some(ref_schema) = self.schemas.get(ref_schema_name)
                                && let Some(enum_vals) =
                                    ref_schema.get("enum").and_then(|e| e.as_array())
                            {
                                assert!(
                                    enum_vals.contains(json_val),
                                    "schema '{schema_name}': field '{prop_name}' value \
                                     {json_val} not in enum {ref_schema_name} {enum_vals:?}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// Wrap a service message in an envelope and serialize to JSON value.
    fn service_envelope_json(msg: ServiceMessage) -> serde_json::Value {
        let envelope = ServiceEnvelope {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            seq: 1,
            message: msg,
        };
        serde_json::to_value(envelope).unwrap()
    }

    /// Wrap a controller message in an envelope and serialize to JSON value.
    fn controller_envelope_json(msg: ControllerMessage) -> serde_json::Value {
        let envelope = ControllerEnvelope {
            protocol_version: CURRENT_PROTOCOL_VERSION,
            seq: 1,
            message: msg,
        };
        serde_json::to_value(envelope).unwrap()
    }

    // ── ServiceMessage spec conformance ─────────────────────────────

    #[test]
    fn spec_conformance_ping() {
        let spec = AsyncApiSpec::load();
        let json = service_envelope_json(ServiceMessage::Ping(PingPayload {
            service_ts: 1706400000000,
        }));
        spec.validate("pingPayload", &json);
    }

    #[test]
    fn spec_conformance_enroll() {
        let spec = AsyncApiSpec::load();
        let json = service_envelope_json(ServiceMessage::Enroll(EnrollPayload {
            hostname: "node-1".to_string(),
            friendly_name: "Node One".to_string(),
            enrollment_token: Some(SecretString::new("tok-123".into())),
            capabilities: agent_capabilities(),
        }));
        spec.validate("enrollPayload", &json);
    }

    #[test]
    fn spec_conformance_request_certificate() {
        let spec = AsyncApiSpec::load();
        let json = service_envelope_json(ServiceMessage::RequestCertificate(
            RequestCertificatePayload {
                csr_pem:
                    "-----BEGIN CERTIFICATE REQUEST-----\ntest\n-----END CERTIFICATE REQUEST-----\n"
                        .to_string(),
            },
        ));
        spec.validate("requestCertificatePayload", &json);
    }

    #[test]
    fn spec_conformance_renew_certificate() {
        let spec = AsyncApiSpec::load();
        let json = service_envelope_json(ServiceMessage::RenewCertificate(
            RenewCertificatePayload {
                csr_pem: "-----BEGIN CERTIFICATE REQUEST-----\nrenew\n-----END CERTIFICATE REQUEST-----\n".to_string(),
            },
        ));
        spec.validate("renewCertificatePayload", &json);
    }

    #[test]
    fn spec_conformance_report_hosts() {
        let spec = AsyncApiSpec::load();
        let json = service_envelope_json(ServiceMessage::ReportHosts(ReportHostsPayload {
            hosts: vec![HostInfo {
                machine_id: "machine-42".to_string(),
                os_type: Some("linux".to_string()),
                os_version: Some("Ubuntu 24.04 LTS".to_string()),
                architecture: Some("x86_64".to_string()),
                hostname: Some("web-01.example.com".to_string()),
                ip_address: Some("10.0.0.5".to_string()),
            }],
            agent_version: "0.0.1".to_string(),
            capabilities: [Capability::SoftwareDiscovery, Capability::GracefulShutdown]
                .into_iter()
                .collect(),
        }));
        spec.validate("reportHostsPayload", &json);
    }

    #[test]
    fn spec_conformance_version_check_results() {
        let spec = AsyncApiSpec::load();
        let json = service_envelope_json(ServiceMessage::VersionCheckResults(
            VersionCheckResultsPayload {
                results: vec![VersionCheckResult {
                    software_item_id: TEST_UUID_1,
                    installed_version: Some("1.2.3".to_string()),
                    latest_version: Some("1.3.0".to_string()),
                    error: None,
                }],
            },
        ));
        spec.validate("versionCheckResultsPayload", &json);
    }

    #[test]
    fn spec_conformance_update_started() {
        let spec = AsyncApiSpec::load();
        let json = service_envelope_json(ServiceMessage::UpdateStarted(UpdateStartedPayload {
            update_history_id: TEST_UUID_1,
            from_version: Some("1.0.0".to_string()),
        }));
        spec.validate("updateStartedPayload", &json);
    }

    #[test]
    fn spec_conformance_update_output() {
        let spec = AsyncApiSpec::load();
        let json = service_envelope_json(ServiceMessage::UpdateOutput(UpdateOutputPayload {
            update_history_id: TEST_UUID_1,
            output: "Downloading package...".to_string(),
            stream: OutputStreamType::Stdout,
        }));
        spec.validate("updateOutputPayload", &json);
    }

    #[test]
    fn spec_conformance_update_result() {
        let spec = AsyncApiSpec::load();
        let json = service_envelope_json(ServiceMessage::UpdateResult(UpdateResultPayload {
            update_history_id: TEST_UUID_1,
            status: UpdateFinalStatus::Completed,
            from_version: Some("1.0.0".to_string()),
            to_version: Some("2.0.0".to_string()),
            output: "Update completed successfully".to_string(),
            error: None,
        }));
        spec.validate("updateResultPayload", &json);
    }

    #[test]
    fn spec_conformance_disconnecting() {
        let spec = AsyncApiSpec::load();
        let json = service_envelope_json(ServiceMessage::Disconnecting(DisconnectingPayload {
            reason: DisconnectReason::Shutdown,
            active_mqtt_clients: vec![TEST_UUID_1],
        }));
        spec.validate("disconnectingPayload", &json);
    }

    #[test]
    fn spec_conformance_register() {
        let spec = AsyncApiSpec::load();
        let json = service_envelope_json(ServiceMessage::Register(MqttRegisterPayload {
            instance_id: "mqtt-node1-01936a1e".to_string(),
            max_tenants: 10,
            active_mqtt_clients: vec![TEST_UUID_1],
            capabilities: [Capability::MqttBridge, Capability::GracefulShutdown]
                .into_iter()
                .collect(),
        }));
        spec.validate("mqttRegisterPayload", &json);
    }

    #[test]
    fn spec_conformance_release_tenants() {
        let spec = AsyncApiSpec::load();
        let json =
            service_envelope_json(ServiceMessage::ReleaseTenants(MqttReleaseTenantsPayload {
                mqtt_client_ids: vec![TEST_UUID_1],
            }));
        spec.validate("mqttReleaseTenantsPayload", &json);
    }

    #[test]
    fn spec_conformance_mqtt_client_status() {
        let spec = AsyncApiSpec::load();
        let json =
            service_envelope_json(ServiceMessage::MqttClientStatus(MqttClientStatusPayload {
                mqtt_client_id: TEST_UUID_1,
                status: MqttClientConnectionStatus::Online,
            }));
        spec.validate("mqttClientStatusPayload", &json);
    }

    // ── ControllerMessage spec conformance ──────────────────────────

    #[test]
    fn spec_conformance_pong() {
        let spec = AsyncApiSpec::load();
        let json = controller_envelope_json(ControllerMessage::Pong(PongPayload {
            service_ts: 1706400000000,
            controller_ts: 1706400000050,
        }));
        spec.validate("pongPayload", &json);
    }

    #[test]
    fn spec_conformance_enrolled() {
        let spec = AsyncApiSpec::load();
        let json = controller_envelope_json(ControllerMessage::Enrolled(EnrolledPayload {
            service_id: TEST_UUID_1,
            enrollment_secret: SecretString::new("secret-abc".into()),
            status: EnrollmentStatus::Pending,
        }));
        spec.validate("enrolledPayload", &json);
    }

    #[test]
    fn spec_conformance_approved() {
        let spec = AsyncApiSpec::load();
        let json = controller_envelope_json(ControllerMessage::Approved(ApprovedPayload {
            service_id: TEST_UUID_1,
        }));
        spec.validate("approvedPayload", &json);
    }

    #[test]
    fn spec_conformance_rejected() {
        let spec = AsyncApiSpec::load();
        let json = controller_envelope_json(ControllerMessage::Rejected(RejectedPayload {
            service_id: TEST_UUID_1,
        }));
        spec.validate("rejectedPayload", &json);
    }

    #[test]
    fn spec_conformance_certificate() {
        let spec = AsyncApiSpec::load();
        let json = controller_envelope_json(ControllerMessage::Certificate(CertificatePayload {
            cert_pem: "-----BEGIN CERTIFICATE-----\nMIIB...\n-----END CERTIFICATE-----\n"
                .to_string(),
            not_after: UtcDateTime::from_unix_timestamp(1_706_400_000).unwrap(),
        }));
        spec.validate("certificatePayload", &json);
    }

    #[test]
    fn spec_conformance_error() {
        let spec = AsyncApiSpec::load();
        let json = controller_envelope_json(ControllerMessage::Error(ErrorPayload {
            code: ErrorCode::EnrollmentFailed,
            message: "The enrollment token is invalid".to_string(),
        }));
        spec.validate("errorPayload", &json);
    }

    #[test]
    fn spec_conformance_service_settings() {
        let spec = AsyncApiSpec::load();
        let json =
            controller_envelope_json(ControllerMessage::ServiceSettings(ServiceSettingsPayload {
                renewal_window_hours: 6,
                ca_bundle_hash: "abc123".to_string(),
                capabilities: [
                    Capability::SoftwareDiscovery,
                    Capability::UpdateHooks,
                    Capability::GracefulShutdown,
                    Capability::MqttBridge,
                    Capability::SshRemote,
                ]
                .into_iter()
                .collect(),
                shutdown_timeout_seconds: Some(120),
                ping_interval: std::time::Duration::from_secs(300),
            }));
        spec.validate("serviceSettingsPayload", &json);
    }

    #[test]
    fn spec_conformance_ca_bundle_updated() {
        let spec = AsyncApiSpec::load();
        let json =
            controller_envelope_json(ControllerMessage::CaBundleUpdated(CaBundleUpdatedPayload {
                ca_bundle_pem: "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n"
                    .to_string(),
            }));
        spec.validate("caBundleUpdatedPayload", &json);
    }

    #[test]
    fn spec_conformance_request_cert_renewal() {
        let spec = AsyncApiSpec::load();
        let json = controller_envelope_json(ControllerMessage::RequestCertRenewal(
            RequestCertRenewalPayload {
                reason: "CA rotation after backend URL change".to_string(),
            },
        ));
        spec.validate("requestCertRenewalPayload", &json);
    }

    #[test]
    fn spec_conformance_server_restarting() {
        let spec = AsyncApiSpec::load();
        let json = controller_envelope_json(ControllerMessage::ServerRestarting(
            ServerRestartingPayload {
                reason: "controller restarting for upgrade".to_string(),
            },
        ));
        spec.validate("serverRestartingPayload", &json);
    }

    #[test]
    fn spec_conformance_check_versions() {
        let spec = AsyncApiSpec::load();
        let json =
            controller_envelope_json(ControllerMessage::CheckVersions(CheckVersionsPayload {
                host_machine_id: "test-machine-id".to_string(),
                assignments: vec![VersionCheckAssignment {
                    software_item_id: TEST_UUID_1,
                    name: "Test Software".to_string(),
                    detect_version: Some(PluginAssignment {
                        plugin_type: PluginType::ReleasesGithub,
                        package_identifier: "octocat/hello-world".to_string(),
                        config: serde_json::json!({}),
                    }),
                    fetch_releases: None,
                }],
            }));
        spec.validate("checkVersionsPayload", &json);
    }

    #[test]
    fn spec_conformance_execute_update() {
        let spec = AsyncApiSpec::load();
        let json = controller_envelope_json(ControllerMessage::ExecuteUpdate(Box::new(
            ExecuteUpdatePayload {
                host_machine_id: "test-machine-id".to_string(),
                update_history_id: TEST_UUID_1,
                software_item_id: TEST_UUID_2,
                software_item_name: "Node.js".to_string(),
                to_version: "20.10.0".to_string(),
                detect_version_plugin: None,
                execute_update_plugin: PluginAssignment {
                    plugin_type: PluginType::ReleasesGithub,
                    package_identifier: "nodejs/node".to_string(),
                    config: serde_json::json!({}),
                },
                pre_update_hooks: vec![],
                post_update_hooks: vec![],
                release_info: None,
                timeout_seconds: DEFAULT_UPDATE_TIMEOUT_SECS,
            },
        )));
        spec.validate("executeUpdatePayload", &json);
    }

    #[test]
    fn spec_conformance_registered() {
        let spec = AsyncApiSpec::load();
        let json = controller_envelope_json(ControllerMessage::Registered(MqttRegisteredPayload {
            instance_id: "mqtt-node1-01936a1e".to_string(),
        }));
        spec.validate("mqttRegisteredPayload", &json);
    }

    #[test]
    fn spec_conformance_tenant_assignments() {
        let spec = AsyncApiSpec::load();
        let json = controller_envelope_json(ControllerMessage::TenantAssignments(
            MqttTenantAssignmentsPayload {
                tenants: vec![MqttTenantConfig {
                    mqtt_client_id: TEST_UUID_3,
                    tenant_id: TEST_UUID_1,
                    enabled: true,
                    transport: MqttTransport::Tls,
                    host: "broker.example.com".to_string(),
                    port: 8883,
                    client_id: "uptrakit".to_string(),
                    username: Some(SecretString::new("user".into())),
                    password: Some(SecretString::new("pass".into())),
                    ca_pem: None,
                    topic_prefix: "home/uptrakit".to_string(),
                    ha_discovery: false,
                    ha_discovery_prefix: "homeassistant".to_string(),
                    updated_at: UtcDateTime::from_unix_timestamp(1706400000).unwrap(),
                }],
            },
        ));
        spec.validate("mqttTenantAssignmentsPayload", &json);
    }

    #[test]
    fn spec_conformance_tenant_config_updated() {
        let spec = AsyncApiSpec::load();
        let json = controller_envelope_json(ControllerMessage::TenantConfigUpdated(
            MqttTenantConfigUpdatedPayload {
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
                    ca_pem: None,
                    topic_prefix: "uptrakit".to_string(),
                    ha_discovery: false,
                    ha_discovery_prefix: "homeassistant".to_string(),
                    updated_at: UtcDateTime::from_unix_timestamp(1706400000).unwrap(),
                },
            },
        ));
        spec.validate("mqttTenantConfigUpdatedPayload", &json);
    }

    #[test]
    fn spec_conformance_tenant_revoked() {
        let spec = AsyncApiSpec::load();
        let json =
            controller_envelope_json(ControllerMessage::TenantRevoked(MqttTenantRevokedPayload {
                mqtt_client_id: TEST_UUID_1,
                reason: "mqtt client disabled".to_string(),
            }));
        spec.validate("mqttTenantRevokedPayload", &json);
    }

    #[test]
    fn spec_conformance_mqtt_client_created() {
        let spec = AsyncApiSpec::load();
        // mqtt_client_created uses a different schema (no seq in required).
        // Serialize just the inner message to match the schema.
        let msg = ControllerMessage::MqttClientCreated(MqttClientCreatedPayload {
            mqtt_client_id: TEST_UUID_2,
        });
        let json = serde_json::to_value(&msg).unwrap();
        spec.validate("mqttClientCreatedPayload", &json);
    }

    // =========================================================================
    // Autodiscovery wire message tests
    // =========================================================================

    #[test]
    fn discover_software_payload_roundtrip() {
        let msg = ControllerMessage::DiscoverSoftware(DiscoverSoftwarePayload {
            host_machine_id: "machine-abc".to_string(),
            plugins: vec![
                DiscoveryPluginAssignment {
                    plugin_config_id: Some(TEST_UUID_1),
                    plugin_type: PluginType::PackageManagerHomebrew,
                    config: serde_json::json!({"package_type": "formula"}),
                },
                DiscoveryPluginAssignment {
                    plugin_config_id: None,
                    plugin_type: PluginType::DiscoveryProxmoxHelperScripts,
                    config: serde_json::Value::Object(Default::default()),
                },
            ],
        });
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn discover_software_payload_type_tag() {
        let msg = ControllerMessage::DiscoverSoftware(DiscoverSoftwarePayload {
            host_machine_id: "machine-abc".to_string(),
            plugins: vec![],
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"discover_software""#));
    }

    #[test]
    fn discovery_results_payload_roundtrip() {
        let msg = ServiceMessage::DiscoveryResults(DiscoveryResultsPayload {
            host_machine_id: "machine-abc".to_string(),
            results: vec![
                DiscoveryPluginResult {
                    plugin_config_id: Some(TEST_UUID_1),
                    plugin_type: PluginType::PackageManagerHomebrew,
                    discoveries: vec![DiscoveredSoftware {
                        package_identifier: "wget".to_string(),
                        name: "Wget".to_string(),
                        installed_version: "1.21.4".to_string(),
                        targets: vec![],
                        extra: Some(serde_json::json!({"package_type": "formula"})),
                    }],
                    error: None,
                },
                DiscoveryPluginResult {
                    plugin_config_id: None,
                    plugin_type: PluginType::DiscoveryProxmoxHelperScripts,
                    discoveries: vec![],
                    error: Some("no update script found".to_string()),
                },
            ],
        });
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn discovery_results_payload_type_tag() {
        let msg = ServiceMessage::DiscoveryResults(DiscoveryResultsPayload {
            host_machine_id: "machine-abc".to_string(),
            results: vec![],
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"discovery_results""#));
    }

    #[test]
    fn discovery_plugin_assignment_none_config_id_omitted() {
        let assignment = DiscoveryPluginAssignment {
            plugin_config_id: None,
            plugin_type: PluginType::PackageManagerHomebrew,
            config: serde_json::Value::Object(Default::default()),
        };
        let json = serde_json::to_value(&assignment).unwrap();
        assert!(!json.as_object().unwrap().contains_key("plugin_config_id"));
    }

    #[test]
    fn spec_conformance_discover_software() {
        let spec = AsyncApiSpec::load();
        let json = controller_envelope_json(ControllerMessage::DiscoverSoftware(
            DiscoverSoftwarePayload {
                host_machine_id: "machine-abc".to_string(),
                plugins: vec![DiscoveryPluginAssignment {
                    plugin_config_id: Some(TEST_UUID_1),
                    plugin_type: PluginType::PackageManagerHomebrew,
                    config: serde_json::json!({"package_type": "formula"}),
                }],
            },
        ));
        spec.validate("discoverSoftwarePayload", &json);
    }

    #[test]
    fn spec_conformance_discovery_results() {
        let spec = AsyncApiSpec::load();
        let json =
            service_envelope_json(ServiceMessage::DiscoveryResults(DiscoveryResultsPayload {
                host_machine_id: "machine-abc".to_string(),
                results: vec![DiscoveryPluginResult {
                    plugin_config_id: Some(TEST_UUID_1),
                    plugin_type: PluginType::PackageManagerHomebrew,
                    discoveries: vec![DiscoveredSoftware {
                        package_identifier: "wget".to_string(),
                        name: "Wget".to_string(),
                        installed_version: "1.21.4".to_string(),
                        targets: vec![],
                        extra: None,
                    }],
                    error: None,
                }],
            }));
        spec.validate("discoveryResultsPayload", &json);
    }

    // =========================================================================
    // Capability enum tests
    // =========================================================================

    #[test]
    fn capability_serde_known_variants() {
        let cases = [
            (Capability::SoftwareDiscovery, "software_discovery"),
            (Capability::UpdateHooks, "update_hooks"),
            (Capability::GracefulShutdown, "graceful_shutdown"),
            (Capability::MqttBridge, "mqtt_bridge"),
            (Capability::SshRemote, "ssh_remote"),
        ];
        for (variant, wire_str) in &cases {
            let json = serde_json::to_string(variant).unwrap();
            assert_eq!(json, format!(r#""{wire_str}""#));
            let deserialized: Capability = serde_json::from_str(&json).unwrap();
            assert_eq!(&deserialized, variant);
        }
    }

    #[test]
    fn capability_other_roundtrip() {
        let cap: Capability = serde_json::from_str(r#""future_capability_xyz""#).unwrap();
        assert_eq!(cap, Capability::Other("future_capability_xyz".to_string()));
        let json = serde_json::to_string(&cap).unwrap();
        assert_eq!(json, r#""future_capability_xyz""#);
    }

    #[test]
    fn capability_display_matches_serde() {
        for cap in [
            Capability::SoftwareDiscovery,
            Capability::UpdateHooks,
            Capability::GracefulShutdown,
            Capability::MqttBridge,
            Capability::SshRemote,
        ] {
            let serde_str = serde_json::to_value(&cap).unwrap();
            assert_eq!(
                cap.to_string(),
                serde_str.as_str().unwrap(),
                "Display must match serde for {cap:?}"
            );
        }
    }

    #[test]
    fn capability_ordering() {
        // BTreeSet should produce a stable sorted order for capabilities.
        let set: BTreeSet<Capability> = [
            Capability::SshRemote,
            Capability::GracefulShutdown,
            Capability::SoftwareDiscovery,
        ]
        .into_iter()
        .collect();
        let mut iter = set.into_iter();
        // Alphabetical by wire string: graceful_shutdown < software_discovery < ssh_remote
        assert_eq!(iter.next(), Some(Capability::GracefulShutdown));
        assert_eq!(iter.next(), Some(Capability::SoftwareDiscovery));
        assert_eq!(iter.next(), Some(Capability::SshRemote));
    }

    #[test]
    fn capability_is_known() {
        assert!(Capability::SoftwareDiscovery.is_known());
        assert!(Capability::UpdateHooks.is_known());
        assert!(Capability::GracefulShutdown.is_known());
        assert!(Capability::MqttBridge.is_known());
        assert!(Capability::SshRemote.is_known());
        assert!(!Capability::Other("future".to_string()).is_known());
    }

    #[test]
    fn report_hosts_empty_capabilities_omitted() {
        let msg = ServiceMessage::ReportHosts(ReportHostsPayload {
            hosts: vec![HostInfo {
                machine_id: "m-1".to_string(),
                os_type: None,
                os_version: None,
                architecture: None,
                hostname: None,
                ip_address: None,
            }],
            agent_version: "0.0.1".to_string(),
            capabilities: BTreeSet::new(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(
            !json.contains("capabilities"),
            "empty capabilities should be omitted"
        );
        // Deserializes back with empty set.
        let deserialized: ServiceMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn capability_intersection_excludes_other() {
        let controller_caps: BTreeSet<Capability> = [
            Capability::SoftwareDiscovery,
            Capability::GracefulShutdown,
            Capability::MqttBridge,
        ]
        .into_iter()
        .collect();
        let service_caps: BTreeSet<Capability> = [
            Capability::SoftwareDiscovery,
            Capability::GracefulShutdown,
            Capability::Other("new_cap".to_string()),
        ]
        .into_iter()
        .collect();
        let agreed: BTreeSet<Capability> = controller_caps
            .intersection(&service_caps)
            .filter(|c| c.is_known())
            .cloned()
            .collect();
        assert_eq!(
            agreed,
            [Capability::SoftwareDiscovery, Capability::GracefulShutdown]
                .into_iter()
                .collect()
        );
    }

    // =========================================================================
    // New capability variants
    // =========================================================================

    #[test]
    fn scheduler_capability_roundtrip() {
        let cap = Capability::Scheduler;
        assert_eq!(cap.as_str(), "scheduler");
        assert!(cap.is_known());
        let json = serde_json::to_string(&cap).unwrap();
        assert_eq!(json, r#""scheduler""#);
        let parsed: Capability = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, cap);
    }

    #[test]
    fn database_access_capability_roundtrip() {
        let cap = Capability::DatabaseAccess;
        assert_eq!(cap.as_str(), "database_access");
        assert!(cap.is_known());
        let parsed: Capability = "database_access".parse().unwrap();
        assert_eq!(parsed, cap);
    }

    #[test]
    fn nats_access_capability_roundtrip() {
        let cap = Capability::NatsAccess;
        assert_eq!(cap.as_str(), "nats_access");
        let parsed: Capability = "nats_access".parse().unwrap();
        assert_eq!(parsed, cap);
    }

    #[test]
    fn master_key_access_capability_roundtrip() {
        let cap = Capability::MasterKeyAccess;
        assert_eq!(cap.as_str(), "master_key_access");
        let parsed: Capability = "master_key_access".parse().unwrap();
        assert_eq!(parsed, cap);
    }

    #[test]
    fn ca_management_capability_roundtrip() {
        let cap = Capability::CaManagement;
        assert_eq!(cap.as_str(), "ca_management");
        let parsed: Capability = "ca_management".parse().unwrap();
        assert_eq!(parsed, cap);
    }

    // =========================================================================
    // ServiceCredentials and RequestCaRotation payloads
    // =========================================================================

    #[test]
    fn service_credentials_serialization_roundtrip() {
        let msg = ControllerMessage::ServiceCredentials(ServiceCredentialsPayload {
            db_url: Some(SecretString::new("postgres://localhost/uptrakit".into())),
            master_key_hex: Some(SecretString::new("aa".repeat(32))),
            nats_url: Some("nats://localhost:4222".to_string()),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"service_credentials"#));
        assert!(json.contains(r#""db_url":"#));
        assert!(json.contains(r#""nats_url":"#));
        assert!(json.contains(r#""master_key_hex":"#));
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    #[test]
    fn service_credentials_omits_none_fields() {
        let msg = ControllerMessage::ServiceCredentials(ServiceCredentialsPayload {
            db_url: Some(SecretString::new("sqlite://test.db".into())),
            master_key_hex: None,
            nats_url: None,
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""db_url":"#));
        assert!(!json.contains("master_key_hex"));
        assert!(!json.contains("nats_url"));
    }

    #[test]
    fn request_ca_rotation_serialization_roundtrip() {
        let msg = ControllerMessage::RequestCaRotation(RequestCaRotationPayload {
            reason: "CA certificate expiring in 30 days".to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"request_ca_rotation"#));
        assert!(json.contains(r#""reason":"CA certificate expiring in 30 days"#));
        let deserialized: ControllerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, msg);
    }

    // ── is_nats_publishable ───────────────────────────────────────────────────

    #[test]
    fn is_nats_publishable_blocks_credential_bearing_variants() {
        // ServiceCredentials must never be published to NATS.
        assert!(!ControllerMessage::ServiceCredentials(ServiceCredentialsPayload {
            db_url: Some(SecretString::new("postgres://localhost/db".into())),
            master_key_hex: None,
            nats_url: None,
        })
        .is_nats_publishable());

        // MQTT tenant credential variants must also be blocked.
        assert!(!ControllerMessage::TenantAssignments(MqttTenantAssignmentsPayload {
            tenants: vec![],
        })
        .is_nats_publishable());

        assert!(!ControllerMessage::TenantConfigUpdated(MqttTenantConfigUpdatedPayload {
            tenant: MqttTenantConfig {
                mqtt_client_id: TEST_UUID_1,
                tenant_id: TEST_UUID_2,
                enabled: true,
                transport: MqttTransport::Tcp,
                host: "localhost".into(),
                port: 1883,
                client_id: "c".into(),
                username: None,
                password: None,
                ca_pem: None,
                topic_prefix: "t/".into(),
                ha_discovery: false,
                ha_discovery_prefix: "homeassistant".into(),
                updated_at: time::UtcDateTime::UNIX_EPOCH,
            },
        })
        .is_nats_publishable());

        assert!(!ControllerMessage::TenantRevoked(MqttTenantRevokedPayload {
            mqtt_client_id: TEST_UUID_1,
            reason: "test".into(),
        })
        .is_nats_publishable());
    }

    #[test]
    fn is_nats_publishable_allows_non_credential_variants() {
        // Ordinary messages must be publishable.
        assert!(ControllerMessage::Pong(PongPayload {
            service_ts: 0,
            controller_ts: 0,
        })
        .is_nats_publishable());

        assert!(ControllerMessage::Approved(ApprovedPayload {
            service_id: TEST_UUID_1,
        })
        .is_nats_publishable());

        assert!(ControllerMessage::RequestCaRotation(RequestCaRotationPayload {
            reason: "test".into(),
        })
        .is_nats_publishable());
    }
}
