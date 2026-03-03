//! `ServiceHandler` implementation for the external scheduler binary.
//!
//! On connect, the scheduler waits for `ServiceCredentials` to arrive. Once
//! received, it initialises the database connection, NATS transport, and
//! encryption key, then spawns the scheduler engine loop.

use std::collections::BTreeSet;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Duration;

use rootcause::prelude::*;
use sea_orm::{ConnectOptions, Database};
use tokio_util::sync::CancellationToken;
use uptrakit_internal_wire::{Capability, ControllerMessage, DisconnectingPayload, ServiceMessage};
use uptrakit_scheduler_engine::executors::{
    auth_cleanup::AuthCleanupExecutor, detect_version::DetectVersionExecutor,
    fetch_releases::FetchReleasesExecutor, stale_lease_cleanup::StaleLeaseCleanupExecutor,
};
use uptrakit_scheduler_engine::{Scheduler, SchedulerConfig, SchedulerNotifier};
use uptrakit_service_sdk::{
    ControllerConnection, LoopError, LoopOutcome, LoopResult, ServiceHandler, ShutdownCause,
    default_resolve_shutdown,
};
use uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType;
use uuid::Uuid;

use crate::nats_notifier::NatsSchedulerNotifier;

/// Active scheduler runtime spawned after credential delivery.
struct SchedulerRuntime {
    cancel: CancellationToken,
    scheduler_handle: tokio::task::JoinHandle<()>,
}

/// Maximum time to wait for the scheduler task to finish after cancellation.
const STOP_SCHEDULER_TIMEOUT: Duration = Duration::from_secs(30);

/// `ServiceHandler` for the external scheduler.
pub struct SchedulerHandler {
    pub poll_interval_secs: u64,
    runtime: Option<SchedulerRuntime>,
    /// Stable service ID captured on connect; used as the claim identity so that
    /// a scheduler that reconnects to a different controller reuses the same UUID.
    service_id: Option<Uuid>,
}

impl SchedulerHandler {
    pub fn new(poll_interval_secs: u64) -> Self {
        Self {
            poll_interval_secs,
            runtime: None,
            service_id: None,
        }
    }

    /// Tear down the running scheduler, releasing DB claims.
    ///
    /// Cancels the token and awaits the scheduler task with a timeout to
    /// ensure the old scheduler has fully stopped before a new one starts
    /// (prevents two scheduler instances racing on claim operations).
    async fn stop_scheduler(&mut self) {
        if let Some(rt) = self.runtime.take() {
            tracing::info!("stopping scheduler engine");
            rt.cancel.cancel();
            match tokio::time::timeout(STOP_SCHEDULER_TIMEOUT, rt.scheduler_handle).await {
                Ok(Ok(())) => {
                    tracing::info!("scheduler engine stopped cleanly");
                }
                Ok(Err(join_err)) => {
                    tracing::error!(error = %join_err, "scheduler task panicked during shutdown");
                }
                Err(_elapsed) => {
                    tracing::warn!(
                        timeout_secs = STOP_SCHEDULER_TIMEOUT.as_secs(),
                        "scheduler task did not stop within timeout"
                    );
                }
            }
        }
    }
}

#[async_trait::async_trait]
impl ServiceHandler for SchedulerHandler {
    const DIR_NAME: &'static str = "scheduler";
    const SERVICE_LABEL: &'static str = "uptrakit-scheduler service";

    type ServiceEvent = std::convert::Infallible;

    async fn on_connected(
        &mut self,
        _conn: &mut ControllerConnection,
        identity: &uptrakit_service_sdk::ServiceIdentityState,
    ) -> LoopResult<()> {
        self.service_id = identity.service_id();
        tracing::info!("connected to controller, waiting for ServiceCredentials");
        Ok(())
    }

    async fn on_message(
        &mut self,
        msg: ControllerMessage,
        _conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        match msg {
            ControllerMessage::ServiceCredentials(creds) => {
                tracing::info!(
                    has_db = creds.db_url.is_some(),
                    has_nats = creds.nats_url.is_some(),
                    has_master_key = creds.master_key_hex.is_some(),
                    "received service credentials"
                );

                // If there's already a running scheduler (re-sent credentials),
                // stop the existing one first.
                self.stop_scheduler().await;

                // 1. Initialize master encryption key (if provided and not already set).
                if let Some(ref hex) = creds.master_key_hex
                    && !uptrakit_crypto::master_key_available()
                {
                    let key_bytes = uptrakit_shared_types::hex::decode(hex.expose_secret())
                        .map_err(|e| {
                            report!(LoopError::Other(format!("invalid master key hex: {e}")))
                        })?;
                    let key: [u8; 32] = key_bytes.try_into().map_err(|v: Vec<u8>| {
                        report!(LoopError::Other(format!(
                            "master key must be 32 bytes, got {}",
                            v.len()
                        )))
                    })?;
                    uptrakit_crypto::init_master_key(zeroize::Zeroizing::new(key))
                        .map_err(|e| report!(LoopError::Other(format!("init master key: {e}"))))?;
                    tracing::info!("master encryption key initialized");
                }

                // 2. Connect to database.
                let db_url = creds
                    .db_url
                    .as_ref()
                    .ok_or_else(|| {
                        report!(LoopError::Other(
                            "ServiceCredentials missing db_url (database_access capability required)"
                                .to_string()
                        ))
                    })?
                    .expose_secret();

                let db_opts = ConnectOptions::new(db_url.to_string());
                let db = Database::connect(db_opts).await.map_err(|e| {
                    report!(LoopError::Other(format!("database connection failed: {e}")))
                })?;
                tracing::info!("database connection established");

                // 3. Connect to NATS.
                let nats_url = creds.nats_url.as_ref().ok_or_else(|| {
                    report!(LoopError::Other(
                        "ServiceCredentials missing nats_url (nats_access capability required)"
                            .to_string()
                    ))
                })?;

                let nats_conn = uptrakit_nats::NatsConnection::connect(nats_url)
                    .await
                    .map_err(|e| {
                        report!(LoopError::Other(format!("NATS connection failed: {e}")))
                    })?;
                nats_conn.ensure_stream().await.map_err(|e| {
                    report!(LoopError::Other(format!("NATS stream setup failed: {e}")))
                })?;
                tracing::info!("NATS connection established");

                // 4. Resolve scheduler identity.
                // Use the stable enrolled service ID so the claim identity persists
                // across reconnects (even to a different controller).
                let scheduler_id = self.service_id.ok_or_else(|| {
                    report!(LoopError::Other(
                        "service_id not available; on_connected must be called first".to_string()
                    ))
                })?;

                // 5. Build notifier and scheduler.
                let notifier: Arc<dyn SchedulerNotifier> =
                    Arc::new(NatsSchedulerNotifier::new(nats_conn, scheduler_id));

                let config = SchedulerConfig {
                    poll_interval: Duration::from_secs(self.poll_interval_secs),
                    controller_id: scheduler_id,
                    task_execution_timeout: uptrakit_scheduler_engine::TASK_EXECUTION_TIMEOUT,
                };

                let mut scheduler = Scheduler::new(
                    db.clone(),
                    config,
                    Arc::new(AtomicBool::new(false)),
                );

                // Register all 4 external executors.
                // Internal tasks (CrlRenewal, CaRotationCheck, ServiceCertCheck)
                // run only on the controller's embedded scheduler.
                scheduler.register(
                    ScheduledTaskType::AuthCleanup,
                    Box::new(AuthCleanupExecutor::new(db.clone())),
                );
                scheduler.register(
                    ScheduledTaskType::StaleLeaseCleanup,
                    Box::new(StaleLeaseCleanupExecutor::new(db.clone())),
                );
                scheduler.register(
                    ScheduledTaskType::FetchReleases,
                    Box::new(FetchReleasesExecutor::new(db.clone(), notifier.clone())),
                );
                scheduler.register(
                    ScheduledTaskType::DetectVersion,
                    Box::new(DetectVersionExecutor::new(db, notifier)),
                );

                // 6. Spawn the scheduler loop.
                let cancel = CancellationToken::new();
                let cancel_clone = cancel.clone();
                let handle = tokio::spawn(async move {
                    scheduler.run(cancel_clone).await;
                });

                self.runtime = Some(SchedulerRuntime {
                    cancel,
                    scheduler_handle: handle,
                });

                tracing::info!("scheduler engine started");
                Ok(None)
            }
            _ => {
                tracing::debug!("ignoring unrecognized message in authenticated loop");
                Ok(None)
            }
        }
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        [
            Capability::SystemService,
            Capability::Scheduler,
            Capability::DatabaseAccess,
            Capability::NatsAccess,
            Capability::MasterKeyAccess,
            Capability::GracefulShutdown,
        ]
        .into_iter()
        .collect()
    }

    async fn poll_service_event(&mut self) -> Self::ServiceEvent {
        // No service-specific events — pend forever.
        std::future::pending().await
    }

    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        _conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        match event {}
    }

    async fn on_shutdown(
        &mut self,
        conn: &mut ControllerConnection,
        cause: ShutdownCause,
        _shutdown_timeout_seconds: u32,
    ) -> LoopOutcome {
        let (disconnect_reason, outcome) = default_resolve_shutdown(cause);

        // Stop the scheduler engine (releases DB claims).
        self.stop_scheduler().await;

        // Send disconnect message to the controller.
        let _ = conn
            .send(ServiceMessage::Disconnecting(DisconnectingPayload::new(
                disconnect_reason,
            )))
            .await;

        outcome
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scheduler_handler_capabilities() {
        let handler = SchedulerHandler::new(15);
        let caps = handler.capabilities();
        assert!(caps.contains(&Capability::SystemService));
        assert!(caps.contains(&Capability::Scheduler));
        assert!(caps.contains(&Capability::DatabaseAccess));
        assert!(caps.contains(&Capability::NatsAccess));
        assert!(caps.contains(&Capability::MasterKeyAccess));
        assert!(caps.contains(&Capability::GracefulShutdown));
        assert!(!caps.contains(&Capability::CaManagement), "CaManagement is internal-only");
        assert_eq!(caps.len(), 6);
    }
}
