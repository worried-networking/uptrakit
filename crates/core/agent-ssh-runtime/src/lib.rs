use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use uptrakit_agent_core::{
    LoopOutcome, UpdateEvent, send_background_result, send_update_output, send_update_result,
};
use uptrakit_internal_wire::extension::{ExtensionRequestPayload, ExtensionResponsePayload};
use uptrakit_internal_wire::{
    Capability, CheckVersionsPayload, ControllerMessage, DisconnectReason, DiscoverSoftwarePayload,
    ExecuteBatchUpdatePayload, ExecuteUpdatePayload, RegisterPayload,
    ReportPluginConfigResponsePayload, ServiceMessage, ServiceTransport, SetUpdateFreezePayload,
    TestPluginConfigPayload, TransportError, UpdateStartedPayload,
};

pub const HOST_RELOAD_INTERVAL: Duration = Duration::from_secs(10);
pub const UPDATE_COOLDOWN: Duration = Duration::from_secs(5);

pub fn ssh_agent_capabilities() -> BTreeSet<Capability> {
    let mut caps = BTreeSet::new();
    caps.insert(Capability::SoftwareDiscovery);
    caps.insert(Capability::SshRemote);
    caps.insert(Capability::UiExtensions);
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

pub async fn handle_set_update_freeze(freeze_file_path: &Path, payload: SetUpdateFreezePayload) {
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
                    "update freeze enabled via remote command"
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
                    "update freeze disabled via remote command"
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
    pub ui_extensions_enabled: bool,
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
}

impl<S> SshAgentRuntimeConfig<S> {
    pub fn new(support: S, freeze_file_path: PathBuf) -> Self {
        Self {
            support,
            freeze_file_path,
        }
    }
}

#[async_trait]
pub trait SshAgentRuntimeSupport: Send + Sync + 'static {
    async fn report_enrolled_hosts(
        &self,
        transport: &mut dyn ServiceTransport,
    ) -> Result<(), TransportError>;

    async fn register_extensions(
        &self,
        encryption_public_key: Option<String>,
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

    async fn handle_extension_request(
        &self,
        request: ExtensionRequestPayload,
        session_state: &RuntimeSessionState,
        bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
        transport: &mut dyn ServiceTransport,
    );

    fn handle_extension_response(&self, response: ExtensionResponsePayload);

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
    session_state: RuntimeSessionState,
    encryption_public_key: Option<String>,
    in_flight_updates: HashMap<String, SshInFlightUpdate>,
    aggregate_rx: tokio::sync::mpsc::Receiver<(String, UpdateEvent)>,
    aggregate_tx: tokio::sync::mpsc::Sender<(String, UpdateEvent)>,
    reload_ticker: Option<tokio::time::Interval>,
    host_snapshot: Vec<HostSnapshot>,
    last_update_per_host: HashMap<String, Instant>,
    bg_rx: tokio::sync::mpsc::Receiver<ServiceMessage>,
    bg_tx: tokio::sync::mpsc::Sender<ServiceMessage>,
    pending_initial_host_report: bool,
}

impl<S> SshAgentRuntime<S>
where
    S: SshAgentRuntimeSupport,
{
    pub fn new(config: SshAgentRuntimeConfig<S>) -> Self {
        let (aggregate_tx, aggregate_rx) = tokio::sync::mpsc::channel(64);
        let (bg_tx, bg_rx) = tokio::sync::mpsc::channel(64);

        Self {
            runtime_instance_id: uuid::Uuid::now_v7(),
            support: config.support,
            freeze_file_path: config.freeze_file_path,
            session_state: RuntimeSessionState::default(),
            encryption_public_key: None,
            in_flight_updates: HashMap::new(),
            aggregate_rx,
            aggregate_tx,
            reload_ticker: None,
            host_snapshot: Vec::new(),
            last_update_per_host: HashMap::new(),
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

        if settings.ui_extensions_enabled
            && let Err(error) = self
                .support
                .register_extensions(self.encryption_public_key.clone(), transport)
                .await
        {
            tracing::warn!(error = %error, "failed to register UI extensions");
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
                if !self.is_update_allowed(&payload.host_machine_id).await {
                    return;
                }
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
            ControllerMessage::ExecuteBatchUpdate(payload) => {
                if !self.is_update_allowed(&payload.host_machine_id).await {
                    return;
                }
                self.last_update_per_host
                    .insert(payload.host_machine_id.clone(), Instant::now());
                self.support
                    .spawn_execute_batch_update(*payload, &self.bg_tx);
            }
            ControllerMessage::DiscoverSoftware(payload) => {
                self.support.spawn_discover_software(payload, &self.bg_tx);
            }
            ControllerMessage::SetUpdateFreeze(payload) => {
                handle_set_update_freeze(&self.freeze_file_path, payload).await;
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
                if self.support.handle_reset_data().await {
                    self.host_snapshot.clear();
                    self.last_update_per_host.clear();
                }
            }
            ControllerMessage::ExtensionRequest(request) => {
                self.support
                    .handle_extension_request(request, &self.session_state, &self.bg_tx, transport)
                    .await;
            }
            ControllerMessage::ExtensionResponse(response) => {
                self.support.handle_extension_response(response);
            }
            _ => {}
        }
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
        match event {
            SshAgentEvent::Update(host_machine_id, update_event) => {
                let Some(update) = self.in_flight_updates.get(&host_machine_id) else {
                    tracing::error!(
                        %host_machine_id,
                        "received update event but no in-flight update found for this host"
                    );
                    return None;
                };
                let update_history_id = update.update_history_id;

                match update_event {
                    UpdateEvent::Output(output_msg) => {
                        send_update_output(transport, update_history_id, output_msg).await;
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
                            return Some(LoopOutcome::Disconnected);
                        }
                        self.in_flight_updates.remove(&host_machine_id);
                    }
                    UpdateEvent::Attention(update_history_id) => {
                        transport
                            .transport_send_best_effort(ServiceMessage::StdinAttention(
                                uptrakit_internal_wire::StdinAttentionPayload::new(
                                    update_history_id,
                                ),
                            ))
                            .await;
                    }
                }

                None
            }
            SshAgentEvent::HostConfigChanged => {
                if transport.is_yielded() {
                    tracing::debug!("SSH agent runtime: yielded, ignoring host config reload");
                    return None;
                }
                self.handle_host_config_changed(transport).await;
                None
            }
            SshAgentEvent::BackgroundResult(msg) => send_background_result(transport, msg).await,
        }
    }

    pub fn handle_extension_response(&mut self, response: ExtensionResponsePayload) {
        self.support.handle_extension_response(response);
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

    async fn is_update_allowed(&self, host_machine_id: &str) -> bool {
        if tokio::fs::try_exists(&self.freeze_file_path)
            .await
            .unwrap_or(false)
        {
            tracing::warn!(
                target: "security_audit",
                freeze_file = %self.freeze_file_path.display(),
                host_machine_id,
                "update rejected because update execution is frozen"
            );
            return false;
        }

        if let Some(last_update_at) = self.last_update_per_host.get(host_machine_id)
            && last_update_at.elapsed() < UPDATE_COOLDOWN
        {
            tracing::warn!(
                target: "security_audit",
                %host_machine_id,
                cooldown_secs = UPDATE_COOLDOWN.as_secs(),
                elapsed_ms = last_update_at.elapsed().as_millis() as u64,
                "update rejected because update cooldown is active"
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
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use uptrakit_service_sdk::test_support::MockTransport;
    use uptrakit_shared_types::PluginTypeId;

    use super::*;

    #[derive(Default)]
    struct FakeSupportState {
        calls: Vec<&'static str>,
        fail_register_extensions: bool,
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

        fn set_fail_register_extensions(&self, fail: bool) {
            self.state.lock().expect("lock").fail_register_extensions = fail;
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

        async fn register_extensions(
            &self,
            _encryption_public_key: Option<String>,
            transport: &mut dyn ServiceTransport,
        ) -> Result<(), TransportError> {
            let fail_register_extensions = {
                let mut state = self.state.lock().expect("lock");
                state.calls.push("register_extensions");
                state.fail_register_extensions
            };
            if fail_register_extensions {
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

        async fn handle_extension_request(
            &self,
            _request: ExtensionRequestPayload,
            _session_state: &RuntimeSessionState,
            _bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
            _transport: &mut dyn ServiceTransport,
        ) {
            self.state
                .lock()
                .expect("lock")
                .calls
                .push("handle_extension_request");
        }

        fn handle_extension_response(&self, _response: ExtensionResponsePayload) {
            self.state
                .lock()
                .expect("lock")
                .calls
                .push("handle_extension_response");
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

    #[tokio::test]
    async fn apply_settings_sends_initial_report_once_and_registers_extensions() {
        let support = FakeSupport::default();
        let support_clone = support.clone();
        let mut runtime = SshAgentRuntime::new(SshAgentRuntimeConfig::new(
            support,
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .join("update-freeze"),
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
                    ui_extensions_enabled: true,
                    ..SshAgentSettings::default()
                },
                &mut transport,
            )
            .await
            .expect("settings");
        runtime
            .apply_settings(
                SshAgentSettings {
                    ui_extensions_enabled: true,
                    ..SshAgentSettings::default()
                },
                &mut transport,
            )
            .await
            .expect("settings");

        assert_eq!(support_clone.call_count("report_enrolled_hosts"), 1);
        assert_eq!(support_clone.call_count("register_extensions"), 2);
        assert_eq!(support_clone.call_count("list_host_snapshots"), 1);
    }

    #[tokio::test]
    async fn register_reuses_runtime_instance_id_across_reconnects() {
        let support = FakeSupport::default();
        let tempdir = tempfile::tempdir().expect("tempdir");
        let mut runtime = SshAgentRuntime::new(SshAgentRuntimeConfig::new(
            support,
            tempdir.path().join("update-freeze"),
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
        support.set_fail_register_extensions(true);
        let support_clone = support.clone();
        let tempdir = tempfile::tempdir().expect("tempdir");
        let mut runtime = SshAgentRuntime::new(SshAgentRuntimeConfig::new(
            support,
            tempdir.path().join("update-freeze"),
        ));
        let mut transport = MockTransport::new();

        runtime
            .on_connected(&mut transport, SshAgentIdentity::default())
            .await
            .expect("connect");

        let result = runtime
            .apply_settings(
                SshAgentSettings {
                    ui_extensions_enabled: true,
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
        assert_eq!(support_clone.call_count("register_extensions"), 1);
    }

    #[tokio::test]
    async fn execute_update_is_rate_limited_per_host() {
        let support = FakeSupport::default();
        let support_clone = support.clone();
        let mut runtime = SshAgentRuntime::new(SshAgentRuntimeConfig::new(
            support,
            tempfile::tempdir()
                .expect("tempdir")
                .path()
                .join("update-freeze"),
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
    async fn host_config_changed_is_ignored_when_transport_is_yielded() {
        let support = FakeSupport::default();
        let support_clone = support.clone();
        let tempdir = tempfile::tempdir().expect("tempdir");
        let mut runtime = SshAgentRuntime::new(SshAgentRuntimeConfig::new(
            support,
            tempdir.path().join("update-freeze"),
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
    async fn freeze_file_create_and_remove() {
        let dir = tempfile::tempdir().expect("tempdir");
        let freeze_path = dir.path().join("embedded-ssh-agent").join("update-freeze");

        handle_set_update_freeze(
            &freeze_path,
            SetUpdateFreezePayload {
                enabled: true,
                reason: Some("test".to_string()),
            },
        )
        .await;
        assert!(tokio::fs::try_exists(&freeze_path).await.expect("exists"));

        handle_set_update_freeze(
            &freeze_path,
            SetUpdateFreezePayload {
                enabled: false,
                reason: None,
            },
        )
        .await;
        assert!(!tokio::fs::try_exists(&freeze_path).await.expect("exists"));
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
