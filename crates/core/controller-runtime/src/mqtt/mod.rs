//! Embedded MQTT service for controller deployments.

use std::collections::BTreeSet;
use std::sync::Arc;

use base64::Engine as _;
use uptrakit_mqtt_runtime::{MqttRuntimeIdentity, mqtt_capabilities as runtime_capabilities};
use uptrakit_wire::{Capability, ControllerMessage};

pub(crate) fn mqtt_capabilities() -> BTreeSet<Capability> {
    runtime_capabilities()
}

pub(crate) async fn send_initial_service_config(
    app_state: &Arc<uptrakit_web_api::AppState>,
    service_id: uuid::Uuid,
) {
    let rows = match uptrakit_web_api::queries::service_config::load_for_service(
        app_state.db(),
        "uptrakit-mqtt",
    )
    .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "embedded MQTT: failed to load initial service config"
            );
            return;
        }
    };

    let entries: Vec<uptrakit_wire::payloads::ServiceConfigEntry> = rows
        .into_iter()
        .map(|row| {
            uptrakit_wire::payloads::ServiceConfigEntry::new(row.tenant_id, row.key, row.value)
        })
        .collect();

    if entries.is_empty() {
        return;
    }

    app_state
        .service_connections
        .send(
            &service_id,
            ControllerMessage::ServiceConfigDelivery(
                uptrakit_wire::payloads::ServiceConfigDeliveryPayload::new(entries),
            ),
        )
        .await;
}

pub(crate) fn generate_ecies_keypair() -> rootcause::Result<MqttRuntimeIdentity> {
    use rootcause::prelude::*;
    let key_pair =
        rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).map_err(|error| {
            report!(std::io::Error::other(format!(
                "P-256 key generation failed: {error}"
            )))
        })?;
    let private_der = key_pair.serialize_der();
    let public_raw = key_pair.public_key_raw().to_vec();
    let public_b64 = base64::engine::general_purpose::STANDARD.encode(&public_raw);

    Ok(MqttRuntimeIdentity {
        service_id: None,
        private_key_der: Some(private_der),
        encryption_public_key: Some(public_b64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mqtt_capabilities_includes_expected_set() {
        let caps = mqtt_capabilities();
        assert!(caps.contains(&Capability::SystemService));
        assert!(caps.contains(&Capability::UpdateTracking));
        assert!(caps.contains(&Capability::GracefulShutdown));
        assert!(caps.contains(&Capability::UiSurfaces));
        assert!(caps.contains(&Capability::WorkloadClaims));
    }

    #[test]
    fn generate_ecies_keypair_produces_valid_pair() {
        let identity = generate_ecies_keypair().unwrap();
        let private_key = identity.private_key_der.expect("private key");
        let public_key = identity.encryption_public_key.expect("public key");

        assert!(!private_key.is_empty());
        assert!(!public_key.is_empty());

        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&public_key)
            .expect("valid base64");
        assert_eq!(decoded.len(), 65);
        assert_eq!(decoded[0], 0x04);
    }
}
