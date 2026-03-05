use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uptrakit_internal_wire::{ControllerMessage, TraceContext};
use uuid::Uuid;

/// Wire envelope for NATS messages.
///
/// Contains the routing metadata alongside the actual [`ControllerMessage`].
#[derive(Serialize, Deserialize)]
pub struct NatsEventEnvelope {
    pub source_controller_id: Uuid,
    pub target_service_id: Option<Uuid>,
    pub target_capability: Option<String>,
    /// Distributed tracing context for correlating this event across controllers.
    #[serde(default)]
    pub trace_context: TraceContext,
    pub message: ControllerMessage,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
}

#[cfg(test)]
mod tests {
    use super::*;
    use uptrakit_internal_wire::{CaBundleUpdatedPayload, current_trace_context};

    #[test]
    fn envelope_serialization_roundtrip() {
        let envelope = NatsEventEnvelope {
            source_controller_id: Uuid::nil(),
            target_service_id: Some(Uuid::nil()),
            target_capability: Some("mqtt_bridge".to_string()),
            trace_context: current_trace_context(),
            message: ControllerMessage::CaBundleUpdated(CaBundleUpdatedPayload {
                ca_bundle_pem: "pem-data".to_string(),
            }),
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
}
