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
use crate::signal::{Signal, SignalWatcher};

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

/// Connection parameters resolved from either CLI args or mDNS discovery.
struct ResolvedConnection {
    host: String,
    port: u16,
    base_url: String,
    pki_addr: Option<String>,
    /// When discovery provides a CA fingerprint, it is passed through to
    /// `bootstrap_ca()` as the TOFU fingerprint for automatic verification.
    discovery_tofu_fingerprint: Option<String>,
    /// Whether TOFU mode should be implicitly enabled (discovery provides
    /// the CA fingerprint, so we trust-on-first-use with fingerprint pinning).
    discovery_tofu: bool,
}

/// Resolve the controller connection from CLI args or mDNS discovery.
#[allow(unused_variables)]
async fn resolve_connection(
    args: &CommonServiceArgs,
    state_dir: &std::path::Path,
) -> std::result::Result<ResolvedConnection, String> {
    // If --url is provided, use it directly (no discovery)
    if args.url.is_some() {
        let (host, port) = args.parsed_url()?;
        let base_url = args.base_url().to_string();
        let pki_addr = args.pki_addr().map(String::from);
        return Ok(ResolvedConnection {
            host,
            port,
            base_url,
            pki_addr,
            discovery_tofu_fingerprint: None,
            discovery_tofu: false,
        });
    }

    // When zeroconf feature is compiled in, run discovery and return.
    // Uses explicit `return` so that the unconditional fallback below is
    // never reached when the feature is enabled — avoiding #[cfg(not(...))]
    // on a code block (which violates the additive-only feature-flag rule).
    #[cfg(feature = "zeroconf")]
    {
        if args.clear_discovery_cache {
            tracing::info!("clearing discovery cache");
            crate::discovery::clear_cache(state_dir)?;
        }

        let result = crate::discovery::discover(state_dir).await?;

        return match result {
            crate::discovery::DiscoveryResult::Cached(cache)
            | crate::discovery::DiscoveryResult::Discovered(cache) => {
                let parsed: url::Url = cache
                    .url
                    .parse()
                    .map_err(|e| format!("invalid discovered URL: {e}"))?;
                if parsed.scheme() != "https" {
                    return Err(format!(
                        "discovered URL scheme must be https, got: {}",
                        parsed.scheme()
                    ));
                }
                let host = parsed
                    .host_str()
                    .ok_or("discovered URL must contain a host")?
                    .to_string();
                let port = parsed.port().unwrap_or(443);
                let base_url = cache.url.trim_end_matches('/').to_string();
                let pki_addr = cache.pki_addr;
                let discovery_tofu_fingerprint = cache.ca_fingerprint;
                let discovery_tofu = discovery_tofu_fingerprint.is_some();
                Ok(ResolvedConnection {
                    host,
                    port,
                    base_url,
                    pki_addr,
                    discovery_tofu_fingerprint,
                    discovery_tofu,
                })
            }
            crate::discovery::DiscoveryResult::NotFound => {
                Err("no controller found via mDNS discovery. \
                 Use --url to specify the controller address explicitly."
                    .to_string())
            }
        };
    }

    // Reached only when the zeroconf feature is absent (the block above is
    // compiled out and the compiler eliminates the dead path). Without
    // discovery, --url is required. The allow suppresses the unreachable-code
    // lint when zeroconf is compiled in.
    #[allow(unreachable_code)]
    Err("--url is required when zeroconf is not available".to_string())
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

    // Resolve application directories first (needed for discovery cache).
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

    // Resolve controller connection (from --url or mDNS discovery).
    let conn = resolve_connection(args, app_dirs.state_dir())
        .await
        .map_err(|s| report!(EnrollmentError::Protocol(ProtocolError::Init(s))))?;

    let host = conn.host;
    let port = conn.port;
    let base_url_owned = conn.base_url;
    let pki_addr_owned = conn.pki_addr;

    let base_url: &str = &base_url_owned;
    let pki_addr: Option<&str> = pki_addr_owned.as_deref();

    // Create and load identity state.
    let mut identity = ServiceIdentityState::new(app_dirs.config_dir(), app_dirs.state_dir());
    identity.load().await?;

    // --force-enroll: clear existing enrollment state (preserves CA cert).
    if args.force_enroll {
        tracing::info!("--force-enroll: clearing existing enrollment state");
        identity.clear_enrollment_state().await?;
    }

    // CA bootstrap: cached → --ca-cert file → --pki-addr → --tofu TOFU → system trust.
    // When discovery provided a CA fingerprint, implicitly enable TOFU with that fingerprint.
    let effective_tofu = args.tofu || conn.discovery_tofu;
    let effective_tofu_fingerprint = args
        .tofu_fingerprint
        .as_deref()
        .or(conn.discovery_tofu_fingerprint.as_deref());
    let ca_pem = crate::ca::bootstrap_ca(
        &mut identity,
        base_url,
        effective_tofu,
        effective_tofu_fingerprint,
        args.ca_cert.as_deref(),
        pki_addr,
    )
    .await?;

    // Create the signal watcher once for the entire lifetime of this service
    // process.  Sharing a single instance across reconnect and enrollment
    // backoff loops ensures signals received during sleep intervals are not
    // lost — tokio buffers one notification per signal kind.
    let mut signals = SignalWatcher::new().map_err(|e| {
        report!(EnrollmentError::Protocol(ProtocolError::Init(format!(
            "failed to register signal handlers: {e}"
        ))))
    })?;

    let auth_params = AuthLoopParams {
        host: &host,
        port,
        base_url,
        pki_addr,
        initial_ca_pem: ca_pem.as_deref(),
    };

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
                &auth_params,
                &mut identity,
                handler,
                &mut signals,
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
                    // Interruptible sleep: a SIGTERM/SIGINT during enrollment
                    // backoff exits cleanly instead of waiting up to 60 s.
                    tokio::select! {
                        () = tokio::time::sleep(delay) => {}
                        signal = signals.recv() => {
                            tracing::info!(%signal, "received signal during enrollment, exiting");
                            return Ok(());
                        }
                    }
                    // Reload identity in case enrollment partially completed.
                    identity.load().await?;
                    continue;
                }
                return Err(e);
            }
        }
    }

    // Enter mTLS loop with reconnect.
    run_authenticated_with_reconnect(&auth_params, &mut identity, handler, &mut signals).await
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

/// Stable connection parameters shared across reconnect iterations.
struct AuthLoopParams<'a> {
    host: &'a str,
    port: u16,
    base_url: &'a str,
    pki_addr: Option<&'a str>,
    /// Initial CA PEM bytes (seeded from bootstrap; updated each iteration
    /// from the in-memory identity so `CaBundleUpdated` is picked up).
    initial_ca_pem: Option<&'a [u8]>,
}

/// Enter the mTLS authenticated loop with automatic reconnection.
async fn run_authenticated_with_reconnect(
    params: &AuthLoopParams<'_>,
    identity: &mut ServiceIdentityState,
    handler: &mut impl ServiceHandler,
    signals: &mut SignalWatcher,
) -> Result<()> {
    let mut reconnect_backoff = Backoff::new(Duration::from_secs(2), Duration::from_secs(60));

    // Seed from the bootstrapped CA bytes. Updated each iteration from the
    // in-memory identity state so that a `CaBundleUpdated` message received
    // during a prior connection (which calls `identity.save_ca_cert()`) is
    // picked up without a restart.
    let mut current_ca: Option<Vec<u8>> = params.initial_ca_pem.map(<[u8]>::to_vec);

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
            base_url: params.base_url,
            pki_addr: params.pki_addr,
            ca_pem: current_ca.as_deref(),
        };

        let outcome = match crate::event_loop::run_event_loop(
            handler,
            params.host,
            params.port,
            &mtls_connector,
            identity,
            &ctx,
            signals,
        )
        .await
        {
            Ok(outcome) => {
                // The event loop connected and ran successfully for some
                // period.  Reset backoff so the next reconnect starts from
                // the base delay instead of continuing to grow from a
                // previous failure streak.
                reconnect_backoff.reset();
                outcome
            }
            Err(e) => match e.current_context() {
                // Transient network errors (broken pipe, connection reset, DNS
                // failure, send timeout, etc.) are recoverable — reconnect with
                // exponential backoff instead of crashing the service.
                LoopError::TransientNetwork(_) | LoopError::ReceiveClosed => {
                    let delay = reconnect_backoff.next_delay();
                    tracing::warn!(
                        error = %e,
                        "connection lost, reconnecting in {delay:?}"
                    );
                    // Interruptible sleep: a SIGTERM/SIGINT during the reconnect
                    // backoff exits cleanly instead of waiting up to 60 s.
                    tokio::select! {
                        () = tokio::time::sleep(delay) => {}
                        signal = signals.recv() => {
                            tracing::info!(%signal, "received signal during reconnect delay, exiting");
                            return Ok(());
                        }
                    }
                    continue;
                }
                // Non-transient errors are fatal — propagate to the caller.
                LoopError::CertExpired => {
                    return Err(e.context_transform(|_: LoopError| {
                        EnrollmentError::Tls(crate::error::TlsError::Rustls(
                            rustls::Error::AlertReceived(
                                rustls::AlertDescription::CertificateExpired,
                            ),
                        ))
                    }));
                }
                LoopError::Other(_) => {
                    return Err(e.context_transform(|e: LoopError| {
                        EnrollmentError::Protocol(ProtocolError::Enrollment(e.to_string()))
                    }));
                }
            },
        };

        match outcome {
            LoopOutcome::Shutdown => return Ok(()),
            LoopOutcome::Reconnect => {
                // backoff already reset above after Ok(outcome)
                tracing::info!("reconnecting with new certificate");
                tokio::select! {
                    () = tokio::time::sleep(CERT_RECONNECT_DELAY) => {}
                    signal = signals.recv() => {
                        tracing::info!(%signal, "received signal during cert reconnect delay, exiting");
                        return Ok(());
                    }
                }
                continue;
            }
            LoopOutcome::Disconnected => {
                let delay = reconnect_backoff.next_delay();
                tracing::warn!("disconnected by controller, reconnecting in {delay:?}");
                // Interruptible sleep: a SIGTERM/SIGINT during the reconnect
                // backoff exits cleanly instead of waiting up to 60 s.
                tokio::select! {
                    () = tokio::time::sleep(delay) => {}
                    signal = signals.recv() => {
                        tracing::info!(%signal, "received signal during reconnect delay, exiting");
                        return Ok(());
                    }
                }
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

    #[test]
    fn loop_error_from_websocket_is_transient_network() {
        let ws_err =
            EnrollmentError::WebSocket(tokio_tungstenite::tungstenite::Error::ConnectionClosed);
        let report: Report<EnrollmentError> = report!(ws_err);
        let converted: Report<LoopError> = report.context_transform(|e: EnrollmentError| {
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
        assert!(
            matches!(converted.current_context(), LoopError::TransientNetwork(_)),
            "WebSocket error should map to TransientNetwork"
        );
    }

    #[test]
    fn loop_error_from_broken_pipe_is_transient_network() {
        let io_err = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "broken pipe");
        let ws_err = EnrollmentError::Io(io_err);
        let report: Report<EnrollmentError> = report!(ws_err);
        let converted: Report<LoopError> = report.context_transform(|e: EnrollmentError| {
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
        assert!(
            matches!(converted.current_context(), LoopError::TransientNetwork(_)),
            "IO broken pipe should map to TransientNetwork"
        );
    }

    #[test]
    fn loop_error_from_enrollment_rejected_is_other() {
        let err = EnrollmentError::Protocol(ProtocolError::EnrollmentRejected);
        let report: Report<EnrollmentError> = report!(err);
        let converted: Report<LoopError> = report.context_transform(|e: EnrollmentError| {
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
        assert!(
            matches!(converted.current_context(), LoopError::Other(_)),
            "EnrollmentRejected should map to Other (fatal)"
        );
    }
}
