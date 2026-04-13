//! Request/response correlation proxy for service-initiated surface action
//! invocations.

use std::collections::HashMap;
use std::time::Duration;

use parking_lot::Mutex;

use uptrakit_internal_wire::{ServiceMessage, surfaces};

/// A pending surface request ready to be sent and awaited.
pub struct PendingSurfaceRequest {
    /// The `ServiceMessage::SurfaceActionRequest` to send to the controller.
    pub message: ServiceMessage,
    /// Receiver for the controller's response.
    rx: tokio::sync::oneshot::Receiver<surfaces::SurfaceActionResponse>,
    /// The generated request ID for cleanup on timeout.
    request_id: uuid::Uuid,
}

impl PendingSurfaceRequest {
    /// Wait for the controller's response with a timeout.
    pub async fn wait(
        self,
        proxy: &ServiceSurfaceProxy,
        timeout: Duration,
    ) -> Result<surfaces::SurfaceActionResponse, ServiceSurfaceProxyError> {
        match tokio::time::timeout(timeout, self.rx).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(_)) => Err(ServiceSurfaceProxyError::Disconnected),
            Err(_) => {
                let mut guard = proxy.pending.lock();
                guard.remove(&self.request_id);
                Err(ServiceSurfaceProxyError::Timeout)
            }
        }
    }
}

/// Errors from the service surface proxy.
#[derive(Debug)]
pub enum ServiceSurfaceProxyError {
    /// The controller did not respond within the allowed timeout.
    Timeout,
    /// The controller disconnected before responding.
    Disconnected,
    /// Failed to send the request message (channel closed).
    SendFailed,
}

impl std::fmt::Display for ServiceSurfaceProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => write!(f, "controller did not respond within the timeout"),
            Self::Disconnected => write!(f, "controller disconnected before responding"),
            Self::SendFailed => write!(f, "failed to send surface request"),
        }
    }
}

impl std::error::Error for ServiceSurfaceProxyError {}

/// Correlates service-initiated surface action requests with responses.
pub struct ServiceSurfaceProxy {
    pending:
        Mutex<HashMap<uuid::Uuid, tokio::sync::oneshot::Sender<surfaces::SurfaceActionResponse>>>,
}

impl Default for ServiceSurfaceProxy {
    fn default() -> Self {
        Self::new()
    }
}

impl ServiceSurfaceProxy {
    /// Creates a new proxy with no pending requests.
    pub fn new() -> Self {
        Self {
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// Prepare a surface action invocation.
    #[allow(clippy::too_many_arguments)]
    pub fn invoke(
        &self,
        tenant_id: &str,
        surface_id: surfaces::SurfaceId,
        interaction_id: surfaces::InteractionId,
        idempotency_key: &str,
        caller_origin: surfaces::CallerOrigin,
        params: serde_json::Map<String, serde_json::Value>,
        encrypted_sensitive_params: Option<surfaces::EncryptedSensitiveParams>,
        target_provider_id: Option<String>,
    ) -> PendingSurfaceRequest {
        let request_id = uuid::Uuid::now_v7();
        let (tx, rx) = tokio::sync::oneshot::channel();

        {
            let mut guard = self.pending.lock();
            guard.insert(request_id, tx);
        }

        let message = ServiceMessage::SurfaceActionRequest(surfaces::SurfaceActionRequest {
            request_id,
            tenant_id: tenant_id.to_string(),
            surface_id,
            interaction_id,
            idempotency_key: idempotency_key.to_string(),
            target_provider_id,
            caller_origin,
            params,
            encrypted_sensitive_params,
        });

        PendingSurfaceRequest {
            message,
            rx,
            request_id,
        }
    }

    /// Completes a pending request by delivering the controller's response.
    pub fn complete(&self, request_id: &uuid::Uuid, response: surfaces::SurfaceActionResponse) {
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

    fn test_surface_id() -> surfaces::SurfaceId {
        surfaces::SurfaceId::new("ssh.guest.panel").unwrap()
    }

    fn test_interaction_id() -> surfaces::InteractionId {
        surfaces::InteractionId::new("refresh").unwrap()
    }

    fn test_origin() -> surfaces::CallerOrigin {
        surfaces::CallerOrigin::Provider {
            provider_id: "uptrakit-agent-ssh".to_string(),
        }
    }

    #[test]
    fn invoke_creates_pending_request() {
        let proxy = ServiceSurfaceProxy::new();
        let pending = proxy.invoke(
            "tenant-1",
            test_surface_id(),
            test_interaction_id(),
            "idem-1",
            test_origin(),
            serde_json::json!({"host_machine_id":"m-1"})
                .as_object()
                .unwrap()
                .clone(),
            None,
            Some("uptrakit-agent-ssh".to_string()),
        );

        assert!(proxy.has_pending());

        if let ServiceMessage::SurfaceActionRequest(payload) = &pending.message {
            assert_eq!(payload.tenant_id, "tenant-1");
            assert_eq!(payload.idempotency_key, "idem-1");
            assert_eq!(
                payload.target_provider_id.as_deref(),
                Some("uptrakit-agent-ssh")
            );
        } else {
            panic!("expected SurfaceActionRequest");
        }
    }

    #[test]
    fn complete_with_unknown_request_id_is_noop() {
        let proxy = ServiceSurfaceProxy::new();
        proxy.complete(
            &uuid::Uuid::now_v7(),
            surfaces::SurfaceActionResponse {
                request_id: uuid::Uuid::now_v7(),
                success: true,
                result: None,
                error: None,
            },
        );
        assert!(!proxy.has_pending());
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_and_complete_succeeds() {
        let proxy = ServiceSurfaceProxy::new();
        let pending = proxy.invoke(
            "tenant-1",
            test_surface_id(),
            test_interaction_id(),
            "idem-1",
            test_origin(),
            serde_json::Map::new(),
            None,
            None,
        );

        let request_id = if let ServiceMessage::SurfaceActionRequest(payload) = &pending.message {
            payload.request_id
        } else {
            panic!("expected SurfaceActionRequest");
        };

        proxy.complete(
            &request_id,
            surfaces::SurfaceActionResponse {
                request_id,
                success: true,
                result: Some(serde_json::json!({"status":"ok"})),
                error: None,
            },
        );

        let response = pending
            .wait(&proxy, Duration::from_secs(5))
            .await
            .expect("should succeed");
        assert!(response.success);
        assert_eq!(response.result, Some(serde_json::json!({"status":"ok"})));
        assert!(!proxy.has_pending());
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_times_out() {
        let proxy = ServiceSurfaceProxy::new();
        let pending = proxy.invoke(
            "tenant-1",
            test_surface_id(),
            test_interaction_id(),
            "idem-1",
            test_origin(),
            serde_json::Map::new(),
            None,
            None,
        );

        let result = pending.wait(&proxy, Duration::from_millis(100)).await;

        assert!(matches!(result, Err(ServiceSurfaceProxyError::Timeout)));
        assert!(!proxy.has_pending());
    }

    #[tokio::test(start_paused = true)]
    async fn invoke_returns_disconnected_when_sender_dropped() {
        let proxy = ServiceSurfaceProxy::new();
        let pending = proxy.invoke(
            "tenant-1",
            test_surface_id(),
            test_interaction_id(),
            "idem-1",
            test_origin(),
            serde_json::Map::new(),
            None,
            None,
        );

        let request_id = if let ServiceMessage::SurfaceActionRequest(payload) = &pending.message {
            payload.request_id
        } else {
            panic!("expected SurfaceActionRequest");
        };

        {
            let mut guard = proxy.pending.lock();
            guard.remove(&request_id);
        }

        let result = pending.wait(&proxy, Duration::from_secs(5)).await;

        assert!(matches!(
            result,
            Err(ServiceSurfaceProxyError::Disconnected)
        ));
    }

    #[test]
    fn error_display_messages() {
        assert_eq!(
            ServiceSurfaceProxyError::Timeout.to_string(),
            "controller did not respond within the timeout"
        );
        assert_eq!(
            ServiceSurfaceProxyError::Disconnected.to_string(),
            "controller disconnected before responding"
        );
        assert_eq!(
            ServiceSurfaceProxyError::SendFailed.to_string(),
            "failed to send surface request"
        );
    }
}
