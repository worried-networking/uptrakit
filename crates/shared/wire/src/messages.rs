use serde::{Deserialize, Serialize};

use super::capabilities::ErrorPayload;
use super::payloads::{
    ApprovedPayload, BatchUpdateResultPayload, BroadcastAdminEventPayload, CaBundleUpdatedPayload,
    CertificatePayload, CheckVersionsPayload, DeleteServiceConfigPayload, DisconnectingPayload,
    DiscoverSoftwarePayload, DiscoveryResultsPayload, EnrollPayload, EnrolledPayload,
    ExecuteBatchUpdatePayload, ExecuteUpdatePayload, HostConnectivityUpdatedPayload, PingPayload,
    PongPayload, RegisterPayload, RejectedPayload, ReportHostsPayload, ReportPluginConfigPayload,
    ReportPluginConfigResponsePayload, RequestCaRotationPayload, RequestCertRenewalPayload,
    RequestCrlRenewalPayload, ServerRestartingPayload, ServiceConfigAckPayload,
    ServiceConfigDeliveryPayload, ServiceConfigUpdatedPayload, ServiceCredentialsPayload,
    ServiceHostBatchUpdateTriggerPayload, ServiceSettingsPayload, ServiceUpdateTriggerPayload,
    SetUpdateFreezePayload, SoftwareStatesChangedPayload, SoftwareStatesPayload,
    StdinAttentionPayload, StoreServiceConfigPayload, TestPluginConfigPayload,
    TestPluginConfigResultPayload, TokenRevokedPayload, UpdateOutputPayload, UpdateResultPayload,
    UpdateStartedPayload, UpdateStdinDataPayload, VersionCheckResultsPayload,
    WorkloadClaimAnnouncementPayload, WorkloadClaimPayload, WorkloadClaimResultPayload,
    WorkloadClaimSyncRequestPayload, WorkloadClaimSyncResponsePayload, WorkloadReleasePayload,
};
use super::surfaces;

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
    ServiceTriggerUpdate(ServiceUpdateTriggerPayload),
    /// Service → Controller: trigger a batch update of all outdated software items on a host.
    ///
    /// Sent when a Home Assistant user presses "Install" on a host update entity.
    ServiceTriggerHostBatchUpdate(ServiceHostBatchUpdateTriggerPayload),
    // -- Capability declaration --
    /// Service declares its capabilities immediately on connect.
    ///
    /// Sent from `on_connected` before `ServiceSettings` is processed.
    /// The controller uses this to establish session-level capability flags
    /// without relying on DB-stored values (which may be absent on first connect).
    Register(RegisterPayload),
    // -- Plugin config reporting --
    /// Service reports a plugin configuration to the controller.
    ///
    /// Sent by agents that detect infrastructure (e.g. PVE nodes) during
    /// bootstrap. The controller creates or returns an existing plugin config
    /// matching `(tenant_id, plugin_type, name)` and responds with
    /// `ReportPluginConfigResponse`.
    ReportPluginConfig(ReportPluginConfigPayload),
    // -- Surfaces --
    /// Service declares its surfaces after connecting.
    ///
    /// Sent once after connection setup by services that participate in the
    /// surface contract.
    SurfaceRegistration(surfaces::SurfaceRegistration),
    /// Response to a proxied surface action invocation.
    ///
    /// Sent by the service after processing a `SurfaceActionRequest` from the
    /// controller.
    SurfaceActionResponse(surfaces::SurfaceActionResponse),
    /// Service requests a surface action invocation from the controller.
    ///
    /// Enables services to call surface actions via the wire protocol and
    /// receive the correlated `ControllerMessage::SurfaceActionResponse`.
    SurfaceActionRequest(surfaces::SurfaceActionRequest),
    // -- Service config store --
    /// Service → Controller: upsert a config entry in the controller DB.
    ///
    /// The controller encrypts sensitive values at rest, ACKs, and broadcasts
    /// `ServiceConfigUpdated` to all connected instances of the same service app.
    StoreServiceConfig(StoreServiceConfigPayload),
    /// Service → Controller: delete a config entry from the controller DB.
    ///
    /// The controller deletes, ACKs, and broadcasts `ServiceConfigUpdated`.
    DeleteServiceConfig(DeleteServiceConfigPayload),
    // -- Workload claim protocol --
    /// Service → Controller: request exclusive ownership of config keys.
    ///
    /// Sent after `ServiceConfigDelivery` is processed and whenever the
    /// desired config set changes. Uses full replacement semantics.
    /// Requires the `WorkloadClaims` capability.
    WorkloadClaim(WorkloadClaimPayload),
    /// Service → Controller: voluntarily release config keys.
    ///
    /// Sent when a service no longer wants to serve certain configs.
    /// Requires the `WorkloadClaims` capability.
    WorkloadRelease(WorkloadReleasePayload),
    /// Agent -> Controller: result of a plugin configuration test.
    ///
    /// Sent after the agent completes a config test request. The controller
    /// uses `request_id` to correlate with the pending REST API request.
    TestPluginConfigResult(TestPluginConfigResultPayload),
    /// Service -> Controller: forwarded semantic audit event.
    ///
    /// The controller re-validates the event and silently drops invalid or
    /// non-forwardable payloads without closing the connection.
    AuditEvent(super::payloads::AuditEventPayload),
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
    /// Controller → Services: reset all tenant-scoped data.
    ///
    /// Broadcast to services with the `ResetData` capability after the
    /// controller has cleared the database. Services should truncate their
    /// local data stores (e.g. SSH host list, Proxmox state).
    ResetData,
    SoftwareStates(SoftwareStatesPayload),
    /// Agent connectivity changed for one or more hosts.
    ///
    /// Published to NATS with `target_capability = "update_tracking"` by the controller
    /// that owns the agent WebSocket connection (on connect and disconnect). The MQTT
    /// service updates its per-tenant connectivity cache and publishes the
    /// `{prefix}/hosts/{h}/connectivity/state` retained topic.
    ///
    /// **Safe to publish via NATS** — contains no credential material.
    HostConnectivityUpdated(HostConnectivityUpdatedPayload),
    // -- Surfaces --
    /// Proxied surface action invocation from the controller to a service.
    ///
    /// Sent to services participating in the surface contract. The service
    /// should process the action and respond with `SurfaceActionResponse`.
    SurfaceActionRequest(surfaces::SurfaceActionRequest),
    /// Cancellation of an in-flight proxied surface action request.
    ///
    /// Session-targeted and never published to NATS.
    SurfaceActionCancel(surfaces::SurfaceActionCancel),
    /// Response to a service-initiated surface action invocation.
    ///
    /// Sent by the controller after processing a
    /// `ServiceMessage::SurfaceActionRequest`.
    SurfaceActionResponse(surfaces::SurfaceActionResponse),
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
    // -- Service config store --
    /// Controller → Service: initial delivery of all stored config entries.
    ///
    /// Sent once after authentication (after credential delivery if applicable).
    /// **Security**: contains decrypted sensitive values — NEVER published to NATS.
    ServiceConfigDelivery(ServiceConfigDeliveryPayload),
    /// Controller → Service: acknowledgment of a store or delete operation.
    ///
    /// **Security**: NEVER published to NATS — session-targeted.
    ServiceConfigAck(ServiceConfigAckPayload),
    /// Controller → Service: incremental update pushed to all instances of the
    /// same `service_app_name` when any instance modifies a config entry.
    ///
    /// **Security**: may contain decrypted sensitive values — NEVER published to NATS.
    ServiceConfigUpdated(ServiceConfigUpdatedPayload),
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
    /// Signal that software states have changed for a tenant.
    ///
    /// Published to the `controller` NATS subject by the external scheduler
    /// after a version-check run completes. The receiving controller loads
    /// the states from the database and pushes them to update-tracking services.
    SoftwareStatesChanged(SoftwareStatesChangedPayload),
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
    /// Cross-controller admin event broadcast.
    ///
    /// Published via NATS to the `controller` subject by any controller
    /// instance when it emits an `AdminEvent` to local SSE subscribers.
    /// Receiving controller instances decode the payload and re-broadcast
    /// to their own local SSE subscribers using `send_local` /
    /// `send_global_local` (without re-publishing to NATS to avoid loops).
    ///
    /// **Safe to publish via NATS** — contains no credential material.
    BroadcastAdminEvent(BroadcastAdminEventPayload),
    // -- Workload claim protocol --
    /// Controller → Service: grant/reject response for a workload claim.
    ///
    /// Sent in response to `WorkloadClaim`, unsolicited for proactive
    /// re-grants when previously rejected keys become available, or for
    /// revocations during cross-controller conflict resolution.
    ///
    /// **Session-targeted**: NEVER published to NATS.
    WorkloadClaimResult(WorkloadClaimResultPayload),
    /// Controller → NATS: announce claim state changes for cross-controller sync.
    ///
    /// Published to the `controller` NATS subject after granting or releasing
    /// claims. Other controllers update their global claim registry from this.
    ///
    /// **Safe to publish via NATS** — contains no credential material.
    WorkloadClaimAnnouncement(WorkloadClaimAnnouncementPayload),
    /// Controller → NATS: request full claim state from all active controllers.
    ///
    /// Published on controller startup. Each active controller responds with
    /// `WorkloadClaimSyncResponse`.
    ///
    /// **NATS-only** (controller-to-controller).
    WorkloadClaimSyncRequest(WorkloadClaimSyncRequestPayload),
    /// Controller → NATS: respond with full local claim state.
    ///
    /// Sent in response to `WorkloadClaimSyncRequest`.
    ///
    /// **NATS-only** (controller-to-controller).
    WorkloadClaimSyncResponse(WorkloadClaimSyncResponsePayload),
    /// Controller -> Agent: test a plugin configuration on a specific host.
    ///
    /// Sent when a user invokes the config test API endpoint for an agent-side
    /// plugin. The agent executes the test and responds with
    /// `ServiceMessage::TestPluginConfigResult`.
    ///
    /// **Security**: session-targeted, NEVER published to NATS.
    TestPluginConfig(TestPluginConfigPayload),
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
    /// Credential-bearing variants (`ServiceCredentials`) and session-targeted
    /// variants (`SurfaceActionRequest`, `SurfaceActionCancel`,
    /// `SurfaceActionResponse`)
    /// must **never** be published to NATS — they are delivered exclusively
    /// over authenticated WebSocket connections. All other variants are safe
    /// to broadcast via NATS.
    ///
    /// This is the authoritative gate used by [`NatsConnection::publish`].
    pub fn is_nats_publishable(&self) -> bool {
        !matches!(
            self,
            ControllerMessage::ServiceCredentials(_)
                | ControllerMessage::SurfaceActionRequest(_)
                | ControllerMessage::SurfaceActionCancel(_)
                | ControllerMessage::SurfaceActionResponse(_)
                | ControllerMessage::UpdateStdinData(_)
                | ControllerMessage::ResetData
                | ControllerMessage::ServiceConfigDelivery(_)
                | ControllerMessage::ServiceConfigAck(_)
                | ControllerMessage::ServiceConfigUpdated(_)
                | ControllerMessage::WorkloadClaimResult(_)
                | ControllerMessage::TestPluginConfig(_)
                | ControllerMessage::Unknown
        )
    }
}
