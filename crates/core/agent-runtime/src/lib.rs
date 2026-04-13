use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use uptrakit_agent_core::{
    ConnectionContext, InFlightUpdate, UpdateEvent, handle_graceful_shutdown,
    send_background_result, send_update_output, send_update_result, spawn_background,
};
use uptrakit_command::{
    CommandExecutor, LocalCommandExecutor, SudoAwareCommandExecutor, SudoContext,
};
use uptrakit_internal_wire::{
    Capability, ControllerMessage, DisconnectReason, RegisterPayload, ReportHostsPayload,
    ServiceMessage, ServiceTransport, SetUpdateFreezePayload, TransportError,
};

/// Minimum interval between accepted update executions on a single agent.
pub const UPDATE_COOLDOWN: Duration = Duration::from_secs(5);

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
}

impl AgentRuntimeConfig {
    pub fn new(
        executor: Arc<dyn CommandExecutor>,
        freeze_file_path: PathBuf,
        agent_version: String,
    ) -> Self {
        Self {
            executor,
            freeze_file_path,
            agent_version,
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
    machine_id: String,
    in_flight_update: Option<InFlightUpdate>,
    freeze_file_path: PathBuf,
    last_update_accepted: Option<Instant>,
    executor: Arc<dyn CommandExecutor>,
    bg_rx: tokio::sync::mpsc::Receiver<ServiceMessage>,
    bg_tx: tokio::sync::mpsc::Sender<ServiceMessage>,
    pending_initial_report: Option<ReportHostsPayload>,
    agent_version: String,
    ctx: ConnectionContext,
}

impl AgentRuntime {
    pub fn new(config: AgentRuntimeConfig) -> Self {
        let (bg_tx, bg_rx) = tokio::sync::mpsc::channel(32);
        Self {
            machine_id: String::new(),
            in_flight_update: None,
            freeze_file_path: config.freeze_file_path,
            last_update_accepted: None,
            executor: config.executor,
            bg_rx,
            bg_tx,
            pending_initial_report: None,
            agent_version: config.agent_version,
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
            .transport_send(ServiceMessage::Register(RegisterPayload::new(
                agent_capabilities(),
            )))
            .await?;

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
                if !self.machine_id_matches("CheckVersions", &payload.host_machine_id) {
                    return;
                }
                let executor = Arc::clone(&self.executor);
                let ctx = self.ctx.clone();
                spawn_background(&self.bg_tx, async move {
                    uptrakit_agent_core::run_check_versions(payload, executor, &ctx).await
                });
            }
            ControllerMessage::ExecuteUpdate(payload) => {
                if !self.machine_id_matches("ExecuteUpdate", &payload.host_machine_id) {
                    return;
                }
                if self.execution_frozen("ExecuteUpdate").await
                    || !self.accept_update_execution("ExecuteUpdate")
                {
                    return;
                }
                uptrakit_agent_core::handle_execute_update(
                    *payload,
                    Arc::clone(&self.executor),
                    &mut self.in_flight_update,
                    transport,
                    &self.ctx,
                )
                .await;
            }
            ControllerMessage::DiscoverSoftware(payload) => {
                if !self.machine_id_matches("DiscoverSoftware", &payload.host_machine_id) {
                    return;
                }
                let executor = Arc::clone(&self.executor);
                let ctx = self.ctx.clone();
                spawn_background(&self.bg_tx, async move {
                    uptrakit_agent_core::run_discover_software(payload, executor, &ctx).await
                });
            }
            ControllerMessage::ExecuteBatchUpdate(payload) => {
                if !self.machine_id_matches("ExecuteBatchUpdate", &payload.host_machine_id) {
                    return;
                }
                if self.execution_frozen("ExecuteBatchUpdate").await
                    || !self.accept_update_execution("ExecuteBatchUpdate")
                {
                    return;
                }
                let executor = Arc::clone(&self.executor);
                let ctx = self.ctx.clone();
                spawn_background(&self.bg_tx, async move {
                    uptrakit_agent_core::run_execute_batch_update(*payload, executor, &ctx).await
                });
            }
            ControllerMessage::SetUpdateFreeze(payload) => {
                handle_set_update_freeze(&self.freeze_file_path, payload).await;
            }
            ControllerMessage::TestPluginConfig(payload) => {
                if !self.machine_id_matches("TestPluginConfig", &payload.host_machine_id) {
                    return;
                }
                let executor = Arc::clone(&self.executor);
                spawn_background(&self.bg_tx, async move {
                    uptrakit_agent_core::config_test::run_config_test(payload, executor).await
                });
            }
            #[cfg(feature = "interactive")]
            ControllerMessage::UpdateStdinData(payload) => {
                handle_update_stdin_data(payload, &self.in_flight_update);
            }
            _ => {}
        }
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
        match event {
            AgentRuntimeEvent::Update(UpdateEvent::Output(output_msg)) => {
                let Some(update) = self.in_flight_update.as_ref() else {
                    tracing::error!("received update output but no in-flight update exists");
                    return None;
                };
                send_update_output(transport, update.update_history_id, output_msg).await;
                None
            }
            AgentRuntimeEvent::Update(UpdateEvent::Completed(result)) => {
                let Some(update) = self.in_flight_update.take() else {
                    tracing::error!("received update completion but no in-flight update exists");
                    return None;
                };
                if let Err(error) =
                    send_update_result(transport, update.update_history_id, result).await
                {
                    tracing::error!(error = %error, "failed to send update result");
                    return Some(uptrakit_agent_core::LoopOutcome::Disconnected);
                }
                None
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
                None
            }
            AgentRuntimeEvent::BackgroundResult(msg) => {
                send_background_result(transport, msg).await
            }
        }
    }

    pub async fn drain_background_results(&mut self, transport: &mut dyn ServiceTransport) {
        while let Ok(msg) = self.bg_rx.try_recv() {
            transport.transport_send_best_effort(msg).await;
        }
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

        tracing::warn!(
            target: "security_audit",
            expected = %self.machine_id,
            received = %host_machine_id,
            "{message_name} machine_id mismatch; ignoring"
        );
        false
    }

    async fn execution_frozen(&self, message_name: &str) -> bool {
        if !is_frozen(&self.freeze_file_path).await {
            return false;
        }

        tracing::warn!(
            target: "security_audit",
            freeze_file = %self.freeze_file_path.display(),
            "{message_name} rejected because update execution is frozen"
        );
        true
    }

    fn accept_update_execution(&mut self, message_name: &str) -> bool {
        if let Some(last) = self.last_update_accepted
            && last.elapsed() < UPDATE_COOLDOWN
        {
            tracing::warn!(
                target: "security_audit",
                cooldown_secs = UPDATE_COOLDOWN.as_secs(),
                elapsed_ms = last.elapsed().as_millis() as u64,
                "{message_name} rejected because update cooldown is active"
            );
            return false;
        }

        self.last_update_accepted = Some(Instant::now());
        true
    }
}

async fn is_frozen(freeze_file_path: &Path) -> bool {
    tokio::fs::try_exists(freeze_file_path)
        .await
        .unwrap_or(false)
}

async fn handle_set_update_freeze(freeze_file_path: &Path, payload: SetUpdateFreezePayload) {
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
            return;
        }
        match tokio::fs::write(freeze_file_path, "").await {
            Ok(()) => {
                tracing::warn!(
                    target: "security_audit",
                    freeze_file = %freeze_file_path.display(),
                    reason,
                    "update freeze enabled via controller"
                );
            }
            Err(error) => {
                tracing::error!(
                    freeze_file = %freeze_file_path.display(),
                    error = %error,
                    "failed to create freeze file"
                );
            }
        }
    } else {
        match tokio::fs::remove_file(freeze_file_path).await {
            Ok(()) => {
                tracing::warn!(
                    target: "security_audit",
                    freeze_file = %freeze_file_path.display(),
                    reason,
                    "update freeze disabled via controller"
                );
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
            }
        }
    }
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
