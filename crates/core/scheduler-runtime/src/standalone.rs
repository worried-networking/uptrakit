use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use rootcause::prelude::*;
use sea_orm::{ConnectOptions, Database};
use tokio::sync::mpsc;
use uptrakit_audit_log::{RuntimeAuditEmitter, RuntimeAuditEvent, RuntimeAuditForwarder};
use uptrakit_scheduler_engine::SchedulerNotifier;
use uptrakit_service_sdk::{
    ControllerConnection, LoopError, LoopOutcome, LoopResult, ServiceHandler, ShutdownCause,
    Signal, default_resolve_shutdown,
};
use uptrakit_wire::{
    Capability, ControllerMessage, DisconnectingPayload, RegisterPayload, ServiceMessage,
    payloads::AuditEventPayload,
};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{ManagedSchedulerRuntime, SchedulerRunConfig, SchedulerStopMode};

pub const STANDALONE_SCHEDULER_DIR_NAME: &str = "scheduler";
pub const STANDALONE_SCHEDULER_LABEL: &str = "uptrakit-scheduler service";
pub const STANDALONE_SCHEDULER_APP_NAME: &str = "uptrakit-scheduler";

pub enum StandaloneSchedulerServiceEvent {
    Forward(ServiceMessage),
}

struct SchedulerAuditForwarder {
    tx: mpsc::UnboundedSender<StandaloneSchedulerServiceEvent>,
}

impl SchedulerAuditForwarder {
    fn new(tx: mpsc::UnboundedSender<StandaloneSchedulerServiceEvent>) -> Self {
        Self { tx }
    }
}

impl RuntimeAuditForwarder for SchedulerAuditForwarder {
    fn forward(&self, event: &RuntimeAuditEvent) {
        let message =
            StandaloneSchedulerServiceEvent::Forward(runtime_audit_event_to_service_message(event));
        if self.tx.send(message).is_err() {
            tracing::warn!(
                action = %event.action,
                "dropping runtime audit event: scheduler audit queue is closed"
            );
        }
    }
}

pub struct StandaloneSchedulerHandler {
    poll_interval: Duration,
    runtime: ManagedSchedulerRuntime,
    service_id: Option<Uuid>,
    audit_emitter: RuntimeAuditEmitter,
    service_event_rx: mpsc::UnboundedReceiver<StandaloneSchedulerServiceEvent>,
}

pub fn standalone_scheduler_capabilities() -> BTreeSet<Capability> {
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

impl StandaloneSchedulerHandler {
    pub fn new(poll_interval_secs: u64) -> Self {
        let (service_event_tx, service_event_rx) = mpsc::unbounded_channel();
        let audit_emitter = RuntimeAuditEmitter::with_forwarder(Arc::new(
            SchedulerAuditForwarder::new(service_event_tx),
        ));

        Self {
            poll_interval: Duration::from_secs(poll_interval_secs),
            runtime: ManagedSchedulerRuntime::new(),
            service_id: None,
            audit_emitter,
            service_event_rx,
        }
    }
}

#[async_trait::async_trait]
impl ServiceHandler for StandaloneSchedulerHandler {
    const DIR_NAME: &'static str = STANDALONE_SCHEDULER_DIR_NAME;
    const SERVICE_LABEL: &'static str = STANDALONE_SCHEDULER_LABEL;
    const SERVICE_APP_NAME: &'static str = STANDALONE_SCHEDULER_APP_NAME;

    type ServiceEvent = StandaloneSchedulerServiceEvent;

    async fn on_connected(
        &mut self,
        conn: &mut ControllerConnection,
        identity: &uptrakit_service_sdk::ServiceIdentityState,
    ) -> LoopResult<()> {
        conn.send(ServiceMessage::Register(RegisterPayload::new(
            standalone_scheduler_capabilities(),
        )))
        .await
        .context_to::<LoopError>()?;

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

                let service_id = self.service_id.ok_or_else(|| {
                    report!(LoopError::Other(
                        "service_id not available; on_connected must be called first".to_string()
                    ))
                })?;

                let config =
                    build_standalone_runtime_config(creds, self.poll_interval, service_id).await?;
                let audit_log_db = config.db.clone();
                let audit_emitter = self.audit_emitter.clone();

                self.runtime
                    .restart(config, move |scheduler| {
                        scheduler.register(
                            uptrakit_shared_db::entity::scheduled_task::ScheduledTaskType::AuditLogCleanup,
                            Box::new(
                                uptrakit_scheduler_engine::executors::audit_log_cleanup::AuditLogCleanupExecutor::new(
                                    audit_log_db.clone(),
                                    audit_emitter.clone(),
                                ),
                            ),
                        );
                    })
                    .await;
                tracing::info!("scheduler engine started");
                Ok(None)
            }
            _ => Ok(None),
        }
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        standalone_scheduler_capabilities()
    }

    async fn poll_service_event(&mut self) -> Self::ServiceEvent {
        match self.service_event_rx.recv().await {
            Some(event) => event,
            None => std::future::pending().await,
        }
    }

    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>> {
        match event {
            StandaloneSchedulerServiceEvent::Forward(message) => {
                conn.send(message).await.context_to::<LoopError>()?;
                Ok(None)
            }
        }
    }

    async fn on_shutdown(
        &mut self,
        conn: &mut ControllerConnection,
        cause: ShutdownCause,
        _shutdown_timeout: Duration,
    ) -> LoopOutcome {
        let (disconnect_reason, outcome) = default_resolve_shutdown(cause);
        let stop_mode = if matches!(
            cause,
            ShutdownCause::ServerRestarting | ShutdownCause::Signal(Signal::Hangup)
        ) {
            SchedulerStopMode::Drain
        } else {
            SchedulerStopMode::Abort
        };
        self.runtime.stop(stop_mode).await;
        self.drain_service_events(conn).await;

        let _ = conn
            .send(ServiceMessage::Disconnecting(DisconnectingPayload::new(
                disconnect_reason,
            )))
            .await;

        outcome
    }
}

impl StandaloneSchedulerHandler {
    async fn drain_service_events(&mut self, conn: &mut ControllerConnection) {
        while let Ok(StandaloneSchedulerServiceEvent::Forward(message)) =
            self.service_event_rx.try_recv()
        {
            if let Err(error) = conn.send(message).await {
                tracing::debug!(
                    error = %error,
                    "failed to drain scheduler audit event during shutdown"
                );
                break;
            }
        }
    }
}

async fn build_standalone_runtime_config(
    creds: uptrakit_wire::payloads::ServiceCredentialsPayload,
    poll_interval: Duration,
    service_id: Uuid,
) -> LoopResult<SchedulerRunConfig> {
    if let Some(ref hex) = creds.master_key_hex
        && !uptrakit_crypto::master_key_available()
    {
        let key_bytes = uptrakit_shared_types::hex::decode(hex.expose_secret())
            .map_err(|e| report!(LoopError::Other(format!("invalid master key hex: {e}"))))?;
        let key: [u8; 32] = key_bytes.try_into().map_err(|v: Vec<u8>| {
            report!(LoopError::Other(format!(
                "master key must be 32 bytes, got {}",
                v.len()
            )))
        })?;
        uptrakit_crypto::init_master_key(Zeroizing::new(key))
            .map_err(|e| report!(LoopError::Other(format!("init master key: {e}"))))?;
        tracing::info!("master encryption key initialized");
    }

    register_column_aad_mappings();

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
    let db = Database::connect(ConnectOptions::new(db_url.to_string()))
        .await
        .map_err(|e| report!(LoopError::Other(format!("database connection failed: {e}"))))?;
    tracing::info!("database connection established");

    init_data_key_ring(&db).await;

    let nats_url = creds.nats_url.as_ref().ok_or_else(|| {
        report!(LoopError::Other(
            "ServiceCredentials missing nats_url (nats_access capability required)".to_string()
        ))
    })?;
    let nats_conn = uptrakit_nats::NatsConnection::connect(nats_url)
        .await
        .map_err(|e| report!(LoopError::Other(format!("NATS connection failed: {e}"))))?;
    nats_conn
        .ensure_stream()
        .await
        .map_err(|e| report!(LoopError::Other(format!("NATS stream setup failed: {e}"))))?;
    tracing::info!("NATS connection established");

    let notifier: Arc<dyn SchedulerNotifier> =
        Arc::new(NatsSchedulerNotifier::new(nats_conn, service_id));

    Ok(
        SchedulerRunConfig::new(db, service_id, notifier, Box::new(|| false))
            .with_poll_interval(poll_interval),
    )
}

fn register_column_aad_mappings() {
    if !uptrakit_crypto::master_key_available() {
        return;
    }

    use uptrakit_crypto::ColumnAadEntry;

    let entries: &[ColumnAadEntry] = &[
        ColumnAadEntry {
            table: "ca_certificates",
            column: "key_pem",
            aad: "uptrakit:ca_certificates:key_pem",
        },
        ColumnAadEntry {
            table: "oidc_providers",
            column: "client_secret",
            aad: "uptrakit:oidc_providers:client_secret",
        },
        ColumnAadEntry {
            table: "pending_oidc_flows",
            column: "pkce_verifier",
            aad: "uptrakit:pending_oidc_flows:pkce_verifier",
        },
        ColumnAadEntry {
            table: "notification_channels",
            column: "config",
            aad: "uptrakit:notification_channels:config",
        },
    ];

    if let Err(e) = uptrakit_crypto::register_column_aad(entries) {
        tracing::warn!(error = %e, "column AAD registry already initialized (harmless)");
    }
}

async fn init_data_key_ring(db: &sea_orm::DatabaseConnection) {
    use sea_orm::EntityTrait;
    use uptrakit_shared_db::entity::data_encryption_key;

    if !uptrakit_crypto::master_key_available() || uptrakit_crypto::data_key_ring_available() {
        return;
    }

    let kek_fp = match uptrakit_crypto::master_key_fingerprint() {
        Ok(fp) => fp,
        Err(e) => {
            tracing::error!(error = %e, "failed to compute KEK fingerprint");
            return;
        }
    };

    let rows = match data_encryption_key::Entity::find().all(db).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "failed to query data_encryption_keys");
            return;
        }
    };

    if rows.is_empty() {
        tracing::info!("no DEKs found in database — waiting for controller to generate them");
        return;
    }

    let mut keys = std::collections::HashMap::new();
    let mut active_key_id: Option<String> = None;

    for row in &rows {
        if row.kek_fingerprint != kek_fp {
            tracing::error!(
                key_id = %row.key_id,
                stored_fp = %row.kek_fingerprint,
                current_fp = %kek_fp,
                "DEK was wrapped with a different KEK — master key mismatch"
            );
            return;
        }

        let dek = match uptrakit_crypto::unwrap_data_key(&row.wrapped_key, &row.key_id) {
            Ok(d) => d,
            Err(e) => {
                tracing::error!(key_id = %row.key_id, error = %e, "failed to unwrap DEK");
                return;
            }
        };
        keys.insert(dek.key_id.clone(), dek.key);

        if row.status == "active" {
            active_key_id = Some(row.key_id.clone());
        }
    }

    let active = match active_key_id {
        Some(id) => id,
        None => {
            tracing::error!("no active DEK found in data_encryption_keys table");
            return;
        }
    };

    let ring = match uptrakit_crypto::DataKeyRing::new(keys, active.clone()) {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "failed to construct data key ring");
            return;
        }
    };
    if let Err(e) = uptrakit_crypto::init_data_key_ring(ring) {
        tracing::warn!(error = %e, "data key ring already initialized (harmless)");
    } else {
        tracing::info!(active_key_id = %active, count = rows.len(), "data key ring initialized");
    }
}

struct NatsSchedulerNotifier {
    nats: uptrakit_nats::NatsConnection,
    scheduler_id: Uuid,
}

impl NatsSchedulerNotifier {
    fn new(nats: uptrakit_nats::NatsConnection, scheduler_id: Uuid) -> Self {
        Self { nats, scheduler_id }
    }
}

#[async_trait::async_trait]
impl SchedulerNotifier for NatsSchedulerNotifier {
    async fn send_to_service(&self, service_id: &Uuid, msg: ControllerMessage) {
        self.nats
            .publish(self.scheduler_id, Some(*service_id), None, msg)
            .await;
    }

    async fn broadcast(&self, msg: ControllerMessage) {
        self.nats.publish(self.scheduler_id, None, None, msg).await;
    }

    async fn send_by_capability(&self, capability: &str, msg: ControllerMessage) {
        self.nats
            .publish(self.scheduler_id, None, Some(capability), msg)
            .await;
    }

    async fn signal_ca_rotation(&self, reason: &str) {
        tracing::info!(reason, "external scheduler requesting CA rotation via NATS");
        self.nats
            .publish(
                self.scheduler_id,
                None,
                Some("controller"),
                ControllerMessage::RequestCaRotation(uptrakit_wire::RequestCaRotationPayload {
                    reason: reason.to_string(),
                }),
            )
            .await;
    }

    async fn signal_software_states_changed(&self, tenant_id: Uuid) {
        self.nats
            .publish(
                self.scheduler_id,
                None,
                Some("controller"),
                ControllerMessage::SoftwareStatesChanged(
                    uptrakit_wire::SoftwareStatesChangedPayload::new(tenant_id),
                ),
            )
            .await;
    }

    async fn signal_crl_renewal(&self) {
        tracing::info!("external scheduler triggering CRL rebuild via NATS");
        self.nats
            .publish(
                self.scheduler_id,
                None,
                Some("controller"),
                ControllerMessage::RequestCrlRenewal(
                    uptrakit_wire::RequestCrlRenewalPayload::default(),
                ),
            )
            .await;
    }
}

fn runtime_audit_event_to_service_message(event: &RuntimeAuditEvent) -> ServiceMessage {
    ServiceMessage::AuditEvent(AuditEventPayload {
        action_type: event.action.to_string(),
        tenant_id: None,
        target_type: None,
        target_id: None,
        target_display: None,
        outcome: runtime_audit_outcome(event.level).to_string(),
        details_json: Some(event.details.to_string()),
        request_id: None,
    })
}

fn runtime_audit_outcome(level: tracing::Level) -> &'static str {
    match level {
        tracing::Level::ERROR => "failed",
        tracing::Level::WARN => "denied",
        tracing::Level::INFO | tracing::Level::DEBUG | tracing::Level::TRACE => "success",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_wire::ServiceMessage;

    #[test]
    fn standalone_capabilities_match_scheduler_contract() {
        let caps = standalone_scheduler_capabilities();
        assert!(caps.contains(&Capability::SystemService));
        assert!(caps.contains(&Capability::Scheduler));
        assert!(caps.contains(&Capability::DatabaseAccess));
        assert!(caps.contains(&Capability::NatsAccess));
        assert!(caps.contains(&Capability::MasterKeyAccess));
        assert!(caps.contains(&Capability::GracefulShutdown));
        assert_eq!(caps.len(), 6);
    }

    #[test]
    fn standalone_handler_capabilities_match_scheduler_contract() {
        let handler = StandaloneSchedulerHandler::new(15);
        assert_eq!(handler.capabilities(), standalone_scheduler_capabilities());
    }

    #[test]
    fn runtime_audit_warn_maps_to_denied_outcome() {
        assert_eq!(runtime_audit_outcome(tracing::Level::WARN), "denied");
    }

    #[tokio::test]
    async fn poll_service_event_drains_forwarded_runtime_audit_message() {
        let mut handler = StandaloneSchedulerHandler::new(15);

        handler.audit_emitter.scheduler_audit_log_cleanup(5, 4, 90);

        let event = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            handler.poll_service_event(),
        )
        .await
        .expect("expected service event");

        match event {
            StandaloneSchedulerServiceEvent::Forward(message) => match message {
                ServiceMessage::AuditEvent(payload) => {
                    assert_eq!(
                        payload.action_type,
                        "system.scheduler.audit_log_cleanup".to_string()
                    );
                    assert_eq!(payload.outcome, "success".to_string());
                    assert!(
                        payload
                            .details_json
                            .as_deref()
                            .is_some_and(|details| details.contains("\"tenant_deleted\":5"))
                    );
                }
                other => panic!("expected ServiceMessage::AuditEvent, got {other:?}"),
            },
        }
    }

    #[tokio::test]
    async fn poll_service_event_does_not_drop_when_many_runtime_audit_events_are_buffered() {
        let mut handler = StandaloneSchedulerHandler::new(15);
        let event_count = 256_u64;

        for index in 0..event_count {
            handler
                .audit_emitter
                .scheduler_audit_log_cleanup(index, index, 90);
        }

        let mut drained = 0_u64;
        while let Ok(event) = handler.service_event_rx.try_recv() {
            drained += 1;
            match event {
                StandaloneSchedulerServiceEvent::Forward(ServiceMessage::AuditEvent(_)) => {}
                _ => panic!("expected forwarded audit event"),
            }
        }

        assert_eq!(drained, event_count);
    }
}
