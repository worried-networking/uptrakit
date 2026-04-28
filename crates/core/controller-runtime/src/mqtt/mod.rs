//! Embedded MQTT service for controller deployments.

use std::collections::BTreeSet;
use std::sync::Arc;

use base64::Engine as _;
use uptrakit_mqtt_runtime::{
    MqttRuntime, MqttRuntimeIdentity, MqttRuntimeLoopOutcome, MqttRuntimeSettings,
    mqtt_capabilities as runtime_capabilities,
};
use uptrakit_wire::{Capability, ControllerMessage, DisconnectReason, ServiceTransport};

use crate::embedded::EmbeddedShutdownTokens;
use crate::embedded::types::EmbeddedTransport;

pub(crate) fn mqtt_capabilities() -> BTreeSet<Capability> {
    runtime_capabilities()
}

pub(crate) async fn run_embedded_mqtt(
    mut transport: EmbeddedTransport,
    tokens: EmbeddedShutdownTokens,
    default_tenant_id: uuid::Uuid,
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
                ui_surfaces_enabled: true,
                tenant_id: Some(default_tenant_id),
            },
            &mut transport,
        )
        .await;
    runtime
        .handle_yield_change(transport.is_yielded(), &mut transport)
        .await;
    let yield_change = transport.yield_change_notifier();

    tracing::info!("embedded MQTT started");

    loop {
        tokio::select! {
            biased;

            () = tokens.drain.cancelled() => {
                tracing::info!("embedded MQTT: draining");
                break;
            }

            () = tokens.abort.cancelled() => {
                tracing::info!("embedded MQTT: aborting");
                break;
            }

            () = yield_change.notified() => {
                runtime
                    .handle_yield_change(transport.is_yielded(), &mut transport)
                    .await;
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

    runtime
        .shutdown(&mut transport, DisconnectReason::Shutdown)
        .await;
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

fn generate_ecies_keypair() -> Result<MqttRuntimeIdentity, String> {
    let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
        .map_err(|error| format!("P-256 key generation failed: {error}"))?;
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
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;

    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;
    use uptrakit_wire::ServiceMessage;

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

    #[tokio::test]
    async fn transport_close_still_runs_runtime_shutdown() {
        let (service_tx, mut service_rx) = mpsc::channel(16);
        let (controller_tx, controller_rx) = mpsc::channel(1);
        let transport = EmbeddedTransport::new(
            service_tx,
            controller_rx,
            Arc::new(AtomicBool::new(false)),
            Arc::new(tokio::sync::Notify::new()),
        );
        let tokens = EmbeddedShutdownTokens {
            drain: CancellationToken::new(),
            abort: CancellationToken::new(),
        };

        drop(controller_tx);
        run_embedded_mqtt(transport, tokens, uuid::Uuid::now_v7()).await;

        let mut messages = Vec::new();
        while let Some(msg) = service_rx.recv().await {
            messages.push(msg);
        }

        assert!(messages.iter().any(|msg| matches!(
            msg,
            ServiceMessage::Disconnecting(payload)
                if payload.reason == DisconnectReason::Shutdown
        )));
    }

    #[tokio::test]
    async fn embedded_mqtt_registers_surface_with_default_tenant_binding() {
        let (service_tx, mut service_rx) = mpsc::channel(16);
        let (controller_tx, controller_rx) = mpsc::channel(1);
        let transport = EmbeddedTransport::new(
            service_tx,
            controller_rx,
            Arc::new(AtomicBool::new(false)),
            Arc::new(tokio::sync::Notify::new()),
        );
        let tokens = EmbeddedShutdownTokens {
            drain: CancellationToken::new(),
            abort: CancellationToken::new(),
        };
        let default_tenant_id = uuid::Uuid::now_v7();

        drop(controller_tx);
        run_embedded_mqtt(transport, tokens, default_tenant_id).await;

        let mut registrations = Vec::new();
        while let Some(msg) = service_rx.recv().await {
            if let ServiceMessage::SurfaceRegistration(registration) = msg {
                registrations.push(registration);
            }
        }

        assert_eq!(registrations.len(), 1);
        let registration = &registrations[0];
        let expected_tenant = default_tenant_id.to_string();
        assert_eq!(
            registration.effective_tenant_binding.tenant_id.as_deref(),
            Some(expected_tenant.as_str())
        );
    }
}
