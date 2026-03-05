use std::time::Duration;

use async_nats::jetstream;
use async_nats::jetstream::stream::RetentionPolicy;
use rootcause::prelude::*;
use time::OffsetDateTime;
use uptrakit_backoff::Backoff;
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
    ///
    /// # Security note
    ///
    /// If the URL scheme is `nats://` (plaintext), a warning is emitted.
    /// In production environments use `nats-tls://` or configure the NATS
    /// server with `tls_required: true`. See
    /// [docs/security/secrets-and-encryption.md](../../../docs/security/secrets-and-encryption.md)
    /// for details.
    pub async fn connect(url: &str) -> Result<Self, Report<NatsError>> {
        // Warn when the operator configured a plaintext (non-TLS) NATS URL.
        // Strip any user-info (user:password@) before parsing the scheme so we
        // match both `nats://host:4222` and `nats://user:pass@host:4222`.
        let scheme = url.split("://").next().unwrap_or("");
        if scheme == "nats" {
            tracing::warn!(
                url,
                "connecting to NATS over plaintext (nats://); \
                 use nats-tls:// or enable TLS on the server side in production — \
                 see docs/security/secrets-and-encryption.md for guidance"
            );
        }

        tracing::info!(url, "connecting to NATS");
        // Retry up to 10 times with exponential backoff (1s base, 30s cap).
        // Transient NATS unavailability at startup should not cause permanent failure.
        const MAX_ATTEMPTS: u32 = 10;
        let client = 'connect: {
            let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(30));
            let mut last_err = None;
            for attempt in 1..=MAX_ATTEMPTS {
                match async_nats::connect(url).await {
                    Ok(c) => break 'connect c,
                    Err(e) => {
                        let delay = backoff.next_delay();
                        tracing::warn!(
                            url,
                            attempt,
                            max_attempts = MAX_ATTEMPTS,
                            delay_ms = delay.as_millis(),
                            error = %e,
                            "NATS connection attempt failed; retrying"
                        );
                        if attempt < MAX_ATTEMPTS {
                            tokio::time::sleep(delay).await;
                        }
                        last_err = Some(e);
                    }
                }
            }
            // All attempts exhausted — propagate the last error.
            return Err(last_err.expect("loop ran at least once")).context_to::<NatsError>();
        };

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
            .context_transform(|e| NatsError::JetStream(e.to_string()))?;

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
    ///
    /// # Panics (debug only)
    ///
    /// Asserts that `msg.is_nats_publishable()` in debug builds.  In both
    /// debug and release builds, credential-bearing messages are **dropped**
    /// and an error is logged instead of being sent to NATS.
    pub async fn publish(
        &self,
        source_controller_id: Uuid,
        target_service_id: Option<Uuid>,
        target_capability: Option<&str>,
        msg: ControllerMessage,
    ) {
        if !msg.is_nats_publishable() {
            tracing::error!(
                msg_type = ?std::mem::discriminant(&msg),
                "BUG: attempted to publish credential-bearing ControllerMessage to NATS; dropped"
            );
            debug_assert!(
                false,
                "credential-bearing message must not reach NatsConnection::publish"
            );
            return;
        }

        // Encrypt plugin config fields before NATS publication so credentials
        // are unreadable in JetStream storage and to unauthorized subscribers.
        let msg = crate::config_protection::encrypt_message_configs(msg);

        let envelope = NatsEventEnvelope {
            source_controller_id,
            target_service_id,
            target_capability: target_capability.map(ToString::to_string),
            trace_context: uptrakit_internal_wire::current_trace_context(),
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
