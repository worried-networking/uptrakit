use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use time::UtcDateTime;
use uuid::Uuid;

use super::capabilities::{Capability, EnrollmentStatus};
use super::shared_types::{DisconnectReason, UpdateFinalStatus};
use crate::serde_helpers::{duration_seconds, option_duration_seconds, utc_datetime_millis};
use uptrakit_shared_types::{
    DiscoveredSoftware, OutputStreamType, PluginTypeId, ReleaseInfo, SecretString, UpdateCategory,
};

/// Payload for ping messages.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PingPayload {
    /// Timestamp when the service sent the ping.
    pub service_ts: super::shared_types::Timestamp,
}

impl PingPayload {
    /// Creates a new `PingPayload` with the given service timestamp.
    pub fn new(service_ts: super::shared_types::Timestamp) -> Self {
        Self { service_ts }
    }
}

/// Payload for pong messages.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PongPayload {
    /// Original timestamp from the service's ping.
    pub service_ts: super::shared_types::Timestamp,
    /// Timestamp when the controller processed the ping.
    pub controller_ts: super::shared_types::Timestamp,
}

impl PongPayload {
    /// Creates a new `PongPayload` with the given service and controller timestamps.
    pub fn new(
        service_ts: super::shared_types::Timestamp,
        controller_ts: super::shared_types::Timestamp,
    ) -> Self {
        Self {
            service_ts,
            controller_ts,
        }
    }
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
    /// Agent-local UUID assigned to this host at bootstrap time.
    ///
    /// When present, the controller uses this as `hosts.id` when creating a
    /// new row, ensuring agent and controller share the same UUID. This is
    /// required for plugin FK operations (e.g. Proxmox host mapping) that
    /// reference `hosts.id` before the controller has generated its own UUID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_host_id: Option<Uuid>,
    /// Agent-probed host features (e.g. `["posix_shell", "privilege_escalation", "systemd"]`).
    ///
    /// `None` for legacy agents that predate feature reporting. Uses `Vec<String>`
    /// (not `BTreeSet<HostFeature>`) on the wire for forward-compatibility: if a
    /// newer agent reports a feature the controller doesn't know, it is stored
    /// losslessly and ignored by `HostCapabilities` parsing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<Vec<String>>,
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
    /// derives behavioral defaults from the resulting [`ServiceProfile`](crate::ServiceProfile).
    pub capabilities: BTreeSet<Capability>,
    /// The binary/crate name of the enrolling service (e.g., `"uptrakit-agent-ssh"`).
    ///
    /// Derived from `env!("CARGO_PKG_NAME")` at compile time. Used for UI
    /// display, extension conflict detection, and distinguishing service binaries.
    pub service_app_name: String,
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

/// Payload for service runtime settings pushed by the controller.
///
/// Used for both agents and MQTT services. `shutdown_timeout` is
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
    /// Per-page item-count limits for paginated service-to-controller reports.
    ///
    /// Services must honor these limits when splitting large `report_hosts`,
    /// `discovery_results`, `version_check_results`, and
    /// `batch_update_result` payloads across pages.
    #[serde(default, skip_serializing_if = "ReportPageLimits::is_default")]
    pub report_page_limits: ReportPageLimits,
    /// Maximum time to wait for in-flight operations during shutdown.
    /// Present for agents, absent for MQTT services.
    ///
    /// Wire field name: `shutdown_timeout_seconds` (kept for backward compatibility).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "option_duration_seconds",
        rename = "shutdown_timeout_seconds"
    )]
    pub shutdown_timeout: Option<std::time::Duration>,
    /// How often the service should send ping messages.
    /// Controller-managed; derived from per-service DB override or service-type default.
    #[serde(with = "duration_seconds")]
    pub ping_interval: std::time::Duration,
    /// Tenant UUID that this service belongs to.
    ///
    /// `None` for system services (MQTT, scheduler) which are not
    /// tenant-scoped. Present for tenant-scoped agents so they can
    /// include the tenant identity in external provisioning operations
    /// (e.g. PVE API credential naming).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<Uuid>,
}

/// Per-page item-count limits for paginated report payloads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportPageLimits {
    /// Maximum `hosts` items per `report_hosts` page.
    pub report_hosts: u32,
    /// Maximum `results` items per `version_check_results` page.
    pub version_check_results: u32,
    /// Maximum `results` items per `discovery_results` page.
    pub discovery_results: u32,
    /// Maximum `results` items per `batch_update_result` page.
    pub batch_update_results: u32,
}

impl ReportPageLimits {
    /// Returns `true` when all fields match the default wire limits.
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

impl Default for ReportPageLimits {
    fn default() -> Self {
        Self {
            report_hosts: crate::limits::MAX_REPORT_HOSTS as u32,
            version_check_results: crate::limits::MAX_VERSION_CHECK_RESULTS as u32,
            discovery_results: crate::limits::MAX_DISCOVERY_PLUGIN_RESULTS as u32,
            batch_update_results: crate::limits::MAX_BATCH_UPDATE_RESULTS as u32,
        }
    }
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
    pub plugin_type: PluginTypeId,
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
    /// Host software item ID for routing results to the host_software_items table.
    /// When set, this assignment is for a host-managed software item rather than
    /// a targeted software item.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "host_package_id"
    )]
    pub host_software_item_id: Option<Uuid>,
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
    /// Classification of the available update (e.g. security, bugfix).
    /// Defaults to `Unknown` when the plugin cannot classify the update.
    #[serde(default)]
    pub update_category: UpdateCategory,
    /// Host software item ID for routing results to the host_software_items table.
    /// Mirrors the value from the corresponding [`VersionCheckAssignment`].
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "host_package_id"
    )]
    pub host_software_item_id: Option<Uuid>,
    /// Human-readable installed version for display when `installed_version`
    /// is opaque (e.g. a Docker SHA256 digest → the image publish date).
    /// Set by the agent from `BatchDetectResult.display_version`.
    /// `None` when the plugin does not provide a display version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_display_version: Option<String>,
}

// --- Update execution messages ---

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
    /// Pre-update hook plugins to execute before the update, ordered by priority.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_update_hook_plugins: Vec<PluginAssignment>,
    /// Post-update hook plugins to execute after the update, ordered by priority.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_update_hook_plugins: Vec<PluginAssignment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_info: Option<ReleaseInfo>,
    /// Timeout for the update execution.
    ///
    /// Wire field name: `timeout_seconds` (kept for backward compatibility).
    #[serde(
        with = "duration_seconds",
        rename = "timeout_seconds",
        default = "super::shared_types::default_update_timeout"
    )]
    pub timeout: std::time::Duration,
    /// When `true`, the agent allocates a PTY and keeps stdin open for forwarding.
    ///
    /// Requires the agent to advertise the `InteractiveUpdates` capability.
    /// Defaults to `false` for backward compatibility with older agents.
    #[serde(default)]
    pub interactive: bool,
}

/// Agent -> Controller: Update is starting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateStartedPayload {
    pub update_history_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_version: Option<String>,
    /// Whether a PTY was actually allocated for this update.
    /// `false` for non-interactive updates or when PTY allocation failed.
    /// Old agents that do not send this field will deserialize as `false`.
    #[serde(default)]
    pub interactive: bool,
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

// --- Batch update messages ---

/// Controller → Agent: execute a batch update of software items.
///
/// Groups multiple items under a single plugin type so the agent can
/// run a single bulk command (e.g., `apt-get upgrade`, `brew upgrade`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecuteBatchUpdatePayload {
    /// The machine_id of the host to run the update on.
    pub host_machine_id: String,
    /// Unique identifier for this batch operation.
    pub batch_id: Uuid,
    /// Plugin type for all items in this batch.
    pub plugin_type: PluginTypeId,
    /// Merged plugin configuration.
    pub plugin_config: serde_json::Value,
    /// Individual items to update.
    pub updates: Vec<BatchUpdateItem>,
    /// Pre-update hook plugins to execute before the batch, ordered by priority.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pre_update_hook_plugins: Vec<PluginAssignment>,
    /// Post-update hook plugins to execute after the batch, ordered by priority.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_update_hook_plugins: Vec<PluginAssignment>,
    /// Timeout for the entire batch operation.
    ///
    /// Wire field name: `timeout_seconds` (kept for backward compatibility).
    #[serde(
        with = "duration_seconds",
        rename = "timeout_seconds",
        default = "super::shared_types::default_update_timeout"
    )]
    pub timeout: std::time::Duration,
    /// When `true`, the agent allocates a PTY and keeps stdin open for forwarding.
    ///
    /// Requires the agent to advertise the `InteractiveUpdates` capability.
    /// Defaults to `false` for backward compatibility with older agents.
    #[serde(default)]
    pub interactive: bool,
}

/// A single software item within a batch update request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchUpdateItem {
    /// Host software item entity ID.
    #[serde(alias = "host_package_id")]
    pub host_software_item_id: Uuid,
    /// Update history record ID (pre-created by the controller).
    pub update_history_id: Uuid,
    /// Plugin-specific package identifier (e.g., APT package name).
    pub package_identifier: String,
    /// Target version to install.
    pub to_version: String,
    /// Optional release metadata from the upstream source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_info: Option<ReleaseInfo>,
}

/// Agent → Controller: result of a batch update.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchUpdateResultPayload {
    /// Batch ID matching the request.
    pub batch_id: Uuid,
    /// Per-item results.
    pub results: Vec<BatchUpdateItemResult>,
}

/// Result of updating a single item within a batch operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchUpdateItemResult {
    /// Host software item entity ID.
    #[serde(alias = "host_package_id")]
    pub host_software_item_id: Uuid,
    /// Update history record ID.
    pub update_history_id: Uuid,
    /// Final status of this item's update.
    pub status: UpdateFinalStatus,
    /// Accumulated output from the update.
    pub output: String,
    /// Detected installed version after the update (if detection succeeded).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
    /// Error message if the update failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// --- Remote update freeze ---

/// Controller → Agent: enable or disable the update freeze.
///
/// When `enabled` is `true`, the agent creates its freeze file, which blocks
/// `ExecuteUpdate` and `ExecuteBatchUpdate` messages until the file
/// is removed (either via a subsequent `SetUpdateFreeze { enabled: false }`
/// message, or manually on the host via `rm <freeze-file>`).
///
/// This message is safe for NATS publication — it contains no credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetUpdateFreezePayload {
    /// Whether to enable (`true`) or disable (`false`) the freeze.
    pub enabled: bool,
    /// Optional human-readable reason for the freeze (audit trail).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// --- Graceful shutdown messages ---

/// Service -> Controller: Notification before disconnecting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisconnectingPayload {
    pub reason: DisconnectReason,
}

impl DisconnectingPayload {
    /// Create a `DisconnectingPayload` with the given reason.
    pub fn new(reason: DisconnectReason) -> Self {
        Self { reason }
    }
}

// =============================================================================
// Capability Management Payloads
// =============================================================================

/// Payload sent by every service on connect to declare its capabilities.
///
/// Sent from `on_connected` before any other messages. The controller uses
/// this as the authoritative source for capability detection on the current
/// session and persists the capability set to the DB.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterPayload {
    /// Capabilities declared by this service instance.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub capabilities: BTreeSet<Capability>,
    /// Runtime instance identity for restart-vs-reconnect detection.
    ///
    /// Optional for mixed-version compatibility: legacy services omit this
    /// field and are treated as service-scoped (not instance-scoped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_instance_id: Option<Uuid>,
}

impl RegisterPayload {
    /// Create a new [`RegisterPayload`] with the given capability set.
    ///
    /// # Example
    ///
    /// ```
    /// use std::collections::BTreeSet;
    /// use uptrakit_internal_wire::{Capability, RegisterPayload};
    ///
    /// let payload = RegisterPayload::new([Capability::SoftwareDiscovery, Capability::UpdateHooks]);
    /// assert!(payload.capabilities.contains(&Capability::SoftwareDiscovery));
    /// ```
    pub fn new(capabilities: impl IntoIterator<Item = Capability>) -> Self {
        Self {
            capabilities: capabilities.into_iter().collect(),
            runtime_instance_id: None,
        }
    }

    /// Set a runtime instance id on this register payload.
    #[must_use]
    pub fn with_runtime_instance_id(mut self, runtime_instance_id: Uuid) -> Self {
        self.runtime_instance_id = Some(runtime_instance_id);
        self
    }
}

// =============================================================================
// Service Config Store Payloads
// =============================================================================

/// A single stored service config entry, delivered to the service.
///
/// Sensitive values are already decrypted by the controller before delivery.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceConfigEntry {
    /// Tenant this entry belongs to, or `None` for global entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<Uuid>,
    /// Entry key (e.g. `"clients.{uuid}"`).
    pub key: String,
    /// Entry value (plaintext JSON; controller decrypts before delivery).
    pub value: serde_json::Value,
}

impl ServiceConfigEntry {
    /// Create a new `ServiceConfigEntry`.
    pub fn new(tenant_id: Option<Uuid>, key: String, value: serde_json::Value) -> Self {
        Self {
            tenant_id,
            key,
            value,
        }
    }
}

/// Identifies a service config entry by scope and key (used in delete notifications).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceConfigKey {
    /// Tenant this entry belongs to, or `None` for global entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<Uuid>,
    /// Entry key.
    pub key: String,
}

impl ServiceConfigKey {
    /// Create a new `ServiceConfigKey`.
    pub fn new(tenant_id: Option<Uuid>, key: String) -> Self {
        Self { tenant_id, key }
    }
}

/// Service → Controller: write or update a config entry.
///
/// The controller upserts the entry in `tenant_service_config` (when
/// `tenant_id` is set) or `global_service_config` (when `None`), encrypts
/// the value if `sensitive` is `true`, ACKs the operation, and broadcasts
/// `ServiceConfigUpdated` to all other connected instances of the same
/// `service_app_name`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreServiceConfigPayload {
    /// Correlation ID for the `ServiceConfigAck` response.
    pub request_id: String,
    /// Tenant scope. `None` = global scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<Uuid>,
    /// Config key (e.g. `"clients.{uuid}"`).
    pub key: String,
    /// Config value (plaintext JSON; controller encrypts at rest if `sensitive`).
    pub value: serde_json::Value,
    /// When `true`, the controller stores the value using `EncryptedString`.
    #[serde(default)]
    pub sensitive: bool,
}

impl StoreServiceConfigPayload {
    /// Create a new `StoreServiceConfigPayload`.
    pub fn new(
        request_id: String,
        tenant_id: Option<Uuid>,
        key: String,
        value: serde_json::Value,
        sensitive: bool,
    ) -> Self {
        Self {
            request_id,
            tenant_id,
            key,
            value,
            sensitive,
        }
    }
}

/// Service → Controller: delete a config entry.
///
/// The controller deletes the entry, ACKs, and broadcasts `ServiceConfigUpdated`
/// to all other connected instances of the same `service_app_name`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteServiceConfigPayload {
    /// Correlation ID for the `ServiceConfigAck` response.
    pub request_id: String,
    /// Tenant scope. `None` = global scope.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<Uuid>,
    /// Config key to delete.
    pub key: String,
}

impl DeleteServiceConfigPayload {
    /// Create a new `DeleteServiceConfigPayload`.
    pub fn new(request_id: String, tenant_id: Option<Uuid>, key: String) -> Self {
        Self {
            request_id,
            tenant_id,
            key,
        }
    }
}

/// Controller → Service: acknowledgment of a `StoreServiceConfig` or
/// `DeleteServiceConfig` operation.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceConfigAckPayload {
    /// Correlation ID matching the request.
    pub request_id: String,
    /// `true` if the operation succeeded.
    pub success: bool,
    /// Error message when `success` is `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ServiceConfigAckPayload {
    /// Create a new success ACK.
    pub fn success(request_id: String) -> Self {
        Self {
            request_id,
            success: true,
            error: None,
        }
    }

    /// Create a new error ACK.
    pub fn error(request_id: String, error: String) -> Self {
        Self {
            request_id,
            success: false,
            error: Some(error),
        }
    }
}

/// Controller → Service: initial delivery of all stored config entries.
///
/// Sent once after the service authenticates (after credential delivery).
/// The service should use this as its authoritative in-memory state.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceConfigDeliveryPayload {
    /// All config entries stored for this `service_app_name`.
    pub entries: Vec<ServiceConfigEntry>,
}

impl ServiceConfigDeliveryPayload {
    /// Create a new `ServiceConfigDeliveryPayload`.
    pub fn new(entries: Vec<ServiceConfigEntry>) -> Self {
        Self { entries }
    }
}

/// Controller → Service: incremental config update notification.
///
/// Pushed to all connected instances of the same `service_app_name` when
/// any instance stores or deletes a config entry. Services should apply
/// these changes to their in-memory state atomically.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceConfigUpdatedPayload {
    /// Entries that were inserted or updated (with decrypted values).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed: Vec<ServiceConfigEntry>,
    /// Keys that were deleted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deleted: Vec<ServiceConfigKey>,
}

impl ServiceConfigUpdatedPayload {
    /// Create a new `ServiceConfigUpdatedPayload`.
    pub fn new(changed: Vec<ServiceConfigEntry>, deleted: Vec<ServiceConfigKey>) -> Self {
        Self { changed, deleted }
    }
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

/// Request all controller instances to rebuild the CRL immediately.
///
/// Published via NATS to the `uptrakit.events.controller` subject by any
/// controller that revokes a certificate or by the `CrlRenewal` scheduled
/// task.  Receiving controllers fire `revocation_notify.notify_one()`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RequestCrlRenewalPayload {}

/// Signal that software states have changed for a tenant and need to be
/// re-loaded and pushed to update-tracking services.
///
/// Published to the `controller` NATS subject by the external scheduler after
/// a version-check run completes. The receiving controller loads the states
/// from the database and pushes them to all connected update-tracking services.
///
/// This is a lightweight signal — it carries only the tenant ID, not the state
/// data itself. This decouples the scheduler from the state-loading logic.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftwareStatesChangedPayload {
    pub tenant_id: uuid::Uuid,
}

impl SoftwareStatesChangedPayload {
    pub fn new(tenant_id: uuid::Uuid) -> Self {
        Self { tenant_id }
    }
}

/// Cross-controller token revocation event.
///
/// Published to the `controller` NATS subject by the controller that wrote
/// the revocation to the DB. Receiving controllers apply the revocation to
/// their in-memory denylist only — they do **not** write to DB (the
/// originating controller already did that).
///
/// A message may carry a JTI-level revocation, a user-level revocation, or
/// both. Fields not relevant to the revocation type are `None`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenRevokedPayload {
    /// JWT ID to deny (`exp` must also be set for JTI-level revocations).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jti: Option<String>,
    /// Token expiry unix timestamp (seconds). Required when `jti` is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<i64>,
    /// User UUID for user-level revocations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_id: Option<Uuid>,
    /// Deny tokens with `iat < iat_cutoff`. Required when `user_id` is set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iat_cutoff: Option<i64>,
    /// Remove the user entry after this unix timestamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purge_after: Option<i64>,
}

/// Per-host metadata published to MQTT for MQTT-browser visibility and Home Assistant.
///
/// Included in [`SoftwareStatesPayload`]. All fields are sourced exclusively
/// from the shared DB — safe for multi-controller deployments.
///
/// Intentionally excludes `ip_address` (network topology risk) and `agent_online`
/// (must come from the event-driven [`HostConnectivityUpdatedPayload`]).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostStateMetadata {
    /// Host UUID.
    pub host_id: Uuid,
    /// Hostname as reported by the agent.
    pub hostname: String,
    /// User-defined display name.
    pub friendly_name: String,
    /// Operating system type (e.g. `"linux"`, `"macos"`). `null` when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_type: Option<String>,
    /// Operating system version (e.g. `"Ubuntu 24.04 LTS"`). `null` when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os_version: Option<String>,
    /// CPU architecture (e.g. `"x86_64"`, `"aarch64"`). `null` when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture: Option<String>,
    /// Organisational tag names assigned to this host (e.g. `["production", "web-server"]`).
    #[serde(default)]
    pub tags: Vec<String>,
    /// Agent binary version string (e.g. `"0.2.1"`). `null` when never connected.
    ///
    /// Sourced from `services.client_version` for the newest approved, non-deactivated
    /// agent linked to this host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_version: Option<String>,
    /// ISO 8601 timestamp of when the agent last sent a message.
    ///
    /// Sourced from `services.last_seen_at`. `null` when never seen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_last_seen_at: Option<String>,
}

impl HostStateMetadata {
    /// Creates a new `HostStateMetadata` with required fields.
    pub fn new(host_id: Uuid, hostname: String, friendly_name: String) -> Self {
        Self {
            host_id,
            hostname,
            friendly_name,
            os_type: None,
            os_version: None,
            architecture: None,
            tags: Vec::new(),
            agent_version: None,
            agent_last_seen_at: None,
        }
    }
}

/// Connectivity status for a single host, used in [`HostConnectivityUpdatedPayload`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostConnectivityUpdate {
    /// Host UUID.
    pub host_id: Uuid,
    /// Whether the agent is currently connected (`true` = online, `false` = offline).
    pub online: bool,
    /// Timestamp of last agent activity (ISO 8601). `null` when unavailable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<String>,
    /// Agent binary version. Present on connect; `null` on disconnect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_version: Option<String>,
}

impl HostConnectivityUpdate {
    /// Creates an online update.
    pub fn online(
        host_id: Uuid,
        last_seen_at: Option<String>,
        agent_version: Option<String>,
    ) -> Self {
        Self {
            host_id,
            online: true,
            last_seen_at,
            agent_version,
        }
    }

    /// Creates an offline update.
    pub fn offline(host_id: Uuid, last_seen_at: Option<String>) -> Self {
        Self {
            host_id,
            online: false,
            last_seen_at,
            agent_version: None,
        }
    }
}

/// Controller → MQTT service: agent connectivity changed for one or more hosts.
///
/// Published to NATS with `target_capability = "update_tracking"` so that the MQTT
/// service on whichever controller the agent is connected to broadcasts the
/// connectivity state to **all** MQTT services across the cluster. This is the
/// canonical source of truth for `{prefix}/hosts/{h}/connectivity/state`.
///
/// Multi-controller safety: published by the controller that owns the agent
/// WebSocket connection (the only one with authoritative live state). All other
/// controllers receive this via NATS and update their caches.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostConnectivityUpdatedPayload {
    /// Tenant this update belongs to.
    pub tenant_id: Uuid,
    /// One entry per host whose connectivity changed.
    pub updates: Vec<HostConnectivityUpdate>,
}

impl HostConnectivityUpdatedPayload {
    /// Creates a new payload.
    pub fn new(tenant_id: Uuid, updates: Vec<HostConnectivityUpdate>) -> Self {
        Self { tenant_id, updates }
    }
}

/// Pagination metadata for a [`SoftwareStatesPayload`] message.
///
/// All payloads carry a `page` field. For single-page delivery use
/// `{ page_index: 0, total_pages: 1 }`. Multi-page delivery uses
/// `page_index` 0…N-1; the last page satisfies `page_index + 1 == total_pages`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftwareStatesPage {
    /// Zero-based index of this page.
    pub page_index: u32,
    /// Total number of pages in this delivery batch.
    pub total_pages: u32,
}

impl SoftwareStatesPage {
    /// Creates a single-page marker (the only page in a single-page delivery).
    pub fn single() -> Self {
        Self {
            page_index: 0,
            total_pages: 1,
        }
    }
}

/// Controller -> MQTT service: current software version state for a tenant.
///
/// Sent after tenant assignment and after any version check or update result.
/// Safe to write to the outbox (contains no credentials).
///
/// Large tenants use multi-page delivery. The `page` field indicates which
/// page this payload represents. Receivers must accumulate all pages before
/// applying the full state update (see `page_index + 1 == total_pages` for
/// the last-page signal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftwareStatesPayload {
    /// Tenant this state belongs to.
    pub tenant_id: Uuid,
    /// All active software items for the tenant with per-host version data.
    pub items: Vec<SoftwareStateItem>,
    /// Per-host aggregate summary of unpinned (unfeatured) software items.
    ///
    /// Each entry summarises all enabled, non-deactivated unfeatured items for
    /// one host. Only hosts with at least one such item are included.
    /// Defaults to an empty list on deserialization for backward compatibility
    /// with older MQTT services.
    #[serde(default, alias = "host_package_hosts")]
    pub host_summaries: Vec<HostPackageSummary>,
    /// Per-host metadata for all hosts referenced in `items` or `host_summaries`.
    ///
    /// Includes OS info, tags, and agent last-seen data. Sourced exclusively from DB.
    /// Defaults to an empty list for backward compatibility with older MQTT services.
    #[serde(default)]
    pub hosts: Vec<HostStateMetadata>,
    /// Pagination metadata indicating which page this payload represents.
    pub page: SoftwareStatesPage,
}

/// A single software item entry in [`SoftwareStatesPayload`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftwareStateItem {
    /// Software item UUID.
    pub software_item_id: Uuid,
    /// Human-readable software item name.
    pub name: String,
    /// Optional HTTPS URL to an icon/logo image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_url: Option<String>,
    /// Per-host version data for this software item.
    pub hosts: Vec<SoftwareStateHostEntry>,
}

/// Per-host version data for a software item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoftwareStateHostEntry {
    /// Host UUID.
    pub host_id: Uuid,
    /// Human-readable hostname.
    pub hostname: String,
    /// User-defined display name for the host.
    pub friendly_name: String,
    /// Currently installed version, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_version: Option<String>,
    /// Latest available version, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    /// Whether an update is available (`latest_version > installed_version`).
    pub update_available: bool,
    /// Whether an update is currently pending or in progress for this host-item pair.
    ///
    /// Set to `true` when an `update_history` record exists with status
    /// `Pending` or `InProgress`. Cleared to `false` once the update
    /// completes or fails. Defaults to `false` when absent (older controller).
    #[serde(default)]
    pub update_in_progress: bool,
    /// URL to the upstream release page (e.g. GitHub release), if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_url: Option<String>,
    /// Release notes or changelog text, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_notes: Option<String>,
    /// Classification of the update (e.g. `"security"`, `"bugfix"`, `"feature"`, `"unknown"`).
    ///
    /// Sourced from `host_software_item.update_category`. Defaults to `"unknown"` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update_category: Option<String>,
    /// Date when the latest release was published (ISO 8601 date string, e.g. `"2025-01-15"`).
    ///
    /// Extracted from `latest_release_metadata.published_at`. `null` when metadata is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    /// Timestamp when the installed version was last detected (ISO 8601).
    ///
    /// Sourced from `host_software_item.installed_version_detected_at`. `null` when never checked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_checked_at: Option<String>,
}

/// Service -> Controller: request to trigger a software update.
///
/// Sent when a Home Assistant user presses "Install" on an update entity.
/// The controller validates and dispatches the update to the appropriate agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceUpdateTriggerPayload {
    /// Tenant UUID (for validation).
    pub tenant_id: Uuid,
    /// Software item to update.
    pub software_item_id: Uuid,
    /// Host to update on.
    pub host_id: Uuid,
    /// Target version to install.
    pub to_version: String,
    /// Service instance UUID that initiated the trigger (used as actor_id).
    pub actor_service_id: Uuid,
}

/// Per-host aggregate summary of unpinned (unfeatured) software items.
///
/// Included in [`SoftwareStatesPayload`] to surface overall update
/// status per host to Home Assistant via a single `update` entity per host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostPackageSummary {
    /// Host UUID.
    pub host_id: Uuid,
    /// Human-readable hostname.
    pub hostname: String,
    /// User-defined display name for the host.
    #[serde(default)]
    pub friendly_name: String,
    /// Count of items where `installed_version != latest_version` (both known).
    pub pending_count: u32,
    /// Count of items where `update_category = "security"` AND versions differ.
    pub security_pending_count: u32,
    /// Total count of enabled, non-deactivated unfeatured items for this host.
    pub total_count: u32,
    /// Whether a batch update is currently pending or in progress for this host.
    pub update_in_progress: bool,
    /// Count of pending packages where `update_category = "bugfix"`.
    ///
    /// Defaults to `0` when absent (older controller that does not compute this field).
    #[serde(default)]
    pub bugfix_count: u32,
    /// Count of pending packages where `update_category = "feature"`.
    ///
    /// Defaults to `0` when absent (older controller that does not compute this field).
    #[serde(default)]
    pub feature_count: u32,
}

/// Service → Controller: trigger a batch update of all outdated software items on a host.
///
/// Sent when a Home Assistant user presses "Install" on a host update
/// entity. The controller resolves the latest versions for all outdated items
/// at trigger time and dispatches a `ExecuteBatchUpdate` to the agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceHostBatchUpdateTriggerPayload {
    /// Tenant UUID (for validation).
    pub tenant_id: Uuid,
    /// Host whose items should be updated.
    pub host_id: Uuid,
    /// Service instance UUID that initiated the trigger (used as actor_id).
    pub actor_service_id: Uuid,
    /// When `true`, only items with `update_category = "security"` are updated.
    #[serde(default)]
    pub security_only: bool,
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
    pub plugin_type: PluginTypeId,
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
    pub plugin_type: PluginTypeId,
    /// Discovered software items (empty on error).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub discoveries: Vec<DiscoveredSoftware>,
    /// Plugin-level error message, if discovery failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// =============================================================================
// Plugin config reporting
// =============================================================================

/// Payload for `ServiceMessage::ReportPluginConfig`.
///
/// Sent by agents that detect infrastructure (e.g. PVE nodes) during bootstrap
/// and want the controller to create or retrieve a plugin configuration.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportPluginConfigPayload {
    /// Unique request identifier for correlating the response.
    pub request_id: String,
    /// Plugin type string (e.g. `"infrastructure_proxmox"`).
    pub plugin_type: String,
    /// Human-readable name for the config (e.g. `"pve.local"`).
    pub name: String,
    /// Plugin-specific configuration JSON.
    pub config: serde_json::Value,
}

/// Payload for `ControllerMessage::ReportPluginConfigResponse`.
///
/// Returned to a service in response to `ReportPluginConfig`. Idempotent:
/// if a config with the same `(tenant_id, plugin_type, name)` already exists,
/// the existing ID is returned without creating a duplicate.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportPluginConfigResponsePayload {
    /// The request ID from the original `ReportPluginConfig` message.
    pub request_id: String,
    /// Whether the operation succeeded.
    pub success: bool,
    /// The plugin config ID (set on success).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_config_id: Option<Uuid>,
    /// Error message (set on failure).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// =============================================================================
// Interactive Update Payloads
// =============================================================================

/// Controller → Agent: forward stdin data or a signal to a running interactive update.
///
/// The `data` field contains raw bytes encoded as base64 to support binary
/// control sequences (e.g., `\x03` for Ctrl+C). When `signal` is set, the
/// agent delivers the signal to the process group instead of writing stdin.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateStdinDataPayload {
    /// The update history record this stdin data belongs to.
    pub update_history_id: Uuid,
    /// Raw bytes encoded as base64 (supports binary: Ctrl+C = \x03, etc.).
    pub data: String,
    /// When set, send this signal to the process group instead of writing stdin.
    /// Values: 2 = SIGINT, 15 = SIGTERM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<i32>,
}

impl UpdateStdinDataPayload {
    /// Create a new stdin data payload.
    pub fn new(update_history_id: Uuid, data: String) -> Self {
        Self {
            update_history_id,
            data,
            signal: None,
        }
    }

    /// Create a new signal payload.
    pub fn with_signal(update_history_id: Uuid, signal: i32) -> Self {
        Self {
            update_history_id,
            data: String::new(),
            signal: Some(signal),
        }
    }
}

/// Agent → Controller: the update process appears to be waiting for stdin input.
///
/// Sent when the agent detects sustained silence from the process (no output for
/// ~10 seconds while still running). The controller broadcasts this to interactive
/// session subscribers and may trigger notifications.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StdinAttentionPayload {
    /// The update history record that needs attention.
    pub update_history_id: Uuid,
    /// Optional hint about what the process might be waiting for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl StdinAttentionPayload {
    /// Create a new attention payload.
    pub fn new(update_history_id: Uuid) -> Self {
        Self {
            update_history_id,
            hint: None,
        }
    }

    /// Create a new attention payload with a hint.
    pub fn with_hint(update_history_id: Uuid, hint: String) -> Self {
        Self {
            update_history_id,
            hint: Some(hint),
        }
    }
}

// --- Cross-controller admin event broadcast ---

/// Cross-controller admin event broadcast payload.
///
/// Published via NATS to the `controller` subject by any controller instance
/// when it emits an [`AdminEvent`](uptrakit_web_api_types::events::AdminEvent)
/// to local SSE subscribers. Receiving controller instances decode the payload
/// and re-broadcast to their own local SSE subscribers without re-publishing
/// to NATS (to avoid infinite loops).
///
/// `tenant_id = None` means the event targets all tenants (system-wide).
///
/// **Safe to publish via NATS** — contains no credential material.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BroadcastAdminEventPayload {
    /// Target tenant, or `None` for system-wide events.
    pub tenant_id: Option<Uuid>,
    /// JSON-serialised `AdminEvent`.
    pub event_json: String,
}

// =============================================================================
// Workload Claim Protocol Payloads
// =============================================================================

/// Service → Controller: request exclusive ownership of config keys.
///
/// Each key in `claims` is a config key (e.g. `"clients.{uuid}"`) and the
/// value is the `tenant_id` that config belongs to. The controller grants
/// unclaimed keys and rejects keys already claimed by another service.
///
/// Uses **full replacement semantics**: each `WorkloadClaim` sends the
/// complete desired config key set. The controller diffs against current
/// grants to determine what to claim/release.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadClaimPayload {
    /// Map of `config_key → tenant_id` representing the full desired set.
    pub claims: BTreeMap<String, Uuid>,
}

impl WorkloadClaimPayload {
    /// Create a new `WorkloadClaimPayload`.
    pub fn new(claims: BTreeMap<String, Uuid>) -> Self {
        Self { claims }
    }
}

/// Controller → Service: grant/reject response for a workload claim request.
///
/// Sent in response to `WorkloadClaim`, or unsolicited when the controller
/// proactively re-grants previously rejected keys that became available
/// (e.g. after another service disconnected), or when revoking keys due
/// to cross-controller conflict resolution.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadClaimResultPayload {
    /// Config keys that were granted (exclusive ownership).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub granted: BTreeSet<String>,
    /// Config keys that were rejected (already claimed by another service).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub rejected: BTreeSet<String>,
}

impl WorkloadClaimResultPayload {
    /// Create a new `WorkloadClaimResultPayload`.
    pub fn new(granted: BTreeSet<String>, rejected: BTreeSet<String>) -> Self {
        Self { granted, rejected }
    }
}

/// Service → Controller: voluntarily release config keys.
///
/// Sent when a service no longer wants to serve certain configs (e.g. after
/// a config deletion). The controller releases the keys and makes them
/// available for other services.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadReleasePayload {
    /// Config keys to release.
    pub keys: BTreeSet<String>,
}

impl WorkloadReleasePayload {
    /// Create a new `WorkloadReleasePayload`.
    pub fn new(keys: BTreeSet<String>) -> Self {
        Self { keys }
    }
}

/// Controller → NATS: announce claim state changes for cross-controller sync.
///
/// Published to the `controller` NATS subject after granting or releasing
/// claims. Other controllers update their global claim registry from this.
///
/// **Safe to publish via NATS** — contains no credential material.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadClaimAnnouncementPayload {
    /// The service that owns these claims.
    pub service_id: Uuid,
    /// The controller that granted these claims.
    pub controller_id: Uuid,
    /// Newly claimed keys: `config_key → tenant_id`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub claimed: BTreeMap<String, Uuid>,
    /// Keys that were released.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub released: BTreeSet<String>,
    /// ISO 8601 timestamp when the claims were granted (for conflict resolution).
    pub claimed_at: String,
}

impl WorkloadClaimAnnouncementPayload {
    /// Create a new `WorkloadClaimAnnouncementPayload`.
    pub fn new(
        service_id: Uuid,
        controller_id: Uuid,
        claimed: BTreeMap<String, Uuid>,
        released: BTreeSet<String>,
        claimed_at: String,
    ) -> Self {
        Self {
            service_id,
            controller_id,
            claimed,
            released,
            claimed_at,
        }
    }
}

/// Controller → NATS: request full claim state from all active controllers.
///
/// Published on controller startup to the `controller` NATS subject.
/// Each active controller responds with `WorkloadClaimSyncResponse`.
///
/// **NATS-only** (controller-to-controller), not service-facing.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadClaimSyncRequestPayload {
    /// The requesting controller's ID.
    pub controller_id: Uuid,
}

impl WorkloadClaimSyncRequestPayload {
    /// Create a new `WorkloadClaimSyncRequestPayload`.
    pub fn new(controller_id: Uuid) -> Self {
        Self { controller_id }
    }
}

/// Controller → NATS: respond with full local claim state.
///
/// Sent in response to `WorkloadClaimSyncRequest`. Contains the responding
/// controller's complete local claim map for merging into the requester's
/// global registry.
///
/// **NATS-only** (controller-to-controller), not service-facing.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadClaimSyncResponsePayload {
    /// The responding controller's ID.
    pub controller_id: Uuid,
    /// Full local claim state: `config_key → (service_id, tenant_id)`.
    pub claims: BTreeMap<String, WorkloadClaimSyncEntry>,
}

impl WorkloadClaimSyncResponsePayload {
    /// Create a new `WorkloadClaimSyncResponsePayload`.
    pub fn new(controller_id: Uuid, claims: BTreeMap<String, WorkloadClaimSyncEntry>) -> Self {
        Self {
            controller_id,
            claims,
        }
    }
}

/// A single entry in a `WorkloadClaimSyncResponse`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadClaimSyncEntry {
    /// The service that owns this claim.
    pub service_id: Uuid,
    /// The tenant this config key belongs to.
    pub tenant_id: Uuid,
    /// ISO 8601 timestamp when the claim was granted.
    pub claimed_at: String,
}

impl WorkloadClaimSyncEntry {
    /// Create a new `WorkloadClaimSyncEntry`.
    pub fn new(service_id: Uuid, tenant_id: Uuid, claimed_at: String) -> Self {
        Self {
            service_id,
            tenant_id,
            claimed_at,
        }
    }
}

// ── Config test payloads ─────────────────────────────────────────────────────

/// The kind of configuration test to perform on the agent.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigTestKind {
    /// Execute `detect_installed_version()` and return output + detected version.
    VersionDetection,
    /// Validate update_command syntax (sh -n check, do NOT execute).
    UpdateCommandValidation,
    /// Execute pre-update hook with mock context.
    PreUpdateHook,
    /// Execute post-update hook with mock context.
    PostUpdateHook,
    /// Test connectivity for controller-side plugins (`fetch_releases`).
    Connectivity,
}

/// Payload for a plugin configuration test request (controller -> agent).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestPluginConfigPayload {
    /// Unique request ID for correlation (UUID v7).
    pub request_id: String,
    /// Target host machine ID on the agent.
    pub host_machine_id: String,
    /// What to test.
    pub test_kind: ConfigTestKind,
    /// The plugin type to test.
    pub plugin_type: String,
    /// The plugin configuration JSON to test.
    pub config: serde_json::Value,
    /// Package identifier for testing (required for version detection).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_identifier: Option<String>,
}

impl TestPluginConfigPayload {
    /// Creates a new test plugin config payload.
    pub fn new(
        request_id: String,
        host_machine_id: String,
        test_kind: ConfigTestKind,
        plugin_type: String,
        config: serde_json::Value,
    ) -> Self {
        Self {
            request_id,
            host_machine_id,
            test_kind,
            plugin_type,
            config,
            package_identifier: None,
        }
    }
}

/// Payload for a plugin configuration test result (agent -> controller).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestPluginConfigResultPayload {
    /// Correlation ID matching the original request.
    pub request_id: String,
    /// Whether the test passed.
    pub success: bool,
    /// Command output or connectivity response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Error message if the test failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Detected version (for version detection tests).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detected_version: Option<String>,
    /// Test duration in milliseconds.
    pub duration_ms: u64,
}

impl TestPluginConfigResultPayload {
    /// Creates a new test result payload.
    pub fn new(request_id: String, success: bool, duration_ms: u64) -> Self {
        Self {
            request_id,
            success,
            output: None,
            error: None,
            detected_version: None,
            duration_ms,
        }
    }
}
