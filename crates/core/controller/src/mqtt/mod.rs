//! Embedded MQTT service for controller deployments.

use std::collections::BTreeSet;
use std::sync::Arc;

use base64::Engine as _;
use uptrakit_internal_wire::{Capability, ControllerMessage, DisconnectReason, ServiceTransport};
use uptrakit_mqtt_runtime::{
    MqttRuntime, MqttRuntimeIdentity, MqttRuntimeLoopOutcome, MqttRuntimeSettings,
    mqtt_capabilities as runtime_capabilities,
};

use crate::embedded::EmbeddedShutdownTokens;
use crate::embedded::types::EmbeddedTransport;

pub(crate) fn mqtt_capabilities() -> BTreeSet<Capability> {
    runtime_capabilities()
}

pub(crate) async fn run_embedded_mqtt(
    mut transport: EmbeddedTransport,
    tokens: EmbeddedShutdownTokens,
) {
    let mut runtime = MqttRuntime::new();

    let identity = match generate_ecies_keypair() {
        Ok(identity) => identity,
        Err(error) => {
            tracing::error!(error = %error, "embedded MQTT: failed to generate ECIES key pair");
            return;
        }
    };

    if let Err(error) = runtime.on_connected(&mut transport, identity).await {
        tracing::error!(error = %error, "embedded MQTT: failed to initialize runtime");
        return;
    }
    runtime
        .apply_settings(
            MqttRuntimeSettings {
                ui_extensions_enabled: true,
            },
            &mut transport,
        )
        .await;
    runtime.handle_yield_change(transport.is_yielded()).await;
    let yield_change = transport.yield_change_notifier();

    tracing::info!("embedded MQTT started");

    loop {
        tokio::select! {
            biased;

            () = tokens.drain.cancelled() => {
                tracing::info!("embedded MQTT: draining");
                runtime.shutdown(&mut transport, DisconnectReason::Shutdown).await;
                break;
            }

            () = tokens.abort.cancelled() => {
                tracing::info!("embedded MQTT: aborting");
                break;
            }

            () = yield_change.notified() => {
                runtime.handle_yield_change(transport.is_yielded()).await;
            }

            event = runtime.poll_event() => {
                if let Some(outcome) = runtime.handle_event(event, &mut transport).await {
                    tracing::warn!(?outcome, "embedded MQTT: runtime requested loop exit");
                    break;
                }
            }

            msg = transport.transport_recv() => {
                let Some(msg) = msg else {
                    tracing::info!("embedded MQTT: transport closed");
                    break;
                };

                if let Some(outcome) = handle_controller_message(&mut runtime, msg, &mut transport).await {
                    tracing::warn!(?outcome, "embedded MQTT: controller handling requested loop exit");
                    break;
                }
            }
        }
    }

    tracing::info!("embedded MQTT stopped");
}

async fn handle_controller_message(
    runtime: &mut MqttRuntime,
    msg: ControllerMessage,
    transport: &mut EmbeddedTransport,
) -> Option<MqttRuntimeLoopOutcome> {
    match runtime.handle_controller_message(msg, transport).await {
        Ok(outcome) => outcome,
        Err(error) => {
            tracing::error!(error = %error, "embedded MQTT: failed to handle controller message");
            Some(MqttRuntimeLoopOutcome::Disconnected)
        }
    }
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

    let entries: Vec<uptrakit_internal_wire::payloads::ServiceConfigEntry> = rows
        .into_iter()
        .map(|row| {
            uptrakit_internal_wire::payloads::ServiceConfigEntry::new(
                row.tenant_id,
                row.key,
                row.value,
            )
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
                uptrakit_internal_wire::payloads::ServiceConfigDeliveryPayload::new(entries),
            ),
        )
        .await;
}

fn generate_ecies_keypair() -> Result<MqttRuntimeIdentity, String> {
    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|error| format!("P-256 key generation failed: {error}"))?;
    let private_der = key_pair.serialize_der();
    let public_raw = key_pair.public_key_raw().to_vec();
    let public_b64 = base64::engine::general_purpose::STANDARD.encode(&public_raw);

    Ok(MqttRuntimeIdentity {
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
        assert!(caps.contains(&Capability::UiExtensions));
        assert!(caps.contains(&Capability::WorkloadClaims));
    }

    #[test]
    fn generate_ecies_keypair_produces_valid_pair() {
        let identity = generate_ecies_keypair().expect("keygen");
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
