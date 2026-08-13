//! Embedded MQTT service for controller deployments.
//!
//! `mqtt_capabilities()` used to live here as a thin wrapper over
//! `uptrakit_mqtt_runtime::bootstrap::capabilities()`; callers now read
//! `bootstrap::capabilities()` directly (see
//! `crates/core/mqtt-runtime/src/bootstrap.rs`), leaving this module with
//! just the initial-config delivery helper below.
//!
//! `send_initial_service_config` is slated for removal in a follow-up task
//! once shared embedded config delivery lands in `web-api` — do not delete it
//! before that replacement exists, or embedded MQTT boots with no stored
//! client configs.

use std::sync::Arc;

use uptrakit_wire::ControllerMessage;

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
