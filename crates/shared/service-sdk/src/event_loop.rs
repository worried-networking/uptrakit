//! SDK-managed event loop for authenticated service connections.
//!
//! [`run_event_loop`] provides the unified `tokio::select!` loop that all
//! services share: ping/pong, certificate renewal, CA updates, signal
//! handling, and close-reason dispatch. Service-specific behaviour is
//! injected through the [`ServiceHandler`](crate::lifecycle::ServiceHandler)
//! trait callbacks.
//!
//! The service event arm (`poll_service_event`) uses a budget of
//! [`MAX_CONSECUTIVE_SERVICE_EVENTS`] to prevent starvation of the ping,
//! renewal, and controller-message arms during event bursts.

use std::path::Path;
use std::pin::Pin;
use std::time::Duration;

use std::collections::BTreeSet;

use uptrakit_internal_wire::{
    Capability, CloseReason, ControllerMessage, PingPayload, ServiceMessage,
    ServiceSettingsPayload, UpdateCapabilitiesPayload, now_millis,
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

/// Maximum number of consecutive service events to process before yielding
/// to other `select!` arms (ping, renewal, controller messages, signals).
///
/// Without this limit, a burst of service events (e.g. rapid MQTT client
/// status changes or SSH update output forwarding) can starve the ping timer,
/// causing the controller to consider the service dead and trigger unnecessary
/// failover.
///
/// When the budget is exhausted, a `yield_now()` arm fires (see below) so
/// that other Tokio tasks and timers advance by one scheduler cycle before
/// the budget resets. This prevents blocking on `conn.recv()` for an entire
/// ping interval (up to 300 s) while update output is pending.
const MAX_CONSECUTIVE_SERVICE_EVENTS: u32 = 16;

/// Default shutdown timeout when `ServiceSettings` does not provide one.
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(120);

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
///
/// `signals` is passed in from [`run_authenticated_with_reconnect`] so that
/// the same watcher instance is shared across reconnect iterations — signals
/// received during backoff delays are not lost and are handled on the next
/// loop iteration.
#[tracing::instrument(skip_all, name = "service.event_loop")]
pub(crate) async fn run_event_loop<H: ServiceHandler>(
    handler: &mut H,
    host: &str,
    port: u16,
    tls_connector: &tokio_rustls::TlsConnector,
    identity: &mut ServiceIdentityState,
    ctx: &EventLoopContext<'_>,
    signals: &mut SignalWatcher,
) -> LoopResult<LoopOutcome> {
    let cert_not_after_ts = identity.cert_not_after_ms();
    // Clone config_dir to avoid borrow conflicts with `&mut identity`.
    let config_dir = identity.config_dir().to_path_buf();

    tracing::info!("connecting to controller (authenticated)");
    let mut conn = ControllerConnection::connect(host, port, tls_connector, None)
        .await
        .context_to::<LoopError>()?;

    // Let the service handle post-connect initialization.
    handler.on_connected(&mut conn, identity).await?;

    // Ping timer — not started until ServiceSettings arrives with ping_interval.
    let mut ping_timer: Option<tokio::time::Interval> = None;

    // Renewal timer — initially far-future, reset when ServiceSettings arrives.
    let mut renewal_sleep = create_renewal_sleep();
    let mut cert_handler = CertificateRenewalHandler::new();

    let mut shutdown_timeout: Duration = DEFAULT_SHUTDOWN_TIMEOUT;

    // Tracks consecutive service events to prevent starvation of ping/renewal
    // arms when a service produces rapid bursts of events (e.g. MQTT bridge).
    let mut consecutive_service_events: u32 = 0;

    let outcome = loop {
        // When the service event budget is exhausted, skip the service event
        // arm for one iteration so that ping, renewal, and controller message
        // arms get a chance to fire. The budget resets when any other arm runs.
        let poll_service = consecutive_service_events < MAX_CONSECUTIVE_SERVICE_EVENTS;

        tokio::select! {
            biased;

            // 1. Service-specific events (highest priority, budget-limited).
            event = handler.poll_service_event(), if poll_service => {
                consecutive_service_events += 1;
                match handler.on_service_event(event, &mut conn).await? {
                    Some(outcome) => break outcome,
                    None => continue,
                }
            }

            // 2. Ping keepalive (only active after ServiceSettings arrives).
            // The `if ping_timer.is_some()` guard ensures we only enter this
            // branch when the timer is set; the inner `if let` avoids the
            // `.expect()` call while communicating the same invariant.
            // The trailing `;` discards the `Instant` return so both branches
            // of the `if let` return `()`, which tokio's select! requires.
            _ = async { if let Some(t) = ping_timer.as_mut() { t.tick().await; } }, if ping_timer.is_some() => {
                consecutive_service_events = 0;
                let service_ts = now_millis();
                tracing::trace!(service_ts, "sending ping");
                if let Err(e) = conn.send(ServiceMessage::Ping(PingPayload::new(service_ts))).await {
                    tracing::warn!(error = %e, "ping send failed, treating as disconnection");
                    break LoopOutcome::Disconnected;
                }
            }

            // 3. Certificate renewal timer.
            _ = &mut renewal_sleep => {
                consecutive_service_events = 0;
                if let Some(o) = cert_handler.handle_renewal_timer(
                    identity, &mut conn, &mut renewal_sleep,
                ).await {
                    break o;
                }
            }

            // 4. Controller messages.
            msg = conn.recv() => {
                consecutive_service_events = 0;
                // Check for transient network errors before converting to
                // LoopError. A broken pipe, connection reset, or similar
                // transport failure during recv() is a disconnection — not
                // a fatal protocol error.
                if let Err(ref e) = msg
                    && (e.current_context().is_transient_network()
                        || e.current_context().is_receive_closed())
                {
                    tracing::warn!(error = %e, "connection lost, will reconnect");
                    break LoopOutcome::Disconnected;
                }
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
                        conn.set_report_page_limits(settings.report_page_limits.clone());
                        tracing::debug!(capabilities = ?agreed, "negotiated protocol capabilities");

                        let mut loop_state = LoopState {
                            shutdown_timeout: &mut shutdown_timeout,
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
                        // Announce the service's full capability set so the
                        // controller can persist it and refresh gating flags
                        // for the current session. This handles services that
                        // gain or drop capabilities across version upgrades
                        // without requiring re-enrollment.
                        let caps_payload = UpdateCapabilitiesPayload {
                            capabilities: handler.capabilities(),
                        };
                        if let Err(e) = conn
                            .send(ServiceMessage::UpdateCapabilities(caps_payload))
                            .await
                        {
                            tracing::warn!(error = %e, "failed to send UpdateCapabilities");
                        }
                        handler.on_settings(&settings, &mut conn).await;
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
                                shutdown_timeout,
                            )
                            .await;
                    }
                    Some(ControllerMessage::ExtensionRequest(payload)) => {
                        handler.on_extension_request(payload, &mut conn).await?;
                    }
                    Some(ControllerMessage::ExtensionResponse(payload)) => {
                        handler.on_extension_response(payload);
                    }
                    Some(ControllerMessage::Unknown) => {
                        tracing::warn!(
                            "received unknown controller message type; \
                             ignoring for forward compatibility"
                        );
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
                        shutdown_timeout,
                    )
                    .await;
            }

            // 6. Budget-reset yield.
            //
            // This arm is active only when the service-event budget is
            // exhausted (`!poll_service`). It yields the task to the Tokio
            // scheduler for exactly one scheduling cycle — long enough for
            // timers (ping, renewal) and incoming messages to register as
            // ready — then resets the counter so service events can be
            // processed again on the next iteration.
            //
            // Without this arm the select would block on `conn.recv()` (arm 4)
            // until the next controller message arrives. For agents whose ping
            // interval is 300 s that means up to 5 minutes of silence while
            // update output is queued in the aggregate channel.
            _ = tokio::task::yield_now(), if !poll_service => {
                consecutive_service_events = 0;
            }
        }
    };

    // Best-effort close with timeout — the peer may have already disconnected
    // or may have stopped reading, which would cause close() to block
    // indefinitely waiting for the TCP send buffer to drain.
    const CLOSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
    match tokio::time::timeout(CLOSE_TIMEOUT, conn.close()).await {
        Ok(Err(e)) => {
            tracing::debug!(error = %e, "websocket close failed (best-effort)");
        }
        Err(_) => {
            tracing::warn!(
                "websocket close timed out after {}s, dropping connection",
                CLOSE_TIMEOUT.as_secs()
            );
        }
        Ok(Ok(())) => {}
    }

    Ok(outcome)
}

/// Mutable state shared across the event loop that `handle_service_settings`
/// needs to update.
struct LoopState<'a> {
    shutdown_timeout: &'a mut Duration,
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
        shutdown_timeout = ?settings.shutdown_timeout,
        "received service settings"
    );

    *state.shutdown_timeout = settings
        .shutdown_timeout
        .unwrap_or(DEFAULT_SHUTDOWN_TIMEOUT);
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
