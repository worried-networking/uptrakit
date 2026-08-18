use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use crate::{HostSnapshot, RuntimeSessionState, SshAgentRuntimeSupport, SshInFlightUpdate};
use async_trait::async_trait;
use parking_lot::Mutex;
use sea_orm::DatabaseConnection;
use uptrakit_plugin_infrastructure_registry::{InfraBundle, agent_infra::InfraPluginContext};
use uptrakit_wire::{
    CheckVersionsPayload, DiscoverSoftwarePayload, ExecuteBatchUpdatePayload, ExecuteUpdatePayload,
    ReportPluginConfigResponsePayload, ServiceMessage, ServiceTransport, TestPluginConfigPayload,
    TransportError,
    surfaces::{SurfaceActionRequest, SurfaceActionResponse},
};

use crate::client::{self, UpdateEvent};
use crate::surface_runtime::{self, InfraActionInvokerImpl, SurfaceRuntimeContext};
use crate::{ServiceSurfaceProxy, db, host_ops, operations, ssh_pool};

/// A `ReportPluginConfig` request awaiting the controller's ack, keyed by
/// `request_id`. Correlates the eventual `ReportPluginConfigResponsePayload`
/// back to the host and plugin type that originated the report, replacing
/// the former positional "first host without a config id" scan.
pub struct PendingConfigReport {
    pub host_id: uuid::Uuid,
    pub plugin_type: String,
}

/// Shared map of in-flight `ReportPluginConfig` requests, owned by the agent-ssh
/// runtime for the lifetime of one WebSocket connection. Cleared on every
/// reconnect (see `on_connected`) since request/response ride a single
/// connection and stranded entries would otherwise accumulate unbounded on a
/// long-lived agent; the correlated host simply re-reports on next sync.
pub type PendingConfigReports = Arc<Mutex<HashMap<String, PendingConfigReport>>>;

/// Shared SSH-agent support implementation reused by the standalone and embedded wrappers.
pub struct AgentSshRuntimeSupport {
    db: DatabaseConnection,
    state_dir: PathBuf,
    pool: ssh_pool::SshConnectionPool,
    surface_proxy: Arc<ServiceSurfaceProxy>,
    infra_bundles: Arc<Vec<InfraBundle>>,
    agent_version: String,
    pending_config_reports: PendingConfigReports,
}

impl AgentSshRuntimeSupport {
    pub fn new(
        db: DatabaseConnection,
        state_dir: PathBuf,
        pool: ssh_pool::SshConnectionPool,
        surface_proxy: Arc<ServiceSurfaceProxy>,
        infra_bundles: Arc<Vec<InfraBundle>>,
        agent_version: String,
    ) -> Self {
        Self {
            db,
            state_dir,
            pool,
            surface_proxy,
            infra_bundles,
            agent_version,
            pending_config_reports: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn pool(&self) -> &ssh_pool::SshConnectionPool {
        &self.pool
    }

    fn build_catalog() -> uptrakit_plugin_infrastructure_registry::PluginCatalog {
        let catalog_config = uptrakit_plugin_infrastructure_registry::CatalogConfig::default();
        #[expect(
            clippy::expect_used,
            reason = "infallible at startup: catalog construction failures are static configuration errors that must abort process initialization"
        )]
        uptrakit_plugin_infrastructure_registry::build_catalog(
            &catalog_config,
            uptrakit_plugin_infrastructure_registry::InstancePluginStates::all_disabled(),
        )
        .expect("plugin catalog must build successfully")
    }

    async fn reset_data_impl(&self) -> bool {
        if cfg!(feature = "reset-data") {
            tracing::info!("received ResetData: truncating local data stores");
            use sea_orm::{
                ConnectionTrait, EntityTrait,
                sea_query::{Alias, Query},
            };
            use uptrakit_db_tx::begin_immediate;
            match begin_immediate(&self.db).await {
                Ok(txn) => {
                    if let Err(error) = db::entity::pending_proxmox_match::Entity::delete_many()
                        .exec(&txn)
                        .await
                    {
                        tracing::error!(error = %error, "failed to truncate pending_proxmox_matches");
                    }
                    // The proxmox_host_state entity is unnameable from this crate (no
                    // dependency on the proxmox plugin), so the table is addressed via
                    // `Alias` rather than `Entity::delete_many`.
                    if let Err(error) = txn
                        .execute(
                            &Query::delete()
                                .from_table(Alias::new("proxmox_host_state"))
                                .to_owned(),
                        )
                        .await
                    {
                        tracing::error!(error = %error, "failed to truncate proxmox_host_state");
                    }
                    if let Err(error) = db::entity::ssh_host::Entity::delete_many().exec(&txn).await
                    {
                        tracing::error!(error = %error, "failed to truncate ssh_hosts");
                    }
                    match txn.commit().await {
                        Ok(()) => {
                            tracing::info!("local data stores truncated successfully");
                            true
                        }
                        Err(error) => {
                            tracing::error!(error = %error, "failed to commit ResetData transaction");
                            false
                        }
                    }
                }
                Err(error) => {
                    tracing::error!(error = %error, "failed to begin ResetData transaction");
                    false
                }
            }
        } else {
            tracing::warn!("received ResetData but reset-data feature is disabled; ignoring");
            false
        }
    }

    fn spawn_post_report_hooks_impl(
        db: DatabaseConnection,
        state_dir: PathBuf,
        surface_proxy: Arc<ServiceSurfaceProxy>,
        infra_bundles: Arc<Vec<InfraBundle>>,
        session_state: RuntimeSessionState,
        bg_tx: tokio::sync::mpsc::Sender<ServiceMessage>,
    ) {
        tokio::spawn(async move {
            let action_invoker =
                InfraActionInvokerImpl::new(&surface_proxy, &bg_tx, session_state.tenant_id);
            let tenant_id = session_state
                .tenant_id
                .map(|tenant_id| tenant_id.to_string());
            let ctx = InfraPluginContext {
                db: &db,
                tenant_id: tenant_id.as_deref(),
                service_id: session_state.service_id,
                state_dir: &state_dir,
                private_key_der: session_state.private_key_der.as_deref(),
                action_invoker: &action_invoker,
                guest_bootstrap: &operations::bootstrap_proxmox::NoopGuestBootstrapExecutor,
                provision_credentials: true,
            };

            for bundle in infra_bundles.iter() {
                if let Some(lifecycle) = bundle.lifecycle.as_ref()
                    && let Err(error) = lifecycle.on_post_report_hosts(&ctx).await
                {
                    tracing::warn!(
                        error = %error,
                        plugin_type = %lifecycle.plugin_type_id(),
                        "plugin on_post_report_hosts failed"
                    );
                }
            }
        });
    }
}

#[async_trait]
impl SshAgentRuntimeSupport for AgentSshRuntimeSupport {
    async fn report_enrolled_hosts(
        &self,
        transport: &mut dyn ServiceTransport,
    ) -> Result<(), TransportError> {
        client::report_enrolled_hosts(&self.db, transport, &self.pool, &self.agent_version).await;
        Ok(())
    }

    async fn register_surfaces(
        &self,
        encryption_public_key: Option<String>,
        session_state: &RuntimeSessionState,
        transport: &mut dyn ServiceTransport,
    ) -> Result<(), TransportError> {
        let catalog = Self::build_catalog();
        let register_payload = surface_runtime::build_surface_registration(
            encryption_public_key,
            &catalog,
            session_state.service_id,
            session_state.tenant_id,
        );
        transport
            .transport_send_best_effort(ServiceMessage::SurfaceRegistration(register_payload))
            .await;
        Ok(())
    }

    async fn list_host_snapshots(&self) -> Result<Vec<HostSnapshot>, String> {
        host_ops::list_host_snapshots(&self.db)
            .await
            .map(|snapshots| {
                snapshots
                    .into_iter()
                    .map(|snapshot| HostSnapshot {
                        id: snapshot.id,
                        updated_at: snapshot.updated_at,
                    })
                    .collect()
            })
            .map_err(|error| error.to_string())
    }

    async fn report_hosts_after_config_change(
        &self,
        transport: &mut dyn ServiceTransport,
        changed_ids: &HashSet<uuid::Uuid>,
    ) -> Result<(), TransportError> {
        let hosts = match host_ops::list_hosts(&self.db).await {
            Ok(hosts) => hosts,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "failed to list hosts for dynamic reload; skipping ReportHosts send"
                );
                return Ok(());
            }
        };

        client::report_hosts_after_config_change(
            &self.db,
            transport,
            &hosts,
            changed_ids,
            &self.pool,
            &self.agent_version,
        )
        .await;
        Ok(())
    }

    async fn evict_host(&self, host_id: uuid::Uuid) {
        self.pool.evict(host_id).await;
    }

    async fn disconnect_all(&self) {
        self.pool.disconnect_all().await;
    }

    fn spawn_check_versions(
        &self,
        payload: CheckVersionsPayload,
        bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    ) {
        client::spawn_check_versions_ssh(payload, &self.db, &self.pool, bg_tx);
    }

    async fn handle_execute_update(
        &self,
        payload: ExecuteUpdatePayload,
        in_flight_updates: &mut HashMap<String, SshInFlightUpdate>,
        aggregate_tx: &tokio::sync::mpsc::Sender<(String, UpdateEvent)>,
        transport: &mut dyn ServiceTransport,
    ) {
        client::handle_execute_update_ssh(
            payload,
            &self.db,
            in_flight_updates,
            aggregate_tx,
            transport,
            &self.pool,
        )
        .await;
    }

    fn spawn_execute_batch_update(
        &self,
        payload: ExecuteBatchUpdatePayload,
        bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    ) {
        client::spawn_execute_batch_update_ssh(payload, &self.db, &self.pool, bg_tx);
    }

    fn spawn_discover_software(
        &self,
        payload: DiscoverSoftwarePayload,
        bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    ) {
        client::spawn_discover_software_ssh(payload, &self.db, &self.pool, bg_tx);
    }

    fn spawn_config_test(
        &self,
        payload: TestPluginConfigPayload,
        bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    ) {
        client::spawn_config_test_ssh(payload, &self.db, &self.pool, bg_tx);
    }

    #[cfg(feature = "interactive")]
    fn handle_update_stdin_data(
        &self,
        payload: uptrakit_wire::UpdateStdinDataPayload,
        in_flight_updates: &HashMap<String, SshInFlightUpdate>,
    ) {
        client::handle_update_stdin_data_ssh(payload, in_flight_updates);
    }

    async fn handle_report_plugin_config_response(
        &self,
        payload: ReportPluginConfigResponsePayload,
    ) {
        // Remove first: the pending-ack map only ever holds requests still in
        // flight, so a successful remove is also the correlation lookup. Lock
        // is dropped at the end of this statement, well before any `.await`.
        let pending = self
            .pending_config_reports
            .lock()
            .remove(&payload.request_id);
        let Some(pending) = pending else {
            tracing::warn!(
                request_id = %payload.request_id,
                "received ReportPluginConfig response for unknown or already-consumed request"
            );
            return;
        };

        if !payload.success {
            tracing::warn!(
                request_id = %payload.request_id,
                error = ?payload.error,
                "plugin config report failed"
            );
            return;
        }

        let Some(config_id) = payload.plugin_config_id else {
            tracing::warn!(
                request_id = %payload.request_id,
                "plugin config report succeeded without a plugin_config_id"
            );
            return;
        };

        for bundle in self.infra_bundles.iter() {
            if let Some(lifecycle) = bundle.lifecycle.as_ref()
                && lifecycle.plugin_type_id().as_str() == pending.plugin_type.as_str()
            {
                if let Err(error) = lifecycle
                    .on_plugin_config_reported(&self.db, config_id, pending.host_id)
                    .await
                {
                    tracing::warn!(
                        error = %error,
                        plugin_type = %lifecycle.plugin_type_id(),
                        host_id = %pending.host_id,
                        "plugin on_plugin_config_reported failed"
                    );
                }
                break;
            }
        }
    }

    async fn handle_reset_data(&self) -> bool {
        self.reset_data_impl().await
    }

    async fn handle_surface_action_request(
        &self,
        request: SurfaceActionRequest,
        session_state: &RuntimeSessionState,
        bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
        transport: &mut dyn ServiceTransport,
    ) {
        let ctx = SurfaceRuntimeContext {
            db: &self.db,
            state_dir: &self.state_dir,
            private_key_der: session_state.private_key_der.as_deref(),
            service_id: session_state.service_id,
            tenant_id: session_state.tenant_id,
            bg_tx,
            surface_proxy: &self.surface_proxy,
            infra_bundles: Arc::clone(&self.infra_bundles),
            pending_config_reports: self.pending_config_reports(),
        };
        surface_runtime::handle_surface_action_request(request, &ctx, transport).await;
    }

    fn handle_surface_action_response(&self, response: SurfaceActionResponse) {
        let request_id = response.request_id;
        self.surface_proxy.complete(&request_id, response);
    }

    fn spawn_post_report_hooks(
        &self,
        session_state: &RuntimeSessionState,
        bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    ) {
        Self::spawn_post_report_hooks_impl(
            self.db.clone(),
            self.state_dir.clone(),
            Arc::clone(&self.surface_proxy),
            Arc::clone(&self.infra_bundles),
            session_state.clone(),
            bg_tx.clone(),
        );
    }

    async fn persist_tenant_id(&self, tenant_id: uuid::Uuid) {
        let mut identity =
            uptrakit_service_sdk::ServiceIdentityState::new_single_dir(&self.state_dir);
        // For embedded services: state_dir contains no service.json (never enrolled).
        // load() succeeds with all fields None; save_tenant_id() returns Ok(()) early
        // when service_id is None — no disk write occurs.
        if let Err(error) = identity.load().await {
            tracing::warn!(error = %error, "failed to load identity for tenant_id persistence");
            return;
        }
        if let Err(error) = identity.save_tenant_id(tenant_id).await {
            tracing::warn!(error = %error, "failed to persist tenant_id to service.json");
        }
    }

    async fn persisted_tenant_id(&self) -> Option<uuid::Uuid> {
        let mut identity =
            uptrakit_service_sdk::ServiceIdentityState::new_single_dir(&self.state_dir);
        if let Err(error) = identity.load().await {
            tracing::warn!(error = %error, "failed to load identity for persisted tenant_id lookup");
            return None;
        }
        identity.tenant_id()
    }

    async fn notify_tenant_changed(&self) {
        for bundle in self.infra_bundles.iter() {
            if let Some(lifecycle) = bundle.lifecycle.as_ref()
                && let Err(error) = lifecycle.on_tenant_changed(&self.db).await
            {
                tracing::warn!(
                    error = %error,
                    plugin_type = %lifecycle.plugin_type_id(),
                    "plugin on_tenant_changed failed"
                );
            }
        }
    }

    fn pending_config_reports(&self) -> PendingConfigReports {
        Arc::clone(&self.pending_config_reports)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::{RuntimeSessionState, SshAgentRuntimeSupport};
    use async_trait::async_trait;
    use sea_orm::{ConnectionTrait, Database, EntityTrait, PaginatorTrait, Statement};
    use uptrakit_wire::{
        ControllerMessage, ReportPluginConfigResponsePayload, ServiceMessage, ServiceTransport,
        TransportClosePolicy, TransportError,
    };

    use super::{AgentSshRuntimeSupport, InfraBundle, PendingConfigReport};

    /// `ReportPluginConfigResponsePayload` is `#[non_exhaustive]`, so callers
    /// outside `uptrakit-wire` cannot use struct-literal syntax; build it via
    /// its `Deserialize` impl instead.
    fn response_payload(
        request_id: &str,
        success: bool,
        plugin_config_id: Option<uuid::Uuid>,
        error: Option<&str>,
    ) -> serde_json::Result<ReportPluginConfigResponsePayload> {
        serde_json::from_value(serde_json::json!({
            "request_id": request_id,
            "success": success,
            "plugin_config_id": plugin_config_id,
            "error": error,
        }))
    }

    #[derive(Default)]
    struct TestTransport {
        send_log: Vec<ServiceMessage>,
        fail_send: bool,
    }

    #[async_trait]
    impl ServiceTransport for TestTransport {
        async fn transport_send(&mut self, msg: ServiceMessage) -> Result<(), TransportError> {
            if self.fail_send {
                return Err(TransportError);
            }
            self.send_log.push(msg);
            Ok(())
        }

        async fn transport_send_best_effort(&mut self, msg: ServiceMessage) {
            self.send_log.push(msg);
        }

        async fn transport_send_auto_paginate(
            &mut self,
            msg: ServiceMessage,
        ) -> Result<(), TransportError> {
            self.transport_send(msg).await
        }

        async fn transport_recv(&mut self) -> Option<ControllerMessage> {
            None
        }

        fn close_policy(&self) -> TransportClosePolicy {
            TransportClosePolicy::Reconnect { reason: None }
        }
    }

    #[tokio::test]
    async fn register_surfaces_uses_best_effort_transport() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should connect");
        let support = AgentSshRuntimeSupport::new(
            db,
            tempfile::tempdir().expect("tempdir").path().to_path_buf(),
            crate::ssh_pool::SshConnectionPool::new(),
            Arc::new(crate::ServiceSurfaceProxy::new()),
            Arc::new(Vec::new()),
            "0.0.0-test".to_string(),
        );
        let session_state = RuntimeSessionState {
            service_id: Some(uuid::Uuid::now_v7()),
            tenant_id: Some(uuid::Uuid::now_v7()),
            private_key_der: None,
        };
        let mut transport = TestTransport {
            send_log: Vec::new(),
            fail_send: true,
        };

        support
            .register_surfaces(
                Some("public-key".to_string()),
                &session_state,
                &mut transport,
            )
            .await
            .expect("surface registration should degrade to best-effort");

        assert!(
            transport
                .send_log
                .iter()
                .any(|message| matches!(message, ServiceMessage::SurfaceRegistration(_))),
            "surface registration should still be emitted via best-effort transport"
        );
    }

    /// Builds `infra_bundles` the same way `AgentSshHandler::new` does
    /// (`handler.rs`), so tests exercise the real proxmox `HostLifecycle`
    /// rather than a hand-rolled fake (not nameable from this crate).
    fn real_infra_bundles()
    -> uptrakit_plugin_infrastructure_registry::PluginResult<Arc<Vec<InfraBundle>>> {
        let catalog_config = uptrakit_plugin_infrastructure_registry::CatalogConfig::default();
        let catalog = uptrakit_plugin_infrastructure_registry::build_catalog(
            &catalog_config,
            uptrakit_plugin_infrastructure_registry::InstancePluginStates::all_disabled(),
        )?;
        Ok(Arc::new(catalog.create_infra_bundles(&catalog_config)))
    }

    #[tokio::test]
    async fn failed_ack_removes_pending_entry() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should connect");
        let support = AgentSshRuntimeSupport::new(
            db,
            tempfile::tempdir().expect("tempdir").path().to_path_buf(),
            crate::ssh_pool::SshConnectionPool::new(),
            Arc::new(crate::ServiceSurfaceProxy::new()),
            Arc::new(Vec::new()),
            "0.0.0-test".to_string(),
        );
        let request_id = uuid::Uuid::now_v7().to_string();
        support.pending_config_reports().lock().insert(
            request_id.clone(),
            PendingConfigReport {
                host_id: uuid::Uuid::now_v7(),
                plugin_type: "infrastructure.proxmox".to_string(),
            },
        );

        support
            .handle_report_plugin_config_response(
                response_payload(
                    &request_id,
                    false,
                    None,
                    Some("controller rejected the config"),
                )
                .expect("ReportPluginConfigResponsePayload JSON is always valid"),
            )
            .await;

        assert!(
            !support
                .pending_config_reports()
                .lock()
                .contains_key(&request_id),
            "a failed ack must remove the pending entry, not leave it stranded"
        );
    }

    #[tokio::test]
    async fn mismatched_plugin_type_ack_does_not_write_proxmox_state() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should connect");
        crate::db::migration::run_migrations(&db)
            .await
            .expect("migrations should run against the in-memory db");

        let infra_bundles =
            real_infra_bundles().expect("plugin catalog should build successfully in tests");
        // Precondition: the catalog must actually contain a proxmox
        // `HostLifecycle`, or this test would pass vacuously (no bundle ever
        // dispatches to, so the assertion below proves nothing).
        assert!(
            infra_bundles.iter().any(|bundle| bundle
                .lifecycle
                .as_ref()
                .is_some_and(|lifecycle| lifecycle.plugin_type_id()
                    == uptrakit_shared_types::plugin_ids::INFRASTRUCTURE_PROXMOX)),
            "test fixture assumption violated: no proxmox HostLifecycle bundle in the catalog"
        );

        let host_id = uuid::Uuid::now_v7();
        let now = "2026-01-01T00:00:00Z";
        // `uptrakit-agent-ssh-runtime` depends on
        // `uptrakit-plugin-infrastructure-registry`, not on
        // `uptrakit-plugin-infrastructure-proxmox` directly, and the registry
        // does not re-export `proxmox_host_state`; its `ActiveModel` is
        // unnameable from this crate. execute_raw with a Statement is the
        // approved exception for raw SQL.
        db.execute_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Sqlite,
            "INSERT INTO proxmox_host_state (host_id, created_at, updated_at) VALUES ($1, $2, $2)",
            [host_id.to_string().into(), now.into()],
        ))
        .await
        .expect("seed proxmox_host_state row");

        let support = AgentSshRuntimeSupport::new(
            db.clone(),
            tempfile::tempdir().expect("tempdir").path().to_path_buf(),
            crate::ssh_pool::SshConnectionPool::new(),
            Arc::new(crate::ServiceSurfaceProxy::new()),
            infra_bundles,
            "0.0.0-test".to_string(),
        );
        let request_id = uuid::Uuid::now_v7().to_string();
        support.pending_config_reports().lock().insert(
            request_id.clone(),
            PendingConfigReport {
                host_id,
                // Deliberately does not match "infrastructure.proxmox": the
                // dispatch loop must never call proxmox's
                // `on_plugin_config_reported` for this ack.
                plugin_type: "infrastructure.some-other-plugin".to_string(),
            },
        );

        support
            .handle_report_plugin_config_response(
                response_payload(&request_id, true, Some(uuid::Uuid::now_v7()), None)
                    .expect("ReportPluginConfigResponsePayload JSON is always valid"),
            )
            .await;

        // Same crate-boundary limitation as the seed insert above:
        // `proxmox_host_state` is unreachable from this crate, so the
        // row is read back via raw SQL too. query_one_raw with a Statement
        // is the approved exception for raw SQL.
        let row = db
            .query_one_raw(Statement::from_sql_and_values(
                sea_orm::DatabaseBackend::Sqlite,
                "SELECT new_pve_plugin_config_id FROM proxmox_host_state WHERE host_id = $1",
                [host_id.to_string().into()],
            ))
            .await
            .expect("query seeded row")
            .expect("seeded row must still exist");
        let new_config_id: Option<String> = row
            .try_get_by_index(0)
            .expect("new_pve_plugin_config_id column");
        assert_eq!(
            new_config_id, None,
            "a mismatched plugin_type ack must never write proxmox host state"
        );
    }

    #[tokio::test]
    async fn reset_data_truncates_proxmox_host_state_and_ssh_hosts() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should connect");
        crate::db::migration::run_migrations(&db)
            .await
            .expect("migrations should run against the in-memory db");

        let host_id = uuid::Uuid::now_v7();
        let now = "2026-01-01T00:00:00Z";
        // Same crate-boundary limitation as
        // `mismatched_plugin_type_ack_does_not_write_proxmox_state`:
        // `proxmox_host_state` is unreachable from this crate, so the seed
        // row is written via raw SQL. execute_raw with a Statement is the
        // approved exception for raw SQL.
        db.execute_raw(Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Sqlite,
            "INSERT INTO proxmox_host_state (host_id, created_at, updated_at) VALUES ($1, $2, $2)",
            [host_id.to_string().into(), now.into()],
        ))
        .await
        .expect("seed proxmox_host_state row");

        // `ssh_hosts` is nameable from this crate, so seed it via the
        // ordinary `host_ops::add_host` insert path instead of raw SQL.
        // Both are no-ops once an earlier test in this process has run them.
        let _ignored = uptrakit_crypto::init_master_key(zeroize::Zeroizing::new([0x24u8; 32]));
        let _ignored = uptrakit_crypto::register_column_aad(&[uptrakit_crypto::ColumnAadEntry {
            table: "ssh_hosts",
            column: "private_key",
            aad: "uptrakit:ssh_hosts:private_key",
        }]);
        let encrypted_key = uptrakit_crypto::EncryptedString::new(
            "test-key-content".to_string(),
            "uptrakit:ssh_hosts:private_key",
        )
        .expect("master key initialized above");
        crate::host_ops::add_host(
            &db,
            crate::host_ops::AddHostParams {
                host_id: uuid::Uuid::now_v7(),
                name: "reset-data-target".to_string(),
                hostname: "reset-data-target.example.test".to_string(),
                port: 22,
                username: "root".to_string(),
                encrypted_key,
                key_type: crate::db::entity::ssh_host::SshKeyType::Ed25519,
                host_key_fingerprint: None,
            },
        )
        .await
        .expect("seed ssh_hosts row");

        let support = AgentSshRuntimeSupport::new(
            db.clone(),
            tempfile::tempdir().expect("tempdir").path().to_path_buf(),
            crate::ssh_pool::SshConnectionPool::new(),
            Arc::new(crate::ServiceSurfaceProxy::new()),
            Arc::new(Vec::new()),
            "0.0.0-test".to_string(),
        );

        let result = support.handle_reset_data().await;

        // `reset_data_impl` (see above) is compiled out behind the
        // `reset-data` feature, so this assertion only exercises the delete
        // path under `--all-features`; without the feature the handler is a
        // no-op and both seeded rows must still be present.
        if cfg!(feature = "reset-data") {
            assert!(
                result,
                "ResetData should report success when the feature is enabled"
            );

            let proxmox_row = db
                .query_one_raw(Statement::from_sql_and_values(
                    sea_orm::DatabaseBackend::Sqlite,
                    "SELECT host_id FROM proxmox_host_state WHERE host_id = $1",
                    [host_id.to_string().into()],
                ))
                .await
                .expect("query proxmox_host_state");
            assert!(
                proxmox_row.is_none(),
                "ResetData must truncate proxmox_host_state"
            );

            let ssh_hosts_count = crate::db::entity::ssh_host::Entity::find()
                .count(&db)
                .await
                .expect("count ssh_hosts");
            assert_eq!(ssh_hosts_count, 0, "ResetData must truncate ssh_hosts");
        } else {
            assert!(
                !result,
                "ResetData should report failure when the feature is disabled"
            );

            let ssh_hosts_count = crate::db::entity::ssh_host::Entity::find()
                .count(&db)
                .await
                .expect("count ssh_hosts");
            assert_eq!(
                ssh_hosts_count, 1,
                "ResetData must not touch ssh_hosts when the feature is disabled"
            );
        }
    }

    #[tokio::test]
    async fn reconnect_clears_stranded_pending_entries() {
        let db = Database::connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite should connect");
        let support = AgentSshRuntimeSupport::new(
            db,
            tempfile::tempdir().expect("tempdir").path().to_path_buf(),
            crate::ssh_pool::SshConnectionPool::new(),
            Arc::new(crate::ServiceSurfaceProxy::new()),
            Arc::new(Vec::new()),
            "0.0.0-test".to_string(),
        );
        // Capture the shared handle before `support` moves into the runtime
        // config below: `SshAgentRuntime::support` is a private field, so
        // this clone is the only way to inspect the map from here after
        // `on_connected` runs.
        let pending = support.pending_config_reports();
        pending.lock().insert(
            uuid::Uuid::now_v7().to_string(),
            PendingConfigReport {
                host_id: uuid::Uuid::now_v7(),
                plugin_type: "infrastructure.proxmox".to_string(),
            },
        );
        assert_eq!(
            pending.lock().len(),
            1,
            "fixture setup: entry must be seeded"
        );

        let config = crate::SshAgentRuntimeConfig::new(
            support,
            tempfile::tempdir().expect("tempdir").path().join("freeze"),
        );
        let mut runtime = crate::SshAgentRuntime::new(config);
        let mut transport = uptrakit_service_sdk::test_support::MockTransport::new();

        runtime
            .on_connected(&mut transport, crate::SshAgentIdentity::default())
            .await
            .expect("on_connected should succeed against a healthy mock transport");

        assert!(
            pending.lock().is_empty(),
            "a reconnect must clear stranded pending-ack entries"
        );
    }
}
