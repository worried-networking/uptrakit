//! Request/response correlation proxy for service-initiated extension action
//! invocations.
//!
//! Mirrors the controller-side [`ExtensionProxy`] pattern. When a service
//! needs to invoke a plugin-backed extension action on the controller (e.g.,
//! SSH agent querying the Proxmox plugin for discovered guests), it:
//!
//! 1. Calls [`ServiceExtensionProxy::invoke`] with the extension/action IDs
//!    and parameters.
//! 2. The proxy generates a UUID v7 `request_id`, creates a oneshot channel,
//!    and returns a [`PendingExtensionRequest`] containing the message to send
//!    and the response receiver.
//! 3. The caller sends the `ServiceMessage::ExtensionRequest` via `bg_tx` (the
//!    background channel that flows through the event loop to `conn.send()`).
//! 4. When the controller responds with `ControllerMessage::ExtensionResponse`,
//!    the event loop calls [`ServiceExtensionProxy::complete`] to deliver the
//!    response.
//!
//! The proxy itself does NOT hold `&mut conn` — it produces messages that the
//! caller routes through the existing `bg_tx` → `on_service_event` → `conn.send()`
//! pipeline.

use std::collections::HashMap;
use std::time::Duration;

use parking_lot::Mutex;

use uptrakit_internal_wire::ServiceMessage;
use uptrakit_internal_wire::extension::{ExtensionRequestPayload, ExtensionResponsePayload};

/// A pending extension request ready to be sent and awaited.
pub struct PendingExtensionRequest {
    /// The `ServiceMessage::ExtensionRequest` to send to the controller.
    pub message: ServiceMessage,
    /// Receiver for the controller's response.
    rx: tokio::sync::oneshot::Receiver<ExtensionResponsePayload>,
    /// The generated request ID for cleanup on timeout.
    request_id: String,
}

impl PendingExtensionRequest {
    /// Wait for the controller's response with a timeout.
    ///
    /// On timeout or sender-dropped, the pending entry is automatically cleaned
    /// up from the proxy's internal map.
    pub async fn wait(
        self,
        proxy: &ServiceExtensionProxy,
        timeout: Duration,
    ) -> Result<ExtensionResponsePayload, ServiceExtensionProxyError> {
        match tokio::time::timeout(timeout, self.rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => {
                // Sender dropped — controller disconnected or proxy cleaned up.
                Err(ServiceExtensionProxyError::Disconnected)
            }
            Err(_) => {
                // Timeout — clean up the pending entry.
                let mut guard = proxy.pending.lock();
                guard.remove(&self.request_id);
                Err(ServiceExtensionProxyError::Timeout)
            }
        }
    }
}

/// Errors from the service extension proxy.
#[derive(Debug)]
pub enum ServiceExtensionProxyError {
    /// The controller did not respond within the allowed timeout.
    Timeout,
    /// The controller disconnected before responding.
    Disconnected,
    /// Failed to send the request message (channel closed).
    SendFailed,
}

impl std::fmt::Display for ServiceExtensionProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => write!(f, "controller did not respond within the timeout"),
            Self::Disconnected => write!(f, "controller disconnected before responding"),
            Self::SendFailed => write!(f, "failed to send extension request"),
        }
    }
}

impl std::error::Error for ServiceExtensionProxyError {}

/// Correlates service-initiated extension action requests with responses.
///
/// Each in-flight request is tracked by a `request_id` (UUID v7). The proxy
/// holds a `oneshot::Sender` for each pending request. When the event loop
/// receives a `ControllerMessage::ExtensionResponse`, it calls [`complete`](Self::complete)
/// to deliver the response to the waiting task.
///
/// Uses [`parking_lot::Mutex`] per project convention. The guard is always
/// dropped before any `.await` point.
pub struct ServiceExtensionProxy {
    pending: Mutex<HashMap<String, tokio::sync::oneshot::Sender<ExtensionResponsePayload>>>,
}

impl Default for ServiceExtensionProxy {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceExtensionProxy {
    /// Creates a new proxy with no pending requests.
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Prepare an extension action invocation.
    ///
    /// Returns a [`PendingExtensionRequest`] containing the message to send
    /// and a handle to await the response. The caller is responsible for
    /// sending the message (typically via `bg_tx`).
    pub fn invoke(
        &self,
        extension_id: &str,
        action_id: &str,
        params: serde_json::Value,
    ) -> PendingExtensionRequest {
        let request_id = uuid::Uuid::now_v7().to_string();
        let (tx, rx) = tokio::sync::oneshot::channel();

        {
            let mut guard = self.pending.lock();
            guard.insert(request_id.clone(), tx);
        }

        let message = ServiceMessage::ExtensionRequest(ExtensionRequestPayload {
            request_id: request_id.clone(),
            extension_id: extension_id.to_string(),
            action_id: action_id.to_string(),
            params,
            sensitive_params: None,
            // Service-initiated requests carry no tenant context — the mTLS
            // channel is already trusted.
            tenant_id: None,
        });

        PendingExtensionRequest {
            message,
            rx,
            request_id,
        }
    }

    /// Completes a pending request by delivering the controller's response.
    ///
    /// Called by the event loop when a `ControllerMessage::ExtensionResponse`
    /// arrives. If no pending request matches the `request_id` (e.g., it already
    /// timed out), this is a no-op.
    pub fn complete(&self, request_id: &str, response: ExtensionResponsePayload) {
        let sender = {
            let mut guard = self.pending.lock();
            guard.remove(request_id)
        };

        if let Some(tx) = sender {
            let _ = tx.send(response);
        }
    }

    /// Returns `true` if there are pending requests.
    #[cfg(test)]
    fn has_pending(&self) -> bool {
        !self.pending.lock().is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invoke_creates_pending_request() {
        let proxy = ServiceExtensionProxy::new();
        let pending = proxy.invoke("ext.test", "action", serde_json::Value::Null);

        assert!(proxy.has_pending());

        // Verify message structure.
        if let ServiceMessage::ExtensionRequest(payload) = &pending.message {
            assert_eq!(payload.extension_id, "ext.test");
            assert_eq!(payload.action_id, "action");
            assert!(payload.sensitive_params.is_none());
        } else {
            panic!("expected ExtensionRequest");
        }

        // Drop pending without completing — should clean up.
        drop(pending);
    }

    #[test]
    fn complete_with_unknown_request_id_is_noop() {
        let proxy = ServiceExtensionProxy::new();
        proxy.complete(
            "nonexistent",
            ExtensionResponsePayload {
                request_id: "nonexistent".into(),
                success: true,
                data: serde_json::Value::Null,
                error: None,
            },
        );
        assert!(!proxy.has_pending());
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_and_complete_succeeds() {
        let proxy = ServiceExtensionProxy::new();
        let pending = proxy.invoke("ext.test", "action", serde_json::json!({"key": "val"}));

        // Extract request_id from the message.
        let request_id = if let ServiceMessage::ExtensionRequest(p) = &pending.message {
            p.request_id.clone()
        } else {
            panic!("expected ExtensionRequest");
        };

        // Complete the request.
        proxy.complete(
            &request_id,
            ExtensionResponsePayload {
                request_id: request_id.clone(),
                success: true,
                data: serde_json::json!({"result": "ok"}),
                error: None,
            },
        );

        let response = pending
            .wait(&proxy, Duration::from_secs(5))
            .await
            .expect("should succeed");
        assert!(response.success);
        assert_eq!(response.data, serde_json::json!({"result": "ok"}));
        assert!(!proxy.has_pending());
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_times_out() {
        let proxy = ServiceExtensionProxy::new();
        let pending = proxy.invoke("ext.test", "action", serde_json::Value::Null);

        let result = pending.wait(&proxy, Duration::from_millis(100)).await;

        assert!(matches!(result, Err(ServiceExtensionProxyError::Timeout)));
        assert!(!proxy.has_pending());
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_returns_disconnected_when_sender_dropped() {
        let proxy = ServiceExtensionProxy::new();
        let pending = proxy.invoke("ext.test", "action", serde_json::Value::Null);

        // Extract request_id and drop the sender without completing.
        let request_id = if let ServiceMessage::ExtensionRequest(p) = &pending.message {
            p.request_id.clone()
        } else {
            panic!("expected ExtensionRequest");
        };

        // Remove the sender from pending (simulates cleanup without completion).
        {
            let mut guard = proxy.pending.lock();
            guard.remove(&request_id);
        }

        let result = pending.wait(&proxy, Duration::from_secs(5)).await;

        assert!(matches!(
            result,
            Err(ServiceExtensionProxyError::Disconnected)
        ));
    }

    #[test]
    fn error_display_messages() {
        assert_eq!(
            ServiceExtensionProxyError::Timeout.to_string(),
            "controller did not respond within the timeout"
        );
        assert_eq!(
            ServiceExtensionProxyError::Disconnected.to_string(),
            "controller disconnected before responding"
        );
        assert_eq!(
            ServiceExtensionProxyError::SendFailed.to_string(),
            "failed to send extension request"
        );
    }
}
