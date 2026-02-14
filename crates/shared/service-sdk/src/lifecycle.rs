//! Service lifecycle management: bootstrap, enrollment, and reconnect loop.
//!
//! Extracts the duplicated bootstrap → enrollment → authenticated-loop flow
//! shared by the agent and MQTT services into a single
//! [`run_service_lifecycle`] function. Each service implements
//! [`ServiceHandler`] to provide its service-specific parts (enrollment info
//! and authenticated event loop), while the SDK owns the common plumbing.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use rootcause::prelude::*;
use uptrakit_internal_wire::ServiceType;

use crate::Backoff;
use crate::cli::CommonServiceArgs;
use crate::error::{EnrollmentError, Result};
use crate::identity::ServiceIdentityState;

/// Outcome of the authenticated event loop, returned by
/// [`ServiceHandler::run_authenticated_loop`].
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

/// Static configuration for a service.
pub struct ServiceConfig {
    /// Directory name used for platform-specific directory resolution
    /// (e.g. `"agent"` or `"mqtt"`).
    pub dir_name: &'static str,
    /// Human-readable label for log messages (e.g. `"uptrakit-agent service"`).
    pub service_label: &'static str,
}

/// Enrollment parameters that vary between services.
pub struct ServiceEnrollmentInfo {
    /// Whether this is an Agent or Mqtt service.
    pub service_type: ServiceType,
}

/// Context provided by the lifecycle to the authenticated loop.
pub struct AuthenticatedContext<'a> {
    /// Controller hostname.
    pub host: &'a str,
    /// Controller port.
    pub port: u16,
    /// Pre-built mTLS connector for this iteration.
    pub tls_connector: tokio_rustls::TlsConnector,
    /// Raw CA PEM bytes, if a pinned CA is in use.
    pub ca_pem: Option<&'a [u8]>,
    /// The loaded identity state (certified, mutable for cert renewal / CA update).
    pub identity: &'a mut ServiceIdentityState,
    /// Base URL for the controller (e.g. `https://host:8443`).
    pub base_url: &'a str,
    /// Optional PKI address.
    pub pki_addr: Option<&'a str>,
}

/// Trait that each service implements to plug into the shared lifecycle.
///
/// Uses a boxed future for [`run_authenticated_loop`](ServiceHandler::run_authenticated_loop)
/// to avoid higher-ranked lifetime issues that arise with `impl Future` in
/// trait methods when the implementation captures references with complex
/// lifetime relationships (e.g. `stream::iter` + `buffer_unordered` patterns).
pub trait ServiceHandler {
    /// Return static configuration for this service.
    fn config(&self) -> ServiceConfig;

    /// Return enrollment-time parameters (service type).
    fn enrollment_info(&self) -> ServiceEnrollmentInfo;

    /// Run the authenticated event loop until a [`LoopOutcome`] is reached.
    ///
    /// The lifecycle rebuilds the mTLS connector on each reconnect iteration
    /// (certificates may have rotated), so `ctx.tls_connector` is always fresh.
    fn run_authenticated_loop<'a>(
        &'a mut self,
        ctx: AuthenticatedContext<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LoopOutcome>> + Send + 'a>>;
}

/// Run the full service lifecycle: directory setup → identity load →
/// CA bootstrap → enrollment → authenticated loop with reconnect.
///
/// This is the single entry point that replaces the per-service `run()`
/// functions in agent and MQTT.
pub async fn run_service_lifecycle(
    args: &CommonServiceArgs,
    handler: &mut impl ServiceHandler,
) -> Result<()> {
    let config = handler.config();
    tracing::info!("starting {}", config.service_label);

    // Parse URL early.
    let (host, port) = args.parsed_url().map_err(|s| {
        report!(EnrollmentError::Enrollment(s))
    })?;
    let base_url = args.base_url();
    let pki_addr = args.pki_addr();

    // Resolve application directories.
    let app_dirs = args.resolve_dirs(config.dir_name).map_err(|e| {
        report!(EnrollmentError::Enrollment(e.to_string()))
    })?;
    app_dirs.ensure_dirs().map_err(|e| {
        report!(EnrollmentError::Enrollment(format!(
            "failed to create directories: {e}"
        )))
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
                if is_receive_closed_report(&e) {
                    let delay = enrollment_backoff.next_delay();
                    tracing::info!("disconnected during enrollment, reconnecting in {delay:?}");
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
async fn do_enrollment(
    args: &CommonServiceArgs,
    host: &str,
    port: u16,
    identity: &mut ServiceIdentityState,
    tls_connector: &tokio_rustls::TlsConnector,
    handler: &mut impl ServiceHandler,
) -> Result<()> {
    if identity.is_enrolled_only() {
        // Resume: reconnect with Bearer header (existing service.json).
        tracing::info!("reconnecting with enrollment secret");
        crate::ws::resume_enrollment(identity, host, port, tls_connector).await?;
    } else {
        // Fresh enrollment.
        let hostname = args.hostname();
        let friendly_name = args.friendly_name_or_hostname();
        let enrollment_info = handler.enrollment_info();

        tracing::info!("enrolling via WebSocket");
        crate::ws::run_enrollment(crate::ws::EnrollmentParams {
            identity,
            host,
            port,
            tls_connector,
            hostname: &hostname,
            friendly_name: &friendly_name,
            enrollment_token: args.enrollment_token.as_deref(),
            service_type: enrollment_info.service_type,
        })
        .await?;
    }

    tracing::info!("enrollment complete, certificate saved to disk");
    Ok(())
}

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
    loop {
        // Rebuild the mTLS connector each iteration (certificates may have rotated).
        let cert_pem = identity.cert_pem().ok_or_else(|| {
            report!(EnrollmentError::NotCertified)
        })?;
        let key_pem = identity.key_pem().ok_or_else(|| {
            report!(EnrollmentError::NotCertified)
        })?;

        let mtls_connector = match ca_pem {
            Some(pem) => crate::tls::build_tls_connector_with_client_cert(pem, cert_pem, &key_pem)?,
            None => crate::tls::build_system_trust_tls_connector_with_client_cert(cert_pem, &key_pem)?,
        };

        let ctx = AuthenticatedContext {
            host,
            port,
            tls_connector: mtls_connector,
            ca_pem,
            identity,
            base_url,
            pki_addr,
        };

        match handler.run_authenticated_loop(ctx).await? {
            LoopOutcome::Shutdown => return Ok(()),
            LoopOutcome::Reconnect => {
                reconnect_backoff.reset();
                tracing::info!("reconnecting with new certificate");
                tokio::time::sleep(Duration::from_secs(2)).await;
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
