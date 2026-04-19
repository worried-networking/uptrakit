use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use uptrakit_agent_core::{
    ConnectionContext, InFlightUpdate, UpdateEvent, handle_graceful_shutdown,
    send_background_result, send_update_output, send_update_result, spawn_background,
};
use uptrakit_audit_log::{RuntimeAuditEmitter, RuntimeAuditEvent, RuntimeAuditForwarder};
use uptrakit_command::{
    CommandExecutor, LocalCommandExecutor, SudoAwareCommandExecutor, SudoContext,
};
use uptrakit_internal_wire::{
    AuditEventPayload, Capability, ControllerMessage, DisconnectReason, RegisterPayload,
    ReportHostsPayload, ServiceMessage, ServiceTransport, SetUpdateFreezePayload, TransportError,
};

/// Minimum interval between accepted update executions on a single agent.
pub const UPDATE_COOLDOWN: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct RuntimeAuditQueueForwarder {
    tx: tokio::sync::mpsc::UnboundedSender<RuntimeAuditEvent>,
}

impl RuntimeAuditQueueForwarder {
    fn new(tx: tokio::sync::mpsc::UnboundedSender<RuntimeAuditEvent>) -> Self {
        Self { tx }
    }
}

impl RuntimeAuditForwarder for RuntimeAuditQueueForwarder {
    fn forward(&self, event: &RuntimeAuditEvent) {
        if self.tx.send(event.clone()).is_err() {
            tracing::warn!(
                audit_action = %event.action,
                "dropping runtime audit event: forwarding queue is closed"
            );
        }
    }
}

fn audit_outcome_for_level(level: tracing::Level) -> &'static str {
    match level {
        tracing::Level::ERROR => "failed",
        tracing::Level::WARN => "denied",
        tracing::Level::INFO | tracing::Level::DEBUG | tracing::Level::TRACE => "success",
    }
}

fn runtime_audit_to_service_message(event: RuntimeAuditEvent) -> ServiceMessage {
    let details_json = Some(event.details.to_string());

    ServiceMessage::AuditEvent(AuditEventPayload {
        action_type: event.action,
        tenant_id: None,
        target_type: None,
        target_id: None,
        target_display: None,
        outcome: audit_outcome_for_level(event.level).to_string(),
        details_json,
        request_id: None,
    })
}

/// Shared agent capability set for both standalone and embedded runners.
pub fn agent_capabilities() -> BTreeSet<Capability> {
    let mut caps = BTreeSet::new();
    caps.insert(Capability::SoftwareDiscovery);
    caps.insert(Capability::UpdateHooks);
    caps.insert(Capability::GracefulShutdown);
    #[cfg(feature = "interactive")]
    caps.insert(Capability::InteractiveUpdates);
    caps
}

/// Build the default local executor used by the standalone and embedded agent.
pub fn make_local_executor() -> Arc<dyn CommandExecutor> {
    let raw: Arc<dyn CommandExecutor> = Arc::new(LocalCommandExecutor);
    Arc::new(SudoAwareCommandExecutor::new(raw, SudoContext::default()))
}

/// Static configuration for [`AgentRuntime`].
pub struct AgentRuntimeConfig {
    executor: Arc<dyn CommandExecutor>,
    freeze_file_path: PathBuf,
    agent_version: String,
    audit_emitter: RuntimeAuditEmitter,
}

impl AgentRuntimeConfig {
    pub fn new(
        executor: Arc<dyn CommandExecutor>,
        freeze_file_path: PathBuf,
        agent_version: String,
    ) -> Self {
        Self::with_audit_emitter(
            executor,
            freeze_file_path,
            agent_version,
            RuntimeAuditEmitter::new(),
        )
    }

    pub fn with_audit_emitter(
        executor: Arc<dyn CommandExecutor>,
        freeze_file_path: PathBuf,
        agent_version: String,
        audit_emitter: RuntimeAuditEmitter,
    ) -> Self {
        Self {
            executor,
            freeze_file_path,
            agent_version,
            audit_emitter,
        }
    }
}

/// Runtime events emitted by [`AgentRuntime::poll_event`].
pub enum AgentRuntimeEvent {
    Update(UpdateEvent),
    BackgroundResult(ServiceMessage),
}

/// Shared single-host agent runtime used by both standalone and embedded adapters.
pub struct AgentRuntime {
    runtime_instance_id: uuid::Uuid,
    machine_id: String,
    in_flight_update: Option<InFlightUpdate>,
    freeze_file_path: PathBuf,
    last_update_accepted: Option<Instant>,
    audit_emitter: RuntimeAuditEmitter,
    executor: Arc<dyn CommandExecutor>,
    audit_rx: tokio::sync::mpsc::UnboundedReceiver<RuntimeAuditEvent>,
    bg_rx: tokio::sync::mpsc::Receiver<ServiceMessage>,
    bg_tx: tokio::sync::mpsc::Sender<ServiceMessage>,
    pending_initial_report: Option<ReportHostsPayload>,
    agent_version: String,
    ctx: ConnectionContext,
}

impl AgentRuntime {
    pub fn new(config: AgentRuntimeConfig) -> Self {
        let AgentRuntimeConfig {
            executor,
            freeze_file_path,
            agent_version,
            audit_emitter,
        } = config;
        let (audit_tx, audit_rx) = tokio::sync::mpsc::unbounded_channel();
        let forwarder: Arc<dyn RuntimeAuditForwarder> =
            Arc::new(RuntimeAuditQueueForwarder::new(audit_tx));
        let (bg_tx, bg_rx) = tokio::sync::mpsc::channel(32);
        Self {
            runtime_instance_id: uuid::Uuid::now_v7(),
            machine_id: String::new(),
            in_flight_update: None,
            freeze_file_path,
            last_update_accepted: None,
            audit_emitter: audit_emitter.with_additional_forwarder(forwarder),
            executor,
            audit_rx,
            bg_rx,
            bg_tx,
            pending_initial_report: None,
            agent_version,
            ctx: ConnectionContext::default(),
        }
    }

    pub fn machine_id(&self) -> Option<&str> {
        if self.machine_id.is_empty() {
            None
        } else {
            Some(self.machine_id.as_str())
        }
    }

    pub async fn on_connected(
        &mut self,
        transport: &mut dyn ServiceTransport,
    ) -> Result<(), TransportError> {
        transport
            .transport_send(ServiceMessage::Register(
                RegisterPayload::new(agent_capabilities())
                    .with_runtime_instance_id(self.runtime_instance_id),
            ))
            .await?;

        if let Some(update) = self.in_flight_update.as_ref() {
            #[cfg(feature = "interactive")]
            let interactive = update.stdin_tx.is_some();
            #[cfg(feature = "interactive")]
            let _ = update;
            #[cfg(not(feature = "interactive"))]
            let interactive = false;

            tracing::debug!(
                update_history_id = %update.update_history_id,
                "re-sending UpdateStarted on reconnect for in-flight update"
            );
            transport
                .transport_send_best_effort(ServiceMessage::UpdateStarted(
                    uptrakit_internal_wire::UpdateStartedPayload {
                        update_history_id: update.update_history_id,
                        from_version: None,
                        interactive,
                    },
                ))
                .await;
        }

        let host_info =
            uptrakit_agent_core::host_info::collect_host_info(self.executor.as_ref()).await;
        self.machine_id = host_info.machine_id.clone();
        self.pending_initial_report = Some(ReportHostsPayload {
            hosts: vec![host_info],
            agent_version: self.agent_version.clone(),
            capabilities: agent_capabilities(),
        });

        Ok(())
    }

    pub async fn send_pending_initial_report(
        &mut self,
        transport: &mut dyn ServiceTransport,
    ) -> Result<bool, TransportError> {
        let Some(payload) = self.pending_initial_report.take() else {
            return Ok(false);
        };

        transport
            .transport_send_auto_paginate(ServiceMessage::ReportHosts(payload))
            .await?;
        Ok(true)
    }

    pub async fn handle_controller_message(
        &mut self,
        msg: ControllerMessage,
        transport: &mut dyn ServiceTransport,
    ) {
        match msg {
            ControllerMessage::CheckVersions(payload) => {
                if self.machine_id_matches("CheckVersions", &payload.host_machine_id) {
                    let executor = Arc::clone(&self.executor);
                    let ctx = self.ctx.clone();
                    spawn_background(&self.bg_tx, async move {
                        uptrakit_agent_core::run_check_versions(payload, executor, &ctx).await
                    });
                }
            }
            ControllerMessage::ExecuteUpdate(payload) => {
                if self.machine_id_matches("ExecuteUpdate", &payload.host_machine_id)
                    && !self.execution_frozen("ExecuteUpdate").await
                    && self.accept_update_execution("ExecuteUpdate")
                {
                    uptrakit_agent_core::handle_execute_update(
                        *payload,
                        Arc::clone(&self.executor),
                        &mut self.in_flight_update,
                        transport,
                        &self.ctx,
                    )
                    .await;
                }
            }
            ControllerMessage::DiscoverSoftware(payload) => {
                if self.machine_id_matches("DiscoverSoftware", &payload.host_machine_id) {
                    let executor = Arc::clone(&self.executor);
                    let ctx = self.ctx.clone();
                    spawn_background(&self.bg_tx, async move {
                        uptrakit_agent_core::run_discover_software(payload, executor, &ctx).await
                    });
                }
            }
            ControllerMessage::ExecuteBatchUpdate(payload) => {
                if self.machine_id_matches("ExecuteBatchUpdate", &payload.host_machine_id)
                    && !self.execution_frozen("ExecuteBatchUpdate").await
                    && self.accept_update_execution("ExecuteBatchUpdate")
                {
                    let executor = Arc::clone(&self.executor);
                    let ctx = self.ctx.clone();
                    spawn_background(&self.bg_tx, async move {
                        uptrakit_agent_core::run_execute_batch_update(*payload, executor, &ctx)
                            .await
                    });
                }
            }
            ControllerMessage::SetUpdateFreeze(payload) => {
                handle_set_update_freeze(&self.freeze_file_path, payload, &self.audit_emitter)
                    .await;
            }
            ControllerMessage::TestPluginConfig(payload) => {
                if self.machine_id_matches("TestPluginConfig", &payload.host_machine_id) {
                    let executor = Arc::clone(&self.executor);
                    spawn_background(&self.bg_tx, async move {
                        uptrakit_agent_core::config_test::run_config_test(payload, executor).await
                    });
                }
            }
            #[cfg(feature = "interactive")]
            ControllerMessage::UpdateStdinData(payload) => {
                handle_update_stdin_data(payload, &self.in_flight_update);
            }
            _ => {}
        }
        self.drain_audit_events(transport).await;
    }

    pub async fn poll_event(&mut self) -> AgentRuntimeEvent {
        tokio::select! {
            biased;
            event = poll_in_flight_update(&mut self.in_flight_update) => {
                AgentRuntimeEvent::Update(event)
            }
            Some(msg) = self.bg_rx.recv() => {
                AgentRuntimeEvent::BackgroundResult(msg)
            }
        }
    }

    pub async fn handle_event(
        &mut self,
        event: AgentRuntimeEvent,
        transport: &mut dyn ServiceTransport,
    ) -> Option<uptrakit_agent_core::LoopOutcome> {
        let mut outcome = None;
        match event {
            AgentRuntimeEvent::Update(UpdateEvent::Output(output_msg)) => {
                let Some(update) = self.in_flight_update.as_ref() else {
                    tracing::error!("received update output but no in-flight update exists");
                    self.drain_audit_events(transport).await;
                    return outcome;
                };
                send_update_output(transport, update.update_history_id, output_msg).await;
            }
            AgentRuntimeEvent::Update(UpdateEvent::Completed(result)) => {
                let Some(update) = self.in_flight_update.take() else {
                    tracing::error!("received update completion but no in-flight update exists");
                    self.drain_audit_events(transport).await;
                    return outcome;
                };
                if let Err(error) =
                    send_update_result(transport, update.update_history_id, result).await
                {
                    tracing::error!(error = %error, "failed to send update result");
                    outcome = Some(uptrakit_agent_core::LoopOutcome::Disconnected);
                }
            }
            AgentRuntimeEvent::Update(UpdateEvent::Attention(update_history_id)) => {
                #[cfg(feature = "interactive")]
                {
                    transport
                        .transport_send_best_effort(ServiceMessage::StdinAttention(
                            uptrakit_internal_wire::StdinAttentionPayload::new(update_history_id),
                        ))
                        .await;
                }
                #[cfg(not(feature = "interactive"))]
                {
                    let _ = update_history_id;
                }
            }
            AgentRuntimeEvent::BackgroundResult(msg) => {
                outcome = send_background_result(transport, msg).await;
            }
        }
        self.drain_audit_events(transport).await;
        outcome
    }

    pub async fn drain_background_results(&mut self, transport: &mut dyn ServiceTransport) {
        while let Ok(msg) = self.bg_rx.try_recv() {
            transport.transport_send_best_effort(msg).await;
        }
        self.drain_audit_events(transport).await;
    }

    pub async fn shutdown(
        &mut self,
        transport: &mut dyn ServiceTransport,
        shutdown_timeout: Duration,
        disconnect_reason: DisconnectReason,
        outcome: uptrakit_agent_core::LoopOutcome,
    ) -> uptrakit_agent_core::LoopOutcome {
        self.drain_background_results(transport).await;
        handle_graceful_shutdown(
            transport,
            self.in_flight_update.take(),
            shutdown_timeout,
            disconnect_reason,
            outcome,
        )
        .await
    }

    fn machine_id_matches(&self, message_name: &str, host_machine_id: &str) -> bool {
        if host_machine_id == self.machine_id {
            return true;
        }

        self.audit_emitter.machine_id_validate(
            message_name,
            &self.machine_id,
            host_machine_id,
            false,
        );
        false
    }

    async fn execution_frozen(&self, message_name: &str) -> bool {
        if !is_frozen(&self.freeze_file_path).await {
            return false;
        }

        self.audit_emitter.update_gate(
            message_name,
            "freeze",
            None,
            Some(&self.freeze_file_path),
            None,
            None,
        );
        true
    }

    fn accept_update_execution(&mut self, message_name: &str) -> bool {
        if let Some(last) = self.last_update_accepted
            && last.elapsed() < UPDATE_COOLDOWN
        {
            self.audit_emitter.update_gate(
                message_name,
                "cooldown",
                None,
                Some(&self.freeze_file_path),
                Some(UPDATE_COOLDOWN.as_secs()),
                Some(last.elapsed().as_millis() as u64),
            );
            return false;
        }

        self.last_update_accepted = Some(Instant::now());
        true
    }

    async fn drain_audit_events(&mut self, transport: &mut dyn ServiceTransport) {
        while let Ok(event) = self.audit_rx.try_recv() {
            transport
                .transport_send_best_effort(runtime_audit_to_service_message(event))
                .await;
        }
    }
}

async fn is_frozen(freeze_file_path: &Path) -> bool {
    tokio::fs::try_exists(freeze_file_path)
        .await
        .unwrap_or(false)
}

async fn handle_set_update_freeze(
    freeze_file_path: &Path,
    payload: SetUpdateFreezePayload,
    audit_emitter: &RuntimeAuditEmitter,
) {
    let reason = payload.reason.as_deref().unwrap_or("(no reason given)");
    if payload.enabled {
        if let Some(parent) = freeze_file_path.parent()
            && let Err(error) = tokio::fs::create_dir_all(parent).await
        {
            tracing::error!(
                freeze_file = %freeze_file_path.display(),
                error = %error,
                "failed to create freeze file directory"
            );
            emit_update_freeze_apply_failure(
                audit_emitter,
                freeze_file_path,
                true,
                reason,
                "create_directory",
                &error,
            );
            return;
        }
        match tokio::fs::write(freeze_file_path, "").await {
            Ok(()) => {
                audit_emitter.update_freeze_apply(freeze_file_path, true, reason);
            }
            Err(error) => {
                tracing::error!(
                    freeze_file = %freeze_file_path.display(),
                    error = %error,
                    "failed to create freeze file"
                );
                emit_update_freeze_apply_failure(
                    audit_emitter,
                    freeze_file_path,
                    true,
                    reason,
                    "write_file",
                    &error,
                );
            }
        }
    } else {
        match tokio::fs::remove_file(freeze_file_path).await {
            Ok(()) => {
                audit_emitter.update_freeze_apply(freeze_file_path, false, reason);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!(
                    freeze_file = %freeze_file_path.display(),
                    "freeze file did not exist; no action taken"
                );
            }
            Err(error) => {
                tracing::error!(
                    freeze_file = %freeze_file_path.display(),
                    error = %error,
                    "failed to remove freeze file"
                );
                emit_update_freeze_apply_failure(
                    audit_emitter,
                    freeze_file_path,
                    false,
                    reason,
                    "remove_file",
                    &error,
                );
            }
        }
    }
}

fn emit_update_freeze_apply_failure(
    audit_emitter: &RuntimeAuditEmitter,
    freeze_file_path: &Path,
    enabled: bool,
    reason: &str,
    operation: &str,
    error: &std::io::Error,
) {
    let details = format!(
        "enabled={enabled} freeze_file={} reason={reason} operation={operation} error={error}",
        freeze_file_path.display()
    );
    audit_emitter.emit(
        "system.service.update_freeze.apply",
        tracing::Level::ERROR,
        details.into(),
    );
}

async fn poll_in_flight_update(in_flight_update: &mut Option<InFlightUpdate>) -> UpdateEvent {
    let Some(update) = in_flight_update else {
        return std::future::pending().await;
    };

    #[cfg(feature = "interactive")]
    let mut attention_rx = update.attention_rx.take();
    #[cfg(feature = "interactive")]
    let update_history_id = update.update_history_id;

    #[cfg(feature = "interactive")]
    let event = tokio::select! {
        biased;
        Some(output_msg) = update.output_rx.recv() => UpdateEvent::Output(output_msg),
        result = &mut update.handle => UpdateEvent::Completed(result),
        Some(()) = recv_attention_rx(&mut attention_rx) => UpdateEvent::Attention(update_history_id),
    };

    #[cfg(not(feature = "interactive"))]
    let event = tokio::select! {
        biased;
        Some(output_msg) = update.output_rx.recv() => UpdateEvent::Output(output_msg),
        result = &mut update.handle => UpdateEvent::Completed(result),
    };

    #[cfg(feature = "interactive")]
    {
        update.attention_rx = attention_rx;
    }

    event
}

#[cfg(feature = "interactive")]
async fn recv_attention_rx(
    attention_rx: &mut Option<tokio::sync::mpsc::Receiver<()>>,
) -> Option<()> {
    if let Some(rx) = attention_rx {
        return rx.recv().await;
    }
    std::future::pending().await
}

#[cfg(feature = "interactive")]
fn handle_update_stdin_data(
    payload: uptrakit_internal_wire::UpdateStdinDataPayload,
    in_flight_update: &Option<InFlightUpdate>,
) {
    let Some(update) = in_flight_update else {
        tracing::debug!(
            update_id = %payload.update_history_id,
            "received UpdateStdinData but no in-flight update exists; ignoring"
        );
        return;
    };
    if update.update_history_id != payload.update_history_id {
        tracing::debug!(
            expected = %update.update_history_id,
            received = %payload.update_history_id,
            "UpdateStdinData update_history_id mismatch; ignoring"
        );
        return;
    }

    if let Some(signal) = payload.signal {
        if let Some(signal_tx) = update.signal_tx.as_ref() {
            if signal_tx.try_send(signal).is_err() {
                tracing::warn!("signal channel full or closed; dropping signal {signal}");
            }
        } else {
            tracing::debug!("signal_tx not available for this update; ignoring signal");
        }
    } else if let Some(stdin_tx) = update.stdin_tx.as_ref() {
        use base64::Engine as _;
        match base64::engine::general_purpose::STANDARD.decode(&payload.data) {
            Ok(bytes) => {
                if stdin_tx.try_send(bytes).is_err() {
                    tracing::warn!("stdin channel full or closed; dropping stdin data");
                }
            }
            Err(error) => {
                tracing::warn!(error = %error, "failed to decode base64 stdin data");
            }
        }
    } else {
        tracing::debug!("stdin_tx not available for this update; ignoring stdin data");
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use serde_json::json;
    use uptrakit_audit_log::{RuntimeAuditEmitter, RuntimeAuditEvent, RuntimeAuditForwarder};
    use uptrakit_command::NoopCommandExecutor;
    use uptrakit_internal_wire::CheckVersionsPayload;
    use uptrakit_service_sdk::test_support::MockTransport;

    use super::*;

    fn runtime_config(
        freeze_file_path: PathBuf,
        audit_emitter: RuntimeAuditEmitter,
    ) -> AgentRuntimeConfig {
        AgentRuntimeConfig::with_audit_emitter(
            Arc::new(NoopCommandExecutor),
            freeze_file_path,
            "test-agent-version".to_string(),
            audit_emitter,
        )
    }

    fn forwarded_audit_events(
        transport: &MockTransport,
    ) -> Vec<uptrakit_internal_wire::AuditEventPayload> {
        transport
            .send_log()
            .iter()
            .filter_map(|message| match message {
                ServiceMessage::AuditEvent(payload) => Some(payload.clone()),
                _ => None,
            })
            .collect()
    }

    #[derive(Default)]
    struct RecordingForwarder {
        events: Mutex<Vec<RuntimeAuditEvent>>,
    }

    impl RecordingForwarder {
        fn events(&self) -> Vec<RuntimeAuditEvent> {
            self.events.lock().expect("lock").clone()
        }
    }

    impl RuntimeAuditForwarder for RecordingForwarder {
        fn forward(&self, event: &RuntimeAuditEvent) {
            self.events.lock().expect("lock").push(event.clone());
        }
    }

    #[tokio::test]
    async fn reconnect_replays_update_started_for_existing_in_flight_update() {
        let temp = tempfile::tempdir().expect("tempdir");
        let mut runtime = AgentRuntime::new(runtime_config(
            temp.path().join("update-freeze"),
            RuntimeAuditEmitter::new(),
        ));

        let update_history_id = uuid::Uuid::now_v7();
        let (_output_tx, output_rx) = tokio::sync::mpsc::channel(1);
        runtime.in_flight_update = Some(InFlightUpdate {
            update_history_id,
            handle: tokio::spawn(async {
                std::future::pending::<uptrakit_agent_core::update::UpdateExecutionResult>().await
            }),
            output_rx,
            #[cfg(feature = "interactive")]
            stdin_tx: None,
            #[cfg(feature = "interactive")]
            signal_tx: None,
            #[cfg(feature = "interactive")]
            attention_rx: None,
        });

        let mut transport = MockTransport::new();
        runtime
            .on_connected(&mut transport)
            .await
            .expect("connect should succeed");

        let replay_msg = transport
            .send_log()
            .iter()
            .find_map(|msg| match msg {
                ServiceMessage::UpdateStarted(payload) => Some(payload),
                _ => None,
            })
            .expect("expected UpdateStarted replay after reconnect");

        assert_eq!(replay_msg.update_history_id, update_history_id);
        assert!(!replay_msg.interactive);

        if let Some(update) = runtime.in_flight_update.take() {
            update.handle.abort();
        }
    }

    #[tokio::test]
    async fn machine_id_mismatch_forwards_service_audit_event() {
        let mut runtime = AgentRuntime::new(runtime_config(
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .join("update-freeze"),
            RuntimeAuditEmitter::new(),
        ));
        runtime.machine_id = "machine-1".to_string();

        let mut transport = MockTransport::new();
        runtime
            .handle_controller_message(
                ControllerMessage::CheckVersions(CheckVersionsPayload {
                    host_machine_id: "machine-2".to_string(),
                    assignments: Vec::new(),
                }),
                &mut transport,
            )
            .await;

        let events = forwarded_audit_events(&transport);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action_type, "system.service.machine_id.validate");
        assert_eq!(events[0].outcome, "denied");
        let details = serde_json::from_str::<serde_json::Value>(
            events[0]
                .details_json
                .as_deref()
                .expect("machine-id mismatch should include details"),
        )
        .expect("details should be valid json");
        assert_eq!(details["expected_machine_id"], json!("machine-1"));
        assert_eq!(details["received_machine_id"], json!("machine-2"));
        assert_eq!(details["accepted"], json!(false));
    }

    #[tokio::test]
    async fn freeze_apply_forwards_service_audit_event() {
        let temp = tempfile::tempdir().expect("tempdir");
        let freeze_path = temp.path().join("update-freeze");
        let mut runtime = AgentRuntime::new(runtime_config(
            freeze_path.clone(),
            RuntimeAuditEmitter::new(),
        ));
        let mut transport = MockTransport::new();

        runtime
            .handle_controller_message(
                ControllerMessage::SetUpdateFreeze(SetUpdateFreezePayload {
                    enabled: true,
                    reason: Some("test".to_string()),
                }),
                &mut transport,
            )
            .await;
        assert!(tokio::fs::try_exists(&freeze_path).await.expect("exists"));

        let events = forwarded_audit_events(&transport);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action_type, "system.service.update_freeze.apply");
        assert_eq!(events[0].outcome, "success");
        let details = serde_json::from_str::<serde_json::Value>(
            events[0]
                .details_json
                .as_deref()
                .expect("freeze apply should include details"),
        )
        .expect("details should be valid json");
        assert_eq!(details["enabled"], json!(true));
        assert_eq!(details["reason"], json!("test"));
    }

    #[tokio::test]
    async fn freeze_apply_directory_creation_failure_emits_failed_audit_event() {
        let temp = tempfile::tempdir().expect("tempdir");
        let blocking_parent = temp.path().join("blocking-parent");
        tokio::fs::write(&blocking_parent, "not-a-directory")
            .await
            .expect("blocking parent file");
        let freeze_path = blocking_parent.join("update-freeze");
        let forwarder = Arc::new(RecordingForwarder::default());

        handle_set_update_freeze(
            &freeze_path,
            SetUpdateFreezePayload {
                enabled: true,
                reason: Some("test".to_string()),
            },
            &RuntimeAuditEmitter::with_forwarder(forwarder.clone()),
        )
        .await;

        let events = forwarder.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "system.service.update_freeze.apply");
        assert_eq!(events[0].level, tracing::Level::ERROR);
        let details = events[0]
            .details
            .as_str()
            .expect("failure details should be a string");
        assert!(details.contains("enabled=true"));
        assert!(details.contains("reason=test"));
        assert!(details.contains("operation=create_directory"));
    }

    #[tokio::test]
    async fn freeze_remove_failure_emits_failed_audit_event() {
        let temp = tempfile::tempdir().expect("tempdir");
        let freeze_path = temp.path().join("update-freeze");
        tokio::fs::create_dir(&freeze_path)
            .await
            .expect("freeze path directory");
        let forwarder = Arc::new(RecordingForwarder::default());

        handle_set_update_freeze(
            &freeze_path,
            SetUpdateFreezePayload {
                enabled: false,
                reason: Some("test".to_string()),
            },
            &RuntimeAuditEmitter::with_forwarder(forwarder.clone()),
        )
        .await;

        let events = forwarder.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action, "system.service.update_freeze.apply");
        assert_eq!(events[0].level, tracing::Level::ERROR);
        let details = events[0]
            .details
            .as_str()
            .expect("failure details should be a string");
        assert!(details.contains("enabled=false"));
        assert!(details.contains("reason=test"));
        assert!(details.contains("operation=remove_file"));
    }

    #[tokio::test]
    async fn freeze_remove_not_found_remains_noop_for_audit() {
        let temp = tempfile::tempdir().expect("tempdir");
        let freeze_path = temp.path().join("update-freeze");
        let forwarder = Arc::new(RecordingForwarder::default());

        handle_set_update_freeze(
            &freeze_path,
            SetUpdateFreezePayload {
                enabled: false,
                reason: Some("test".to_string()),
            },
            &RuntimeAuditEmitter::with_forwarder(forwarder.clone()),
        )
        .await;

        assert!(forwarder.events().is_empty());
    }

    #[tokio::test]
    async fn audit_forwarding_does_not_drop_when_many_events_are_buffered() {
        let mut runtime = AgentRuntime::new(runtime_config(
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .join("update-freeze"),
            RuntimeAuditEmitter::new(),
        ));
        let mut transport = MockTransport::new();

        let event_count = 256;
        for _ in 0..event_count {
            runtime
                .audit_emitter
                .update_gate("ExecuteUpdate", "cooldown", None, None, None, None);
        }

        runtime.drain_audit_events(&mut transport).await;

        assert_eq!(forwarded_audit_events(&transport).len(), event_count);
    }

    #[tokio::test]
    async fn injected_audit_emitter_is_preserved_when_runtime_forwarding_is_added() {
        let forwarder = Arc::new(RecordingForwarder::default());
        let mut runtime = AgentRuntime::new(runtime_config(
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .join("update-freeze"),
            RuntimeAuditEmitter::with_forwarder(forwarder.clone()),
        ));
        runtime.machine_id = "machine-1".to_string();

        let mut transport = MockTransport::new();
        runtime
            .handle_controller_message(
                ControllerMessage::CheckVersions(CheckVersionsPayload {
                    host_machine_id: "machine-2".to_string(),
                    assignments: Vec::new(),
                }),
                &mut transport,
            )
            .await;

        assert_eq!(forwarder.events().len(), 1);
        assert_eq!(forwarded_audit_events(&transport).len(), 1);
    }
}
