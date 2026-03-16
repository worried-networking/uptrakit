use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

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
    /// Service tracks software update state and host connectivity.
    ///
    /// The controller uses this capability to route software-state broadcasts
    /// and connectivity updates. Also gates MQTT-specific lease coordination
    /// for MQTT bridge services.
    ///
    /// Wire string: `update_tracking`.
    UpdateTracking,
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
    /// Service supports pre-/post-update lifecycle hook plugins
    /// (`PluginAssignment` in `ExecuteUpdatePayload`). The controller omits
    /// hook plugins when absent.
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
    /// `db_url` in [`ServiceCredentialsPayload`](super::payloads::ServiceCredentialsPayload).
    ///
    /// Wire string: `database_access`.
    DatabaseAccess,
    /// Service requires NATS access. The controller will include `nats_url`
    /// in [`ServiceCredentialsPayload`](super::payloads::ServiceCredentialsPayload) (if NATS is configured).
    ///
    /// Wire string: `nats_access`.
    NatsAccess,
    /// Service requires the master encryption key. The controller will include
    /// `master_key_hex` in [`ServiceCredentialsPayload`](super::payloads::ServiceCredentialsPayload) (if encryption is enabled).
    ///
    /// Wire string: `master_key_access`.
    MasterKeyAccess,
    /// Service can request CA certificate rotation via [`RequestCaRotationPayload`](super::payloads::RequestCaRotationPayload).
    /// The controller will accept `RequestCaRotation` messages from services
    /// with this capability (via NATS or local delivery).
    ///
    /// Wire string: `ca_management`.
    CaManagement,
    /// Service is a global infrastructure service, not bound to any tenant.
    ///
    /// When present in an `EnrollPayload`, the controller routes enrollment to
    /// the `system_services` table instead of the per-tenant `services` table.
    ///
    /// **Credential guard**: any service requesting `DatabaseAccess`,
    /// `NatsAccess`, `MasterKeyAccess`, or `CaManagement` without also
    /// advertising `SystemService` will be rejected at enrollment with a 403
    /// error. This prevents regular tenant agents from claiming infrastructure
    /// credentials.
    ///
    /// Wire string: `system_service`.
    SystemService,
    /// Service supports UI extensions: it will send `ExtensionRegister` after
    /// connection and respond to `ExtensionRequest` messages.
    ///
    /// Wire string: `ui_extensions`.
    UiExtensions,
    /// Service supports interactive update sessions: PTY allocation, stdin
    /// forwarding, and signal delivery during update execution.
    ///
    /// When present, the controller may set `interactive: true` on
    /// `ExecuteUpdatePayload` and send `UpdateStdinData` messages to this
    /// service. The service allocates a PTY for the update process and keeps
    /// stdin open for forwarding.
    ///
    /// Wire string: `interactive_updates`.
    InteractiveUpdates,
    /// Service supports the reset-data protocol: truncates local data stores
    /// when the controller broadcasts a data reset.
    ///
    /// Wire string: `reset_data`.
    ResetData,
    /// Service participates in the workload claim protocol for exclusive
    /// config-key ownership.
    ///
    /// Services with this capability send `WorkloadClaim` after receiving
    /// `ServiceConfigDelivery` to request exclusive ownership of config keys.
    /// The controller responds with `WorkloadClaimResult` and routes
    /// tenant-scoped messages only to services that hold granted claims.
    ///
    /// Wire string: `workload_claims`.
    WorkloadClaims,
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
            Self::UpdateTracking => "update_tracking",
            Self::SshRemote => "ssh_remote",
            Self::Scheduler => "scheduler",
            Self::DatabaseAccess => "database_access",
            Self::NatsAccess => "nats_access",
            Self::MasterKeyAccess => "master_key_access",
            Self::CaManagement => "ca_management",
            Self::SystemService => "system_service",
            Self::UiExtensions => "ui_extensions",
            Self::InteractiveUpdates => "interactive_updates",
            Self::ResetData => "reset_data",
            Self::WorkloadClaims => "workload_claims",
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

impl FromStr for Capability {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(match s {
            "software_discovery" => Self::SoftwareDiscovery,
            "update_hooks" => Self::UpdateHooks,
            "graceful_shutdown" => Self::GracefulShutdown,
            "update_tracking" => Self::UpdateTracking,
            "ssh_remote" => Self::SshRemote,
            "scheduler" => Self::Scheduler,
            "database_access" => Self::DatabaseAccess,
            "nats_access" => Self::NatsAccess,
            "master_key_access" => Self::MasterKeyAccess,
            "ca_management" => Self::CaManagement,
            "system_service" => Self::SystemService,
            "ui_extensions" => Self::UiExtensions,
            "interactive_updates" => Self::InteractiveUpdates,
            "reset_data" => Self::ResetData,
            "workload_claims" => Self::WorkloadClaims,
            other => {
                tracing::debug!(capability = other, "received unknown capability from peer");
                Self::Other(other.to_string())
            }
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
///
/// # Wire forward-compatibility
///
/// `Other(String)` is a catch-all for status strings received from a newer
/// controller that this build does not yet recognise. Serde deserialization
/// is infallible: an unknown string becomes `Other(...)` rather than a parse
/// error, allowing older agents to survive rolling upgrades without dropping
/// the enclosing `Enrolled` message.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnrollmentStatus {
    Pending,
    Approved,
    /// An unknown status received from a newer peer.
    ///
    /// The inner string is the raw snake_case value as it appeared on the wire.
    Other(String),
}

impl EnrollmentStatus {
    /// Returns the string representation.
    ///
    /// For [`EnrollmentStatus::Other`], returns the inner string as-is.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl fmt::Display for EnrollmentStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for EnrollmentStatus {
    /// Converts a snake_case string to an enrollment status.
    ///
    /// Unknown strings map to [`EnrollmentStatus::Other`] rather than failing.
    fn from(s: String) -> Self {
        match s.as_str() {
            "pending" => Self::Pending,
            "approved" => Self::Approved,
            _ => {
                tracing::debug!(status = s, "received unknown enrollment status from peer");
                Self::Other(s)
            }
        }
    }
}

impl Serialize for EnrollmentStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EnrollmentStatus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(EnrollmentStatus::from)
    }
}

/// Machine-readable error code sent in `ErrorPayload`.
///
/// # Wire forward-compatibility
///
/// `Other(String)` is a catch-all for error codes received from a newer
/// controller that this build does not yet recognise. Serde deserialization
/// is infallible: an unknown string becomes `Other(...)` rather than a parse
/// error, allowing older agents to survive rolling upgrades without dropping
/// the enclosing `Error` message.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// An unknown error code received from a newer peer.
    ///
    /// The inner string is the raw snake_case value as it appeared on the wire.
    Other(String),
}

impl ErrorCode {
    /// Returns the string representation.
    ///
    /// For [`ErrorCode::Other`], returns the inner string as-is.
    pub fn as_str(&self) -> &str {
        match self {
            Self::BadRequest => "bad_request",
            Self::EnrollmentFailed => "enrollment_failed",
            Self::NotApproved => "not_approved",
            Self::Forbidden => "forbidden",
            Self::CertificateError => "certificate_error",
            Self::InternalError => "internal_error",
            Self::SequenceError => "sequence_error",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for ErrorCode {
    /// Converts a snake_case string to an error code.
    ///
    /// Unknown strings map to [`ErrorCode::Other`] rather than failing.
    fn from(s: String) -> Self {
        match s.as_str() {
            "bad_request" => Self::BadRequest,
            "enrollment_failed" => Self::EnrollmentFailed,
            "not_approved" => Self::NotApproved,
            "forbidden" => Self::Forbidden,
            "certificate_error" => Self::CertificateError,
            "internal_error" => Self::InternalError,
            "sequence_error" => Self::SequenceError,
            _ => {
                tracing::debug!(error_code = s, "received unknown error code from peer");
                Self::Other(s)
            }
        }
    }
}

impl Serialize for ErrorCode {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ErrorCode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(ErrorCode::from)
    }
}

/// Payload for error responses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub code: ErrorCode,
    pub message: String,
}
