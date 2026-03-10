use std::fmt;

use serde::{Deserialize, Serialize};
use time::UtcDateTime;

use super::capabilities::ErrorPayload;
use super::extension;
use super::payloads::{
    ApprovedPayload, BatchUpdateResultPayload, CaBundleUpdatedPayload, CertificatePayload,
    CheckVersionsPayload, DisconnectingPayload, DiscoverSoftwarePayload, DiscoveryResultsPayload,
    EnrollPayload, EnrolledPayload, ExecuteBatchUpdatePayload, ExecuteUpdatePayload,
    HostConnectivityUpdatedPayload, MqttClientCreatedPayload, MqttClientStatusPayload,
    MqttRegisterPayload, MqttRegisteredPayload, MqttReleaseTenantsPayload,
    MqttSoftwareStatesPayload, MqttTenantAssignmentsPayload, MqttTenantConfigUpdatedPayload,
    MqttTenantRevokedPayload, MqttTriggerHostBatchUpdatePayload, MqttUpdateTriggerPayload,
    PingPayload, PongPayload, RejectedPayload, ReportHostsPayload, ReportPluginConfigPayload,
    ReportPluginConfigResponsePayload, RequestCaRotationPayload, RequestCertRenewalPayload,
    RequestCrlRenewalPayload, ServerRestartingPayload, ServiceCredentialsPayload,
    ServiceSettingsPayload, SetUpdateFreezePayload, StdinAttentionPayload, TokenRevokedPayload,
    UpdateCapabilitiesPayload, UpdateOutputPayload, UpdateResultPayload, UpdateStartedPayload,
    UpdateStdinDataPayload, VersionCheckResultsPayload,
};
use uptrakit_shared_types::HookShell;

/// Unix epoch timestamp in milliseconds.
pub type Timestamp = i64;

/// Returns the current time as Unix epoch milliseconds.
pub fn now_millis() -> Timestamp {
    let now = UtcDateTime::now();
    now.unix_timestamp() * 1000 + i64::from(now.millisecond())
}

/// A single hook command to execute on the agent.
///
/// Predefined hooks use the `Exec` variant which avoids shell interpretation.
/// Custom commands use the `Shell` variant which runs through a shell.
///
/// # Wire forward-compatibility
///
/// `Other { raw }` is a catch-all for hook command types introduced in a
/// newer agent build. Serde deserialization is infallible: an unknown
/// variant becomes `Other { raw: ... }` rather than a parse error, allowing
/// older controllers to survive rolling upgrades.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
// Note: Eq is not derived because the Other variant contains serde_json::Value
// which does not implement Eq.
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
    /// Unknown hook command from a newer peer.
    ///
    /// The raw JSON value is preserved for logging. The receiver should
    /// log a warning and skip execution.
    #[serde(skip)]
    Other { raw: serde_json::Value },
}

impl<'de> Deserialize<'de> for HookCommand {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = serde_json::Value::deserialize(deserializer)?;
        if let Some(obj) = raw.as_object() {
            if let Some(shell_val) = obj.get("shell") {
                #[derive(Deserialize)]
                struct ShellFields {
                    command: String,
                    #[serde(default)]
                    shell: HookShell,
                }
                if let Ok(f) = serde_json::from_value::<ShellFields>(shell_val.clone()) {
                    return Ok(HookCommand::Shell {
                        command: f.command,
                        shell: f.shell,
                    });
                }
            }
            if let Some(exec_val) = obj.get("exec") {
                #[derive(Deserialize)]
                struct ExecFields {
                    program: String,
                    #[serde(default)]
                    args: Vec<String>,
                    #[serde(default)]
                    working_dir: Option<String>,
                }
                if let Ok(f) = serde_json::from_value::<ExecFields>(exec_val.clone()) {
                    return Ok(HookCommand::Exec {
                        program: f.program,
                        args: f.args,
                        working_dir: f.working_dir,
                    });
                }
            }
        }
        Ok(HookCommand::Other { raw })
    }
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
            Self::Other { raw } => write!(f, "<unknown hook command: {raw}>"),
        }
    }
}

/// Final status of an update execution.
///
/// # Wire forward-compatibility
///
/// `Other(String)` is a catch-all for status strings received from a newer
/// agent that this build does not yet recognise. Serde deserialization is
/// infallible: an unknown string becomes `Other(...)` rather than a parse
/// error, allowing older controllers to survive rolling upgrades without
/// dropping the enclosing `UpdateResult` message.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateFinalStatus {
    Completed,
    Failed,
    /// An unknown status received from a newer peer.
    ///
    /// The inner string is the raw snake_case value as it appeared on the wire.
    Other(String),
}

impl UpdateFinalStatus {
    /// Returns the string representation.
    ///
    /// For [`UpdateFinalStatus::Other`], returns the inner string as-is.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl fmt::Display for UpdateFinalStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for UpdateFinalStatus {
    fn from(s: String) -> Self {
        match s.as_str() {
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            _ => Self::Other(s),
        }
    }
}

impl Serialize for UpdateFinalStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for UpdateFinalStatus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(UpdateFinalStatus::from)
    }
}

/// Default timeout for update execution (2 hours).
pub const DEFAULT_UPDATE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(7200);

/// Default timeout for update execution.
pub(crate) fn default_update_timeout() -> std::time::Duration {
    DEFAULT_UPDATE_TIMEOUT
}

/// Reason for service disconnection.
///
/// # Wire forward-compatibility
///
/// `Other(String)` is a catch-all for reason strings received from a newer
/// peer that this build does not yet recognise. Serde deserialization is
/// infallible: an unknown string becomes `Other(...)` rather than a parse
/// error, allowing rolling upgrades without dropping the `Disconnecting` message.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisconnectReason {
    /// SIGTERM/SIGINT - clean exit.
    Shutdown,
    /// SIGHUP - will reconnect after external restart.
    Restart,
    /// An unknown reason received from a newer peer.
    ///
    /// The inner string is the raw snake_case value as it appeared on the wire.
    Other(String),
}

impl DisconnectReason {
    /// Returns the string representation.
    ///
    /// For [`DisconnectReason::Other`], returns the inner string as-is.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Shutdown => "shutdown",
            Self::Restart => "restart",
            Self::Other(s) => s.as_str(),
        }
    }
}

impl fmt::Display for DisconnectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<String> for DisconnectReason {
    fn from(s: String) -> Self {
        match s.as_str() {
            "shutdown" => Self::Shutdown,
            "restart" => Self::Restart,
            _ => Self::Other(s),
        }
    }
}

impl Serialize for DisconnectReason {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for DisconnectReason {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(DisconnectReason::from)
    }
}

/// Messages sent from a service (agent or MQTT) to the controller.
///
/// ## Forward compatibility
///
/// The `Unknown` variant is a catch-all for message types introduced in newer
/// service builds that an older controller does not yet recognise. When
/// encountered, the controller logs a warning and continues without closing the
/// connection, allowing rolling upgrades where services and controllers are not
/// updated simultaneously.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServiceMessage {
    // -- Shared enrollment + lifecycle --
    Ping(PingPayload),
    Enroll(EnrollPayload),
    RequestCertificate(super::payloads::RequestCertificatePayload),
    RenewCertificate(super::payloads::RenewCertificatePayload),
    Disconnecting(DisconnectingPayload),
    // -- Agent-specific --
    ReportHosts(ReportHostsPayload),
    VersionCheckResults(VersionCheckResultsPayload),
    UpdateStarted(UpdateStartedPayload),
    UpdateOutput(UpdateOutputPayload),
    UpdateResult(UpdateResultPayload),
    #[serde(alias = "batch_host_package_update_result")]
    BatchUpdateResult(BatchUpdateResultPayload),
    DiscoveryResults(DiscoveryResultsPayload),
    /// Agent → Controller: the update process appears to be waiting for stdin input.
    ///
    /// Sent when the agent detects that the process has produced no output for
    /// a sustained period while still running (heuristic: ~10 seconds of silence).
    /// The controller broadcasts this to interactive session subscribers and may
    /// trigger notifications.
    StdinAttention(StdinAttentionPayload),
    // -- MQTT-specific --
    Register(MqttRegisterPayload),
    ReleaseTenants(MqttReleaseTenantsPayload),
    MqttClientStatus(MqttClientStatusPayload),
    MqttTriggerUpdate(MqttUpdateTriggerPayload),
    /// MQTT service → Controller: trigger a batch update of all outdated software items on a host.
    ///
    /// Sent when a Home Assistant user presses "Install" on a host update entity.
    #[serde(
        rename = "mqtt_trigger_host_batch_update",
        alias = "mqtt_trigger_host_package_update"
    )]
    MqttTriggerHostBatchUpdate(MqttTriggerHostBatchUpdatePayload),
    // -- Capability management --
    /// Service announces its current capability set to the controller.
    ///
    /// Sent automatically by the SDK after `ServiceSettings` is processed.
    /// The controller persists the new capability set in the database so that
    /// routing and gating decisions reflect the service's installed version,
    /// even across upgrades that add or remove capabilities. On the current
    /// session the controller also refreshes its in-memory capability flags
    /// so that subsequent messages are gated correctly without requiring a
    /// reconnect.
    UpdateCapabilities(UpdateCapabilitiesPayload),
    // -- Plugin config reporting --
    /// Service reports a plugin configuration to the controller.
    ///
    /// Sent by agents that detect infrastructure (e.g. PVE nodes) during
    /// bootstrap. The controller creates or returns an existing plugin config
    /// matching `(tenant_id, plugin_type, name)` and responds with
    /// `ReportPluginConfigResponse`.
    ReportPluginConfig(ReportPluginConfigPayload),
    // -- UI Extensions --
    /// Service declares its UI extensions after connecting.
    ///
    /// Sent once after connection setup by services with the `UiExtensions`
    /// capability. Contains one or more extension manifests that the controller
    /// registers in its in-memory extension registry.
    ExtensionRegister(extension::ExtensionRegisterPayload),
    /// Registers an action library — a flat catalogue of [`extension::ActionDef`]
    /// entries that can be referenced by `action_id` from any extension manifest
    /// of the same source. Requires the `UiExtensions` capability.
    ///
    /// Sent independently of `ExtensionRegister` so that actions and manifests
    /// can be registered in any order. Subsequent sends replace the previous
    /// action set for this service.
    ExtensionActionsRegister(extension::ExtensionActionsPayload),
    /// Response to a proxied extension action invocation.
    ///
    /// Sent by the service after processing an `ExtensionRequest` from the
    /// controller. The `request_id` correlates this response with the original
    /// request.
    ExtensionResponse(extension::ExtensionResponsePayload),
    /// Service requests an extension action invocation from the controller.
    ///
    /// Enables services to call plugin-backed or other-service-backed extension
    /// actions via the wire protocol (e.g., SSH agent querying the Proxmox
    /// plugin for discovered guests). The controller dispatches the action
    /// exactly as it would for a REST-originated request and responds with
    /// `ControllerMessage::ExtensionResponse`.
    ///
    /// Reuses `ExtensionRequestPayload` for consistency — `sensitive_params`
    /// is always `None` for service-initiated requests (the mTLS channel is
    /// already trusted).
    ExtensionRequest(extension::ExtensionRequestPayload),
    /// Unknown message type from a newer service build.
    ///
    /// Deserialized when the `type` tag does not match any known variant.
    /// The payload is discarded. The receiver should log a warning and
    /// continue processing other messages.
    #[serde(other)]
    Unknown,
}

/// Messages sent from the controller to a service (agent or MQTT).
///
/// ## Forward compatibility
///
/// The `Unknown` variant is a catch-all for message types introduced in newer
/// controller builds that an older service does not yet recognise. When
/// encountered, the service logs a warning and continues without closing the
/// connection, allowing rolling upgrades where services and controllers are not
/// updated simultaneously.
// Note: Eq is not derived because ExecuteUpdate/ExecuteBatchUpdate contain
// HookCommand which holds serde_json::Value in its Other variant (not Eq).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    #[serde(alias = "execute_batch_host_package_update")]
    ExecuteBatchUpdate(Box<ExecuteBatchUpdatePayload>),
    DiscoverSoftware(DiscoverSoftwarePayload),
    SetUpdateFreeze(SetUpdateFreezePayload),
    /// Controller → Agent: forward stdin data or a signal to the running update process.
    ///
    /// Only sent to agents that advertise the `InteractiveUpdates` capability
    /// and have an in-flight interactive update matching the `update_history_id`.
    ///
    /// **Security**: session-targeted, NEVER published to NATS.
    UpdateStdinData(UpdateStdinDataPayload),
    // -- MQTT-specific --
    Registered(MqttRegisteredPayload),
    TenantAssignments(MqttTenantAssignmentsPayload),
    TenantConfigUpdated(MqttTenantConfigUpdatedPayload),
    TenantRevoked(MqttTenantRevokedPayload),
    MqttClientCreated(MqttClientCreatedPayload),
    SoftwareStates(MqttSoftwareStatesPayload),
    /// Agent connectivity changed for one or more hosts.
    ///
    /// Published to NATS with `target_capability = "mqtt_bridge"` by the controller
    /// that owns the agent WebSocket connection (on connect and disconnect). The MQTT
    /// service updates its per-tenant connectivity cache and publishes the
    /// `{prefix}/hosts/{h}/connectivity/state` retained topic.
    ///
    /// **Safe to publish via NATS** — contains no credential material.
    HostConnectivityUpdated(HostConnectivityUpdatedPayload),
    // -- UI Extensions --
    /// Proxied action invocation from the controller to a service.
    ///
    /// Sent to services with the `UiExtensions` capability when a REST client
    /// invokes an extension action. The service should process the action and
    /// respond with `ExtensionResponse`.
    ExtensionRequest(extension::ExtensionRequestPayload),
    /// Response to a service-initiated extension action invocation.
    ///
    /// Sent by the controller after processing a `ServiceMessage::ExtensionRequest`.
    /// The `request_id` correlates this response with the original request,
    /// completing the `ServiceExtensionProxy` oneshot channel on the service side.
    ExtensionResponse(extension::ExtensionResponsePayload),
    // -- Plugin config reporting --
    /// Response to a `ReportPluginConfig` request from a service.
    ///
    /// Contains the plugin config ID if the operation succeeded, or an error
    /// message if it failed. Idempotent: returns the existing config ID if a
    /// matching `(tenant_id, plugin_type, name)` already exists.
    ReportPluginConfigResponse(ReportPluginConfigResponsePayload),
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
    /// Request all controller instances to rebuild the CRL immediately.
    ///
    /// Published via NATS to the controller subject by any controller that
    /// revokes a certificate or by the `CrlRenewal` scheduled task.
    /// Receiving controllers fire `revocation_notify.notify_one()` so that
    /// `CrlManager::run()` rebuilds and hot-reloads the TLS configuration.
    RequestCrlRenewal(RequestCrlRenewalPayload),
    /// Token revocation event published by the originating controller to the
    /// "controller" NATS subject so that all other instances update their
    /// in-memory denylist caches without a per-request DB query.
    ///
    /// A message carries either a JTI-level revocation (when `jti` and `exp`
    /// are set) or a user-level revocation (when `user_id`, `iat_cutoff`, and
    /// `purge_after` are set). Both kinds may be present in a single message
    /// (e.g. when revoking a specific token *and* all prior tokens for a user).
    ///
    /// **Safe to publish via NATS** — contains no credential material.
    TokenRevoked(TokenRevokedPayload),
    /// Unknown message type from a newer controller build.
    ///
    /// Deserialized when the `type` tag does not match any known variant.
    /// The payload is discarded. The receiver should log a warning and
    /// continue processing other messages.
    ///
    /// **Security**: Never published to NATS — we cannot re-publish a message
    /// whose payload has been discarded.
    #[serde(other)]
    Unknown,
}

impl ControllerMessage {
    /// Returns `true` if this message may be published to NATS JetStream.
    ///
    /// Credential-bearing variants (`ServiceCredentials`, `TenantAssignments`,
    /// `TenantConfigUpdated`, `TenantRevoked`) and session-targeted variants
    /// (`ExtensionRequest`, `ExtensionResponse`) must **never** be published
    /// to NATS — they are delivered exclusively over authenticated WebSocket
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
                | ControllerMessage::ExtensionRequest(_)
                | ControllerMessage::ExtensionResponse(_)
                | ControllerMessage::UpdateStdinData(_)
                | ControllerMessage::Unknown
        )
    }
}
