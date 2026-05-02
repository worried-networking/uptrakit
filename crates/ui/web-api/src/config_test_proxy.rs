//! Request/response correlation proxy for plugin configuration test invocations.
//!
//! Bridges the REST API with the WebSocket-based agent communication for
//! agent-side config tests (version detection, command validation, hooks).
//!
//! The [`ConfigTestProxy`] follows the same request/response correlation pattern:
//! each in-flight
//! request is tracked by a `request_id` (UUID v7) with a `oneshot::Sender`
//! held in a pending map. When the WebSocket handler receives a
//! `TestPluginConfigResult` message, it calls [`ConfigTestProxy::complete`]
//! to deliver the response to the waiting REST handler.

#![expect(
    clippy::let_underscore_must_use,
    reason = "fire-and-forget channel send intentionally drops the result"
)]

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use parking_lot::Mutex;
use uuid::Uuid;

use uptrakit_wire::{ControllerMessage, TestPluginConfigPayload, TestPluginConfigResultPayload};

use crate::service_connections::ServiceConnectionRegistry;

// -- Error type ---------------------------------------------------------------

/// Errors that can occur when invoking a config test through the proxy.
#[derive(Debug)]
pub enum ConfigTestProxyError {
    /// The target service is not currently connected via WebSocket.
    ServiceDisconnected,
    /// Failed to send the request message to the service (channel full or closed).
    SendFailed,
    /// The service did not respond within the allowed timeout.
    Timeout,
}

impl fmt::Display for ConfigTestProxyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ServiceDisconnected => {
                write!(f, "target service disconnected before responding")
            }
            Self::SendFailed => write!(f, "failed to send request to service"),
            Self::Timeout => write!(f, "service did not respond within the timeout"),
        }
    }
}

impl std::error::Error for ConfigTestProxyError {}

// -- Proxy --------------------------------------------------------------------

/// Correlates config test requests with their responses.
///
/// Each in-flight request is tracked by a `request_id` (UUID v7). The proxy
/// holds a `oneshot::Sender` for each pending request. When the WebSocket
/// handler receives a `TestPluginConfigResult`, it calls [`complete`](Self::complete)
/// to deliver the response to the waiting REST handler.
///
/// The pending map uses [`parking_lot::Mutex`] per project convention. The
/// guard is always dropped before any `.await` point.
pub struct ConfigTestProxy {
    /// Map of pending request_id to the oneshot sender that will deliver the response.
    pending: Mutex<HashMap<String, tokio::sync::oneshot::Sender<TestPluginConfigResultPayload>>>,
}

impl Default for ConfigTestProxy {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigTestProxy {
    /// Creates a new proxy with no pending requests.
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Sends a config test request to the given service and waits for the response.
    ///
    /// # Errors
    ///
    /// - [`ConfigTestProxyError::SendFailed`] -- could not deliver the message to the service.
    /// - [`ConfigTestProxyError::Timeout`] -- the service did not respond in time.
    /// - [`ConfigTestProxyError::ServiceDisconnected`] -- the service disconnected mid-flight.
    pub async fn invoke(
        &self,
        service_connections: &ServiceConnectionRegistry,
        service_id: &Uuid,
        payload: TestPluginConfigPayload,
        timeout: Duration,
    ) -> Result<TestPluginConfigResultPayload, ConfigTestProxyError> {
        // 1. The request_id is already set on the payload by the caller.
        let request_id = payload.request_id.clone();

        // 2. Create the oneshot channel for the response.
        let (tx, rx) = tokio::sync::oneshot::channel();

        // 3. Insert the sender into the pending map.
        //    Lock is dropped immediately (before any .await).
        {
            let mut guard = self.pending.lock();
            guard.insert(request_id.clone(), tx);
        }

        // 4. Send the TestPluginConfig message to the service.
        let msg = ControllerMessage::TestPluginConfig(payload);

        let sent = service_connections.send(service_id, msg).await;
        if !sent {
            // Clean up the pending entry on send failure.
            let mut guard = self.pending.lock();
            guard.remove(&request_id);
            return Err(ConfigTestProxyError::SendFailed);
        }

        // 5. Await the response with a timeout.
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => {
                // Sender was dropped -- service disconnected.
                Err(ConfigTestProxyError::ServiceDisconnected)
            }
            Err(_) => {
                // Timeout -- clean up the pending entry.
                let mut guard = self.pending.lock();
                guard.remove(&request_id);
                Err(ConfigTestProxyError::Timeout)
            }
        }
    }

    /// Completes a pending request by delivering the response.
    ///
    /// Called by the WebSocket handler when a `TestPluginConfigResult` message
    /// arrives from a service. If no pending request matches the `request_id`
    /// (e.g., it already timed out), this is a no-op.
    pub fn complete(&self, request_id: &str, response: TestPluginConfigResultPayload) {
        let sender = {
            let mut guard = self.pending.lock();
            guard.remove(request_id)
        };

        if let Some(tx) = sender {
            // If the receiver was dropped (e.g., the REST handler timed out and
            // moved on), the send will fail silently -- that is fine.
            let _ = tx.send(response);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[tokio::test(start_paused = true)]
    async fn invoke_times_out() {
        let proxy = ConfigTestProxy::new();
        let service_connections = ServiceConnectionRegistry::new();
        let service_id = Uuid::now_v7();

        // Register the service but never consume messages (no complete call).
        let (_rx, _cancel) = service_connections
            .register(service_id, BTreeSet::new(), None, None, None)
            .await;

        let payload = TestPluginConfigPayload::new(
            Uuid::now_v7().to_string(),
            "machine-1".to_string(),
            uptrakit_wire::ConfigTestKind::VersionDetection,
            "generic_shell".to_string(),
            serde_json::json!({"version_command": "echo 1.0"}),
        );

        let result = proxy
            .invoke(
                &service_connections,
                &service_id,
                payload,
                Duration::from_millis(100),
            )
            .await;

        assert!(
            matches!(result, Err(ConfigTestProxyError::Timeout)),
            "expected Timeout, got {result:?}"
        );

        // Pending map should be cleaned up after timeout.
        assert!(proxy.pending.lock().is_empty());
    }

    #[test]
    fn complete_with_unknown_request_id_is_noop() {
        let proxy = ConfigTestProxy::new();

        // Should not panic.
        proxy.complete(
            "nonexistent-request-id",
            TestPluginConfigResultPayload::new("nonexistent-request-id".to_string(), true, 0),
        );

        assert!(proxy.pending.lock().is_empty());
    }

    #[test]
    fn error_display_messages() {
        assert_eq!(
            ConfigTestProxyError::ServiceDisconnected.to_string(),
            "target service disconnected before responding"
        );
        assert_eq!(
            ConfigTestProxyError::SendFailed.to_string(),
            "failed to send request to service"
        );
        assert_eq!(
            ConfigTestProxyError::Timeout.to_string(),
            "service did not respond within the timeout"
        );
    }
}
