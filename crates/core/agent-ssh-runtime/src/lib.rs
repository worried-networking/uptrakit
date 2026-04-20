use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use uptrakit_agent_core::{
    LoopOutcome, UpdateEvent, send_background_result, send_update_output, send_update_result,
};
use uptrakit_audit_log::{RuntimeAuditEmitter, RuntimeAuditEvent, RuntimeAuditForwarder};
use uptrakit_internal_wire::{
    AuditEventPayload, Capability, CheckVersionsPayload, ControllerMessage, DisconnectReason,
    DiscoverSoftwarePayload, ExecuteBatchUpdatePayload, ExecuteUpdatePayload, RegisterPayload,
    ReportPluginConfigResponsePayload, ServiceMessage, ServiceTransport, SetUpdateFreezePayload,
    TestPluginConfigPayload, TransportError, UpdateStartedPayload,
    surfaces::{SurfaceActionRequest, SurfaceActionResponse},
};

pub const HOST_RELOAD_INTERVAL: Duration = Duration::from_secs(10);
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
        action_type: event.action.to_string(),
        tenant_id: None,
        target_type: None,
        target_id: None,
        target_display: None,
        outcome: audit_outcome_for_level(event.level).to_string(),
        details_json,
        request_id: None,
    })
}

pub fn ssh_agent_capabilities() -> BTreeSet<Capability> {
    let mut caps = BTreeSet::new();
    caps.insert(Capability::SoftwareDiscovery);
    caps.insert(Capability::SshRemote);
    caps.insert(Capability::UpdateHooks);
    caps.insert(Capability::UiSurfaces);
    caps.insert(Capability::GracefulShutdown);
    #[cfg(feature = "interactive")]
    caps.insert(Capability::InteractiveUpdates);
    caps
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostSnapshot {
    pub id: uuid::Uuid,
    pub updated_at: time::OffsetDateTime,
}

pub fn diff_host_snapshots(
    prev: &[HostSnapshot],
    curr: &[HostSnapshot],
) -> (Vec<uuid::Uuid>, HashSet<uuid::Uuid>) {
    let prev_map: HashMap<uuid::Uuid, time::OffsetDateTime> = prev
        .iter()
        .map(|snapshot| (snapshot.id, snapshot.updated_at))
        .collect();
    let curr_ids: HashSet<uuid::Uuid> = curr.iter().map(|snapshot| snapshot.id).collect();

    let deleted = prev
        .iter()
        .filter(|snapshot| !curr_ids.contains(&snapshot.id))
        .map(|snapshot| snapshot.id)
        .collect();

    let mut changed = HashSet::new();
    for snapshot in curr {
        match prev_map.get(&snapshot.id) {
            Some(prev_updated_at) if *prev_updated_at != snapshot.updated_at => {
                changed.insert(snapshot.id);
            }
            None => {
                changed.insert(snapshot.id);
            }
            _ => {}
        }
    }

    (deleted, changed)
}

pub async fn handle_set_update_freeze(
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
        uptrakit_audit_log::AuditActionType::SYSTEM_SERVICE_UPDATE_FREEZE_APPLY,
        tracing::Level::ERROR,
        details.into(),
    );
}

pub struct SshInFlightUpdate {
    pub update_history_id: uuid::Uuid,
    pub forwarder: tokio::task::JoinHandle<()>,
    #[cfg(feature = "interactive")]
    pub stdin_tx: Option<tokio::sync::mpsc::Sender<Vec<u8>>>,
    #[cfg(feature = "interactive")]
    pub signal_tx: Option<tokio::sync::mpsc::Sender<i32>>,
}

pub enum SshAgentEvent {
    Update(String, UpdateEvent),
    HostConfigChanged,
    BackgroundResult(ServiceMessage),
}

#[derive(Debug, Clone, Default)]
pub struct SshAgentIdentity {
    pub service_id: Option<uuid::Uuid>,
    pub private_key_der: Option<Vec<u8>>,
    pub encryption_public_key: Option<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SshAgentSettings {
    pub tenant_id: Option<uuid::Uuid>,
    pub ui_surfaces_enabled: bool,
    pub persist_tenant_id: bool,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeSessionState {
    pub service_id: Option<uuid::Uuid>,
    pub tenant_id: Option<uuid::Uuid>,
    pub private_key_der: Option<Vec<u8>>,
}

pub struct SshAgentRuntimeConfig<S> {
    support: S,
    freeze_file_path: PathBuf,
    audit_emitter: RuntimeAuditEmitter,
}

impl<S> SshAgentRuntimeConfig<S> {
    pub fn new(support: S, freeze_file_path: PathBuf) -> Self {
        Self::with_audit_emitter(support, freeze_file_path, RuntimeAuditEmitter::new())
    }

    pub fn with_audit_emitter(
        support: S,
        freeze_file_path: PathBuf,
        audit_emitter: RuntimeAuditEmitter,
    ) -> Self {
        Self {
            support,
            freeze_file_path,
            audit_emitter,
        }
    }
}

#[async_trait]
pub trait SshAgentRuntimeSupport: Send + Sync + 'static {
    async fn report_enrolled_hosts(
        &self,
        transport: &mut dyn ServiceTransport,
    ) -> Result<(), TransportError>;

    async fn register_surfaces(
        &self,
        encryption_public_key: Option<String>,
        session_state: &RuntimeSessionState,
        transport: &mut dyn ServiceTransport,
    ) -> Result<(), TransportError>;

    async fn list_host_snapshots(&self) -> Result<Vec<HostSnapshot>, String>;

    async fn report_hosts_after_config_change(
        &self,
        transport: &mut dyn ServiceTransport,
        changed_ids: &HashSet<uuid::Uuid>,
    ) -> Result<(), TransportError>;

    async fn evict_host(&self, host_id: uuid::Uuid);

    async fn disconnect_all(&self);

    fn spawn_check_versions(
        &self,
        payload: CheckVersionsPayload,
        bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    );

    async fn handle_execute_update(
        &self,
        payload: ExecuteUpdatePayload,
        in_flight_updates: &mut HashMap<String, SshInFlightUpdate>,
        aggregate_tx: &tokio::sync::mpsc::Sender<(String, UpdateEvent)>,
        transport: &mut dyn ServiceTransport,
    );

    fn spawn_execute_batch_update(
        &self,
        payload: ExecuteBatchUpdatePayload,
        bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    );

    fn spawn_discover_software(
        &self,
        payload: DiscoverSoftwarePayload,
        bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    );

    fn spawn_config_test(
        &self,
        payload: TestPluginConfigPayload,
        bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    );

    #[cfg(feature = "interactive")]
    fn handle_update_stdin_data(
        &self,
        payload: uptrakit_internal_wire::UpdateStdinDataPayload,
        in_flight_updates: &HashMap<String, SshInFlightUpdate>,
    );

    async fn handle_report_plugin_config_response(
        &self,
        payload: ReportPluginConfigResponsePayload,
    );

    async fn handle_reset_data(&self) -> bool;

    async fn handle_surface_action_request(
        &self,
        request: SurfaceActionRequest,
        session_state: &RuntimeSessionState,
        bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
        transport: &mut dyn ServiceTransport,
    );

    fn handle_surface_action_response(&self, response: SurfaceActionResponse);

    fn spawn_post_report_hooks(
        &self,
        session_state: &RuntimeSessionState,
        bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    );

    async fn persist_tenant_id(&self, tenant_id: uuid::Uuid);
}

pub struct SshAgentRuntime<S> {
    runtime_instance_id: uuid::Uuid,
    support: S,
    freeze_file_path: PathBuf,
    audit_emitter: RuntimeAuditEmitter,
    session_state: RuntimeSessionState,
    encryption_public_key: Option<String>,
    in_flight_updates: HashMap<String, SshInFlightUpdate>,
    aggregate_rx: tokio::sync::mpsc::Receiver<(String, UpdateEvent)>,
    aggregate_tx: tokio::sync::mpsc::Sender<(String, UpdateEvent)>,
    reload_ticker: Option<tokio::time::Interval>,
    host_snapshot: Vec<HostSnapshot>,
    last_update_per_host: HashMap<String, Instant>,
    audit_rx: tokio::sync::mpsc::UnboundedReceiver<RuntimeAuditEvent>,
    bg_rx: tokio::sync::mpsc::Receiver<ServiceMessage>,
    bg_tx: tokio::sync::mpsc::Sender<ServiceMessage>,
    pending_initial_host_report: bool,
}

impl<S> SshAgentRuntime<S>
where
    S: SshAgentRuntimeSupport,
{
    pub fn new(config: SshAgentRuntimeConfig<S>) -> Self {
        let SshAgentRuntimeConfig {
            support,
            freeze_file_path,
            audit_emitter,
        } = config;
        let (aggregate_tx, aggregate_rx) = tokio::sync::mpsc::channel(64);
        let (bg_tx, bg_rx) = tokio::sync::mpsc::channel(64);
        let (audit_tx, audit_rx) = tokio::sync::mpsc::unbounded_channel();
        let forwarder: Arc<dyn RuntimeAuditForwarder> =
            Arc::new(RuntimeAuditQueueForwarder::new(audit_tx));

        Self {
            runtime_instance_id: uuid::Uuid::now_v7(),
            support,
            freeze_file_path,
            audit_emitter: audit_emitter.with_additional_forwarder(forwarder),
            session_state: RuntimeSessionState::default(),
            encryption_public_key: None,
            in_flight_updates: HashMap::new(),
            aggregate_rx,
            aggregate_tx,
            reload_ticker: None,
            host_snapshot: Vec::new(),
            last_update_per_host: HashMap::new(),
            audit_rx,
            bg_rx,
            bg_tx,
            pending_initial_host_report: false,
        }
    }

    pub async fn on_connected(
        &mut self,
        transport: &mut dyn ServiceTransport,
        identity: SshAgentIdentity,
    ) -> Result<(), TransportError> {
        transport
            .transport_send(ServiceMessage::Register(
                RegisterPayload::new(ssh_agent_capabilities())
                    .with_runtime_instance_id(self.runtime_instance_id),
            ))
            .await?;
        self.session_state.service_id = identity.service_id;
        self.session_state.private_key_der = identity.private_key_der;
        self.encryption_public_key = identity.encryption_public_key;
        self.pending_initial_host_report = true;
        self.host_snapshot.clear();
        self.reload_ticker = None;
        Ok(())
    }

    pub async fn apply_settings(
        &mut self,
        settings: SshAgentSettings,
        transport: &mut dyn ServiceTransport,
    ) -> Result<(), TransportError> {
        if let Some(tenant_id) = settings.tenant_id {
            self.session_state.tenant_id = Some(tenant_id);
            if settings.persist_tenant_id {
                self.support.persist_tenant_id(tenant_id).await;
            }
        }

        if self.pending_initial_host_report {
            self.support.report_enrolled_hosts(transport).await?;
            self.host_snapshot = match self.support.list_host_snapshots().await {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    tracing::warn!(
                        error,
                        "failed to initialize host snapshot; dynamic reload will trigger a full re-report on the first tick"
                    );
                    Vec::new()
                }
            };

            let start = tokio::time::Instant::now() + HOST_RELOAD_INTERVAL;
            let mut ticker = tokio::time::interval_at(start, HOST_RELOAD_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            self.reload_ticker = Some(ticker);
            self.pending_initial_host_report = false;

            for (host_machine_id, update) in &self.in_flight_updates {
                #[cfg(feature = "interactive")]
                let interactive = update.stdin_tx.is_some();
                #[cfg(feature = "interactive")]
                let _ = update;
                #[cfg(not(feature = "interactive"))]
                let interactive = false;

                tracing::debug!(
                    %host_machine_id,
                    update_history_id = %update.update_history_id,
                    "re-sending UpdateStarted on reconnect for in-flight update"
                );
                transport
                    .transport_send_best_effort(ServiceMessage::UpdateStarted(
                        UpdateStartedPayload {
                            update_history_id: update.update_history_id,
                            from_version: None,
                            interactive,
                        },
                    ))
                    .await;
            }

            self.support
                .spawn_post_report_hooks(&self.session_state, &self.bg_tx);
        }

        if settings.ui_surfaces_enabled
            && let Err(error) = self
                .support
                .register_surfaces(
                    self.encryption_public_key.clone(),
                    &self.session_state,
                    transport,
                )
                .await
        {
            tracing::warn!(error = %error, "failed to register UI surfaces");
        }

        Ok(())
    }

    pub async fn handle_controller_message(
        &mut self,
        msg: ControllerMessage,
        transport: &mut dyn ServiceTransport,
    ) {
        match msg {
            ControllerMessage::CheckVersions(payload) => {
                self.support.spawn_check_versions(payload, &self.bg_tx);
            }
            ControllerMessage::ExecuteUpdate(payload) => {
                let allowed = self
                    .is_update_allowed("ExecuteUpdate", &payload.host_machine_id)
                    .await;
                if allowed {
                    self.last_update_per_host
                        .insert(payload.host_machine_id.clone(), Instant::now());
                    self.support
                        .handle_execute_update(
                            *payload,
                            &mut self.in_flight_updates,
                            &self.aggregate_tx,
                            transport,
                        )
                        .await;
                }
            }
            ControllerMessage::ExecuteBatchUpdate(payload) => {
                let allowed = self
                    .is_update_allowed("ExecuteBatchUpdate", &payload.host_machine_id)
                    .await;
                if allowed {
                    self.last_update_per_host
                        .insert(payload.host_machine_id.clone(), Instant::now());
                    self.support
                        .spawn_execute_batch_update(*payload, &self.bg_tx);
                }
            }
            ControllerMessage::DiscoverSoftware(payload) => {
                self.support.spawn_discover_software(payload, &self.bg_tx);
            }
            ControllerMessage::SetUpdateFreeze(payload) => {
                handle_set_update_freeze(&self.freeze_file_path, payload, &self.audit_emitter)
                    .await;
            }
            ControllerMessage::TestPluginConfig(payload) => {
                self.support.spawn_config_test(payload, &self.bg_tx);
            }
            #[cfg(feature = "interactive")]
            ControllerMessage::UpdateStdinData(payload) => {
                self.support
                    .handle_update_stdin_data(payload, &self.in_flight_updates);
            }
            ControllerMessage::ReportPluginConfigResponse(payload) => {
                self.support
                    .handle_report_plugin_config_response(payload)
                    .await;
            }
            ControllerMessage::ResetData => {
                let reset = self.support.handle_reset_data().await;
                if reset {
                    self.host_snapshot.clear();
                    self.last_update_per_host.clear();
                }
            }
            ControllerMessage::SurfaceActionRequest(request) => {
                self.support
                    .handle_surface_action_request(
                        request,
                        &self.session_state,
                        &self.bg_tx,
                        transport,
                    )
                    .await;
            }
            ControllerMessage::SurfaceActionResponse(response) => {
                self.support.handle_surface_action_response(response);
            }
            _ => {}
        }
        self.drain_audit_events(transport).await;
    }

    pub async fn poll_event(&mut self) -> SshAgentEvent {
        tokio::select! {
            biased;
            (host_machine_id, event) = Self::poll_updates(&mut self.aggregate_rx, &self.in_flight_updates) => {
                SshAgentEvent::Update(host_machine_id, event)
            }
            Some(msg) = self.bg_rx.recv() => {
                SshAgentEvent::BackgroundResult(msg)
            }
            _ = Self::poll_reload_tick(&mut self.reload_ticker) => {
                SshAgentEvent::HostConfigChanged
            }
        }
    }

    pub async fn handle_event(
        &mut self,
        event: SshAgentEvent,
        transport: &mut dyn ServiceTransport,
    ) -> Option<LoopOutcome> {
        let outcome = match event {
            SshAgentEvent::Update(host_machine_id, update_event) => {
                if let Some(update) = self.in_flight_updates.get(&host_machine_id) {
                    let update_history_id = update.update_history_id;

                    match update_event {
                        UpdateEvent::Output(output_msg) => {
                            send_update_output(transport, update_history_id, output_msg).await;
                            None
                        }
                        UpdateEvent::Completed(result) => {
                            if let Err(error) =
                                send_update_result(transport, update_history_id, result).await
                            {
                                tracing::error!(
                                    error = %error,
                                    "failed to send UpdateResult; disconnecting"
                                );
                                self.in_flight_updates.remove(&host_machine_id);
                                Some(LoopOutcome::Disconnected)
                            } else {
                                self.in_flight_updates.remove(&host_machine_id);
                                None
                            }
                        }
                        UpdateEvent::Attention(update_history_id) => {
                            transport
                                .transport_send_best_effort(ServiceMessage::StdinAttention(
                                    uptrakit_internal_wire::StdinAttentionPayload::new(
                                        update_history_id,
                                    ),
                                ))
                                .await;
                            None
                        }
                    }
                } else {
                    tracing::error!(
                        %host_machine_id,
                        "received update event but no in-flight update found for this host"
                    );
                    None
                }
            }
            SshAgentEvent::HostConfigChanged => {
                if transport.is_yielded() {
                    tracing::debug!("SSH agent runtime: yielded, ignoring host config reload");
                    None
                } else {
                    self.handle_host_config_changed(transport).await;
                    None
                }
            }
            SshAgentEvent::BackgroundResult(msg) => send_background_result(transport, msg).await,
        };
        self.drain_audit_events(transport).await;
        outcome
    }

    pub fn handle_surface_action_response(&mut self, response: SurfaceActionResponse) {
        self.support.handle_surface_action_response(response);
    }

    pub async fn shutdown(
        &mut self,
        transport: &mut dyn ServiceTransport,
        shutdown_timeout: Duration,
        disconnect_reason: DisconnectReason,
        outcome: LoopOutcome,
    ) -> LoopOutcome {
        if !self.in_flight_updates.is_empty() {
            let deadline = tokio::time::Instant::now() + shutdown_timeout;
            tracing::info!(
                count = self.in_flight_updates.len(),
                timeout = ?shutdown_timeout,
                "waiting for in-flight SSH updates to complete before shutdown"
            );

            while !self.in_flight_updates.is_empty() {
                tokio::select! {
                    biased;
                    Some((host_machine_id, event)) = self.aggregate_rx.recv() => {
                        if let Some(update) = self.in_flight_updates.get(&host_machine_id) {
                            let update_history_id = update.update_history_id;
                            match event {
                                UpdateEvent::Output(output_msg) => {
                                    send_update_output(transport, update_history_id, output_msg).await;
                                }
                                UpdateEvent::Completed(result) => {
                                    if let Err(error) = send_update_result(transport, update_history_id, result).await {
                                        tracing::warn!(
                                            error = %error,
                                            "failed to send UpdateResult during shutdown"
                                        );
                                    }
                                    self.in_flight_updates.remove(&host_machine_id);
                                }
                                UpdateEvent::Attention(_) => {}
                            }
                        }
                    }
                    _ = tokio::time::sleep_until(deadline) => {
                        tracing::warn!(
                            remaining = self.in_flight_updates.len(),
                            "shutdown timeout reached, abandoning remaining in-flight updates"
                        );
                        for (_, update) in self.in_flight_updates.drain() {
                            transport
                                .transport_send_best_effort(ServiceMessage::UpdateResult(
                                    uptrakit_internal_wire::UpdateResultPayload {
                                        update_history_id: update.update_history_id,
                                        status: uptrakit_internal_wire::UpdateFinalStatus::Failed,
                                        from_version: None,
                                        to_version: None,
                                        output: String::new(),
                                        error: Some(format!(
                                            "Agent shutdown timeout ({}s) reached",
                                            shutdown_timeout.as_secs()
                                        )),
                                    },
                                ))
                                .await;
                            update.forwarder.abort();
                        }
                        break;
                    }
                }
            }

            while let Ok((host_machine_id, event)) = self.aggregate_rx.try_recv() {
                if let Some(update) = self.in_flight_updates.get(&host_machine_id)
                    && let UpdateEvent::Output(output_msg) = event
                {
                    send_update_output(transport, update.update_history_id, output_msg).await;
                }
            }
        }

        if let Err(error) = transport
            .transport_send(ServiceMessage::Disconnecting(
                uptrakit_internal_wire::DisconnectingPayload::new(disconnect_reason),
            ))
            .await
        {
            tracing::debug!(error = %error, "failed to send Disconnecting message");
        }

        self.drain_background_results(transport).await;
        self.support.disconnect_all().await;
        outcome
    }

    pub async fn drain_background_results(&mut self, transport: &mut dyn ServiceTransport) {
        while let Ok(msg) = self.bg_rx.try_recv() {
            transport.transport_send_best_effort(msg).await;
        }
        self.drain_audit_events(transport).await;
    }

    pub async fn poll_updates(
        aggregate_rx: &mut tokio::sync::mpsc::Receiver<(String, UpdateEvent)>,
        in_flight_updates: &HashMap<String, SshInFlightUpdate>,
    ) -> (String, UpdateEvent) {
        if in_flight_updates.is_empty() {
            std::future::pending().await
        } else {
            match aggregate_rx.recv().await {
                Some(event) => event,
                None => std::future::pending().await,
            }
        }
    }

    async fn poll_reload_tick(ticker: &mut Option<tokio::time::Interval>) -> tokio::time::Instant {
        if let Some(ticker) = ticker {
            ticker.tick().await
        } else {
            std::future::pending::<tokio::time::Instant>().await
        }
    }

    async fn is_update_allowed(&self, message_name: &str, host_machine_id: &str) -> bool {
        if tokio::fs::try_exists(&self.freeze_file_path)
            .await
            .unwrap_or(false)
        {
            self.audit_emitter.update_gate(
                message_name,
                "freeze",
                Some(host_machine_id),
                Some(&self.freeze_file_path),
                None,
                None,
            );
            return false;
        }

        if let Some(last_update_at) = self.last_update_per_host.get(host_machine_id)
            && last_update_at.elapsed() < UPDATE_COOLDOWN
        {
            self.audit_emitter.update_gate(
                message_name,
                "cooldown",
                Some(host_machine_id),
                Some(&self.freeze_file_path),
                Some(UPDATE_COOLDOWN.as_secs()),
                Some(last_update_at.elapsed().as_millis() as u64),
            );
            return false;
        }

        true
    }

    async fn handle_host_config_changed(&mut self, transport: &mut dyn ServiceTransport) {
        let current_snapshot = match self.support.list_host_snapshots().await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::warn!(
                    error,
                    "failed to list host snapshots during reload check; skipping"
                );
                return;
            }
        };

        if current_snapshot == self.host_snapshot {
            return;
        }

        let (deleted_ids, changed_ids) =
            diff_host_snapshots(&self.host_snapshot, &current_snapshot);

        for host_id in &deleted_ids {
            self.support.evict_host(*host_id).await;
        }
        for host_id in &changed_ids {
            self.support.evict_host(*host_id).await;
        }

        self.host_snapshot = current_snapshot;

        tracing::info!(
            changed = changed_ids.len(),
            deleted = deleted_ids.len(),
            "host configuration changed — sending updated ReportHosts"
        );

        if let Err(error) = self
            .support
            .report_hosts_after_config_change(transport, &changed_ids)
            .await
        {
            tracing::warn!(
                error = %error,
                "failed to send ReportHosts after host config change"
            );
        }

        self.support
            .spawn_post_report_hooks(&self.session_state, &self.bg_tx);
    }

    async fn drain_audit_events(&mut self, transport: &mut dyn ServiceTransport) {
        while let Ok(event) = self.audit_rx.try_recv() {
            transport
                .transport_send_best_effort(runtime_audit_to_service_message(event))
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::Map;
    use serde_json::json;
    use uptrakit_audit_log::RuntimeAuditEmitter;
    use uptrakit_service_sdk::test_support::MockTransport;
    use uptrakit_shared_types::PluginTypeId;

    use super::*;

    #[derive(Default)]
    struct FakeSupportState {
        calls: Vec<&'static str>,
        fail_register_surfaces: bool,
        surface_request_tenant_ids: Vec<Option<uuid::Uuid>>,
    }

    #[derive(Default, Clone)]
    struct FakeSupport {
        state: Arc<Mutex<FakeSupportState>>,
    }

    impl FakeSupport {
        fn call_count(&self, name: &str) -> usize {
            self.state
                .lock()
                .expect("lock")
                .calls
                .iter()
                .filter(|entry| **entry == name)
                .count()
        }

        fn set_fail_register_surfaces(&self, fail: bool) {
            self.state.lock().expect("lock").fail_register_surfaces = fail;
        }

        fn last_surface_request_tenant_id(&self) -> Option<Option<uuid::Uuid>> {
            self.state
                .lock()
                .expect("lock")
                .surface_request_tenant_ids
                .last()
                .copied()
        }
    }

    #[async_trait]
    impl SshAgentRuntimeSupport for FakeSupport {
        async fn report_enrolled_hosts(
            &self,
            transport: &mut dyn ServiceTransport,
        ) -> Result<(), TransportError> {
            self.state
                .lock()
                .expect("lock")
                .calls
                .push("report_enrolled_hosts");
            transport.transport_send(ServiceMessage::Unknown).await
        }

        async fn register_surfaces(
            &self,
            _encryption_public_key: Option<String>,
            _session_state: &RuntimeSessionState,
            transport: &mut dyn ServiceTransport,
        ) -> Result<(), TransportError> {
            let fail_register_surfaces = {
                let mut state = self.state.lock().expect("lock");
                state.calls.push("register_surfaces");
                state.fail_register_surfaces
            };
            if fail_register_surfaces {
                return Err(TransportError);
            }
            transport.transport_send(ServiceMessage::Unknown).await
        }

        async fn list_host_snapshots(&self) -> Result<Vec<HostSnapshot>, String> {
            self.state
                .lock()
                .expect("lock")
                .calls
                .push("list_host_snapshots");
            Ok(Vec::new())
        }

        async fn report_hosts_after_config_change(
            &self,
            _transport: &mut dyn ServiceTransport,
            _changed_ids: &HashSet<uuid::Uuid>,
        ) -> Result<(), TransportError> {
            self.state
                .lock()
                .expect("lock")
                .calls
                .push("report_hosts_after_config_change");
            Ok(())
        }

        async fn evict_host(&self, _host_id: uuid::Uuid) {
            self.state.lock().expect("lock").calls.push("evict_host");
        }

        async fn disconnect_all(&self) {
            self.state
                .lock()
                .expect("lock")
                .calls
                .push("disconnect_all");
        }

        fn spawn_check_versions(
            &self,
            _payload: CheckVersionsPayload,
            _bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
        ) {
            self.state
                .lock()
                .expect("lock")
                .calls
                .push("spawn_check_versions");
        }

        async fn handle_execute_update(
            &self,
            _payload: ExecuteUpdatePayload,
            _in_flight_updates: &mut HashMap<String, SshInFlightUpdate>,
            _aggregate_tx: &tokio::sync::mpsc::Sender<(String, UpdateEvent)>,
            _transport: &mut dyn ServiceTransport,
        ) {
            self.state
                .lock()
                .expect("lock")
                .calls
                .push("handle_execute_update");
        }

        fn spawn_execute_batch_update(
            &self,
            _payload: ExecuteBatchUpdatePayload,
            _bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
        ) {
            self.state
                .lock()
                .expect("lock")
                .calls
                .push("spawn_execute_batch_update");
        }

        fn spawn_discover_software(
            &self,
            _payload: DiscoverSoftwarePayload,
            _bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
        ) {
            self.state
                .lock()
                .expect("lock")
                .calls
                .push("spawn_discover_software");
        }

        fn spawn_config_test(
            &self,
            _payload: TestPluginConfigPayload,
            _bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
        ) {
            self.state
                .lock()
                .expect("lock")
                .calls
                .push("spawn_config_test");
        }

        #[cfg(feature = "interactive")]
        fn handle_update_stdin_data(
            &self,
            _payload: uptrakit_internal_wire::UpdateStdinDataPayload,
            _in_flight_updates: &HashMap<String, SshInFlightUpdate>,
        ) {
            self.state
                .lock()
                .expect("lock")
                .calls
                .push("handle_update_stdin_data");
        }

        async fn handle_report_plugin_config_response(
            &self,
            _payload: ReportPluginConfigResponsePayload,
        ) {
            self.state
                .lock()
                .expect("lock")
                .calls
                .push("handle_report_plugin_config_response");
        }

        async fn handle_reset_data(&self) -> bool {
            self.state
                .lock()
                .expect("lock")
                .calls
                .push("handle_reset_data");
            true
        }

        async fn handle_surface_action_request(
            &self,
            _request: SurfaceActionRequest,
            session_state: &RuntimeSessionState,
            _bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
            _transport: &mut dyn ServiceTransport,
        ) {
            let mut state = self.state.lock().expect("lock");
            state.calls.push("handle_surface_action_request");
            state
                .surface_request_tenant_ids
                .push(session_state.tenant_id);
        }

        fn handle_surface_action_response(&self, _response: SurfaceActionResponse) {
            self.state
                .lock()
                .expect("lock")
                .calls
                .push("handle_surface_action_response");
        }

        fn spawn_post_report_hooks(
            &self,
            _session_state: &RuntimeSessionState,
            _bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
        ) {
            self.state
                .lock()
                .expect("lock")
                .calls
                .push("spawn_post_report_hooks");
        }

        async fn persist_tenant_id(&self, _tenant_id: uuid::Uuid) {
            self.state
                .lock()
                .expect("lock")
                .calls
                .push("persist_tenant_id");
        }
    }

    fn execute_update_payload(host_machine_id: &str) -> ExecuteUpdatePayload {
        ExecuteUpdatePayload {
            host_machine_id: host_machine_id.to_string(),
            update_history_id: uuid::Uuid::nil(),
            software_item_id: uuid::Uuid::nil(),
            software_item_name: "demo".to_string(),
            to_version: "1.2.3".to_string(),
            detect_version_plugin: None,
            execute_update_plugin: uptrakit_internal_wire::PluginAssignment {
                plugin_type: PluginTypeId::new("generic_shell"),
                package_identifier: "demo".to_string(),
                config: serde_json::json!({}),
            },
            pre_update_hook_plugins: Vec::new(),
            post_update_hook_plugins: Vec::new(),
            release_info: None,
            timeout: Duration::from_secs(30),
            interactive: false,
        }
    }

    fn runtime_config(
        support: FakeSupport,
        freeze_file_path: PathBuf,
        audit_emitter: RuntimeAuditEmitter,
    ) -> SshAgentRuntimeConfig<FakeSupport> {
        SshAgentRuntimeConfig::with_audit_emitter(support, freeze_file_path, audit_emitter)
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
    async fn apply_settings_sends_initial_report_once_and_registers_extensions() {
        let support = FakeSupport::default();
        let support_clone = support.clone();
        let mut runtime = SshAgentRuntime::new(runtime_config(
            support,
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .join("update-freeze"),
            RuntimeAuditEmitter::new(),
        ));
        let mut transport = MockTransport::new();

        runtime
            .on_connected(
                &mut transport,
                SshAgentIdentity {
                    encryption_public_key: Some("pubkey".to_string()),
                    ..SshAgentIdentity::default()
                },
            )
            .await
            .expect("connect");

        runtime
            .apply_settings(
                SshAgentSettings {
                    ui_surfaces_enabled: true,
                    ..SshAgentSettings::default()
                },
                &mut transport,
            )
            .await
            .expect("settings");
        runtime
            .apply_settings(
                SshAgentSettings {
                    ui_surfaces_enabled: true,
                    ..SshAgentSettings::default()
                },
                &mut transport,
            )
            .await
            .expect("settings");

        assert_eq!(support_clone.call_count("report_enrolled_hosts"), 1);
        assert_eq!(support_clone.call_count("register_surfaces"), 2);
        assert_eq!(support_clone.call_count("list_host_snapshots"), 1);
    }

    #[tokio::test]
    async fn register_reuses_runtime_instance_id_across_reconnects() {
        let support = FakeSupport::default();
        let tempdir = tempfile::tempdir().expect("tempdir");
        let mut runtime = SshAgentRuntime::new(runtime_config(
            support,
            tempdir.path().join("update-freeze"),
            RuntimeAuditEmitter::new(),
        ));

        let mut first_transport = MockTransport::new();
        runtime
            .on_connected(&mut first_transport, SshAgentIdentity::default())
            .await
            .expect("first connect");

        let mut second_transport = MockTransport::new();
        runtime
            .on_connected(&mut second_transport, SshAgentIdentity::default())
            .await
            .expect("second connect");

        let first_id = match &first_transport.send_log()[0] {
            ServiceMessage::Register(payload) => payload.runtime_instance_id,
            other => panic!("expected register message, got {other:?}"),
        };
        let second_id = match &second_transport.send_log()[0] {
            ServiceMessage::Register(payload) => payload.runtime_instance_id,
            other => panic!("expected register message, got {other:?}"),
        };

        assert!(
            first_id.is_some(),
            "register must include runtime_instance_id"
        );
        assert_eq!(
            first_id, second_id,
            "runtime_instance_id must remain stable across reconnects",
        );
    }

    #[tokio::test]
    async fn apply_settings_ignores_extension_registration_error() {
        let support = FakeSupport::default();
        support.set_fail_register_surfaces(true);
        let support_clone = support.clone();
        let tempdir = tempfile::tempdir().expect("tempdir");
        let mut runtime = SshAgentRuntime::new(runtime_config(
            support,
            tempdir.path().join("update-freeze"),
            RuntimeAuditEmitter::new(),
        ));
        let mut transport = MockTransport::new();

        runtime
            .on_connected(&mut transport, SshAgentIdentity::default())
            .await
            .expect("connect");

        let result = runtime
            .apply_settings(
                SshAgentSettings {
                    ui_surfaces_enabled: true,
                    ..SshAgentSettings::default()
                },
                &mut transport,
            )
            .await;

        assert!(
            result.is_ok(),
            "extension registration failures must not fail settings application"
        );
        assert_eq!(support_clone.call_count("report_enrolled_hosts"), 1);
        assert_eq!(support_clone.call_count("register_surfaces"), 1);
    }

    #[tokio::test]
    async fn execute_update_is_rate_limited_per_host() {
        let support = FakeSupport::default();
        let support_clone = support.clone();
        let mut runtime = SshAgentRuntime::new(runtime_config(
            support,
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .join("update-freeze"),
            RuntimeAuditEmitter::new(),
        ));
        let mut transport = MockTransport::new();

        runtime
            .handle_controller_message(
                ControllerMessage::ExecuteUpdate(Box::new(execute_update_payload("host-1"))),
                &mut transport,
            )
            .await;
        runtime
            .handle_controller_message(
                ControllerMessage::ExecuteUpdate(Box::new(execute_update_payload("host-1"))),
                &mut transport,
            )
            .await;

        assert_eq!(support_clone.call_count("handle_execute_update"), 1);
    }

    #[tokio::test]
    async fn surface_action_request_routes_to_support_with_session_tenant() {
        let support = FakeSupport::default();
        let support_clone = support.clone();
        let mut runtime = SshAgentRuntime::new(runtime_config(
            support,
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .join("update-freeze"),
            RuntimeAuditEmitter::new(),
        ));
        let mut transport = MockTransport::new();
        let tenant_id = uuid::Uuid::now_v7();

        runtime
            .apply_settings(
                SshAgentSettings {
                    tenant_id: Some(tenant_id),
                    ui_surfaces_enabled: false,
                    persist_tenant_id: false,
                },
                &mut transport,
            )
            .await
            .expect("settings");

        let request = SurfaceActionRequest {
            request_id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::now_v7().to_string(),
            surface_id: uptrakit_internal_wire::surfaces::SurfaceId::new("ssh-agent.hosts")
                .expect("surface id"),
            interaction_id: uptrakit_internal_wire::surfaces::InteractionId::new(
                "bootstrap-connect",
            )
            .expect("interaction id"),
            idempotency_key: uuid::Uuid::now_v7().to_string(),
            target_provider_id: None,
            caller_origin: uptrakit_internal_wire::surfaces::CallerOrigin::BuiltInSystem {
                principal: "test".to_string(),
            },
            params: Map::new(),
            encrypted_sensitive_params: None,
        };

        runtime
            .handle_controller_message(
                ControllerMessage::SurfaceActionRequest(request),
                &mut transport,
            )
            .await;

        assert_eq!(support_clone.call_count("handle_surface_action_request"), 1);
        assert_eq!(
            support_clone.last_surface_request_tenant_id(),
            Some(Some(tenant_id))
        );
    }

    #[tokio::test]
    async fn surface_action_response_routes_to_support() {
        let support = FakeSupport::default();
        let support_clone = support.clone();
        let mut runtime = SshAgentRuntime::new(runtime_config(
            support,
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .join("update-freeze"),
            RuntimeAuditEmitter::new(),
        ));
        let mut transport = MockTransport::new();

        let response = SurfaceActionResponse {
            request_id: uuid::Uuid::now_v7(),
            success: true,
            result: Some(serde_json::json!({ "ok": true })),
            error: None,
        };

        runtime
            .handle_controller_message(
                ControllerMessage::SurfaceActionResponse(response),
                &mut transport,
            )
            .await;

        assert_eq!(
            support_clone.call_count("handle_surface_action_response"),
            1
        );
    }

    #[tokio::test]
    async fn host_config_changed_is_ignored_when_transport_is_yielded() {
        let support = FakeSupport::default();
        let support_clone = support.clone();
        let tempdir = tempfile::tempdir().expect("tempdir");
        let mut runtime = SshAgentRuntime::new(runtime_config(
            support,
            tempdir.path().join("update-freeze"),
            RuntimeAuditEmitter::new(),
        ));
        let mut transport = MockTransport::new();
        transport.set_yielded(true);

        runtime
            .handle_event(SshAgentEvent::HostConfigChanged, &mut transport)
            .await;

        assert_eq!(support_clone.call_count("list_host_snapshots"), 0);
        assert_eq!(
            support_clone.call_count("report_hosts_after_config_change"),
            0
        );
        assert_eq!(support_clone.call_count("spawn_post_report_hooks"), 0);
    }

    #[tokio::test]
    async fn freeze_apply_forwards_service_audit_event() {
        let dir = tempfile::tempdir().expect("tempdir");
        let freeze_path = dir.path().join("update-freeze");
        let mut runtime = SshAgentRuntime::new(runtime_config(
            FakeSupport::default(),
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
        let dir = tempfile::tempdir().expect("tempdir");
        let blocking_parent = dir.path().join("blocking-parent");
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
        let dir = tempfile::tempdir().expect("tempdir");
        let freeze_path = dir.path().join("update-freeze");
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
        let dir = tempfile::tempdir().expect("tempdir");
        let freeze_path = dir.path().join("update-freeze");
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
    async fn freeze_update_gate_forwards_service_audit_event() {
        let dir = tempfile::tempdir().expect("tempdir");
        let freeze_path = dir.path().join("update-freeze");
        tokio::fs::write(&freeze_path, "")
            .await
            .expect("freeze file");

        let support = FakeSupport::default();
        let support_clone = support.clone();
        let mut runtime = SshAgentRuntime::new(runtime_config(
            support,
            freeze_path.clone(),
            RuntimeAuditEmitter::new(),
        ));
        let mut transport = MockTransport::new();

        runtime
            .handle_controller_message(
                ControllerMessage::ExecuteUpdate(Box::new(execute_update_payload("host-1"))),
                &mut transport,
            )
            .await;
        assert_eq!(support_clone.call_count("handle_execute_update"), 0);

        let events = forwarded_audit_events(&transport);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action_type, "system.service.update_gate");
        assert_eq!(events[0].outcome, "denied");
        let details = serde_json::from_str::<serde_json::Value>(
            events[0]
                .details_json
                .as_deref()
                .expect("update gate should include details"),
        )
        .expect("details should be valid json");
        assert_eq!(details["gate"], json!("freeze"));
        assert_eq!(details["host_machine_id"], json!("host-1"));
    }

    #[tokio::test]
    async fn cooldown_update_gate_forwards_service_audit_event() {
        let dir = tempfile::tempdir().expect("tempdir");
        let support = FakeSupport::default();
        let support_clone = support.clone();
        let mut runtime = SshAgentRuntime::new(runtime_config(
            support,
            dir.path().join("update-freeze"),
            RuntimeAuditEmitter::new(),
        ));
        runtime
            .last_update_per_host
            .insert("host-1".to_string(), Instant::now());
        let mut transport = MockTransport::new();

        runtime
            .handle_controller_message(
                ControllerMessage::ExecuteUpdate(Box::new(execute_update_payload("host-1"))),
                &mut transport,
            )
            .await;
        assert_eq!(support_clone.call_count("handle_execute_update"), 0);

        let events = forwarded_audit_events(&transport);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].action_type, "system.service.update_gate");
        assert_eq!(events[0].outcome, "denied");
        let details = serde_json::from_str::<serde_json::Value>(
            events[0]
                .details_json
                .as_deref()
                .expect("update gate should include details"),
        )
        .expect("details should be valid json");
        assert_eq!(details["gate"], json!("cooldown"));
        assert_eq!(details["host_machine_id"], json!("host-1"));
    }

    #[tokio::test]
    async fn batch_update_gate_uses_batch_message_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let freeze_path = dir.path().join("update-freeze");
        tokio::fs::write(&freeze_path, "")
            .await
            .expect("freeze file");

        let mut runtime = SshAgentRuntime::new(runtime_config(
            FakeSupport::default(),
            freeze_path,
            RuntimeAuditEmitter::new(),
        ));
        let mut transport = MockTransport::new();

        runtime
            .handle_controller_message(
                ControllerMessage::ExecuteBatchUpdate(Box::new(ExecuteBatchUpdatePayload {
                    host_machine_id: "host-1".to_string(),
                    batch_id: uuid::Uuid::now_v7(),
                    plugin_type: PluginTypeId::new("generic_shell"),
                    plugin_config: serde_json::json!({}),
                    updates: Vec::new(),
                    pre_update_hook_plugins: Vec::new(),
                    post_update_hook_plugins: Vec::new(),
                    timeout: Duration::from_secs(30),
                    interactive: false,
                })),
                &mut transport,
            )
            .await;

        let events = forwarded_audit_events(&transport);
        assert_eq!(events.len(), 1);
        let details = serde_json::from_str::<serde_json::Value>(
            events[0]
                .details_json
                .as_deref()
                .expect("update gate should include details"),
        )
        .expect("details should be valid json");
        assert_eq!(details["message_name"], json!("ExecuteBatchUpdate"));
    }

    #[tokio::test]
    async fn audit_forwarding_does_not_drop_when_many_events_are_buffered() {
        let mut runtime = SshAgentRuntime::new(runtime_config(
            FakeSupport::default(),
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .join("update-freeze"),
            RuntimeAuditEmitter::new(),
        ));
        let mut transport = MockTransport::new();

        let event_count = 256;
        for _ in 0..event_count {
            runtime.audit_emitter.update_gate(
                "ExecuteUpdate",
                "cooldown",
                Some("host-1"),
                None,
                None,
                None,
            );
        }

        runtime.drain_audit_events(&mut transport).await;

        assert_eq!(forwarded_audit_events(&transport).len(), event_count);
    }

    #[tokio::test]
    async fn injected_audit_emitter_is_preserved_when_runtime_forwarding_is_added() {
        let forwarder = Arc::new(RecordingForwarder::default());
        let mut runtime = SshAgentRuntime::new(runtime_config(
            FakeSupport::default(),
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .join("update-freeze"),
            RuntimeAuditEmitter::with_forwarder(forwarder.clone()),
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

        assert_eq!(forwarder.events().len(), 1);
        assert_eq!(forwarded_audit_events(&transport).len(), 1);
    }

    #[tokio::test]
    async fn poll_updates_parks_when_map_is_empty() {
        let (_tx, mut rx) = tokio::sync::mpsc::channel::<(String, UpdateEvent)>(4);

        let timed_out = tokio::time::timeout(
            Duration::from_millis(50),
            SshAgentRuntime::<FakeSupport>::poll_updates(&mut rx, &HashMap::new()),
        )
        .await
        .is_err();

        assert!(timed_out);
    }

    #[tokio::test]
    async fn poll_updates_returns_event_when_map_is_non_empty() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<(String, UpdateEvent)>(4);
        let mut in_flight_updates = HashMap::new();
        in_flight_updates.insert(
            "host-1".to_string(),
            SshInFlightUpdate {
                update_history_id: uuid::Uuid::nil(),
                forwarder: tokio::spawn(std::future::pending()),
                #[cfg(feature = "interactive")]
                stdin_tx: None,
                #[cfg(feature = "interactive")]
                signal_tx: None,
            },
        );

        let exec_result = tokio::spawn(async {
            uptrakit_agent_core::update::UpdateExecutionResult {
                result: uptrakit_internal_wire::UpdateResultPayload {
                    update_history_id: uuid::Uuid::nil(),
                    status: uptrakit_internal_wire::UpdateFinalStatus::Completed,
                    from_version: None,
                    to_version: None,
                    output: String::new(),
                    error: None,
                },
            }
        })
        .await;

        tx.send(("host-1".to_string(), UpdateEvent::Completed(exec_result)))
            .await
            .expect("send");

        let (host_id, _) = tokio::time::timeout(
            Duration::from_millis(100),
            SshAgentRuntime::<FakeSupport>::poll_updates(&mut rx, &in_flight_updates),
        )
        .await
        .expect("timeout");

        assert_eq!(host_id, "host-1");

        for (_, update) in in_flight_updates.drain() {
            update.forwarder.abort();
        }
    }
}
