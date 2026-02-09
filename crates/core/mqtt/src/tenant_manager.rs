use std::collections::HashMap;

use futures_util::stream::{FuturesUnordered, StreamExt};
use uptrakit_internal_wire::MqttTenantConfig;
use uuid::Uuid;

use crate::mqtt_client::{MqttClientStatusEvent, MqttConfig, MqttHandle};
use tokio::sync::mpsc;
use uptrakit_internal_wire::MqttClientConnectionStatus;

/// Tracks the cached state for an MQTT client.
struct ClientState {
    handle: MqttHandle,
    config_hash: u64,
}

/// Manages per-MQTT-client lifecycles with push-based config updates.
///
/// Unlike the database-polling version, this manager receives configuration
/// updates from the controller via WebSocket messages.
pub struct TenantManager {
    clients: HashMap<Uuid, ClientState>,
    status_tx: Option<mpsc::UnboundedSender<MqttClientStatusEvent>>,
}

impl TenantManager {
    pub fn new(status_tx: Option<mpsc::UnboundedSender<MqttClientStatusEvent>>) -> Self {
        Self {
            clients: HashMap::new(),
            status_tx,
        }
    }

    /// Apply MQTT client assignments from the controller.
    ///
    /// This is called when receiving `TenantAssignments` message.
    pub async fn apply_assignments(&mut self, configs: Vec<MqttTenantConfig>) {
        for config in configs {
            if config.enabled {
                self.start_or_update_client(config).await;
            } else {
                self.stop_client(&config.mqtt_client_id).await;
            }
        }
    }

    /// Reload a single MQTT client's configuration.
    ///
    /// This is called when receiving `TenantConfigUpdated` message.
    pub async fn reload_client(&mut self, config: MqttTenantConfig) {
        if config.enabled {
            self.start_or_update_client(config).await;
        } else {
            self.stop_client(&config.mqtt_client_id).await;
        }
    }

    /// Stop an MQTT client.
    ///
    /// This is called when receiving `TenantRevoked` message or when config is disabled.
    pub async fn stop_client(&mut self, mqtt_client_id: &Uuid) {
        if let Some(state) = self.clients.remove(mqtt_client_id) {
            tracing::info!(%mqtt_client_id, "shutting down MQTT client");
            self.report_status(*mqtt_client_id, MqttClientConnectionStatus::Offline);
            state.handle.shutdown().await;
        }
    }

    /// Return list of active MQTT client IDs (used in `Disconnecting` payload).
    pub fn active_mqtt_client_ids(&self) -> Vec<Uuid> {
        self.clients.keys().copied().collect()
    }

    /// Graceful shutdown: stop all MQTT clients.
    pub async fn shutdown_all(&mut self) {
        let clients = std::mem::take(&mut self.clients);
        let mut tasks = FuturesUnordered::new();

        for (mqtt_client_id, state) in clients {
            tasks.push(async move {
                tracing::info!(%mqtt_client_id, "shutting down MQTT client");
                state.handle.shutdown().await;
            });
            self.report_status(mqtt_client_id, MqttClientConnectionStatus::Offline);
        }

        while tasks.next().await.is_some() {}
    }

    /// Start or update an MQTT client.
    async fn start_or_update_client(&mut self, config: MqttTenantConfig) {
        let mqtt_client_id = config.mqtt_client_id;
        let new_hash = compute_config_hash(&config);

        // Check if we already have this client with same config
        if let Some(state) = self.clients.get(&mqtt_client_id) {
            if state.config_hash == new_hash {
                tracing::debug!(%mqtt_client_id, "config unchanged, skipping update");
                return;
            }
            tracing::info!(%mqtt_client_id, "config changed, reloading");
        }

        // Stop existing client if any
        if let Some(state) = self.clients.remove(&mqtt_client_id) {
            self.report_status(mqtt_client_id, MqttClientConnectionStatus::Offline);
            state.handle.shutdown().await;
        }

        // Build and start new client
        let mqtt_config = build_config_from_wire(&config);
        tracing::info!(%mqtt_client_id, config = ?mqtt_config, "starting MQTT client");

        match crate::mqtt_client::start(mqtt_config, self.status_tx.clone(), mqtt_client_id).await {
            Ok(handle) => {
                self.clients.insert(
                    mqtt_client_id,
                    ClientState {
                        handle,
                        config_hash: new_hash,
                    },
                );
            }
            Err(e) => {
                tracing::warn!(%mqtt_client_id, error = ?e, "MQTT client startup failed");
                self.report_status(mqtt_client_id, MqttClientConnectionStatus::Offline);
            }
        }
    }

    fn report_status(&self, mqtt_client_id: Uuid, status: MqttClientConnectionStatus) {
        let Some(sender) = self.status_tx.as_ref() else {
            return;
        };

        let _ = sender.send(MqttClientStatusEvent {
            mqtt_client_id,
            status,
        });
    }
}

impl Default for TenantManager {
    fn default() -> Self {
        Self::new(None)
    }
}

/// Convert a wire protocol `MqttTransport` to the `web-api-types` `MqttTransport`.
fn local_mqtt_transport(
    wire: uptrakit_internal_wire::MqttTransport,
) -> uptrakit_web_api_types::mqtt_transport::MqttTransport {
    match wire {
        uptrakit_internal_wire::MqttTransport::Tcp => {
            uptrakit_web_api_types::mqtt_transport::MqttTransport::Tcp
        }
        uptrakit_internal_wire::MqttTransport::Tls => {
            uptrakit_web_api_types::mqtt_transport::MqttTransport::Tls
        }
    }
}

/// Build MqttConfig from wire protocol config.
fn build_config_from_wire(config: &MqttTenantConfig) -> MqttConfig {
    let transport = local_mqtt_transport(config.transport);
    let port = if config.port == 0 {
        transport.default_port()
    } else {
        config.port
    };

    MqttConfig {
        transport,
        host: config.host.clone(),
        port,
        client_id: config.client_id.clone(),
        username: config.username.clone(),
        password: config.password.clone(),
        topic_prefix: config.topic_prefix.clone(),
    }
}

/// Compute a hash of the config for change detection.
fn compute_config_hash(config: &MqttTenantConfig) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    config.transport.hash(&mut hasher);
    config.host.hash(&mut hasher);
    config.port.hash(&mut hasher);
    config.client_id.hash(&mut hasher);
    config.username.hash(&mut hasher);
    config.password.hash(&mut hasher);
    config.topic_prefix.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::UtcDateTime;
    use uptrakit_web_api_types::mqtt_transport::MqttTransport;

    #[test]
    fn build_config_from_wire_correct() {
        let config = MqttTenantConfig {
            mqtt_client_id: Uuid::parse_str("019471a0-0000-7000-8000-000000000001").unwrap(),
            tenant_id: Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap(),
            enabled: true,
            transport: uptrakit_internal_wire::MqttTransport::Tls,
            host: "broker.example.com".to_string(),
            port: 8883,
            client_id: "my-client".to_string(),
            username: Some("user".to_string()),
            password: Some("pass".to_string()),
            topic_prefix: "home/uptrakit".to_string(),
            updated_at: UtcDateTime::UNIX_EPOCH,
        };

        let mqtt_config = build_config_from_wire(&config);

        assert_eq!(mqtt_config.transport, MqttTransport::Tls);
        assert_eq!(mqtt_config.host, "broker.example.com");
        assert_eq!(mqtt_config.port, 8883);
        assert_eq!(mqtt_config.client_id, "my-client");
        assert_eq!(mqtt_config.username.as_deref(), Some("user"));
        assert_eq!(mqtt_config.password.as_deref(), Some("pass"));
        assert_eq!(mqtt_config.topic_prefix, "home/uptrakit");
    }

    #[test]
    fn build_config_uses_default_port_when_zero() {
        let config = MqttTenantConfig {
            mqtt_client_id: Uuid::parse_str("019471a0-0000-7000-8000-000000000002").unwrap(),
            tenant_id: Uuid::nil(),
            enabled: true,
            transport: uptrakit_internal_wire::MqttTransport::Tls,
            host: "broker.example.com".to_string(),
            port: 0,
            client_id: "client".to_string(),
            username: None,
            password: None,
            topic_prefix: "uptrakit".to_string(),
            updated_at: UtcDateTime::UNIX_EPOCH,
        };

        let mqtt_config = build_config_from_wire(&config);
        assert_eq!(mqtt_config.port, 8883); // TLS default port
    }

    #[test]
    fn config_hash_changes_on_different_values() {
        let config1 = MqttTenantConfig {
            mqtt_client_id: Uuid::parse_str("019471a0-0000-7000-8000-000000000003").unwrap(),
            tenant_id: Uuid::nil(),
            enabled: true,
            transport: uptrakit_internal_wire::MqttTransport::Tls,
            host: "broker1.example.com".to_string(),
            port: 8883,
            client_id: "client".to_string(),
            username: None,
            password: None,
            topic_prefix: "uptrakit".to_string(),
            updated_at: UtcDateTime::UNIX_EPOCH,
        };

        let config2 = MqttTenantConfig {
            mqtt_client_id: Uuid::parse_str("019471a0-0000-7000-8000-000000000003").unwrap(),
            tenant_id: Uuid::nil(),
            enabled: true,
            transport: uptrakit_internal_wire::MqttTransport::Tls,
            host: "broker2.example.com".to_string(), // Different host
            port: 8883,
            client_id: "client".to_string(),
            username: None,
            password: None,
            topic_prefix: "uptrakit".to_string(),
            updated_at: UtcDateTime::UNIX_EPOCH,
        };

        assert_ne!(compute_config_hash(&config1), compute_config_hash(&config2));
    }

    #[test]
    fn config_hash_same_for_same_values() {
        let config1 = MqttTenantConfig {
            mqtt_client_id: Uuid::parse_str("019471a0-0000-7000-8000-000000000004").unwrap(),
            tenant_id: Uuid::nil(),
            enabled: true,
            transport: uptrakit_internal_wire::MqttTransport::Tls,
            host: "broker.example.com".to_string(),
            port: 8883,
            client_id: "client".to_string(),
            username: None,
            password: None,
            topic_prefix: "uptrakit".to_string(),
            updated_at: UtcDateTime::UNIX_EPOCH,
        };

        let config2 = MqttTenantConfig {
            mqtt_client_id: Uuid::parse_str("019471a0-0000-7000-8000-000000000004").unwrap(),
            tenant_id: Uuid::nil(),
            enabled: true,
            transport: uptrakit_internal_wire::MqttTransport::Tls,
            host: "broker.example.com".to_string(),
            port: 8883,
            client_id: "client".to_string(),
            username: None,
            password: None,
            topic_prefix: "uptrakit".to_string(),
            updated_at: UtcDateTime::from_unix_timestamp(12345).unwrap(), // Different updated_at doesn't matter
        };

        assert_eq!(compute_config_hash(&config1), compute_config_hash(&config2));
    }
}
