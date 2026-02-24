//! SDK-managed event loop for authenticated service connections.
//!
//! [`run_event_loop`] provides the unified `tokio::select!` loop that all
//! services share: ping/pong, certificate renewal, CA updates, signal
//! handling, and close-reason dispatch. Service-specific behaviour is
//! injected through the [`ServiceHandler`](crate::lifecycle::ServiceHandler)
//! trait callbacks.

use std::path::Path;
use std::pin::Pin;

use std::collections::BTreeSet;

use uptrakit_internal_wire::{
    Capability, CloseReason, ControllerMessage, PingPayload, ServiceMessage,
    ServiceSettingsPayload, now_millis,
};

use rootcause::prelude::*;

use crate::cert_handler::{
    CertificateRenewalHandler, create_renewal_sleep, update_renewal_schedule,
};
use crate::connection::ControllerConnection;
use crate::identity::ServiceIdentityState;
use crate::lifecycle::{LoopError, LoopOutcome, LoopResult, ServiceHandler, ShutdownCause};
use crate::signal::SignalWatcher;

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

/// Run the unified event loop for an authenticated service connection.
///
/// Handles:
///
/// - Ping/pong keepalive
/// - Certificate renewal (timer-based and controller-requested)
/// - CA bundle updates
/// - `ServiceSettings` processing (capability negotiation, renewal schedule,
///   shutdown timeout, CA staleness)
/// - Signal handling (`SIGINT`, `SIGTERM`, `SIGHUP`)
/// - Connection close reason dispatch
///
/// Service-specific behaviour is injected through [`ServiceHandler`]
/// callbacks.
pub(crate) async fn run_event_loop<H: ServiceHandler>(
    handler: &mut H,
    host: &str,
    port: u16,
    tls_connector: &tokio_rustls::TlsConnector,
    identity: &mut ServiceIdentityState,
    ctx: &EventLoopContext<'_>,
) -> LoopResult<LoopOutcome> {
    const DEFAULT_SHUTDOWN_TIMEOUT: u32 = 120;

    let cert_not_after_ts = identity.cert_not_after_ms();
    // Clone config_dir to avoid borrow conflicts with `&mut identity`.
    let config_dir = identity.config_dir().to_path_buf();

    tracing::info!("connecting to controller (authenticated)");
    let mut conn = ControllerConnection::connect(host, port, tls_connector, None)
        .await
        .context_to::<LoopError>()?;

    // Let the service handle post-connect initialization.
    handler.on_connected(&mut conn, identity).await?;

    // Signal handler.
    let mut signals = SignalWatcher::new().map_err(|e| {
        report!(LoopError::Other(format!(
            "failed to register signal handlers: {e}"
        )))
    })?;

    // Ping timer — not started until ServiceSettings arrives with ping_interval.
    let mut ping_timer: Option<tokio::time::Interval> = None;

    // Renewal timer — initially far-future, reset when ServiceSettings arrives.
    let mut renewal_sleep = create_renewal_sleep();
    let mut cert_handler = CertificateRenewalHandler::new();

    let mut shutdown_timeout_seconds: u32 = DEFAULT_SHUTDOWN_TIMEOUT;

    let outcome = loop {
        tokio::select! {
            biased;

            // 1. Service-specific events (highest priority).
            event = handler.poll_service_event() => {
                match handler.on_service_event(event, &mut conn).await? {
                    Some(outcome) => break outcome,
                    None => continue,
                }
            }

            // 2. Ping keepalive (only active after ServiceSettings arrives).
            _ = async { ping_timer.as_mut().expect("ping timer should be set").tick().await }, if ping_timer.is_some() => {
                let service_ts = now_millis();
                tracing::trace!(service_ts, "sending ping");
                conn.send(ServiceMessage::Ping(PingPayload { service_ts }))
                    .await
                    .context_to::<LoopError>()?;
            }

            // 3. Certificate renewal timer.
            _ = &mut renewal_sleep => {
                if let Some(o) = cert_handler.handle_renewal_timer(
                    identity, &mut conn, &mut renewal_sleep,
                ).await {
                    break o;
                }
            }

            // 4. Controller messages.
            msg = conn.recv() => {
                match msg.context_to::<LoopError>()? {
                    Some(ControllerMessage::Pong(pong)) => {
                        let rtt = now_millis() - pong.service_ts;
                        tracing::trace!(
                            service_ts = pong.service_ts,
                            controller_ts = pong.controller_ts,
                            rtt_ms = rtt,
                            "received pong"
                        );
                    }
                    Some(ControllerMessage::Certificate(payload)) => {
                        break cert_handler
                            .handle_certificate(identity, &payload)
                            .await
                            .context_to::<LoopError>()?;
                    }
                    Some(ControllerMessage::ServiceSettings(settings)) => {
                        // Compute agreed capabilities: intersection of controller's
                        // advertised set with this service's own capabilities,
                        // keeping only typed (known) variants.
                        let agreed: BTreeSet<Capability> = settings
                            .capabilities
                            .intersection(&handler.capabilities())
                            .filter(|c| c.is_known())
                            .cloned()
                            .collect();
                        conn.set_agreed_capabilities(agreed.clone());
                        tracing::debug!(capabilities = ?agreed, "negotiated protocol capabilities");

                        let mut loop_state = LoopState {
                            shutdown_timeout_seconds: &mut shutdown_timeout_seconds,
                            renewal_sleep: &mut renewal_sleep,
                            ping_timer: &mut ping_timer,
                            cert_not_after_ts,
                            config_dir: &config_dir,
                        };
                        handle_service_settings(
                            &settings,
                            &mut loop_state,
                            identity,
                            ctx,
                        ).await;
                        handler.on_settings(&settings).await;
                    }
                    Some(ControllerMessage::CaBundleUpdated(payload)) => {
                        cert_handler.handle_ca_bundle_updated(identity, &payload).await;
                    }
                    Some(ControllerMessage::RequestCertRenewal(payload)) => {
                        if let Some(o) = cert_handler
                            .handle_request_cert_renewal(identity, &mut conn, &payload)
                            .await
                        {
                            break o;
                        }
                    }
                    Some(ControllerMessage::ServerRestarting(payload)) => {
                        tracing::info!(
                            reason = %payload.reason,
                            "controller is restarting, initiating graceful shutdown"
                        );
                        break handler
                            .on_shutdown(
                                &mut conn,
                                ShutdownCause::ServerRestarting,
                                shutdown_timeout_seconds,
                            )
                            .await;
                    }
                    Some(msg) => {
                        match handler.on_message(msg, &mut conn).await? {
                            Some(outcome) => break outcome,
                            None => continue,
                        }
                    }
                    None => {
                        break dispatch_close_reason(conn.close_reason());
                    }
                }
            }

            // 5. OS signals.
            signal = signals.recv() => {
                tracing::info!(%signal, "received signal, initiating graceful shutdown");
                break handler
                    .on_shutdown(
                        &mut conn,
                        ShutdownCause::Signal(signal),
                        shutdown_timeout_seconds,
                    )
                    .await;
            }
        }
    };

    // Best-effort close — the peer may have already disconnected.
    let _ = conn.close().await;

    Ok(outcome)
}

/// Mutable state shared across the event loop that `handle_service_settings`
/// needs to update.
struct LoopState<'a> {
    shutdown_timeout_seconds: &'a mut u32,
    renewal_sleep: &'a mut Pin<Box<tokio::time::Sleep>>,
    ping_timer: &'a mut Option<tokio::time::Interval>,
    cert_not_after_ts: Option<i64>,
    config_dir: &'a Path,
}

/// Process shared `ServiceSettings` fields: shutdown timeout, renewal schedule,
/// ping interval, and CA staleness.
async fn handle_service_settings(
    settings: &ServiceSettingsPayload,
    state: &mut LoopState<'_>,
    identity: &mut ServiceIdentityState,
    ctx: &EventLoopContext<'_>,
) {
    tracing::trace!(
        renewal_window_hours = settings.renewal_window_hours,
        shutdown_timeout = ?settings.shutdown_timeout_seconds,
        "received service settings"
    );

    *state.shutdown_timeout_seconds = settings.shutdown_timeout_seconds.unwrap_or(120);
    update_renewal_schedule(
        state.renewal_sleep,
        state.cert_not_after_ts,
        settings.renewal_window_hours,
    );

    // Create or update the ping timer with the controller-provided interval.
    // Use interval_at so the first tick fires after one full period, not immediately.
    let interval_duration = settings.ping_interval;
    tracing::debug!(ping_interval_secs = ?interval_duration.as_secs(), "setting ping interval from controller");
    let new_timer = tokio::time::interval_at(
        tokio::time::Instant::now() + interval_duration,
        interval_duration,
    );
    *state.ping_timer = Some(new_timer);

    crate::ca::check_ca_staleness(
        &settings.ca_bundle_hash,
        state.config_dir,
        identity,
        ctx.pki_addr,
        ctx.base_url,
        ctx.ca_pem,
    )
    .await;
}

/// Map a WebSocket close reason to a [`LoopOutcome`].
fn dispatch_close_reason(reason: Option<&CloseReason>) -> LoopOutcome {
    match reason {
        Some(CloseReason::CertificateRotated) => {
            tracing::info!("connection closed: certificate rotated");
            LoopOutcome::Reconnect
        }
        Some(CloseReason::CertificateRevoked) => {
            tracing::warn!("connection closed: certificate revoked");
            LoopOutcome::Disconnected
        }
        Some(reason) => {
            tracing::warn!(%reason, "connection closed by controller");
            LoopOutcome::Disconnected
        }
        None => {
            tracing::info!("connection closed by controller");
            LoopOutcome::Disconnected
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_close_reason_cert_rotated() {
        let reason = CloseReason::CertificateRotated;
        assert_eq!(dispatch_close_reason(Some(&reason)), LoopOutcome::Reconnect);
    }

    #[test]
    fn dispatch_close_reason_cert_revoked() {
        let reason = CloseReason::CertificateRevoked;
        assert_eq!(
            dispatch_close_reason(Some(&reason)),
            LoopOutcome::Disconnected
        );
    }

    #[test]
    fn dispatch_close_reason_unknown() {
        let reason = CloseReason::Unknown("test".to_string());
        assert_eq!(
            dispatch_close_reason(Some(&reason)),
            LoopOutcome::Disconnected
        );
    }

    #[test]
    fn dispatch_close_reason_none() {
        assert_eq!(dispatch_close_reason(None), LoopOutcome::Disconnected);
    }
}
