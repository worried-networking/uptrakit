use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use base64::Engine as _;
use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use uptrakit_audit_log::RuntimeAuditEmitter;
use uptrakit_service_sdk::{
    LoopError, LoopOutcome, LoopResult, ServiceHandler, ServiceIdentityState, ShutdownCause,
    default_resolve_shutdown,
};
use uptrakit_wire::{Capability, ControllerMessage, ServiceTransport};

use crate::runtime_support::AgentSshRuntimeSupport;
use crate::ssh_pool::SshConnectionPool;
use crate::{
    ServiceSurfaceProxy, SshAgentEvent, SshAgentIdentity, SshAgentRuntime, SshAgentRuntimeConfig,
    SshAgentSettings, ssh_agent_capabilities,
};

pub struct AgentSshHandler {
    runtime: SshAgentRuntime<AgentSshRuntimeSupport>,
}

impl AgentSshHandler {
    pub fn new(db: DatabaseConnection, state_dir: PathBuf, agent_version: String) -> Self {
        let catalog_config = uptrakit_plugin_infrastructure_registry::CatalogConfig::default();
        #[expect(
            clippy::expect_used,
            reason = "infallible at startup: catalog construction failures are static \
                      configuration errors that must abort process initialization"
        )]
        let catalog = uptrakit_plugin_infrastructure_registry::build_catalog(
            &catalog_config,
            uptrakit_plugin_infrastructure_registry::InstancePluginStates::all_disabled(),
        )
        .expect("plugin catalog must build successfully");
        let infra_bundles = Arc::new(catalog.create_infra_bundles(&catalog_config));
        let surface_proxy = Arc::new(ServiceSurfaceProxy::new());
        let support = AgentSshRuntimeSupport::new(
            db,
            state_dir.clone(),
            SshConnectionPool::new(),
            surface_proxy,
            infra_bundles,
            agent_version,
        );
        let runtime = SshAgentRuntime::new(SshAgentRuntimeConfig::with_audit_emitter(
            support,
            state_dir.join("update-freeze"),
            RuntimeAuditEmitter::new(),
        ));
        Self { runtime }
    }

    pub fn column_aad_entries() -> &'static [uptrakit_crypto::ColumnAadEntry] {
        &[uptrakit_crypto::ColumnAadEntry {
            table: "ssh_hosts",
            column: "private_key",
            aad: crate::AAD_SSH_PRIVATE_KEY,
        }]
    }
}

#[async_trait::async_trait]
impl ServiceHandler for AgentSshHandler {
    const DIR_NAME: &'static str = "agent-ssh";
    const SERVICE_LABEL: &'static str = "uptrakit-agent-ssh service";
    const SERVICE_APP_NAME: &'static str = "uptrakit-agent-ssh";

    type ServiceEvent = SshAgentEvent;

    async fn on_connected(
        &mut self,
        conn: &mut dyn ServiceTransport,
        identity: &ServiceIdentityState,
    ) -> LoopResult<()> {
        let enc_pub = identity
            .public_key_raw()
            .map(|bytes| base64::engine::general_purpose::STANDARD.encode(bytes));
        self.runtime
            .on_connected(
                conn,
                SshAgentIdentity {
                    service_id: identity.service_id(),
                    private_key_der: identity.private_key_pkcs8_der(),
                    encryption_public_key: enc_pub,
                },
            )
            .await
            .map_err(|error| report!(LoopError::Other(error.to_string())))
    }

    async fn on_message(
        &mut self,
        msg: ControllerMessage,
        conn: &mut dyn ServiceTransport,
    ) -> LoopResult<Option<LoopOutcome>> {
        self.runtime.handle_controller_message(msg, conn).await;
        Ok(None)
    }

    async fn on_settings(
        &mut self,
        settings: &uptrakit_wire::ServiceSettingsPayload,
        conn: &mut dyn ServiceTransport,
        agreed_capabilities: &BTreeSet<Capability>,
    ) {
        if let Err(error) = self
            .runtime
            .apply_settings(
                SshAgentSettings {
                    tenant_id: settings.tenant_id,
                    ui_surfaces_enabled: agreed_capabilities.contains(&Capability::UiSurfaces),
                },
                conn,
            )
            .await
        {
            tracing::warn!(error = %error, "failed to apply SSH agent service settings");
        }
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        ssh_agent_capabilities()
    }

    async fn poll_service_event(&mut self) -> Self::ServiceEvent {
        self.runtime.poll_event().await
    }

    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        conn: &mut dyn ServiceTransport,
    ) -> LoopResult<Option<LoopOutcome>> {
        Ok(self.runtime.handle_event(event, conn).await)
    }

    fn on_surface_action_response(
        &mut self,
        response: uptrakit_wire::surfaces::SurfaceActionResponse,
    ) {
        self.runtime.handle_surface_action_response(response);
    }

    async fn on_surface_action_request(
        &mut self,
        request: uptrakit_wire::surfaces::SurfaceActionRequest,
        conn: &mut dyn ServiceTransport,
    ) -> LoopResult<()> {
        self.runtime
            .handle_controller_message(ControllerMessage::SurfaceActionRequest(request), conn)
            .await;
        Ok(())
    }

    async fn on_shutdown(
        &mut self,
        conn: &mut dyn ServiceTransport,
        cause: ShutdownCause,
        shutdown_timeout: std::time::Duration,
    ) -> LoopOutcome {
        let (disconnect_reason, outcome) = default_resolve_shutdown(cause);
        self.runtime
            .shutdown(conn, shutdown_timeout, disconnect_reason, outcome)
            .await
    }

    #[cfg(feature = "service-migrations")]
    fn service_migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>>
    where
        Self: Sized,
    {
        use sea_orm_migration::MigratorTrait as _;
        crate::db::migration::Migrator::migrations()
    }
}
