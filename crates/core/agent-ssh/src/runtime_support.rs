use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::DatabaseConnection;
use uptrakit_agent_ssh_runtime::{
    HostSnapshot, RuntimeSessionState, SshAgentRuntimeSupport, SshInFlightUpdate,
};
use uptrakit_internal_wire::extension::{ExtensionActionsPayload, ExtensionRequestPayload};
use uptrakit_internal_wire::{
    CheckVersionsPayload, DiscoverSoftwarePayload, ExecuteBatchUpdatePayload, ExecuteUpdatePayload,
    ReportPluginConfigResponsePayload, ServiceMessage, ServiceTransport, TestPluginConfigPayload,
    TransportError,
};
use uptrakit_plugin_infrastructure_registry::{InfraBundle, agent_infra::InfraPluginContext};

use crate::client::{self, UpdateEvent};
use crate::extension::{self, ExtensionContext, InfraActionInvokerImpl};
use crate::{ServiceExtensionProxy, db, host_ops, operations, ssh_pool};

/// Shared SSH-agent support implementation reused by the standalone and embedded wrappers.
pub struct AgentSshRuntimeSupport {
    db: DatabaseConnection,
    state_dir: PathBuf,
    pool: ssh_pool::SshConnectionPool,
    extension_proxy: Arc<ServiceExtensionProxy>,
    infra_bundles: Arc<Vec<InfraBundle>>,
    persist_tenant_id: bool,
}

impl AgentSshRuntimeSupport {
    pub fn new(
        db: DatabaseConnection,
        state_dir: PathBuf,
        pool: ssh_pool::SshConnectionPool,
        extension_proxy: Arc<ServiceExtensionProxy>,
        infra_bundles: Arc<Vec<InfraBundle>>,
        persist_tenant_id: bool,
    ) -> Self {
        Self {
            db,
            state_dir,
            pool,
            extension_proxy,
            infra_bundles,
            persist_tenant_id,
        }
    }

    pub fn pool(&self) -> &ssh_pool::SshConnectionPool {
        &self.pool
    }

    fn build_catalog() -> uptrakit_plugin_infrastructure_registry::PluginCatalog {
        let catalog_config = uptrakit_plugin_infrastructure_registry::CatalogConfig::default();
        uptrakit_plugin_infrastructure_registry::build_catalog(&catalog_config)
            .expect("plugin catalog must build successfully")
    }

    async fn reset_data_impl(&self) -> bool {
        if cfg!(feature = "reset-data") {
            tracing::info!("received ResetData: truncating local data stores");
            use sea_orm::{ConnectionTrait, EntityTrait, TransactionTrait};
            match self.db.begin().await {
                Ok(txn) => {
                    if let Err(error) = db::entity::pending_proxmox_match::Entity::delete_many()
                        .exec(&txn)
                        .await
                    {
                        tracing::error!(error = %error, "failed to truncate pending_proxmox_matches");
                    }
                    if let Err(error) = txn
                        .execute_unprepared("DELETE FROM proxmox_host_state")
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
        extension_proxy: Arc<ServiceExtensionProxy>,
        infra_bundles: Arc<Vec<InfraBundle>>,
        session_state: RuntimeSessionState,
        bg_tx: tokio::sync::mpsc::Sender<ServiceMessage>,
    ) {
        tokio::spawn(async move {
            let action_invoker = InfraActionInvokerImpl::new(&extension_proxy, &bg_tx);
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
        client::report_enrolled_hosts(&self.db, transport, &self.pool).await;
        Ok(())
    }

    async fn register_extensions(
        &self,
        encryption_public_key: Option<String>,
        transport: &mut dyn ServiceTransport,
    ) -> Result<(), TransportError> {
        let catalog = Self::build_catalog();
        let register_payload = extension::build_register_payload(encryption_public_key, &catalog);
        transport
            .transport_send(ServiceMessage::ExtensionRegister(register_payload))
            .await?;

        transport
            .transport_send(ServiceMessage::ExtensionActionsRegister(
                ExtensionActionsPayload::new(extension::build_actions()),
            ))
            .await?;

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
        payload: uptrakit_internal_wire::UpdateStdinDataPayload,
        in_flight_updates: &HashMap<String, SshInFlightUpdate>,
    ) {
        client::handle_update_stdin_data_ssh(payload, in_flight_updates);
    }

    async fn handle_report_plugin_config_response(
        &self,
        payload: ReportPluginConfigResponsePayload,
    ) {
        if payload.success {
            if let Some(config_id) = payload.plugin_config_id {
                let request_id = payload.request_id.clone();
                for bundle in self.infra_bundles.iter() {
                    if let Some(lifecycle) = bundle.lifecycle.as_ref()
                        && let Err(error) = lifecycle
                            .on_plugin_config_reported(&self.db, config_id, &request_id)
                            .await
                    {
                        tracing::warn!(
                            error = %error,
                            plugin_type = %lifecycle.plugin_type_id(),
                            "plugin on_plugin_config_reported failed"
                        );
                    }
                }
            }
        } else {
            tracing::warn!(
                request_id = %payload.request_id,
                error = ?payload.error,
                "plugin config report failed"
            );
        }
    }

    async fn handle_reset_data(&self) -> bool {
        self.reset_data_impl().await
    }

    async fn handle_extension_request(
        &self,
        request: ExtensionRequestPayload,
        session_state: &RuntimeSessionState,
        bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
        transport: &mut dyn ServiceTransport,
    ) {
        let ctx = ExtensionContext {
            db: &self.db,
            state_dir: &self.state_dir,
            private_key_der: session_state.private_key_der.as_deref(),
            service_id: session_state.service_id,
            tenant_id: session_state.tenant_id,
            bg_tx,
            extension_proxy: &self.extension_proxy,
            infra_bundles: Arc::clone(&self.infra_bundles),
        };
        extension::handle_extension_request(request, &ctx, transport).await;
    }

    fn handle_extension_response(
        &self,
        response: uptrakit_internal_wire::extension::ExtensionResponsePayload,
    ) {
        let request_id = response.request_id.clone();
        self.extension_proxy.complete(&request_id, response);
    }

    fn spawn_post_report_hooks(
        &self,
        session_state: &RuntimeSessionState,
        bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    ) {
        Self::spawn_post_report_hooks_impl(
            self.db.clone(),
            self.state_dir.clone(),
            Arc::clone(&self.extension_proxy),
            Arc::clone(&self.infra_bundles),
            session_state.clone(),
            bg_tx.clone(),
        );
    }

    async fn persist_tenant_id(&self, tenant_id: uuid::Uuid) {
        if !self.persist_tenant_id {
            return;
        }

        let mut identity =
            uptrakit_service_sdk::ServiceIdentityState::new_single_dir(&self.state_dir);
        if let Err(error) = identity.load().await {
            tracing::warn!(error = %error, "failed to load identity for tenant_id persistence");
            return;
        }
        if let Err(error) = identity.save_tenant_id(tenant_id).await {
            tracing::warn!(error = %error, "failed to persist tenant_id to service.json");
        }
    }
}
