use std::collections::HashMap;
use std::sync::Arc;

use tokio::sync::{RwLock, mpsc};
use uptrakit_internal_wire::ControllerMessage;
use uuid::Uuid;

/// Registry of connected agents with push channels.
///
/// The controller registers agents when they connect via WebSocket and
/// unregisters them on disconnect. Admin actions (approve/reject) use
/// `send()` to push messages to connected agents in real time.
#[derive(Clone)]
pub struct AgentConnectionRegistry {
    inner: Arc<RwLock<HashMap<Uuid, mpsc::Sender<ControllerMessage>>>>,
}

impl Default for AgentConnectionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentConnectionRegistry {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a connected agent and return a receiver for push messages.
    pub async fn register(&self, agent_id: Uuid) -> mpsc::Receiver<ControllerMessage> {
        let (tx, rx) = mpsc::channel(16);
        self.inner.write().await.insert(agent_id, tx);
        rx
    }

    /// Remove an agent from the registry on disconnect.
    pub async fn unregister(&self, agent_id: &Uuid) {
        self.inner.write().await.remove(agent_id);
    }

    /// Push a message to a connected agent. Returns `true` if sent.
    pub async fn send(&self, agent_id: &Uuid, msg: ControllerMessage) -> bool {
        let guard = self.inner.read().await;
        if let Some(tx) = guard.get(agent_id) {
            tx.send(msg).await.is_ok()
        } else {
            false
        }
    }

    /// Check whether an agent is currently connected.
    pub async fn is_connected(&self, agent_id: &Uuid) -> bool {
        self.inner.read().await.contains_key(agent_id)
    }

    /// Broadcast a message to all connected agents.
    pub async fn broadcast(&self, msg: ControllerMessage) {
        let guard = self.inner.read().await;
        for tx in guard.values() {
            let _ = tx.send(msg.clone()).await;
        }
    }

    /// Broadcast a CA bundle update to all connected agents.
    pub async fn broadcast_ca_bundle_updated(
        &self,
        payload: uptrakit_internal_wire::CaBundleUpdatedPayload,
    ) {
        let msg = ControllerMessage::CaBundleUpdated(payload);
        self.broadcast(msg).await;
    }

    /// Broadcast a certificate renewal request to all connected agents.
    pub async fn broadcast_request_cert_renewal(
        &self,
        payload: uptrakit_internal_wire::RequestCertRenewalPayload,
    ) {
        let msg = ControllerMessage::RequestCertRenewal(payload);
        self.broadcast(msg).await;
    }
}
