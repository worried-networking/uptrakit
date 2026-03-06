//! Service lifecycle management: bootstrap, enrollment, and reconnect loop.
//!
//! Extracts the duplicated bootstrap → enrollment → authenticated-loop flow
//! shared by all services into a single [`run_service_lifecycle`] function.
//! Each service implements [`ServiceHandler`] to provide its service-specific
//! parts (callbacks for connection, messages, shutdown), while the SDK owns
//! the common plumbing including the unified event loop.

use std::collections::BTreeSet;
use std::time::Duration;

use async_trait::async_trait;

use rootcause::prelude::*;
use uptrakit_internal_wire::{
    Capability, ControllerMessage, ServiceMessage, ServiceSettingsPayload,
    extension::{ExtensionRequestPayload, ExtensionResponsePayload},
};
use uptrakit_shared_macros::impl_report_conversion;

use crate::Backoff;
use crate::cli::CommonServiceArgs;
use crate::connection::ControllerConnection;
use crate::error::{EnrollmentError, IdentityError, ProtocolError, Result};
use crate::identity::ServiceIdentityState;
use crate::signal::Signal;

/// Error type returned by event loop callbacks in [`ServiceHandler`].
///
/// Each variant carries the semantic meaning needed by the lifecycle to
/// decide whether to re-enroll, reconnect with backoff, or propagate —
/// without requiring services to construct internal SDK error types.
#[derive(Debug, thiserror::Error)]
pub enum LoopError {
    /// TLS handshake rejected: server considers our client certificate expired.
    #[error("certificate expired")]
    CertExpired,
    /// WebSocket connection cleanly closed by the controller.
    #[error("connection closed by controller")]
    ReceiveClosed,
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
        shutdown_timeout_seconds: u32,
    ) -> LoopOutcome;
}

/// Default shutdown cause → outcome mapping shared by all service binaries.
///
/// | Cause | `DisconnectReason` | `LoopOutcome` |
/// | --- | --- | --- |
/// | `Signal(Hangup)` | `Restart` | `Restart` |
/// | `Signal(_)` | `Shutdown` | `Shutdown` |
/// | `ServerRestarting` | `Restart` | `Disconnected` |
pub fn default_resolve_shutdown(
    cause: ShutdownCause,
) -> (uptrakit_internal_wire::DisconnectReason, LoopOutcome) {
    use uptrakit_internal_wire::DisconnectReason;

    match cause {
        ShutdownCause::Signal(Signal::Hangup) => (DisconnectReason::Restart, LoopOutcome::Restart),
        ShutdownCause::Signal(_) => (DisconnectReason::Shutdown, LoopOutcome::Shutdown),
        ShutdownCause::ServerRestarting => (DisconnectReason::Restart, LoopOutcome::Disconnected),
    }
}

/// Run the full service lifecycle: directory setup → identity load →
/// CA bootstrap → enrollment → authenticated loop with reconnect.
///
/// This is the single entry point that replaces the per-service `run()`
/// functions in agent and MQTT.
pub async fn run_service_lifecycle<H: ServiceHandler>(
    args: &CommonServiceArgs,
    handler: &mut H,
) -> Result<()> {
    tracing::info!("starting {}", H::SERVICE_LABEL);

    // Parse URL early.
    let (host, port) = args
        .parsed_url()
        .map_err(|s| report!(EnrollmentError::Protocol(ProtocolError::Init(s))))?;
    let base_url = args.base_url();
    let pki_addr = args.pki_addr();

    // Resolve application directories.
    let app_dirs = args.resolve_dirs(H::DIR_NAME).map_err(|e| {
        report!(EnrollmentError::Protocol(ProtocolError::Init(
            e.to_string()
        )))
    })?;
    app_dirs.ensure_dirs().await.map_err(|e| {
        report!(EnrollmentError::Protocol(ProtocolError::Init(format!(
            "failed to create directories: {e}"
        ))))
    })?;
    tracing::info!("config directory: {}", app_dirs.config_dir().display());
    tracing::info!("state directory: {}", app_dirs.state_dir().display());

    // Create and load identity state.
    let mut identity = ServiceIdentityState::new(app_dirs.config_dir(), app_dirs.state_dir());
    identity.load().await?;

    // --force-enroll: clear existing enrollment state (preserves CA cert).
    if args.force_enroll {
        tracing::info!("--force-enroll: clearing existing enrollment state");
        identity.clear_enrollment_state().await?;
    }

    // CA bootstrap: cached → --ca-cert file → --pki-addr → --tofu TOFU → system trust.
    let ca_pem = crate::ca::bootstrap_ca(
        &mut identity,
        base_url,
        args.tofu,
        args.tofu_fingerprint.as_deref(),
        args.ca_cert.as_deref(),
        pki_addr,
    )
    .await?;

    // Check for existing certificate.
    if identity.is_certified() {
        let cert_not_after_ts = identity.cert_not_after_ms();
        let cert_expired =
            cert_not_after_ts.is_some_and(|ts| uptrakit_internal_wire::now_millis() >= ts);

        if cert_expired {
            tracing::warn!("certificate expired, falling back to fresh enrollment");
            identity.clear_enrollment_state().await?;
            // Fall through to enrollment below.
        } else {
            tracing::info!("loaded existing certificate from disk");
            match run_authenticated_with_reconnect(
                &host,
                port,
                base_url,
                pki_addr,
                ca_pem.as_deref(),
                &mut identity,
                handler,
            )
            .await
            {
                Ok(()) => return Ok(()),
                Err(e) => {
                    if is_cert_expired_report(&e) {
                        tracing::warn!("certificate expired, falling back to enrollment");
                        identity.clear_enrollment_state().await?;
                        // Fall through to enrollment below.
                    } else {
                        return Err(e);
                    }
                }
            }
        }
    }

    // Build TLS connector for enrollment (server-auth only, no client cert).
    let tls_connector = match ca_pem.as_deref() {
        Some(pem) => crate::tls::build_tls_connector(pem)?,
        None => crate::tls::build_system_trust_tls_connector()?,
    };

    // Enrollment with backoff loop.
    let mut enrollment_backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));
    loop {
        match do_enrollment(args, &host, port, &mut identity, &tls_connector, handler).await {
            Ok(()) => break,
            Err(e) => {
                if is_receive_closed_report(&e) || is_transient_network_report(&e) {
                    let delay = enrollment_backoff.next_delay();
                    tracing::info!(
                        error = %e,
                        "transient enrollment error, reconnecting in {delay:?}"
                    );
                    tokio::time::sleep(delay).await;
                    // Reload identity in case enrollment partially completed.
                    identity.load().await?;
                    continue;
                }
                return Err(e);
            }
        }
    }

    // Enter mTLS loop with reconnect.
    run_authenticated_with_reconnect(
        &host,
        port,
        base_url,
        pki_addr,
        ca_pem.as_deref(),
        &mut identity,
        handler,
    )
    .await
}

/// Run enrollment using the shared enrollment module.
async fn do_enrollment<H: ServiceHandler>(
    args: &CommonServiceArgs,
    host: &str,
    port: u16,
    identity: &mut ServiceIdentityState,
    tls_connector: &tokio_rustls::TlsConnector,
    handler: &mut H,
) -> Result<()> {
    if identity.is_enrolled_only() {
        // Resume: reconnect with Bearer header (existing service.json).
        tracing::info!("reconnecting with enrollment secret");
        crate::ws::resume_enrollment(identity, host, port, tls_connector).await?;
    } else {
        // Fresh enrollment.
        let hostname = args.hostname();
        let friendly_name = args.friendly_name_or_hostname();

        tracing::info!("enrolling via WebSocket");
        crate::ws::run_enrollment(crate::ws::EnrollmentParams {
            identity,
            host,
            port,
            tls_connector,
            hostname: &hostname,
            friendly_name: &friendly_name,
            enrollment_token: args.enrollment_token.as_deref(),
            capabilities: handler.capabilities(),
            service_app_name: H::SERVICE_APP_NAME,
        })
        .await?;
    }

    tracing::info!("enrollment complete, certificate saved to disk");
    Ok(())
}

/// Delay before reconnecting after certificate rotation. Allows the
/// controller to finalize rotation before the service reconnects with
/// its new certificate.
const CERT_RECONNECT_DELAY: Duration = Duration::from_secs(2);

/// Enter the mTLS authenticated loop with automatic reconnection.
async fn run_authenticated_with_reconnect(
    host: &str,
    port: u16,
    base_url: &str,
    pki_addr: Option<&str>,
    ca_pem: Option<&[u8]>,
    identity: &mut ServiceIdentityState,
    handler: &mut impl ServiceHandler,
) -> Result<()> {
    let mut reconnect_backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));

    // Seed from the bootstrapped CA bytes. Updated each iteration from the
    // in-memory identity state so that a `CaBundleUpdated` message received
    // during a prior connection (which calls `identity.save_ca_cert()`) is
    // picked up without a restart.
    let mut current_ca: Option<Vec<u8>> = ca_pem.map(<[u8]>::to_vec);

    loop {
        // Refresh the CA from the in-memory identity cache.
        // `save_ca_cert` (called by `CaBundleUpdated` and `check_ca_staleness`)
        // keeps `identity.ca_cert_pem()` current between reconnects.
        if let Some(s) = identity.ca_cert_pem() {
            current_ca = Some(s.as_bytes().to_vec());
        }

        // Rebuild the mTLS connector each iteration (certificates may have rotated).
        let cert_pem = identity
            .cert_pem()
            .ok_or_else(|| report!(EnrollmentError::Identity(IdentityError::NotCertified)))?;
        let key_pem = identity
            .key_pem()
            .ok_or_else(|| report!(EnrollmentError::Identity(IdentityError::NotCertified)))?;

        let mtls_connector = match current_ca.as_deref() {
            Some(pem) => crate::tls::build_tls_connector_with_client_cert(pem, cert_pem, &key_pem)?,
            None => {
                crate::tls::build_system_trust_tls_connector_with_client_cert(cert_pem, &key_pem)?
            }
        };

        let ctx = crate::event_loop::EventLoopContext {
            base_url,
            pki_addr,
            ca_pem: current_ca.as_deref(),
        };

        match crate::event_loop::run_event_loop(
            handler,
            host,
            port,
            &mtls_connector,
            identity,
            &ctx,
        )
        .await
        .context_transform(|e: LoopError| match e {
            LoopError::CertExpired => EnrollmentError::Tls(crate::error::TlsError::Rustls(
                rustls::Error::AlertReceived(rustls::AlertDescription::CertificateExpired),
            )),
            LoopError::ReceiveClosed => EnrollmentError::Protocol(ProtocolError::ReceiveClosed),
            LoopError::Other(msg) => EnrollmentError::Protocol(ProtocolError::Enrollment(msg)),
        })? {
            LoopOutcome::Shutdown => return Ok(()),
            LoopOutcome::Reconnect => {
                reconnect_backoff.reset();
                tracing::info!("reconnecting with new certificate");
                tokio::time::sleep(CERT_RECONNECT_DELAY).await;
                continue;
            }
            LoopOutcome::Disconnected => {
                let delay = reconnect_backoff.next_delay();
                tracing::warn!("disconnected by controller, reconnecting in {delay:?}");
                tokio::time::sleep(delay).await;
                continue;
            }
            LoopOutcome::Restart => {
                tracing::info!("restart requested, exiting for external restart");
                return Ok(());
            }
        }
    }
}

/// Check if a `Report<EnrollmentError>` represents a cert-expired condition.
fn is_cert_expired_report(report: &Report<EnrollmentError>) -> bool {
    report.current_context().is_cert_expired()
}

/// Check if a `Report<EnrollmentError>` represents a receive-closed condition.
fn is_receive_closed_report(report: &Report<EnrollmentError>) -> bool {
    report.current_context().is_receive_closed()
}

/// Check if a `Report<EnrollmentError>` represents a transient network error
/// that should be retried with backoff (DNS failure, TCP refused, etc.).
fn is_transient_network_report(report: &Report<EnrollmentError>) -> bool {
    report.current_context().is_transient_network()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_internal_wire::DisconnectReason;

    #[test]
    fn default_resolve_shutdown_hangup() {
        let (reason, outcome) = default_resolve_shutdown(ShutdownCause::Signal(Signal::Hangup));
        assert_eq!(reason, DisconnectReason::Restart);
        assert_eq!(outcome, LoopOutcome::Restart);
    }

    #[test]
    fn default_resolve_shutdown_terminate() {
        let (reason, outcome) = default_resolve_shutdown(ShutdownCause::Signal(Signal::Terminate));
        assert_eq!(reason, DisconnectReason::Shutdown);
        assert_eq!(outcome, LoopOutcome::Shutdown);
    }

    #[test]
    fn default_resolve_shutdown_server_restarting() {
        let (reason, outcome) = default_resolve_shutdown(ShutdownCause::ServerRestarting);
        assert_eq!(reason, DisconnectReason::Restart);
        assert_eq!(outcome, LoopOutcome::Disconnected);
    }
}
