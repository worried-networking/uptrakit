//! Embedded MQTT service for controller deployments.

use std::collections::BTreeSet;
use std::sync::Arc;

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
    let (private_der, public_b64) = uptrakit_service_sdk::generate_p256_keypair_for_ecies()
        .map_err(|e| {
            report!(std::io::Error::other(format!(
                "embedded MQTT: ECIES keygen failed: {e}"
            )))
        })?;
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
}
