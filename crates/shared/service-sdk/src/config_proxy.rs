//! Request/response correlation proxy for service-initiated config store
//! operations.
//!
//! When a service needs to write or delete a config entry on the controller,
//! it:
//!
//! 1. Calls [`ServiceConfigProxy::store`] or [`ServiceConfigProxy::delete`].
//! 2. The proxy generates a UUID v7 `request_id`, creates a oneshot channel,
//!    and returns a [`PendingServiceConfigRequest`] with the message to send.
//! 3. The caller sends the `ServiceMessage::StoreServiceConfig` or
//!    `ServiceMessage::DeleteServiceConfig` via `bg_tx`.
//! 4. When the controller responds with `ControllerMessage::ServiceConfigAck`,
//!    the event loop calls [`ServiceConfigProxy::complete`] to deliver the ACK.

use std::collections::HashMap;
use std::time::Duration;

use parking_lot::Mutex;
use uuid::Uuid;

use crate::wire_api::ServiceMessage;
use crate::wire_api::payloads::{
    DeleteServiceConfigPayload, ServiceConfigAckPayload, StoreServiceConfigPayload,
};

/// A pending config request ready to be sent and awaited.
pub struct PendingServiceConfigRequest {
    /// The `ServiceMessage` to send to the controller.
    pub message: ServiceMessage,
    /// Receiver for the controller's acknowledgment.
    rx: tokio::sync::oneshot::Receiver<ServiceConfigAckPayload>,
    /// The generated request ID for cleanup on timeout.
    request_id: String,
}

impl PendingServiceConfigRequest {
    /// Wait for the controller's acknowledgment with a timeout.
    ///
    /// On timeout or sender-dropped, the pending entry is automatically
    /// cleaned up from the proxy's internal map.
    pub async fn wait(
        self,
        proxy: &ServiceConfigProxy,
        timeout: Duration,
    ) -> Result<(), ServiceConfigProxyError> {
        match tokio::time::timeout(timeout, self.rx).await {
            Ok(Ok(ack)) => {
                if ack.success {
                    Ok(())
                } else {
                    Err(ServiceConfigProxyError::ControllerError(
                        ack.error.unwrap_or_else(|| "unknown error".to_string()),
                    ))
                }
            }
            Ok(Err(_)) => Err(ServiceConfigProxyError::Disconnected),
            Err(_) => {
                let mut guard = proxy.pending.lock();
                guard.remove(&self.request_id);
                Err(ServiceConfigProxyError::Timeout)
            }
        }
    }
}

/// Errors from the service config proxy.
#[derive(Debug)]
pub enum ServiceConfigProxyError {
    /// The controller did not respond within the allowed timeout.
    Timeout,
    /// The controller disconnected before responding.
    Disconnected,
    /// The controller returned an error.
    ControllerError(String),
}

impl std::fmt::Display for ServiceConfigProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => write!(f, "controller did not respond within the timeout"),
            Self::Disconnected => write!(f, "controller disconnected before responding"),
            Self::ControllerError(msg) => write!(f, "controller error: {msg}"),
        }
    }
}

impl std::error::Error for ServiceConfigProxyError {}

/// Correlates service-initiated config store/delete requests with ACKs.
///
/// Each in-flight request is tracked by a `request_id` (UUID v7). The proxy
/// holds a `oneshot::Sender` for each pending request. When the event loop
/// receives a `ControllerMessage::ServiceConfigAck`, it calls
/// [`complete`](Self::complete) to deliver the ACK to the waiting task.
///
/// Uses [`parking_lot::Mutex`] per project convention. The guard is always
/// dropped before any `.await` point.
pub struct ServiceConfigProxy {
    pending: Mutex<HashMap<String, tokio::sync::oneshot::Sender<ServiceConfigAckPayload>>>,
}

impl Default for ServiceConfigProxy {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceConfigProxy {
    /// Creates a new proxy with no pending requests.
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Prepare a store config operation.
    ///
    /// Returns a [`PendingServiceConfigRequest`] with the message to send
    /// and a handle to await the ACK. The caller is responsible for sending
    /// the message (typically via `bg_tx`).
    pub fn store(
        &self,
        tenant_id: Option<Uuid>,
        key: String,
        value: serde_json::Value,
        sensitive: bool,
    ) -> PendingServiceConfigRequest {
        let request_id = Uuid::now_v7().to_string();
        let (tx, rx) = tokio::sync::oneshot::channel();

        {
            let mut guard = self.pending.lock();
            guard.insert(request_id.clone(), tx);
        }

        let message = ServiceMessage::StoreServiceConfig(StoreServiceConfigPayload::new(
            request_id.clone(),
            tenant_id,
            key,
            value,
            sensitive,
        ));

        PendingServiceConfigRequest {
            message,
            rx,
            request_id,
        }
    }

    /// Prepare a delete config operation.
    ///
    /// Returns a [`PendingServiceConfigRequest`] with the message to send
    /// and a handle to await the ACK.
    pub fn delete(&self, tenant_id: Option<Uuid>, key: String) -> PendingServiceConfigRequest {
        let request_id = Uuid::now_v7().to_string();
        let (tx, rx) = tokio::sync::oneshot::channel();

        {
            let mut guard = self.pending.lock();
            guard.insert(request_id.clone(), tx);
        }

        let message = ServiceMessage::DeleteServiceConfig(DeleteServiceConfigPayload::new(
            request_id.clone(),
            tenant_id,
            key,
        ));

        PendingServiceConfigRequest {
            message,
            rx,
            request_id,
        }
    }

    /// Completes a pending request by delivering the controller's ACK.
    ///
    /// Called by the event loop when a `ControllerMessage::ServiceConfigAck`
    /// arrives. If no pending request matches the `request_id` (e.g., it
    /// already timed out), this is a no-op.
    pub fn complete(&self, request_id: &str, ack: ServiceConfigAckPayload) {
        let sender = {
            let mut guard = self.pending.lock();
            guard.remove(request_id)
        };

        if let Some(tx) = sender {
            let _ = tx.send(ack);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_creates_pending_request() {
        let proxy = ServiceConfigProxy::new();
        let pending = proxy.store(None, "key".to_string(), serde_json::json!({}), false);

        assert!(!proxy.pending.lock().is_empty());

        if let ServiceMessage::StoreServiceConfig(p) = &pending.message {
            assert_eq!(p.key, "key");
            assert!(!p.sensitive);
            assert!(p.tenant_id.is_none());
        } else {
            panic!("expected StoreServiceConfig");
        }
    }

    #[test]
    fn delete_creates_pending_request() {
        let proxy = ServiceConfigProxy::new();
        let pending = proxy.delete(None, "key".to_string());

        assert!(!proxy.pending.lock().is_empty());

        if let ServiceMessage::DeleteServiceConfig(p) = &pending.message {
            assert_eq!(p.key, "key");
        } else {
            panic!("expected DeleteServiceConfig");
        }
    }

    #[test]
    fn complete_with_unknown_request_id_is_noop() {
        let proxy = ServiceConfigProxy::new();
        proxy.complete(
            "nonexistent",
            ServiceConfigAckPayload::success("nonexistent".into()),
        );
        assert!(proxy.pending.lock().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn store_and_complete_succeeds() {
        let proxy = ServiceConfigProxy::new();
        let pending = proxy.store(
            None,
            "cfg.key".to_string(),
            serde_json::json!({"v": 1}),
            true,
        );

        let request_id = if let ServiceMessage::StoreServiceConfig(p) = &pending.message {
            p.request_id.clone()
        } else {
            panic!("expected StoreServiceConfig");
        };

        proxy.complete(
            &request_id,
            ServiceConfigAckPayload::success(request_id.clone()),
        );

        pending
            .wait(&proxy, Duration::from_secs(5))
            .await
            .expect("should succeed");
        assert!(proxy.pending.lock().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn store_times_out() {
        let proxy = ServiceConfigProxy::new();
        let pending = proxy.store(None, "key".to_string(), serde_json::Value::Null, false);

        let result = pending.wait(&proxy, Duration::from_millis(100)).await;
        assert!(matches!(result, Err(ServiceConfigProxyError::Timeout)));
        assert!(proxy.pending.lock().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn controller_error_propagated() {
        let proxy = ServiceConfigProxy::new();
        let pending = proxy.store(None, "key".to_string(), serde_json::Value::Null, false);

        let request_id = if let ServiceMessage::StoreServiceConfig(p) = &pending.message {
            p.request_id.clone()
        } else {
            panic!("expected StoreServiceConfig");
        };

        proxy.complete(
            &request_id,
            ServiceConfigAckPayload::error(request_id.clone(), "db write failed".to_string()),
        );

        let result = pending.wait(&proxy, Duration::from_secs(5)).await;
        assert!(matches!(
            result,
            Err(ServiceConfigProxyError::ControllerError(msg)) if msg == "db write failed"
        ));
    }

    #[test]
    fn error_display_messages() {
        assert_eq!(
            ServiceConfigProxyError::Timeout.to_string(),
            "controller did not respond within the timeout"
        );
        assert_eq!(
            ServiceConfigProxyError::Disconnected.to_string(),
            "controller disconnected before responding"
        );
        assert_eq!(
            ServiceConfigProxyError::ControllerError("oops".to_string()).to_string(),
            "controller error: oops"
        );
    }
}
