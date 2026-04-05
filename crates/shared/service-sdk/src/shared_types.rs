//! Shared types used by both the lifecycle and event-loop modules.
//!
//! This module breaks the bidirectional dependency between `lifecycle.rs`
//! and `event_loop.rs` by providing a neutral home for the types that both
//! modules need: error types, outcome enums, the [`ServiceHandler`] trait,
//! and the [`EventLoopContext`] struct.

use std::collections::BTreeSet;
use std::time::Duration;

use async_trait::async_trait;

use rootcause::prelude::*;
use uptrakit_internal_wire::{
    Capability, ControllerMessage, ServiceMessage, ServiceSettingsPayload,
    extension::{ExtensionRequestPayload, ExtensionResponsePayload},
};
use uptrakit_shared_macros::impl_report_conversion;

use crate::connection::ControllerConnection;
use crate::error::EnrollmentError;
use crate::identity::ServiceIdentityState;
use crate::signal::Signal;

/// Error type returned by event loop callbacks in [`ServiceHandler`].
///
/// Each variant carries the semantic meaning needed by the lifecycle to
/// decide whether to re-enroll, reconnect with backoff, or propagate —
/// without requiring services to construct internal SDK error types.
///
/// Classification priority for `EnrollmentError -> LoopError` conversion:
///
/// | Priority | Predicate | `LoopError` variant |
/// | --- | --- | --- |
/// | 1 | `is_cert_expired()` | `CertExpired` |
/// | 2 | `is_receive_closed()` | `ReceiveClosed` |
/// | 3 | `is_transient_network()` | `TransientNetwork` |
/// | 4 | fallback | `Other` |
///
/// Priority order is strict. Without this ordering and the guards in
/// `is_transient_network()`, `WebSocket(Io(cert_expired))` could be
/// misclassified as transient instead of `CertExpired`.
#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    /// TLS handshake rejected: server considers our client certificate expired.
    #[error("certificate expired")]
    CertExpired,
    /// WebSocket connection cleanly closed by the controller.
    ///
    /// This variant is not produced by `ControllerConnection::recv()` inside
    /// the authenticated event loop (that method maps close/EOF to `Ok(None)`).
    /// It remains part of `LoopError` because the shared conversion from
    /// `EnrollmentError` is reused by enrollment/lifecycle callers.
    #[error("connection closed by controller")]
    ReceiveClosed,
    /// Transient network error (broken pipe, connection reset, DNS failure, etc.)
    /// that should trigger reconnection with backoff rather than a fatal exit.
    #[error("transient network error: {0}")]
    TransientNetwork(String),
    /// Other error during the event loop.
    #[error("{0}")]
    Other(String),
}

/// Result alias for [`ServiceHandler`] callbacks and the event loop.
pub type LoopResult<T> = std::result::Result<T, Report<LoopError>>;

impl_report_conversion!(EnrollmentError => LoopError, |e| {
    if e.is_cert_expired() {
        LoopError::CertExpired
    } else if e.is_receive_closed() {
        LoopError::ReceiveClosed
    } else if e.is_transient_network() {
        LoopError::TransientNetwork(e.to_string())
    } else {
        LoopError::Other(e.to_string())
    }
});

/// Cause of a service shutdown, passed to [`ServiceHandler::on_shutdown`].
///
/// Services use this to choose the appropriate [`DisconnectReason`] and
/// [`LoopOutcome`]:
///
/// | Cause | `DisconnectReason` | `LoopOutcome` |
/// | --- | --- | --- |
/// | `Signal(Hangup)` | `Restart` | `Restart` |
/// | `Signal(_)` | `Shutdown` | `Shutdown` |
/// | `ServerRestarting` | `Restart` | `Disconnected` |
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShutdownCause {
    /// An OS signal was received (`SIGINT`, `SIGTERM`, `SIGHUP`).
    Signal(Signal),
    /// The controller sent `ServerRestarting`; the service should disconnect
    /// and reconnect once the controller is available again.
    ServerRestarting,
}

/// Outcome of the authenticated event loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopOutcome {
    /// SIGINT/SIGTERM received — exit the lifecycle cleanly.
    Shutdown,
    /// Certificate rotated — reconnect immediately (reset backoff).
    Reconnect,
    /// Connection closed — reconnect with backoff.
    Disconnected,
    /// Service-specific restart (agent SIGHUP) — exit the lifecycle.
    Restart,
}

/// Context for the event loop, providing connection metadata that callbacks
/// may need.
pub struct EventLoopContext<'a> {
    /// Base URL for the controller (e.g. `https://host:8443`).
    pub base_url: &'a str,
    /// Optional PKI address.
    pub pki_addr: Option<&'a str>,
    /// Raw CA PEM bytes, if a pinned CA is in use.
    pub ca_pem: Option<&'a [u8]>,
}

/// Trait that each service implements to plug into the shared lifecycle
/// and unified event loop.
///
/// All async methods are desugared by [`async_trait`] into
/// `Pin<Box<dyn Future + Send + '_>>`, matching the established pattern
/// used across the codebase (Plugin, CommandExecutor, TaskExecutor, etc.).
#[async_trait]
pub trait ServiceHandler: Send {
    /// Directory name used for platform-specific directory resolution
    /// (e.g. `"agent"` or `"mqtt"`).
    const DIR_NAME: &'static str;

    /// Human-readable label for log messages (e.g. `"uptrakit-agent service"`).
    const SERVICE_LABEL: &'static str;

    /// The binary/crate name sent during enrollment (e.g., `"uptrakit-agent-ssh"`).
    ///
    /// Implementors should set this to `env!("CARGO_PKG_NAME")` so it's derived
    /// automatically from each binary's `Cargo.toml`.
    const SERVICE_APP_NAME: &'static str;

    /// Service-specific event type from [`poll_service_event`](Self::poll_service_event).
    ///
    /// Use [`std::convert::Infallible`] for services with no custom select arms.
    type ServiceEvent: Send;

    /// Called after the WebSocket connection is established.
    ///
    /// Send initial messages (e.g. `ReportHosts`, `Register`) here.
    async fn on_connected(
        &mut self,
        conn: &mut ControllerConnection,
        identity: &ServiceIdentityState,
    ) -> LoopResult<()>;

    /// Handle a [`ControllerMessage`] not handled by the SDK.
    ///
    /// The SDK handles: `Pong`, `Certificate`, `ServiceSettings`,
    /// `CaBundleUpdated`, `RequestCertRenewal`, `ServerRestarting`.
    /// Everything else is delegated to this callback.
    ///
    /// Return `Ok(Some(outcome))` to break the loop, `Ok(None)` to continue.
    async fn on_message(
        &mut self,
        msg: ControllerMessage,
        conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>>;

    /// Called after the SDK processes shared `ServiceSettings` fields.
    ///
    /// The SDK already handles capability negotiation, renewal schedule,
    /// shutdown timeout, and CA staleness. Override this to send capability-
    /// dependent messages (e.g. `ExtensionRegister` when `UiExtensions` is in
    /// `conn.agreed_capabilities()`) or for additional service-specific
    /// settings processing.
    async fn on_settings(
        &mut self,
        _settings: &ServiceSettingsPayload,
        _conn: &mut ControllerConnection,
    ) {
    }

    /// Returns the set of capabilities this service supports.
    ///
    /// The SDK intersects this set with the controller's advertised capabilities
    /// (from `ServiceSettings`) to compute the agreed capability set. Only typed
    /// (known) variants participate in the intersection.
    ///
    /// Services should override this to advertise their actual capabilities.
    /// The default implementation returns an empty set.
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::new()
    }

    /// Poll for service-specific events (additional `select!` arm).
    ///
    /// Return [`std::future::pending()`] if the service has no custom events.
    /// The returned future is dropped when another `select!` arm fires,
    /// releasing the `&mut self` borrow.
    async fn poll_service_event(&mut self) -> Self::ServiceEvent;

    /// Handle a resolved service event.
    ///
    /// Return `Ok(Some(outcome))` to break the loop, `Ok(None)` to continue.
    async fn on_service_event(
        &mut self,
        event: Self::ServiceEvent,
        conn: &mut ControllerConnection,
    ) -> LoopResult<Option<LoopOutcome>>;

    /// Handle an extension action response from the controller.
    ///
    /// Called when the controller sends `ControllerMessage::ExtensionResponse`
    /// in reply to a service-initiated `ServiceMessage::ExtensionRequest`.
    /// The default implementation does nothing. Services using
    /// [`ServiceExtensionProxy`](crate::ServiceExtensionProxy) should override
    /// this to call `proxy.complete()`.
    fn on_extension_response(&mut self, _response: ExtensionResponsePayload) {}

    /// Handle a service config ACK from the controller.
    ///
    /// Called when the controller sends `ControllerMessage::ServiceConfigAck`
    /// in reply to a service-initiated `StoreServiceConfig` or
    /// `DeleteServiceConfig`. The default implementation does nothing. Services
    /// using [`ServiceConfigProxy`](crate::ServiceConfigProxy) should override
    /// this to call `proxy.complete()`.
    fn on_service_config_ack(
        &self,
        _ack: uptrakit_internal_wire::payloads::ServiceConfigAckPayload,
    ) {
    }

    /// Handle an extension action request from the controller.
    ///
    /// The default implementation responds with a "not supported" error.
    /// Services that register UI extensions should override this to handle
    /// their specific actions.
    async fn on_extension_request(
        &mut self,
        request: ExtensionRequestPayload,
        conn: &mut ControllerConnection,
    ) -> LoopResult<()> {
        let response = ExtensionResponsePayload {
            request_id: request.request_id,
            success: false,
            data: serde_json::Value::Null,
            error: Some("Extension actions not supported by this service".to_owned()),
        };
        conn.send(ServiceMessage::ExtensionResponse(response))
            .await
            .map_err(|e| {
                report!(LoopError::Other(format!(
                    "failed to send extension response: {e}"
                )))
            })?;
        Ok(())
    }

    /// Graceful shutdown: send `Disconnecting` and drain in-flight work.
    ///
    /// `cause` indicates whether shutdown was triggered by an OS signal or
    /// by the controller sending `ServerRestarting`. Services should map the
    /// cause to a [`DisconnectReason`] and a [`LoopOutcome`] following the
    /// table in [`ShutdownCause`].
    async fn on_shutdown(
        &mut self,
        conn: &mut ControllerConnection,
        cause: ShutdownCause,
        shutdown_timeout: Duration,
    ) -> LoopOutcome;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::*;

    #[test]
    fn conversion_cert_expired_tls() {
        let enrollment_err = EnrollmentError::Tls(TlsError::Rustls(rustls::Error::AlertReceived(
            rustls::AlertDescription::CertificateExpired,
        )));
        let report: Report<EnrollmentError> = report!(enrollment_err);
        let loop_report: Report<LoopError> = report.context_to();
        assert!(matches!(
            loop_report.current_context(),
            LoopError::CertExpired
        ));
    }

    #[test]
    fn conversion_cert_expired_websocket_io() {
        let enrollment_err = EnrollmentError::WebSocket(tokio_tungstenite::tungstenite::Error::Io(
            std::io::Error::other(rustls::Error::AlertReceived(
                rustls::AlertDescription::CertificateExpired,
            )),
        ));
        let report: Report<EnrollmentError> = report!(enrollment_err);
        let loop_report: Report<LoopError> = report.context_to();
        assert!(matches!(
            loop_report.current_context(),
            LoopError::CertExpired
        ));
    }

    #[test]
    fn conversion_cert_revoked_tls() {
        let enrollment_err = EnrollmentError::Tls(TlsError::Rustls(rustls::Error::AlertReceived(
            rustls::AlertDescription::CertificateRevoked,
        )));
        let report: Report<EnrollmentError> = report!(enrollment_err);
        let loop_report: Report<LoopError> = report.context_to();
        assert!(matches!(loop_report.current_context(), LoopError::Other(_)));
    }

    #[test]
    fn conversion_cert_revoked_websocket_io() {
        let enrollment_err = EnrollmentError::WebSocket(tokio_tungstenite::tungstenite::Error::Io(
            std::io::Error::other(rustls::Error::AlertReceived(
                rustls::AlertDescription::CertificateRevoked,
            )),
        ));
        let report: Report<EnrollmentError> = report!(enrollment_err);
        let loop_report: Report<LoopError> = report.context_to();
        assert!(matches!(loop_report.current_context(), LoopError::Other(_)));
    }

    #[test]
    fn conversion_cert_revoked_io_direct() {
        let enrollment_err = EnrollmentError::Io(std::io::Error::other(
            rustls::Error::AlertReceived(rustls::AlertDescription::CertificateRevoked),
        ));
        let report: Report<EnrollmentError> = report!(enrollment_err);
        let loop_report: Report<LoopError> = report.context_to();
        assert!(matches!(loop_report.current_context(), LoopError::Other(_)));
    }

    #[test]
    fn conversion_transient_websocket() {
        let enrollment_err = EnrollmentError::WebSocket(tokio_tungstenite::tungstenite::Error::Io(
            std::io::Error::from(std::io::ErrorKind::ConnectionReset),
        ));
        let report: Report<EnrollmentError> = report!(enrollment_err);
        let loop_report: Report<LoopError> = report.context_to();
        assert!(matches!(
            loop_report.current_context(),
            LoopError::TransientNetwork(_)
        ));
    }

    #[test]
    fn conversion_receive_closed() {
        let enrollment_err = EnrollmentError::Protocol(ProtocolError::ReceiveClosed);
        let report: Report<EnrollmentError> = report!(enrollment_err);
        let loop_report: Report<LoopError> = report.context_to();
        assert!(matches!(
            loop_report.current_context(),
            LoopError::ReceiveClosed
        ));
    }

    #[test]
    fn conversion_version_mismatch() {
        let enrollment_err = EnrollmentError::Protocol(ProtocolError::VersionMismatch {
            expected: 1,
            received: 2,
        });
        let report: Report<EnrollmentError> = report!(enrollment_err);
        let loop_report: Report<LoopError> = report.context_to();
        assert!(matches!(loop_report.current_context(), LoopError::Other(_)));
    }

    #[test]
    fn conversion_sequence_validation() {
        let enrollment_err = EnrollmentError::Protocol(ProtocolError::Enrollment(
            "sequence validation failed: expected 5 got 3".to_string(),
        ));
        let report: Report<EnrollmentError> = report!(enrollment_err);
        let loop_report: Report<LoopError> = report.context_to();
        assert!(matches!(loop_report.current_context(), LoopError::Other(_)));
    }
}
