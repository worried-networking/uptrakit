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
    ServiceSettingsPayload, now_millis,
};

use rootcause::prelude::*;

use crate::cert_handler::{
    CertificateRenewalHandler, create_renewal_sleep, update_renewal_schedule,
};
use crate::connection::ControllerConnection;
use crate::identity::ServiceIdentityState;
use crate::shared_types::EventLoopContext;
use crate::shared_types::{LoopError, LoopOutcome, LoopResult, ServiceHandler, ShutdownCause};
use crate::signal::SignalWatcher;

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

/// Send a keepalive ping to the controller.
///
/// Returns `Some(LoopOutcome::Disconnected)` when the send fails (indicating
/// the connection is lost), or `None` on success.
async fn send_keepalive_ping(conn: &mut ControllerConnection) -> Option<LoopOutcome> {
    let service_ts = now_millis();
    tracing::trace!(service_ts, "sending ping");
    if let Err(e) = conn
        .send(ServiceMessage::Ping(PingPayload::new(service_ts)))
        .await
    {
        tracing::warn!(error = %e, "ping send failed, treating as disconnection");
        return Some(LoopOutcome::Disconnected);
    }
    None
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
    tracing::info!("connecting to controller (authenticated)");
    let mut conn = ControllerConnection::connect(host, port, tls_connector, None)
        .await
        .context_to::<LoopError>()?;

    run_event_loop_connected(handler, &mut conn, identity, ctx, signals).await
}

/// Inner connected event loop shared by the lifecycle and tests.
pub(crate) async fn run_event_loop_connected<H: ServiceHandler>(
    handler: &mut H,
    conn: &mut ControllerConnection,
    identity: &mut ServiceIdentityState,
    ctx: &EventLoopContext<'_>,
    signals: &mut SignalWatcher,
) -> LoopResult<LoopOutcome> {
    let cert_not_after_ts = identity.cert_not_after_ms();
    // Clone config_dir to avoid borrow conflicts with `&mut identity`.
    let config_dir = identity.config_dir().to_path_buf();

    // Let the service handle post-connect initialization.
    handler.on_connected(conn, identity).await?;

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
                match handler.on_service_event(event, conn).await? {
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
                if let Some(outcome) = send_keepalive_ping(conn).await {
                    break outcome;
                }
            }

            // 3. Certificate renewal timer.
            _ = &mut renewal_sleep => {
                consecutive_service_events = 0;
                if let Some(o) = cert_handler.handle_renewal_timer(
                    identity, conn, &mut renewal_sleep,
                ).await {
                    break o;
                }
            }

            // 4. Controller messages.
            msg = conn.recv() => {
                consecutive_service_events = 0;
                let mut loop_state = LoopState {
                    shutdown_timeout: &mut shutdown_timeout,
                    renewal_sleep: &mut renewal_sleep,
                    ping_timer: &mut ping_timer,
                    cert_not_after_ts,
                    config_dir: &config_dir,
                };
                if let Some(outcome) = handle_controller_message(
                    msg,
                    handler,
                    conn,
                    &mut cert_handler,
                    &mut loop_state,
                    identity,
                    ctx,
                ).await? {
                    break outcome;
                }
            }

            // 5. OS signals.
            signal = signals.recv() => {
                tracing::info!(%signal, "received signal, initiating graceful shutdown");
                break handler
                    .on_shutdown(
                        conn,
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

/// `true` when a `recv()` error should be absorbed by Phase 1 as
/// `LoopOutcome::Disconnected` (triggering reconnection with backoff reset)
/// rather than falling through to Phase 2 `context_to::<LoopError>()` conversion.
///
/// The `!is_cert_expired()` guard ensures cert-expired errors are never
/// misclassified as transient — they must reach Phase 2 to produce
/// `LoopError::CertExpired`. This implements the Phase 1 guard from the
/// classification priority table (see `LoopError` doc-comment).
fn should_absorb_as_disconnected(ctx: &crate::error::EnrollmentError) -> bool {
    !ctx.is_cert_expired() && (ctx.is_transient_network() || ctx.is_receive_closed())
}

/// Classify a `recv()` error through the Phase 1 / Phase 2 pipeline.
///
/// - **Phase 1**: if `should_absorb_as_disconnected()` returns `true`, the
///   error is a transient disconnection → return `Ok(Some(Disconnected))`
///   (the lifecycle resets backoff and reconnects).
/// - **Phase 2**: otherwise, convert via `context_to::<LoopError>()` and
///   propagate as `Err(Report<LoopError>)` (the lifecycle applies backoff
///   based on the `LoopError` variant).
///
/// This function is the single production branch point for recv errors.
/// Both `handle_controller_message()` and unit tests invoke it directly.
fn handle_recv_error(
    error: Report<crate::error::EnrollmentError>,
) -> LoopResult<Option<LoopOutcome>> {
    if should_absorb_as_disconnected(error.current_context()) {
        tracing::warn!(error = %error, "connection lost, will reconnect");
        Ok(Some(LoopOutcome::Disconnected))
    } else {
        Err(error.context_to())
    }
}

/// Dispatch a single controller message received from `conn.recv()`.
///
/// Returns `Ok(Some(outcome))` when the event loop should break with that
/// outcome, or `Ok(None)` when the loop should continue to the next
/// iteration.
///
/// Receive-error handling is delegated to [`handle_recv_error()`], which is
/// the single production Phase 1/Phase 2 branch point:
///
/// - Phase 1 (`should_absorb_as_disconnected`) => `Ok(Some(Disconnected))`
/// - Phase 2 (`context_to::<LoopError>()`) => `Err(Report<LoopError>)`
///
/// The `match msg { ... }` body below handles only successful `recv()` paths
/// (`Option<ControllerMessage>`), not the error classification logic.
async fn handle_controller_message<H: ServiceHandler>(
    msg: crate::error::Result<Option<ControllerMessage>>,
    handler: &mut H,
    conn: &mut ControllerConnection,
    cert_handler: &mut CertificateRenewalHandler,
    loop_state: &mut LoopState<'_>,
    identity: &mut ServiceIdentityState,
    ctx: &EventLoopContext<'_>,
) -> LoopResult<Option<LoopOutcome>> {
    // Handle recv errors: Phase 1 absorption or Phase 2 conversion.
    let msg = match msg {
        Ok(msg) => msg,
        Err(e) => return handle_recv_error(e),
    };

    // ── SDK/handler dispatch boundary ────────────────────────────────────────
    //
    // This match implements the SDK-tier dispatch: selected variants are
    // consumed or callback-routed here; all remaining variants fall through to
    // `on_message`.
    //
    // Internally-consumed SDK variants (handled without a callback):
    //   • Pong          — RTT logging
    //   • Certificate   — certificate storage and renewal scheduling
    //   • CaBundleUpdated — CA bundle hot-reload
    //   • RequestCertRenewal — controller-requested cert renewal
    //   • Unknown       — forward-compatibility no-op
    //
    // Callback-routed SDK variants (dispatched to ServiceHandler methods):
    //   • ServiceSettings  → on_settings()
    //   • ServerRestarting → on_shutdown()
    //   • SurfaceActionRequest → on_surface_action_request()
    //   • SurfaceActionResponse → on_surface_action_response()
    //   • ServiceConfigAck → on_service_config_ack()
    //
    // All other variants are forwarded to handler.on_message() via the
    // catch-all `Some(msg)` arm at the bottom of this match.
    //
    // Authoritative classification lives in `classify_controller_message_variant()`
    // in `crates/shared/wire/src/tests.rs`.  When adding a new SDK-owned arm here,
    // also update that function and the `expected_sdk_owned` set in
    // `test_variant_catalog_classification`.
    match msg {
        Some(ControllerMessage::Pong(pong)) => {
            let rtt = now_millis() - pong.service_ts;
            tracing::trace!(
                service_ts = pong.service_ts,
                controller_ts = pong.controller_ts,
                rtt_ms = rtt,
                "received pong"
            );
            Ok(None)
        }
        Some(ControllerMessage::Certificate(payload)) => {
            let outcome = cert_handler
                .handle_certificate(identity, &payload)
                .await
                .context_to::<LoopError>()?;
            Ok(Some(outcome))
        }
        Some(ControllerMessage::ServiceSettings(settings)) => {
            process_service_settings(&settings, handler, conn, loop_state, identity, ctx).await;
            Ok(None)
        }
        Some(ControllerMessage::CaBundleUpdated(payload)) => {
            cert_handler
                .handle_ca_bundle_updated(identity, &payload)
                .await;
            Ok(None)
        }
        Some(ControllerMessage::RequestCertRenewal(payload)) => Ok(cert_handler
            .handle_request_cert_renewal(identity, conn, &payload)
            .await),
        Some(ControllerMessage::ServerRestarting(payload)) => {
            tracing::info!(
                reason = %payload.reason,
                "controller is restarting, initiating graceful shutdown"
            );
            let outcome = handler
                .on_shutdown(
                    conn,
                    ShutdownCause::ServerRestarting,
                    *loop_state.shutdown_timeout,
                )
                .await;
            Ok(Some(outcome))
        }
        Some(ControllerMessage::SurfaceActionRequest(payload)) => {
            handler.on_surface_action_request(payload, conn).await?;
            Ok(None)
        }
        Some(ControllerMessage::SurfaceActionResponse(payload)) => {
            handler.on_surface_action_response(payload);
            Ok(None)
        }
        Some(ControllerMessage::ServiceConfigAck(ack)) => {
            handler.on_service_config_ack(ack);
            Ok(None)
        }
        Some(ControllerMessage::Unknown) => {
            tracing::warn!(
                "received unknown controller message type; \
                 ignoring for forward compatibility"
            );
            Ok(None)
        }
        Some(msg) => handler.on_message(msg, conn).await,
        None => Ok(Some(dispatch_close_reason(conn.close_reason()))),
    }
}

/// Negotiate capabilities with the controller, apply shared settings
/// (shutdown timeout, renewal schedule, ping interval, CA staleness),
/// and announce this service's capability set.
async fn process_service_settings<H: ServiceHandler>(
    settings: &ServiceSettingsPayload,
    handler: &mut H,
    conn: &mut ControllerConnection,
    loop_state: &mut LoopState<'_>,
    identity: &mut ServiceIdentityState,
    ctx: &EventLoopContext<'_>,
) {
    // Compute agreed capabilities: intersection of controller's advertised
    // set with this service's own capabilities, keeping only typed (known)
    // variants.
    let agreed: BTreeSet<Capability> = settings
        .capabilities
        .intersection(&handler.capabilities())
        .filter(|c| c.is_known())
        .cloned()
        .collect();
    conn.set_agreed_capabilities(agreed.clone());
    conn.set_report_page_limits(settings.report_page_limits.clone());
    tracing::debug!(capabilities = ?agreed, "negotiated protocol capabilities");

    handle_service_settings(settings, loop_state, identity, ctx).await;

    handler.on_settings(settings, conn).await;
}

/// Map a WebSocket close reason to a [`LoopOutcome`].
///
/// | `CloseReason` | `LoopOutcome` |
/// | --- | --- |
/// | `CertificateRotated` | `Reconnect` |
/// | `CertificateRevoked` | `Disconnected` |
/// | any other close reason | `Disconnected` |
/// | `None` | `Disconnected` |
///
/// All close reasons flow through the `Ok` path in the lifecycle (backoff is
/// reset). For `CertificateRevoked`, two runtime paths exist:
///
/// - TLS handshake may fail first, producing `LoopError::Other` via
///   [`handle_recv_error()`].
/// - If the server accepts then sends a `CertificateRevoked` close frame,
///   this function maps it to `Disconnected`.
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
    use crate::error::{EnrollmentError, ProtocolError, TlsError};
    use async_trait::async_trait;
    use futures_util::Stream;
    use futures_util::stream;
    use serde_json::Map;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};
    use tokio::sync::mpsc;

    type TestReadItem = std::result::Result<
        tokio_tungstenite::tungstenite::Message,
        tokio_tungstenite::tungstenite::Error,
    >;

    #[derive(Clone, Copy)]
    enum ReadPhase {
        AwaitFirstExhaustion,
        WaitForSecondBurst,
        Disconnected,
    }

    struct BudgetReadStream {
        rx: mpsc::UnboundedReceiver<TestReadItem>,
        hold_tx: Option<mpsc::UnboundedSender<TestReadItem>>,
        processed_count: Arc<AtomicU32>,
        service_tx: mpsc::Sender<()>,
        second_burst: u32,
        target_count: u32,
        phase: ReadPhase,
    }

    impl Stream for BudgetReadStream {
        type Item = TestReadItem;

        fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            let this = self.as_mut().get_mut();
            match this.phase {
                ReadPhase::AwaitFirstExhaustion => {
                    let first_poll_count = this.processed_count.load(Ordering::SeqCst);
                    assert_eq!(
                        first_poll_count, MAX_CONSECUTIVE_SERVICE_EVENTS,
                        "conn.recv() must first be polled exactly when the service-event budget is exhausted"
                    );
                    for _ in 0..this.second_burst {
                        this.service_tx
                            .try_send(())
                            .expect("second burst enqueue must fit in event channel");
                    }
                    this.phase = ReadPhase::WaitForSecondBurst;
                    let pending = this.rx.poll_recv(cx);
                    assert!(
                        pending.is_pending(),
                        "read stream must stay pending at first budget exhaustion to force yield_now"
                    );
                    Poll::Pending
                }
                ReadPhase::WaitForSecondBurst => {
                    if this.processed_count.load(Ordering::SeqCst) < this.target_count {
                        return Poll::Pending;
                    }
                    this.hold_tx.take();
                    this.phase = ReadPhase::Disconnected;
                    let closed = this.rx.poll_recv(cx);
                    assert!(
                        matches!(closed, Poll::Ready(None)),
                        "read stream must close on second poll to end loop as Disconnected"
                    );
                    closed
                }
                ReadPhase::Disconnected => Poll::Ready(None),
            }
        }
    }

    struct BudgetTestHandler {
        event_rx: mpsc::Receiver<()>,
        processed_count: Arc<AtomicU32>,
    }

    #[async_trait]
    impl ServiceHandler for BudgetTestHandler {
        const DIR_NAME: &'static str = "test";
        const SERVICE_LABEL: &'static str = "budget-test";
        const SERVICE_APP_NAME: &'static str = "budget-test";

        type ServiceEvent = ();

        async fn on_connected(
            &mut self,
            _conn: &mut ControllerConnection,
            _identity: &ServiceIdentityState,
        ) -> LoopResult<()> {
            Ok(())
        }

        async fn on_message(
            &mut self,
            _msg: ControllerMessage,
            _conn: &mut ControllerConnection,
        ) -> LoopResult<Option<LoopOutcome>> {
            Ok(None)
        }

        async fn poll_service_event(&mut self) -> Self::ServiceEvent {
            self.event_rx
                .recv()
                .await
                .expect("service event channel closed unexpectedly")
        }

        async fn on_service_event(
            &mut self,
            _event: Self::ServiceEvent,
            _conn: &mut ControllerConnection,
        ) -> LoopResult<Option<LoopOutcome>> {
            self.processed_count.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        }

        async fn on_shutdown(
            &mut self,
            _conn: &mut ControllerConnection,
            _cause: ShutdownCause,
            _shutdown_timeout: Duration,
        ) -> LoopOutcome {
            LoopOutcome::Shutdown
        }
    }

    /// Proves that first budget exhaustion triggers the yield/reset path and
    /// re-enables service-event polling for a second burst before disconnect.
    #[tokio::test(start_paused = true)]
    async fn budget_disables_service_arm_on_first_exhaustion() {
        let first_burst = MAX_CONSECUTIVE_SERVICE_EVENTS + 1;
        let second_burst = MAX_CONSECUTIVE_SERVICE_EVENTS
            .checked_sub(1)
            .expect("MAX_CONSECUTIVE_SERVICE_EVENTS must be > 0");
        let total_events = first_burst + second_burst;
        let (event_tx, event_rx) = mpsc::channel::<()>(total_events as usize);
        let (read_hold_tx, read_rx) = mpsc::unbounded_channel::<TestReadItem>();
        let processed_count = Arc::new(AtomicU32::new(0));

        let mut handler = BudgetTestHandler {
            event_rx,
            processed_count: Arc::clone(&processed_count),
        };

        let read_stream: Pin<Box<dyn Stream<Item = TestReadItem> + Send>> =
            Box::pin(BudgetReadStream {
                rx: read_rx,
                hold_tx: Some(read_hold_tx),
                processed_count: Arc::clone(&processed_count),
                service_tx: event_tx.clone(),
                second_burst,
                target_count: total_events,
                phase: ReadPhase::AwaitFirstExhaustion,
            });
        let mut conn = ControllerConnection::new_test(read_stream);
        let tmp = tempfile::tempdir().expect("tempdir must be created");
        let mut identity = ServiceIdentityState::new_single_dir(tmp.path());
        let ctx = EventLoopContext {
            base_url: "https://test.local",
            pki_addr: None,
            ca_pem: None,
        };
        let mut signals = SignalWatcher::new().expect("signal watcher must initialize");

        for _ in 0..first_burst {
            event_tx
                .send(())
                .await
                .expect("event channel must stay open during prefill");
        }
        drop(event_tx);

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            run_event_loop_connected(&mut handler, &mut conn, &mut identity, &ctx, &mut signals),
        )
        .await;

        let loop_result = result
            .expect("test timed out while waiting for loop completion")
            .expect("event loop should complete without loop error");
        assert_eq!(
            loop_result,
            LoopOutcome::Disconnected,
            "expected conn.recv() to end loop after budget reset allowed a second burst"
        );

        assert_eq!(
            processed_count.load(Ordering::SeqCst),
            total_events,
            "expected exactly two MAX_CONSECUTIVE_SERVICE_EVENTS bursts before disconnect"
        );
    }

    #[derive(Default, Clone)]
    struct SurfaceDispatchState {
        surface_request_count: usize,
        surface_response_count: usize,
        extension_request_on_message_count: usize,
        extension_response_on_message_count: usize,
        last_surface_request_tenant_id: Option<String>,
    }

    #[derive(Default)]
    struct SurfaceDispatchHandler {
        state: Arc<Mutex<SurfaceDispatchState>>,
    }

    #[async_trait]
    impl ServiceHandler for SurfaceDispatchHandler {
        const DIR_NAME: &'static str = "test";
        const SERVICE_LABEL: &'static str = "surface-dispatch-test";
        const SERVICE_APP_NAME: &'static str = "surface-dispatch-test";

        type ServiceEvent = std::convert::Infallible;

        async fn on_connected(
            &mut self,
            _conn: &mut ControllerConnection,
            _identity: &ServiceIdentityState,
        ) -> LoopResult<()> {
            Ok(())
        }

        async fn on_message(
            &mut self,
            msg: ControllerMessage,
            _conn: &mut ControllerConnection,
        ) -> LoopResult<Option<LoopOutcome>> {
            let mut state = self.state.lock().expect("lock");
            match msg {
                ControllerMessage::ExtensionRequest(_) => {
                    state.extension_request_on_message_count += 1;
                }
                ControllerMessage::ExtensionResponse(_) => {
                    state.extension_response_on_message_count += 1;
                }
                _ => {}
            }
            Ok(None)
        }

        async fn poll_service_event(&mut self) -> Self::ServiceEvent {
            std::future::pending().await
        }

        async fn on_service_event(
            &mut self,
            _event: Self::ServiceEvent,
            _conn: &mut ControllerConnection,
        ) -> LoopResult<Option<LoopOutcome>> {
            Ok(None)
        }

        async fn on_surface_action_request(
            &mut self,
            request: uptrakit_internal_wire::surfaces::SurfaceActionRequest,
            _conn: &mut ControllerConnection,
        ) -> LoopResult<()> {
            let mut state = self.state.lock().expect("lock");
            state.surface_request_count += 1;
            state.last_surface_request_tenant_id = Some(request.tenant_id);
            Ok(())
        }

        fn on_surface_action_response(
            &mut self,
            _response: uptrakit_internal_wire::surfaces::SurfaceActionResponse,
        ) {
            let mut state = self.state.lock().expect("lock");
            state.surface_response_count += 1;
        }

        async fn on_shutdown(
            &mut self,
            _conn: &mut ControllerConnection,
            _cause: ShutdownCause,
            _shutdown_timeout: Duration,
        ) -> LoopOutcome {
            LoopOutcome::Shutdown
        }
    }

    fn build_loop_state_for_tests<'a>(
        identity: &ServiceIdentityState,
        renewal_sleep: &'a mut Pin<Box<tokio::time::Sleep>>,
        shutdown_timeout: &'a mut Duration,
        ping_timer: &'a mut Option<tokio::time::Interval>,
        config_dir: &'a std::path::Path,
    ) -> LoopState<'a> {
        LoopState {
            shutdown_timeout,
            renewal_sleep,
            ping_timer,
            cert_not_after_ts: identity.cert_not_after_ms(),
            config_dir,
        }
    }

    #[tokio::test]
    async fn handle_controller_message_routes_surface_action_request_to_handler_callback() {
        let mut handler = SurfaceDispatchHandler::default();
        let mut conn = ControllerConnection::new_test(Box::pin(stream::pending()));
        let mut cert_handler = CertificateRenewalHandler::new();
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(tmp.path());
        let mut renewal_sleep = create_renewal_sleep();
        let mut shutdown_timeout = Duration::from_secs(30);
        let mut ping_timer: Option<tokio::time::Interval> = None;
        let mut loop_state = build_loop_state_for_tests(
            &identity,
            &mut renewal_sleep,
            &mut shutdown_timeout,
            &mut ping_timer,
            tmp.path(),
        );
        let ctx = EventLoopContext {
            base_url: "https://test.local",
            pki_addr: None,
            ca_pem: None,
        };

        let request = uptrakit_internal_wire::surfaces::SurfaceActionRequest {
            request_id: uuid::Uuid::now_v7(),
            tenant_id: "tenant-a".to_string(),
            surface_id: uptrakit_internal_wire::surfaces::SurfaceId::new("ssh-agent.hosts")
                .expect("surface id"),
            interaction_id: uptrakit_internal_wire::surfaces::InteractionId::new(
                "bootstrap-connect",
            )
            .expect("interaction id"),
            idempotency_key: uuid::Uuid::now_v7().to_string(),
            target_provider_id: None,
            caller_origin: uptrakit_internal_wire::surfaces::CallerOrigin::BuiltInSystem {
                principal: "test".to_string(),
            },
            params: Map::new(),
            encrypted_sensitive_params: None,
        };

        let outcome = handle_controller_message(
            Ok(Some(ControllerMessage::SurfaceActionRequest(request))),
            &mut handler,
            &mut conn,
            &mut cert_handler,
            &mut loop_state,
            &mut identity,
            &ctx,
        )
        .await
        .expect("message handled");
        assert!(outcome.is_none());

        let state = handler.state.lock().expect("lock").clone();
        assert_eq!(state.surface_request_count, 1);
        assert_eq!(
            state.last_surface_request_tenant_id.as_deref(),
            Some("tenant-a")
        );
    }

    #[tokio::test]
    async fn handle_controller_message_routes_extension_request_to_on_message_callback() {
        let mut handler = SurfaceDispatchHandler::default();
        let mut conn = ControllerConnection::new_test(Box::pin(stream::pending()));
        let mut cert_handler = CertificateRenewalHandler::new();
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(tmp.path());
        let mut renewal_sleep = create_renewal_sleep();
        let mut shutdown_timeout = Duration::from_secs(30);
        let mut ping_timer: Option<tokio::time::Interval> = None;
        let mut loop_state = build_loop_state_for_tests(
            &identity,
            &mut renewal_sleep,
            &mut shutdown_timeout,
            &mut ping_timer,
            tmp.path(),
        );
        let ctx = EventLoopContext {
            base_url: "https://test.local",
            pki_addr: None,
            ca_pem: None,
        };

        let request = uptrakit_internal_wire::extension::ExtensionRequestPayload {
            request_id: "req-1".to_string(),
            extension_id: "ext.test".to_string(),
            action_id: "act.test".to_string(),
            params: serde_json::Value::Null,
            sensitive_params: None,
            tenant_id: None,
        };

        let outcome = handle_controller_message(
            Ok(Some(ControllerMessage::ExtensionRequest(request))),
            &mut handler,
            &mut conn,
            &mut cert_handler,
            &mut loop_state,
            &mut identity,
            &ctx,
        )
        .await
        .expect("message handled");
        assert!(outcome.is_none());

        let state = handler.state.lock().expect("lock").clone();
        assert_eq!(state.extension_request_on_message_count, 1);
    }

    #[tokio::test]
    async fn handle_controller_message_routes_extension_response_to_on_message_callback() {
        let mut handler = SurfaceDispatchHandler::default();
        let mut conn = ControllerConnection::new_test(Box::pin(stream::pending()));
        let mut cert_handler = CertificateRenewalHandler::new();
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(tmp.path());
        let mut renewal_sleep = create_renewal_sleep();
        let mut shutdown_timeout = Duration::from_secs(30);
        let mut ping_timer: Option<tokio::time::Interval> = None;
        let mut loop_state = build_loop_state_for_tests(
            &identity,
            &mut renewal_sleep,
            &mut shutdown_timeout,
            &mut ping_timer,
            tmp.path(),
        );
        let ctx = EventLoopContext {
            base_url: "https://test.local",
            pki_addr: None,
            ca_pem: None,
        };

        let response = uptrakit_internal_wire::extension::ExtensionResponsePayload {
            request_id: "req-2".to_string(),
            success: true,
            data: serde_json::json!({"ok": true}),
            error: None,
        };

        let outcome = handle_controller_message(
            Ok(Some(ControllerMessage::ExtensionResponse(response))),
            &mut handler,
            &mut conn,
            &mut cert_handler,
            &mut loop_state,
            &mut identity,
            &ctx,
        )
        .await
        .expect("message handled");
        assert!(outcome.is_none());

        let state = handler.state.lock().expect("lock").clone();
        assert_eq!(state.extension_response_on_message_count, 1);
    }

    #[tokio::test]
    async fn handle_controller_message_routes_surface_action_response_to_handler_callback() {
        let mut handler = SurfaceDispatchHandler::default();
        let mut conn = ControllerConnection::new_test(Box::pin(stream::pending()));
        let mut cert_handler = CertificateRenewalHandler::new();
        let tmp = tempfile::tempdir().expect("tempdir");
        let mut identity = ServiceIdentityState::new_single_dir(tmp.path());
        let mut renewal_sleep = create_renewal_sleep();
        let mut shutdown_timeout = Duration::from_secs(30);
        let mut ping_timer: Option<tokio::time::Interval> = None;
        let mut loop_state = build_loop_state_for_tests(
            &identity,
            &mut renewal_sleep,
            &mut shutdown_timeout,
            &mut ping_timer,
            tmp.path(),
        );
        let ctx = EventLoopContext {
            base_url: "https://test.local",
            pki_addr: None,
            ca_pem: None,
        };

        let response = uptrakit_internal_wire::surfaces::SurfaceActionResponse {
            request_id: uuid::Uuid::now_v7(),
            success: true,
            result: Some(serde_json::json!({ "ok": true })),
            error: None,
        };

        let outcome = handle_controller_message(
            Ok(Some(ControllerMessage::SurfaceActionResponse(response))),
            &mut handler,
            &mut conn,
            &mut cert_handler,
            &mut loop_state,
            &mut identity,
            &ctx,
        )
        .await
        .expect("message handled");
        assert!(outcome.is_none());

        let state = handler.state.lock().expect("lock").clone();
        assert_eq!(state.surface_response_count, 1);
    }

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

    #[test]
    fn phase1_cert_expired_websocket_io_not_absorbed() {
        let err = EnrollmentError::WebSocket(tokio_tungstenite::tungstenite::Error::Io(
            std::io::Error::other(rustls::Error::AlertReceived(
                rustls::AlertDescription::CertificateExpired,
            )),
        ));
        assert!(!should_absorb_as_disconnected(&err));
    }

    #[test]
    fn phase1_cert_expired_tls_direct_not_absorbed() {
        let err = EnrollmentError::Tls(TlsError::Rustls(rustls::Error::AlertReceived(
            rustls::AlertDescription::CertificateExpired,
        )));
        assert!(!should_absorb_as_disconnected(&err));
    }

    #[test]
    fn phase1_transient_connection_reset_absorbed() {
        let err = EnrollmentError::WebSocket(tokio_tungstenite::tungstenite::Error::Io(
            std::io::Error::from(std::io::ErrorKind::ConnectionReset),
        ));
        assert!(should_absorb_as_disconnected(&err));
    }

    #[test]
    fn phase1_transient_connection_timeout_absorbed() {
        let err = EnrollmentError::Protocol(ProtocolError::ConnectionTimeout);
        assert!(should_absorb_as_disconnected(&err));
    }

    #[test]
    fn phase1_cert_revoked_websocket_io_not_absorbed() {
        let err = EnrollmentError::WebSocket(tokio_tungstenite::tungstenite::Error::Io(
            std::io::Error::other(rustls::Error::AlertReceived(
                rustls::AlertDescription::CertificateRevoked,
            )),
        ));
        assert!(!should_absorb_as_disconnected(&err));
    }

    #[test]
    fn phase1_receive_closed_absorbed() {
        let err = EnrollmentError::Protocol(ProtocolError::ReceiveClosed);
        assert!(should_absorb_as_disconnected(&err));
    }

    #[test]
    fn phase1_version_mismatch_not_absorbed() {
        let err = EnrollmentError::Protocol(ProtocolError::VersionMismatch {
            expected: 1,
            received: 2,
        });
        assert!(!should_absorb_as_disconnected(&err));
    }

    #[test]
    fn recv_error_transient_reset_produces_disconnected() {
        let err = EnrollmentError::WebSocket(tokio_tungstenite::tungstenite::Error::Io(
            std::io::Error::from(std::io::ErrorKind::ConnectionReset),
        ));
        let result = handle_recv_error(report!(err));
        assert!(matches!(result, Ok(Some(LoopOutcome::Disconnected))));
    }

    #[test]
    fn recv_error_cert_expired_produces_loop_error() {
        let err = EnrollmentError::WebSocket(tokio_tungstenite::tungstenite::Error::Io(
            std::io::Error::other(rustls::Error::AlertReceived(
                rustls::AlertDescription::CertificateExpired,
            )),
        ));
        let result = handle_recv_error(report!(err));
        assert!(result.is_err());
        let loop_err = result.expect_err("cert-expired must flow to phase 2");
        assert!(matches!(loop_err.current_context(), LoopError::CertExpired));
    }

    #[test]
    fn recv_error_fatal_version_mismatch_produces_other() {
        let err = EnrollmentError::Protocol(ProtocolError::VersionMismatch {
            expected: 1,
            received: 2,
        });
        let result = handle_recv_error(report!(err));
        assert!(result.is_err());
        let loop_err = result.expect_err("version mismatch must flow to phase 2");
        assert!(matches!(loop_err.current_context(), LoopError::Other(_)));
    }

    #[test]
    fn recv_error_cert_revoked_produces_other() {
        let err = EnrollmentError::WebSocket(tokio_tungstenite::tungstenite::Error::Io(
            std::io::Error::other(rustls::Error::AlertReceived(
                rustls::AlertDescription::CertificateRevoked,
            )),
        ));
        let result = handle_recv_error(report!(err));
        assert!(result.is_err());
        let loop_err = result.expect_err("cert-revoked must flow to phase 2");
        assert!(matches!(loop_err.current_context(), LoopError::Other(_)));
    }
}
