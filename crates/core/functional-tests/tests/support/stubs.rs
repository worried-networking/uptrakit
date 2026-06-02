use std::collections::BTreeSet;
use std::sync::Arc;

use time::OffsetDateTime;
use tokio::sync::mpsc;
use uuid::Uuid;

use uptrakit_controller_core::connections::ServiceConnectionRegistry;
use uptrakit_controller_core::notification::{
    EventBroadcaster, NotificationDispatcher, NotificationService, NotificationState,
};
use uptrakit_controller_core::update::UpdateOutputStream;
use uptrakit_plugin_infrastructure_core::plugin_ops::PluginOps;
use uptrakit_plugin_infrastructure_core::{CatalogConfig, InstancePluginStates, PluginCatalog};
use uptrakit_plugin_infrastructure_proxmox::DESCRIPTOR as PROXMOX_DESCRIPTOR;
use uptrakit_shared_types::OutputStreamType;
use uptrakit_wire::ControllerMessage;

/// Build a real `PluginCatalog` over `Arc<dyn PluginOps>`.
///
/// `with_proxmox = true` registers the Proxmox descriptor — both
/// `controller_update_protection()` and `controller_update_hook()` return the
/// real plugin. `false` returns an empty catalog where both accessors are
/// `None` (used by Test 3 — no Proxmox path).
pub(crate) fn build_plugin_ops(with_proxmox: bool) -> Arc<dyn PluginOps> {
    let descriptors = if with_proxmox {
        vec![&PROXMOX_DESCRIPTOR]
    } else {
        vec![]
    };
    let catalog = PluginCatalog::new(
        descriptors,
        &CatalogConfig {
            allow_private_urls: false,
            global_provider_lookup: None,
            http_client: None,
            cancellation_token: None,
        },
        // Proxmox is Tenant-scoped; InstancePluginStates only gates
        // Instance-scoped plugins, so `all_disabled()` is correct here.
        InstancePluginStates::all_disabled(),
    )
    .expect("PluginCatalog::new must succeed");
    Arc::new(catalog)
}

/// Holds a real `NotificationState` plus the receiver end of the agent's
/// `ControllerMessage` channel, so tests can capture the dispatched
/// `ExecuteUpdate` payload.
pub(crate) struct TestNotificationSetup {
    pub(crate) notification_state: NotificationState,
    pub(crate) message_rx: mpsc::Receiver<ControllerMessage>,
}

impl TestNotificationSetup {
    pub(crate) async fn new(agent_service_id: Uuid) -> Self {
        let registry = ServiceConnectionRegistry::new();
        let (message_rx, _handle) = registry
            .register(agent_service_id, BTreeSet::new(), None, None, None)
            .await;
        let notification_service = NotificationService::new(registry, Uuid::now_v7());
        let (dispatcher, _event_rx) = NotificationDispatcher::test_channel();
        let event_broadcaster = EventBroadcaster::new();
        Self {
            notification_state: NotificationState::new(
                notification_service,
                dispatcher,
                event_broadcaster,
            ),
            message_rx,
        }
    }

    pub(crate) fn captured_messages(&mut self) -> Vec<ControllerMessage> {
        let mut msgs = vec![];
        while let Ok(m) = self.message_rx.try_recv() {
            msgs.push(m);
        }
        msgs
    }
}

/// No-op `UpdateOutputStream`. All three trait methods drop their inputs.
pub(crate) struct NoopOutputStream;

#[async_trait::async_trait]
impl UpdateOutputStream for NoopOutputStream {
    async fn create_channel(&self, _update_id: Uuid) {}

    async fn send_line(
        &self,
        _update_id: Uuid,
        _line_id: Uuid,
        _text: String,
        _stream: OutputStreamType,
        _ts: OffsetDateTime,
    ) {
    }

    async fn send_completed(
        &self,
        _update_id: Uuid,
        _outcome: uptrakit_controller_core::update::DispatchOutcome,
        _error: Option<String>,
    ) {
    }
}
