//! Request/response correlation proxy for UI extension action invocations.
//!
//! The [`ExtensionProxy`] bridges the REST API (synchronous request/response)
//! with the WebSocket-based service communication (asynchronous message
//! passing). When a client invokes an extension action via the REST API, the
//! proxy:
//!
//! 1. Resolves the target service instance.
//! 2. Sends an `ExtensionRequest` message over the service's WebSocket.
//! 3. Waits for the matching `ExtensionResponse` (correlated by `request_id`).
//! 4. Returns the response payload to the REST handler.
//!
//! The WebSocket handler calls [`ExtensionProxy::complete`] when an
//! `ExtensionResponse` message arrives from a service.

use std::collections::HashMap;
use std::fmt;
use std::time::Duration;

use parking_lot::Mutex;
use uuid::Uuid;

use uptrakit_internal_wire::ControllerMessage;
use uptrakit_internal_wire::extension::{ExtensionRequestPayload, ExtensionResponsePayload};

use crate::extension_registry::ExtensionRegistry;
use crate::service_connections::ServiceConnectionRegistry;

// ── Error type ──────────────────────────────────────────────────────────────

/// Errors that can occur when invoking an extension action through the proxy.
#[derive(Debug)]
pub enum ExtensionProxyError {
    /// No service provides the requested extension.
    NoProvider,
    /// The specified service override is not a registered provider for the extension.
    InvalidProvider(Uuid),
    /// The target service is not currently connected via WebSocket.
    ServiceDisconnected,
    /// Failed to send the request message to the service (channel full or closed).
    SendFailed,
    /// The service did not respond within the allowed timeout.
    Timeout,
}

impl fmt::Display for ExtensionProxyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoProvider => write!(f, "no service provides this extension"),
            Self::InvalidProvider(id) => {
                write!(f, "service {id} is not a provider for this extension")
            }
            Self::ServiceDisconnected => {
                write!(f, "target service disconnected before responding")
            }
            Self::SendFailed => write!(f, "failed to send request to service"),
            Self::Timeout => write!(f, "service did not respond within the timeout"),
        }
    }
}

impl std::error::Error for ExtensionProxyError {}

// ── Proxy ───────────────────────────────────────────────────────────────────

/// Correlates extension action requests with their responses.
///
/// Each in-flight request is tracked by a `request_id` (UUID v7). The proxy
/// holds a `oneshot::Sender` for each pending request. When the WebSocket
/// handler receives an `ExtensionResponse`, it calls [`complete`](Self::complete)
/// to deliver the response to the waiting REST handler.
///
/// The pending map uses [`parking_lot::Mutex`] per project convention. The
/// guard is always dropped before any `.await` point.
pub struct ExtensionProxy {
    /// Map of pending request_id to the oneshot sender that will deliver the response.
    pending: Mutex<HashMap<String, tokio::sync::oneshot::Sender<ExtensionResponsePayload>>>,
}

impl Default for ExtensionProxy {
    fn default() -> Self {
        Self::new()
    }
}

impl ExtensionProxy {
    /// Creates a new proxy with no pending requests.
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Invokes an extension action and waits for the response.
    ///
    /// # Resolution
    ///
    /// If `service_id_override` is `Some`, validates that the given service is
    /// a registered provider for the extension. If not, returns
    /// [`ExtensionProxyError::InvalidProvider`].
    ///
    /// Otherwise, calls [`ExtensionRegistry::pick_provider`] to select a
    /// service automatically. If no provider is available, returns
    /// [`ExtensionProxyError::NoProvider`].
    ///
    /// # Errors
    ///
    /// - [`ExtensionProxyError::NoProvider`] -- no service provides this extension.
    /// - [`ExtensionProxyError::InvalidProvider`] -- override service is not a provider.
    /// - [`ExtensionProxyError::SendFailed`] -- could not deliver the message to the service.
    /// - [`ExtensionProxyError::Timeout`] -- the service did not respond in time.
    /// - [`ExtensionProxyError::ServiceDisconnected`] -- the service disconnected mid-flight.
    #[allow(clippy::too_many_arguments)]
    pub async fn invoke(
        &self,
        service_connections: &ServiceConnectionRegistry,
        registry: &ExtensionRegistry,
        extension_id: &str,
        action_id: &str,
        params: serde_json::Value,
        service_id_override: Option<Uuid>,
        timeout: Duration,
    ) -> Result<ExtensionResponsePayload, ExtensionProxyError> {
        // 1. Resolve target service_id.
        let service_id = match service_id_override {
            Some(id) => {
                let providers = registry.providers(extension_id);
                if !providers.contains(&id) {
                    return Err(ExtensionProxyError::InvalidProvider(id));
                }
                id
            }
            None => registry
                .pick_provider(extension_id, None)
                .ok_or(ExtensionProxyError::NoProvider)?,
        };

        // 2. Generate a UUID v7 request_id for correlation.
        let request_id = Uuid::now_v7().to_string();

        // 3. Create the oneshot channel for the response.
        let (tx, rx) = tokio::sync::oneshot::channel();

        // 4. Insert the sender into the pending map.
        //    Lock is dropped immediately (before any .await).
        {
            let mut guard = self.pending.lock();
            guard.insert(request_id.clone(), tx);
        }

        // 5. Send the ExtensionRequest to the service.
        let msg = ControllerMessage::ExtensionRequest(ExtensionRequestPayload {
            request_id: request_id.clone(),
            extension_id: extension_id.to_string(),
            action_id: action_id.to_string(),
            params,
        });

        let sent = service_connections.send(&service_id, msg).await;
        if !sent {
            // Clean up the pending entry on send failure.
            let mut guard = self.pending.lock();
            guard.remove(&request_id);
            return Err(ExtensionProxyError::SendFailed);
        }

        // 6. Await the response with a timeout.
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => {
                // Sender was dropped -- service disconnected.
                // Entry is already removed from pending (sender consumed).
                Err(ExtensionProxyError::ServiceDisconnected)
            }
            Err(_) => {
                // Timeout -- clean up the pending entry.
                let mut guard = self.pending.lock();
                guard.remove(&request_id);
                Err(ExtensionProxyError::Timeout)
            }
        }
    }

    /// Completes a pending request by delivering the response.
    ///
    /// Called by the WebSocket handler when an `ExtensionResponse` message
    /// arrives from a service. If no pending request matches the `request_id`
    /// (e.g., it already timed out), this is a no-op.
    pub fn complete(&self, request_id: &str, response: ExtensionResponsePayload) {
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
    use uptrakit_internal_wire::extension::ExtensionManifest;

    /// Helper: create a minimal test manifest via JSON deserialization
    /// (required because `ExtensionManifest` is `#[non_exhaustive]`).
    fn test_manifest(id: &str) -> ExtensionManifest {
        serde_json::from_value(serde_json::json!({
            "id": id,
            "label": format!("Test {id}"),
            "placement": {
                "type": "page",
                "nav_section": "test"
            },
            "targeting": "universal",
            "ui": {
                "type": "actions",
                "actions": []
            }
        }))
        .expect("test manifest JSON should be valid")
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_succeeds_when_complete_is_called() {
        let registry = ExtensionRegistry::new(vec![]);
        let service_connections = ServiceConnectionRegistry::new();
        let service_id = Uuid::now_v7();

        let (mut rx, _cancel) = service_connections
            .register(service_id, BTreeSet::new(), None, None)
            .await;
        registry
            .register_service(service_id, "test-app", vec![test_manifest("ext.test")])
            .unwrap();

        let proxy = std::sync::Arc::new(ExtensionProxy::new());
        let proxy_clone = std::sync::Arc::clone(&proxy);

        // Spawn a task that receives the ControllerMessage and calls complete.
        let complete_handle = tokio::spawn(async move {
            let msg = rx.recv().await.expect("should receive a message");
            match msg {
                ControllerMessage::ExtensionRequest(payload) => {
                    let response = ExtensionResponsePayload {
                        request_id: payload.request_id.clone(),
                        success: true,
                        data: serde_json::json!({"result": "ok"}),
                        error: None,
                    };
                    proxy_clone.complete(&payload.request_id, response);
                }
                _ => panic!("expected ExtensionRequest"),
            }
        });

        let result = proxy
            .invoke(
                &service_connections,
                &registry,
                "ext.test",
                "do-thing",
                serde_json::json!({"key": "value"}),
                None,
                Duration::from_secs(5),
            )
            .await;

        complete_handle.await.unwrap();

        let response = result.expect("invoke should succeed");
        assert!(response.success);
        assert_eq!(response.data, serde_json::json!({"result": "ok"}));
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_times_out() {
        let proxy = ExtensionProxy::new();
        let registry = ExtensionRegistry::new(vec![]);
        let service_connections = ServiceConnectionRegistry::new();
        let service_id = Uuid::now_v7();

        // Register the service but never consume messages (no complete call).
        let (_rx, _cancel) = service_connections
            .register(service_id, BTreeSet::new(), None, None)
            .await;
        registry
            .register_service(service_id, "test-app", vec![test_manifest("ext.test")])
            .unwrap();

        let result = proxy
            .invoke(
                &service_connections,
                &registry,
                "ext.test",
                "do-thing",
                serde_json::Value::Null,
                None,
                Duration::from_millis(100),
            )
            .await;

        assert!(
            matches!(result, Err(ExtensionProxyError::Timeout)),
            "expected Timeout, got {result:?}"
        );

        // Pending map should be cleaned up after timeout.
        assert!(proxy.pending.lock().is_empty());
    }

    #[test]
    fn complete_with_unknown_request_id_is_noop() {
        let proxy = ExtensionProxy::new();

        // Should not panic.
        proxy.complete(
            "nonexistent-request-id",
            ExtensionResponsePayload {
                request_id: "nonexistent-request-id".to_string(),
                success: true,
                data: serde_json::Value::Null,
                error: None,
            },
        );

        assert!(proxy.pending.lock().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_returns_no_provider_when_extension_not_registered() {
        let proxy = ExtensionProxy::new();
        let registry = ExtensionRegistry::new(vec![]);
        let service_connections = ServiceConnectionRegistry::new();

        let result = proxy
            .invoke(
                &service_connections,
                &registry,
                "ext.nonexistent",
                "action",
                serde_json::Value::Null,
                None,
                Duration::from_secs(5),
            )
            .await;

        assert!(
            matches!(result, Err(ExtensionProxyError::NoProvider)),
            "expected NoProvider, got {result:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_returns_invalid_provider_for_wrong_override() {
        let proxy = ExtensionProxy::new();
        let registry = ExtensionRegistry::new(vec![]);
        let service_connections = ServiceConnectionRegistry::new();

        let real_svc = Uuid::now_v7();
        let wrong_svc = Uuid::now_v7();

        registry
            .register_service(real_svc, "app", vec![test_manifest("ext.test")])
            .unwrap();

        let result = proxy
            .invoke(
                &service_connections,
                &registry,
                "ext.test",
                "action",
                serde_json::Value::Null,
                Some(wrong_svc),
                Duration::from_secs(5),
            )
            .await;

        assert!(
            matches!(result, Err(ExtensionProxyError::InvalidProvider(id)) if id == wrong_svc),
            "expected InvalidProvider({wrong_svc}), got {result:?}"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_returns_send_failed_when_service_not_connected() {
        let proxy = ExtensionProxy::new();
        let registry = ExtensionRegistry::new(vec![]);
        let service_connections = ServiceConnectionRegistry::new();
        let service_id = Uuid::now_v7();

        // Register the extension provider but do NOT register the service connection.
        registry
            .register_service(service_id, "app", vec![test_manifest("ext.test")])
            .unwrap();

        let result = proxy
            .invoke(
                &service_connections,
                &registry,
                "ext.test",
                "action",
                serde_json::Value::Null,
                None,
                Duration::from_secs(5),
            )
            .await;

        assert!(
            matches!(result, Err(ExtensionProxyError::SendFailed)),
            "expected SendFailed, got {result:?}"
        );

        // Pending map should be cleaned up after send failure.
        assert!(proxy.pending.lock().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_returns_service_disconnected_when_sender_dropped() {
        let proxy = std::sync::Arc::new(ExtensionProxy::new());
        let registry = ExtensionRegistry::new(vec![]);
        let service_connections = ServiceConnectionRegistry::new();
        let service_id = Uuid::now_v7();

        let (mut rx, _cancel) = service_connections
            .register(service_id, BTreeSet::new(), None, None)
            .await;
        registry
            .register_service(service_id, "app", vec![test_manifest("ext.test")])
            .unwrap();

        let proxy_clone = std::sync::Arc::clone(&proxy);

        // Spawn a task that receives the message then drops the pending sender
        // without calling complete, simulating a service disconnect.
        tokio::spawn(async move {
            let msg = rx.recv().await.expect("should receive a message");
            if let ControllerMessage::ExtensionRequest(payload) = msg {
                // Remove the pending sender without sending a response.
                // The sender is dropped, causing the oneshot receiver to get RecvError.
                let mut guard = proxy_clone.pending.lock();
                guard.remove(&payload.request_id);
            }
        });

        let result = proxy
            .invoke(
                &service_connections,
                &registry,
                "ext.test",
                "action",
                serde_json::Value::Null,
                None,
                Duration::from_secs(5),
            )
            .await;

        assert!(
            matches!(result, Err(ExtensionProxyError::ServiceDisconnected)),
            "expected ServiceDisconnected, got {result:?}"
        );
    }

    #[test]
    fn error_display_messages() {
        let svc_id = Uuid::nil();

        assert_eq!(
            ExtensionProxyError::NoProvider.to_string(),
            "no service provides this extension"
        );
        assert_eq!(
            ExtensionProxyError::InvalidProvider(svc_id).to_string(),
            format!("service {svc_id} is not a provider for this extension")
        );
        assert_eq!(
            ExtensionProxyError::ServiceDisconnected.to_string(),
            "target service disconnected before responding"
        );
        assert_eq!(
            ExtensionProxyError::SendFailed.to_string(),
            "failed to send request to service"
        );
        assert_eq!(
            ExtensionProxyError::Timeout.to_string(),
            "service did not respond within the timeout"
        );
    }
}
