//! NATS JetStream transport for cross-controller messaging.
//!
//! This module provides NATS-based pub/sub for delivering
//! [`ControllerMessage`](uptrakit_internal_wire::ControllerMessage)s across
//! multiple controller instances. Only compiled when the `nats` feature is
//! enabled.
//!
//! ## Subject scheme
//!
//! | Routing | Subject |
//! |---------|---------|
//! | Broadcast (no filter) | `uptrakit.events.broadcast` |
//! | Service-targeted | `uptrakit.events.service.<uuid>` |
//! | Capability-targeted | `uptrakit.events.capability.<cap>` |
//! | Controller events | `uptrakit.events.controller` |
//!
//! ## Stream configuration
//!
//! - Name: `UPTRAKIT_EVENTS`
//! - Subjects: `uptrakit.events.>`
//! - Max age: 24 hours
//! - Storage: File

use std::sync::Arc;
use std::time::Duration;

use async_nats::jetstream;
use async_nats::jetstream::consumer::PullConsumer;
use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use thiserror::Error;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use uptrakit_backoff::Backoff;
use uptrakit_internal_wire::ControllerMessage;
use uptrakit_nats::{NatsConnection, NatsEventEnvelope};
use uptrakit_shared_macros::impl_report_conversion;
use uuid::Uuid;

use crate::service_connections::ServiceConnectionRegistry;

/// Maximum delivery attempts before a message is dropped.
const MAX_DELIVER: i64 = 3;

/// Batch size for consumer pull requests.
const PULL_BATCH_SIZE: usize = 10;

/// Consumer pull request expiry.
const PULL_EXPIRES: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum NatsTransportError {
    #[error("NATS connection failed: {0}")]
    Connection(String),
    #[error("NATS JetStream setup failed: {0}")]
    JetStream(String),
    #[error("publish failed")]
    Publish,
    #[error("consumer error")]
    Consumer,
}

impl_report_conversion!(async_nats::ConnectError => NatsTransportError,
    |e| NatsTransportError::Connection(e.to_string())
);

/// NATS transport handle used by
/// [`NotificationService`](crate::notification_service::NotificationService) to
/// publish messages across controllers.
#[derive(Clone)]
pub struct NatsTransport {
    conn: NatsConnection,
    controller_id: Uuid,
}

impl NatsTransport {
    /// Connect to NATS, create the JetStream context, and ensure the stream
    /// exists.
    ///
    /// Fails hard on connection or stream setup errors — if NATS is configured,
    /// the user expects it to work.
    #[tracing::instrument(skip_all, fields(%controller_id, nats_url = %url))]
    pub async fn connect(
        url: &str,
        controller_id: Uuid,
    ) -> Result<Self, Report<NatsTransportError>> {
        let conn = NatsConnection::connect(url)
            .await
            .context_transform(|e| match e {
                uptrakit_nats::NatsError::Connection(msg) => NatsTransportError::Connection(msg),
                uptrakit_nats::NatsError::JetStream(msg) => NatsTransportError::JetStream(msg),
            })?;

        conn.ensure_stream()
            .await
            .context_transform(|e| NatsTransportError::JetStream(e.to_string()))?;

        Ok(Self {
            conn,
            controller_id,
        })
    }

    /// Publish a message to NATS JetStream.
    ///
    /// Fire-and-forget: errors are logged, not propagated. This matches the
    /// old DB outbox semantics where a write failure was not fatal to the
    /// caller.
    #[tracing::instrument(skip_all, fields(%source_controller_id, target_service_id = ?target_service_id, target_capability = ?target_capability))]
    pub async fn publish(
        &self,
        source_controller_id: Uuid,
        target_service_id: Option<Uuid>,
        target_capability: Option<&str>,
        msg: ControllerMessage,
    ) {
        self.conn
            .publish(
                source_controller_id,
                target_service_id,
                target_capability,
                msg,
            )
            .await;
    }

    /// Main consumer loop: pull messages from JetStream, filter self-originated
    /// messages, deliver via shared routing logic, and ack/nack.
    ///
    /// This loop runs until `cancel` is triggered.
    #[tracing::instrument(skip_all, fields(controller_id = %self.controller_id))]
    pub async fn run_consumer(
        self,
        registry: ServiceConnectionRegistry,
        db: DatabaseConnection,
        ca_rotation_trigger: Option<Arc<Notify>>,
        revocation_notify: Option<Arc<Notify>>,
        token_denylist: Option<Arc<crate::auth::token_denylist::TokenDenylist>>,
        cancel: CancellationToken,
    ) {
        let consumer = match self.create_consumer().await {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = ?e, "failed to create NATS consumer, exiting consumer loop");
                return;
            }
        };

        tracing::info!("NATS consumer started");

        let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(30));

        loop {
            // Race each pull request against the shutdown token so the consumer
            // exits immediately when cancelled rather than waiting up to PULL_EXPIRES
            // for an in-flight fetch to time out.
            let messages = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    tracing::info!("NATS consumer shutting down");
                    break;
                }
                result = consumer
                    .fetch()
                    .max_messages(PULL_BATCH_SIZE)
                    .expires(PULL_EXPIRES)
                    .messages() =>
                {
                    match result {
                        Ok(m) => {
                            backoff.reset();
                            m
                        }
                        Err(e) => {
                            let delay = backoff.next_delay();
                            tracing::warn!(
                                error = %e,
                                delay_ms = delay.as_millis(),
                                "NATS consumer fetch error, retrying with backoff"
                            );
                            // Also make the backoff sleep cancellable.
                            tokio::select! {
                                biased;
                                _ = cancel.cancelled() => {
                                    tracing::info!("NATS consumer shutting down during backoff");
                                    break;
                                }
                                _ = tokio::time::sleep(delay) => {}
                            }
                            continue;
                        }
                    }
                }
            };

            use futures_util::StreamExt;
            let mut messages = std::pin::pin!(messages);
            while let Some(result) = messages.next().await {
                if cancel.is_cancelled() {
                    break;
                }
                let msg = match result {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::warn!(error = %e, "NATS message receive error");
                        continue;
                    }
                };

                let envelope: NatsEventEnvelope = match serde_json::from_slice(&msg.payload) {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::warn!(error = %e, "failed to deserialize NATS envelope, acking to skip");
                        let _ = msg.ack().await;
                        continue;
                    }
                };

                // Skip self-originated messages.
                if envelope.source_controller_id == self.controller_id {
                    let _ = msg.ack().await;
                    continue;
                }

                // Decrypt plugin config fields that were encrypted before NATS publication.
                let message =
                    uptrakit_nats::config_protection::decrypt_message_configs(envelope.message);

                let resources = crate::event_delivery::ControllerResources {
                    ca_rotation_trigger: ca_rotation_trigger.as_ref(),
                    revocation_notify: revocation_notify.as_ref(),
                    token_denylist: token_denylist.as_ref(),
                };
                let delivered = crate::event_delivery::deliver_event(
                    &registry,
                    &db,
                    &resources,
                    envelope.target_service_id,
                    envelope.target_capability.as_deref(),
                    message,
                )
                .await;

                if delivered {
                    let _ = msg.ack().await;
                } else {
                    let _ = msg
                        .ack_with(async_nats::jetstream::AckKind::Nak(None))
                        .await;
                }
            }
        }
    }

    /// Access the underlying NATS client (for health checks).
    pub fn nats_client(&self) -> async_nats::Client {
        self.conn.client().clone()
    }

    /// Access the underlying `NatsConnection`.
    pub fn connection(&self) -> &NatsConnection {
        &self.conn
    }

    /// Create a durable pull consumer for this controller instance.
    #[tracing::instrument(skip_all, fields(controller_id = %self.controller_id))]
    async fn create_consumer(&self) -> Result<PullConsumer, Report<NatsTransportError>> {
        let consumer_name = format!("controller-{}", self.controller_id.simple());
        let stream = self
            .conn
            .js()
            .get_stream(uptrakit_nats::subjects::STREAM_NAME)
            .await
            .context_transform(|_| NatsTransportError::Consumer)?;

        stream
            .get_or_create_consumer(
                &consumer_name,
                jetstream::consumer::pull::Config {
                    durable_name: Some(consumer_name.clone()),
                    deliver_policy: jetstream::consumer::DeliverPolicy::New,
                    ack_policy: jetstream::consumer::AckPolicy::Explicit,
                    max_deliver: MAX_DELIVER,
                    filter_subject: format!("{}.>", uptrakit_nats::subjects::SUBJECT_PREFIX),
                    ..Default::default()
                },
            )
            .await
            .context_transform(|_| NatsTransportError::Consumer)
    }
}

#[cfg(test)]
mod tests {
    use uptrakit_nats::subjects::determine;
    use uuid::Uuid;

    #[test]
    fn determine_subject_broadcast() {
        assert_eq!(determine(None, None), "uptrakit.events.broadcast");
    }

    #[test]
    fn determine_subject_service() {
        let id = Uuid::nil();
        assert_eq!(
            determine(Some(id), None),
            format!("uptrakit.events.service.{id}")
        );
    }

    #[test]
    fn determine_subject_capability() {
        assert_eq!(
            determine(None, Some("mqtt_bridge")),
            "uptrakit.events.capability.mqtt_bridge"
        );
    }

    #[test]
    fn determine_subject_controller() {
        assert_eq!(
            determine(None, Some("controller")),
            "uptrakit.events.controller"
        );
    }

    #[test]
    fn determine_subject_service_takes_precedence_over_capability() {
        let id = Uuid::nil();
        assert_eq!(
            determine(Some(id), Some("mqtt_bridge")),
            format!("uptrakit.events.service.{id}")
        );
    }

    #[test]
    fn envelope_serialization_roundtrip() {
        use time::OffsetDateTime;
        use uptrakit_internal_wire::ControllerMessage;
        use uptrakit_nats::NatsEventEnvelope;

        let envelope = NatsEventEnvelope {
            source_controller_id: Uuid::nil(),
            target_service_id: Some(Uuid::nil()),
            target_capability: Some("mqtt_bridge".to_string()),
            trace_context: uptrakit_internal_wire::current_trace_context(),
            message: ControllerMessage::CaBundleUpdated(
                uptrakit_internal_wire::CaBundleUpdatedPayload {
                    ca_bundle_pem: "pem-data".to_string(),
                },
            ),
            created_at: OffsetDateTime::UNIX_EPOCH,
        };

        let json = serde_json::to_vec(&envelope).unwrap();
        let deserialized: NatsEventEnvelope = serde_json::from_slice(&json).unwrap();

        assert_eq!(
            deserialized.source_controller_id,
            envelope.source_controller_id
        );
        assert_eq!(deserialized.target_service_id, envelope.target_service_id);
        assert_eq!(deserialized.target_capability, envelope.target_capability);
        assert_eq!(deserialized.created_at, envelope.created_at);
    }

    /// Integration test: connect to NATS, publish, consume.
    ///
    /// Requires a running NATS server with JetStream enabled.
    /// Run: `cargo test -p uptrakit-web-api --features nats nats_connect_publish_consume -- --ignored`
    #[tokio::test]
    #[ignore = "requires running NATS server (nats-server -js)"]
    async fn nats_connect_publish_consume() {
        use std::time::Duration;

        let controller_a = Uuid::now_v7();
        let controller_b = Uuid::now_v7();

        let nats_url =
            std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());

        // Controller A publishes
        let transport_a = super::NatsTransport::connect(&nats_url, controller_a)
            .await
            .expect("failed to connect controller A");

        // Controller B consumes
        let transport_b = super::NatsTransport::connect(&nats_url, controller_b)
            .await
            .expect("failed to connect controller B");

        // Create consumer for B first so it doesn't miss the message
        let consumer_b = transport_b
            .create_consumer()
            .await
            .expect("failed to create consumer for B");

        // Small delay to let consumer be ready
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Publish from A
        let msg = uptrakit_internal_wire::ControllerMessage::CaBundleUpdated(
            uptrakit_internal_wire::CaBundleUpdatedPayload {
                ca_bundle_pem: "test-pem".to_string(),
            },
        );
        transport_a
            .publish(controller_a, None, None, msg.clone())
            .await;

        // Give JetStream a moment to persist
        tokio::time::sleep(Duration::from_millis(500)).await;

        // Consume on B
        let messages = consumer_b
            .fetch()
            .max_messages(10)
            .expires(Duration::from_secs(3))
            .messages()
            .await
            .expect("fetch failed");

        use futures_util::StreamExt;
        let mut messages = std::pin::pin!(messages);
        let mut found = false;

        while let Some(Ok(msg)) = messages.next().await {
            let envelope: uptrakit_nats::NatsEventEnvelope =
                serde_json::from_slice(&msg.payload).expect("deserialize failed");
            if envelope.source_controller_id == controller_a {
                found = true;
                let _ = msg.ack().await;
                break;
            }
            let _ = msg.ack().await;
        }

        assert!(found, "controller B should have received A's message");
    }
}
