//! Boot NATS wiring — concentrates every `#[cfg(feature = "nats")]` block from
//! the web-api construction tail.
//!
//! The sole public entry point is [`wire`], which:
//!
//! 1. Connects to the NATS server (async) using the reconciled URL.
//! 2. Augments the notification service, event broadcaster, and batch progress
//!    broadcaster with NATS support.
//! 3. Returns a [`NatsBits`] struct carrying the optional transport and the
//!    augmented objects so that `components::build` can remain `#[cfg]`-free.
//!
//! This entire module is `#[cfg(feature = "nats")]`-gated.

use std::sync::Arc;

use rootcause::prelude::*;

use crate::AppError;

/// Output of the NATS wiring step: the connected transport and the augmented
/// notification and broadcaster objects.
pub(crate) struct NatsBits {
    /// Connected NATS transport, or `None` when no NATS URL is configured.
    pub transport: Option<uptrakit_web_api::nats_transport::NatsTransport>,
    /// Notification service, possibly augmented with a NATS subscriber.
    pub notification_service: uptrakit_web_api::notification_service::NotificationService,
    /// Admin event broadcaster, possibly augmented with NATS fan-out.
    pub event_broadcaster: uptrakit_web_api::event_broadcaster::EventBroadcaster,
    /// Batch progress broadcaster, possibly augmented with NATS.
    pub batch_progress_broadcaster:
        uptrakit_web_api::batch_progress_broadcaster::BatchProgressBroadcaster,
}

/// Connect to NATS (if configured) and wire the augmented broadcaster/service
/// objects.
///
/// Accepts the reconciled NATS URL, the controller ID (for event fan-out), and
/// the three objects to potentially augment. Returns a [`NatsBits`] struct
/// holding `Option<NatsTransport>` plus the final (possibly-augmented) objects.
///
/// # Error mapping
///
/// NATS connection errors are mapped to [`AppError::Config`] using the same
/// `context_transform` logic that lived in `boot::run_server` before this
/// extraction.
pub(crate) async fn wire(
    nats_url: Option<&str>,
    controller_id: uuid::Uuid,
    mut notification_service: uptrakit_web_api::notification_service::NotificationService,
    mut event_broadcaster: uptrakit_web_api::event_broadcaster::EventBroadcaster,
    mut batch_progress_broadcaster: uptrakit_web_api::batch_progress_broadcaster::BatchProgressBroadcaster,
) -> crate::Result<NatsBits> {
    use uptrakit_web_api::nats_transport::NatsTransportError;

    let transport = if let Some(url) = nats_url {
        let nats = uptrakit_web_api::nats_transport::NatsTransport::connect(url, controller_id)
            .await
            .context_transform(|e| match e {
                NatsTransportError::Connection(msg) => {
                    AppError::Config(format!("NATS connection failed: {msg}"))
                }
                NatsTransportError::JetStream(msg) => AppError::Config(format!(
                    "NATS JetStream setup failed: {msg}\n\
                     Ensure JetStream is enabled on the NATS server: start with the \
                     -js flag, or add `jetstream: {{enabled: true}}` to nats-server.conf"
                )),
                _ => AppError::Config("NATS initialization failed".to_string()),
            })?;

        notification_service = notification_service.with_nats(Arc::new(nats.clone()));
        batch_progress_broadcaster = batch_progress_broadcaster.with_nats(nats.nats_client());
        event_broadcaster = event_broadcaster.with_nats(Arc::new(nats.clone()), controller_id);

        Some(nats)
    } else {
        None
    };

    Ok(NatsBits {
        transport,
        notification_service,
        event_broadcaster,
        batch_progress_broadcaster,
    })
}
