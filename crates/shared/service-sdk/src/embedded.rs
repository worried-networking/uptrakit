//! Entry point for running a [`ServiceHandler`] in embedded mode.
//!
//! Embedded services receive messages over an in-process channel
//! (`EmbeddedTransport`) instead of a WebSocket. They skip enrollment,
//! certificate management, and OS signal handling. Shutdown is driven by
//! two `CancellationToken`s: `drain` (graceful) and `abort` (immediate).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

use tokio_util::sync::CancellationToken;

use crate::shared_types::{LoopOutcome, ServiceHandler, ShutdownCause};
use crate::wire_api::{Capability, ControllerMessage, ServiceSettingsPayload, ServiceTransport};

/// Startup timeout for the initial `ServiceSettings` message.
const EMBEDDED_STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

/// Default shutdown timeout, used until `ServiceSettings` provides one.
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(120);

/// Run a [`ServiceHandler`] in embedded mode.
///
/// Startup sequence:
/// 1. Wait up to 10 s for the controller to send `ServiceSettings` (first message).
/// 2. Compute agreed capabilities (intersection of handler's and controller's).
/// 3. Call `on_settings` — first handler callback (no `on_connected` for embedded).
/// 4. Call `on_yield_change` with the transport's current yield state.
/// 5. Enter the two-phase event loop.
///
/// Exits when: drain fires (graceful via `on_shutdown`), abort fires
/// (immediate), transport closes, or a handler callback requests exit.
pub async fn run_embedded_service<H: ServiceHandler>(
    mut handler: H,
    mut transport: impl ServiceTransport,
    drain: CancellationToken,
    abort: CancellationToken,
) {
    // ── Startup: wait for ServiceSettings ──────────────────────────────────
    let first_msg = tokio::select! {
        biased;
        () = abort.cancelled() => return,
        result = tokio::time::timeout(
            EMBEDDED_STARTUP_TIMEOUT,
            transport.transport_recv(),
        ) => {
            match result {
                Err(_elapsed) => {
                    tracing::error!(
                        service = H::SERVICE_LABEL,
                        "embedded service did not receive ServiceSettings within 10s; aborting"
                    );
                    return;
                }
                Ok(None) => {
                    tracing::error!(
                        service = H::SERVICE_LABEL,
                        "embedded transport closed before ServiceSettings arrived"
                    );
                    return;
                }
                Ok(Some(msg)) => msg,
            }
        }
    };

    let settings = match first_msg {
        ControllerMessage::ServiceSettings(s) => s,
        other => {
            tracing::warn!(
                service = H::SERVICE_LABEL,
                ?other,
                "expected ServiceSettings as first embedded message; aborting"
            );
            return;
        }
    };

    let mut shutdown_timeout = settings
        .shutdown_timeout
        .unwrap_or(DEFAULT_SHUTDOWN_TIMEOUT);

    let agreed = compute_agreed_capabilities(&handler, &settings);

    handler
        .on_settings(&settings, &mut transport, &agreed)
        .await;

    // Initial yield notification.
    {
        let is_yielded = transport.is_yielded();
        handler.on_yield_change(is_yielded, &mut transport).await;
    }

    // ── Obtain yield notifier handle ───────────────────────────────────────
    let yield_notifier: Option<Arc<tokio::sync::Notify>> = transport.yield_change_notifier();

    // ── Event loop ─────────────────────────────────────────────────────────
    loop {
        // Phase 1: resolve the next event.
        let maybe_event = tokio::select! {
            biased;
            () = abort.cancelled() => break,
            () = drain.cancelled() => {
                handler
                    .on_shutdown(&mut transport, ShutdownCause::EmbeddedDrain, shutdown_timeout)
                    .await;
                break;
            }
            () = async {
                if let Some(n) = &yield_notifier { n.notified().await }
                // `pending()` makes this arm never-ready when yield signalling is
                // unsupported. Do NOT replace with `unreachable!()` or `unwrap()`.
                else { std::future::pending().await }
            } => {
                let is_yielded = transport.is_yielded();
                handler.on_yield_change(is_yielded, &mut transport).await;
                None
            }
            event = handler.poll_service_event() => Some(event),
            msg = transport.transport_recv() => {
                match msg {
                    None => break,
                    Some(msg) => {
                        if !transport.is_yielded()
                            && let Some(outcome) =
                                dispatch_message(msg, &mut handler, &mut transport, &mut shutdown_timeout).await
                        {
                            let _ = outcome;
                            break;
                        }
                        None
                    }
                }
            }
        };

        // Phase 2: run on_service_event with drain/abort guards.
        if let Some(event) = maybe_event {
            let should_break = tokio::select! {
                biased;
                () = abort.cancelled() => true,
                () = drain.cancelled() => {
                    handler
                        .on_shutdown(&mut transport, ShutdownCause::EmbeddedDrain, shutdown_timeout)
                        .await;
                    true
                }
                outcome = handler.on_service_event(event, &mut transport) => {
                    match outcome {
                        Ok(Some(_)) => true,
                        Ok(None) => false,
                        Err(e) => {
                            tracing::error!(
                                service = H::SERVICE_LABEL,
                                error = %e,
                                "embedded service event handler error; exiting"
                            );
                            true
                        }
                    }
                }
            };
            if should_break {
                break;
            }
        }
    }
}

fn compute_agreed_capabilities<H: ServiceHandler>(
    handler: &H,
    settings: &ServiceSettingsPayload,
) -> BTreeSet<Capability> {
    settings
        .capabilities
        .intersection(&handler.capabilities())
        .filter(|c| c.is_known())
        .cloned()
        .collect()
}

async fn dispatch_message<H: ServiceHandler>(
    msg: ControllerMessage,
    handler: &mut H,
    transport: &mut dyn ServiceTransport,
    shutdown_timeout: &mut Duration,
) -> Option<LoopOutcome> {
    match msg {
        ControllerMessage::ServiceSettings(settings) => {
            *shutdown_timeout = settings
                .shutdown_timeout
                .unwrap_or(DEFAULT_SHUTDOWN_TIMEOUT);
            let agreed = compute_agreed_capabilities(handler, &settings);
            handler.on_settings(&settings, transport, &agreed).await;
            let is_yielded = transport.is_yielded();
            handler.on_yield_change(is_yielded, transport).await;
            None
        }
        ControllerMessage::SurfaceActionRequest(payload) => {
            if let Err(e) = handler.on_surface_action_request(payload, transport).await {
                tracing::error!(error = %e, "surface action request handler error");
            }
            None
        }
        ControllerMessage::SurfaceActionResponse(payload) => {
            handler.on_surface_action_response(payload);
            None
        }
        ControllerMessage::ServiceConfigAck(ack) => {
            handler.on_service_config_ack(ack);
            None
        }
        ControllerMessage::ServerRestarting(_payload) => {
            tracing::info!(
                service = H::SERVICE_LABEL,
                "controller restarting; embedded service shutting down"
            );
            let outcome = handler
                .on_shutdown(
                    transport,
                    ShutdownCause::ServerRestarting,
                    *shutdown_timeout,
                )
                .await;
            Some(outcome)
        }
        ControllerMessage::Unknown => {
            tracing::warn!(
                service = H::SERVICE_LABEL,
                "received unknown controller message type in embedded mode; ignoring"
            );
            None
        }
        ControllerMessage::Certificate(_)
        | ControllerMessage::CaBundleUpdated(_)
        | ControllerMessage::Pong(_)
        | ControllerMessage::RequestCertRenewal(_) => {
            tracing::debug!(
                service = H::SERVICE_LABEL,
                "ignoring cert/CA/pong message in embedded mode"
            );
            None
        }
        msg => match handler.on_message(msg, transport).await {
            Ok(Some(outcome)) => Some(outcome),
            Ok(None) => None,
            Err(e) => {
                tracing::error!(service = H::SERVICE_LABEL, error = %e, "on_message error");
                None
            }
        },
    }
}
