use async_nats::jetstream;
use async_nats::jetstream::stream::RetentionPolicy;
use rootcause::prelude::*;
use time::OffsetDateTime;
use uptrakit_internal_wire::ControllerMessage;
use uuid::Uuid;

use crate::envelope::NatsEventEnvelope;
use crate::error::NatsError;
use crate::subjects;

/// Shared NATS JetStream connection wrapper.
///
/// Used by both the controller's `NatsTransport` and the external scheduler's
/// `NatsSchedulerNotifier` to publish messages to the JetStream stream.
#[derive(Clone)]
pub struct NatsConnection {
    js: jetstream::Context,
    client: async_nats::Client,
}

impl NatsConnection {
    /// Connect to NATS and create the JetStream context.
    pub async fn connect(url: &str) -> Result<Self, Report<NatsError>> {
        tracing::info!(url, "connecting to NATS");
        let client = async_nats::connect(url).await.context_to::<NatsError>()?;

        let js = jetstream::new(client.clone());
        Ok(Self { js, client })
    }

    /// Ensure the JetStream stream exists (idempotent — safe for concurrent startup).
    pub async fn ensure_stream(&self) -> Result<(), Report<NatsError>> {
        self.js
            .get_or_create_stream(jetstream::stream::Config {
                name: subjects::STREAM_NAME.to_string(),
                subjects: vec![format!("{}.>", subjects::SUBJECT_PREFIX)],
                max_age: subjects::STREAM_MAX_AGE,
                retention: RetentionPolicy::Limits,
                ..Default::default()
            })
            .await
            .context_transform(|_| NatsError::JetStream)?;

        tracing::info!("NATS JetStream stream ready: {}", subjects::STREAM_NAME);
        Ok(())
    }

    /// Publish a `NatsEventEnvelope` to the appropriate NATS subject.
    ///
    /// Fire-and-forget: errors are logged, not propagated. This matches the
    /// semantics where a publish failure is not fatal to the caller.
    pub async fn publish_envelope(&self, envelope: &NatsEventEnvelope) {
        let subject = subjects::determine(
            envelope.target_service_id,
            envelope.target_capability.as_deref(),
        );

        let payload = match serde_json::to_vec(envelope) {
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

    /// Publish a message with routing metadata.
    pub async fn publish(
        &self,
        source_controller_id: Uuid,
        target_service_id: Option<Uuid>,
        target_capability: Option<&str>,
        msg: ControllerMessage,
    ) {
        let envelope = NatsEventEnvelope {
            source_controller_id,
            target_service_id,
            target_capability: target_capability.map(ToString::to_string),
            message: msg,
            created_at: OffsetDateTime::now_utc(),
        };
        self.publish_envelope(&envelope).await;
    }

    /// Access the underlying NATS client (for health checks or consumer creation).
    pub fn client(&self) -> &async_nats::Client {
        &self.client
    }

    /// Access the JetStream context (for consumer creation).
    pub fn js(&self) -> &jetstream::Context {
        &self.js
    }
}
