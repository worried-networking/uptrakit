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

use std::time::Duration;

use async_nats::jetstream;
use async_nats::jetstream::consumer::PullConsumer;
use async_nats::jetstream::stream::RetentionPolicy;
use rootcause::prelude::*;
use sea_orm::DatabaseConnection;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;
use tokio_util::sync::CancellationToken;
use uptrakit_internal_wire::ControllerMessage;
use uptrakit_shared_macros::impl_report_conversion;
use uuid::Uuid;

use crate::service_connections::ServiceConnectionRegistry;

/// Stream name in JetStream.
const STREAM_NAME: &str = "UPTRAKIT_EVENTS";

/// Subject prefix for all events.
const SUBJECT_PREFIX: &str = "uptrakit.events";

/// Maximum age for messages in the stream (24 hours).
const STREAM_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// Maximum delivery attempts before a message is dropped.
const MAX_DELIVER: i64 = 3;

/// Batch size for consumer pull requests.
const PULL_BATCH_SIZE: usize = 10;

/// Consumer pull request expiry.
const PULL_EXPIRES: Duration = Duration::from_secs(5);

#[derive(Debug, Error)]
pub enum NatsTransportError {
    #[error("NATS connection failed")]
    Connection,
    #[error("JetStream setup failed")]
    JetStream,
    #[error("publish failed")]
    Publish,
    #[error("consumer error")]
    Consumer,
}

impl_report_conversion!(async_nats::ConnectError => NatsTransportError,
    |_e| NatsTransportError::Connection
);

/// Wire envelope for NATS messages.
///
/// Contains the routing metadata alongside the actual [`ControllerMessage`].
#[derive(Serialize, Deserialize)]
pub(crate) struct NatsEventEnvelope {
    pub source_controller_id: Uuid,
    pub target_service_id: Option<Uuid>,
    pub target_capability: Option<String>,
    pub message: ControllerMessage,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

/// NATS transport handle used by
/// [`NotificationService`](crate::notification_service::NotificationService) to
/// publish messages across controllers.
#[derive(Clone)]
pub struct NatsTransport {
    js: jetstream::Context,
    controller_id: Uuid,
}

impl NatsTransport {
    /// Connect to NATS, create the JetStream context, and ensure the stream
    /// exists.
    ///
    /// Fails hard on connection or stream setup errors — if NATS is configured,
    /// the user expects it to work.
    pub async fn connect(
        url: &str,
        controller_id: Uuid,
    ) -> Result<Self, Report<NatsTransportError>> {
        tracing::info!(url, "connecting to NATS");
        let client = async_nats::connect(url)
            .await
            .context_to::<NatsTransportError>()?;

        let js = jetstream::new(client);

        // Create or update the stream (idempotent — safe for multi-controller
        // startup race).
        js.get_or_create_stream(jetstream::stream::Config {
            name: STREAM_NAME.to_string(),
            subjects: vec![format!("{SUBJECT_PREFIX}.>")],
            max_age: STREAM_MAX_AGE,
            retention: RetentionPolicy::Limits,
            ..Default::default()
        })
        .await
        .context_transform(|_| NatsTransportError::JetStream)?;

        tracing::info!("NATS JetStream stream ready: {STREAM_NAME}");

        Ok(Self { js, controller_id })
    }

    /// Publish a message to NATS JetStream.
    ///
    /// Fire-and-forget: errors are logged, not propagated. This matches the
    /// old DB outbox semantics where a write failure was not fatal to the
    /// caller.
    pub async fn publish(
        &self,
        source_controller_id: Uuid,
        target_service_id: Option<Uuid>,
        target_capability: Option<&str>,
        msg: ControllerMessage,
    ) {
        let subject = determine_subject(target_service_id, target_capability);
        let envelope = NatsEventEnvelope {
            source_controller_id,
            target_service_id,
            target_capability: target_capability.map(ToString::to_string),
            message: msg,
            created_at: OffsetDateTime::now_utc(),
        };

        let payload = match serde_json::to_vec(&envelope) {
            Ok(p) => p,
            Err(e) => {
                tracing::error!(error = %e, "failed to serialize NATS envelope");
                return;
            }
        };

        if let Err(e) = self.js.publish(subject.clone(), payload.into()).await {
            tracing::warn!(error = %e, %subject, "NATS publish failed");
        }
    }

    /// Main consumer loop: pull messages from JetStream, filter self-originated
    /// messages, deliver via shared routing logic, and ack/nack.
    ///
    /// This loop runs until `cancel` is triggered.
    pub async fn run_consumer(
        self,
        registry: ServiceConnectionRegistry,
        db: DatabaseConnection,
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

        loop {
            if cancel.is_cancelled() {
                tracing::info!("NATS consumer shutting down");
                break;
            }

            let messages = match consumer
                .fetch()
                .max_messages(PULL_BATCH_SIZE)
                .expires(PULL_EXPIRES)
                .messages()
                .await
            {
                Ok(m) => m,
                Err(e) => {
                    tracing::warn!(error = %e, "NATS consumer fetch error, retrying");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
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

                let delivered = crate::event_delivery::deliver_event(
                    &registry,
                    &db,
                    envelope.target_service_id,
                    envelope.target_capability.as_deref(),
                    envelope.message,
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
        self.js.client()
    }

    /// Create a durable pull consumer for this controller instance.
    async fn create_consumer(&self) -> Result<PullConsumer, Report<NatsTransportError>> {
        let consumer_name = format!("controller-{}", self.controller_id.simple());
        let stream = self
            .js
            .get_stream(STREAM_NAME)
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
                    filter_subject: format!("{SUBJECT_PREFIX}.>"),
                    ..Default::default()
                },
            )
            .await
            .context_transform(|_| NatsTransportError::Consumer)
    }
}

/// Determine the NATS subject for a message based on routing metadata.
fn determine_subject(target_service_id: Option<Uuid>, target_capability: Option<&str>) -> String {
    match (target_service_id, target_capability) {
        (Some(id), _) => format!("{SUBJECT_PREFIX}.service.{id}"),
        (None, Some(cap)) => {
            if cap == "controller" {
                format!("{SUBJECT_PREFIX}.controller")
            } else {
                format!("{SUBJECT_PREFIX}.capability.{cap}")
            }
        }
        (None, None) => format!("{SUBJECT_PREFIX}.broadcast"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn determine_subject_broadcast() {
        assert_eq!(determine_subject(None, None), "uptrakit.events.broadcast");
    }

    #[test]
    fn determine_subject_service() {
        let id = Uuid::nil();
        assert_eq!(
            determine_subject(Some(id), None),
            format!("uptrakit.events.service.{id}")
        );
    }

    #[test]
    fn determine_subject_capability() {
        assert_eq!(
            determine_subject(None, Some("mqtt_bridge")),
            "uptrakit.events.capability.mqtt_bridge"
        );
    }

    #[test]
    fn determine_subject_controller() {
        assert_eq!(
            determine_subject(None, Some("controller")),
            "uptrakit.events.controller"
        );
    }

    #[test]
    fn determine_subject_service_takes_precedence_over_capability() {
        let id = Uuid::nil();
        assert_eq!(
            determine_subject(Some(id), Some("mqtt_bridge")),
            format!("uptrakit.events.service.{id}")
        );
    }

    #[test]
    fn envelope_serialization_roundtrip() {
        let envelope = NatsEventEnvelope {
            source_controller_id: Uuid::nil(),
            target_service_id: Some(Uuid::nil()),
            target_capability: Some("mqtt_bridge".to_string()),
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
        let controller_a = Uuid::now_v7();
        let controller_b = Uuid::now_v7();

        let nats_url =
            std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".to_string());

        // Controller A publishes
        let transport_a = NatsTransport::connect(&nats_url, controller_a)
            .await
            .expect("failed to connect controller A");

        // Controller B consumes
        let transport_b = NatsTransport::connect(&nats_url, controller_b)
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
        let msg =
            ControllerMessage::CaBundleUpdated(uptrakit_internal_wire::CaBundleUpdatedPayload {
                ca_bundle_pem: "test-pem".to_string(),
            });
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
            let envelope: NatsEventEnvelope =
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
