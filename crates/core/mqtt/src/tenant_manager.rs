use std::collections::HashMap;

use uptrakit_internal_wire::MqttTenantConfig;
use uptrakit_web_api_types::mqtt_transport::MqttTransport;

use crate::mqtt_client::{MqttConfig, MqttHandle};

/// Tracks the cached state for a tenant's MQTT client.
struct TenantState {
    handle: MqttHandle,
    config_hash: u64,
}

/// Manages per-tenant MQTT client lifecycles with push-based config updates.
///
/// Unlike the database-polling version, this manager receives configuration
/// updates from the controller via WebSocket messages.
pub struct TenantManager {
    tenants: HashMap<String, TenantState>,
}

impl TenantManager {
    pub fn new() -> Self {
        Self {
            tenants: HashMap::new(),
        }
    }

    /// Apply tenant assignments from the controller.
    ///
    /// This is called when receiving `TenantAssignments` message.
    pub async fn apply_assignments(&mut self, configs: Vec<MqttTenantConfig>) {
        for config in configs {
            if config.enabled {
                self.start_or_update_tenant(config).await;
            } else {
                self.stop_tenant(&config.tenant_id).await;
            }
        }
    }

    /// Reload a single tenant's configuration.
    ///
    /// This is called when receiving `TenantConfigUpdated` message.
    pub async fn reload_tenant(&mut self, config: MqttTenantConfig) {
        if config.enabled {
            self.start_or_update_tenant(config).await;
        } else {
            self.stop_tenant(&config.tenant_id).await;
        }
    }

    /// Stop a tenant's MQTT client.
    ///
    /// This is called when receiving `TenantRevoked` message or when config is disabled.
    pub async fn stop_tenant(&mut self, tenant_id: &str) {
        if let Some(state) = self.tenants.remove(tenant_id) {
            tracing::info!(%tenant_id, "shutting down MQTT client");
            state.handle.shutdown().await;
        }
    }

    /// Return list of active tenant IDs (for heartbeat).
    pub fn active_tenant_ids(&self) -> Vec<String> {
        self.tenants.keys().cloned().collect()
    }

    /// Graceful shutdown: stop all MQTT clients.
    pub async fn shutdown_all(&mut self) {
        let tenant_ids: Vec<String> = self.tenants.keys().cloned().collect();
        for tenant_id in tenant_ids {
            self.stop_tenant(&tenant_id).await;
        }
    }

    /// Start or update a tenant's MQTT client.
    async fn start_or_update_tenant(&mut self, config: MqttTenantConfig) {
        let tenant_id = config.tenant_id.clone();
        let new_hash = compute_config_hash(&config);

        // Check if we already have this tenant with same config
        if let Some(state) = self.tenants.get(&tenant_id) {
            if state.config_hash == new_hash {
                tracing::debug!(%tenant_id, "config unchanged, skipping update");
                return;
            }
            tracing::info!(%tenant_id, "config changed, reloading");
        }

        // Stop existing client if any
        if let Some(state) = self.tenants.remove(&tenant_id) {
            state.handle.shutdown().await;
        }

        // Build and start new client
        let mqtt_config = build_config_from_wire(&config);
        tracing::info!(%tenant_id, config = ?mqtt_config, "starting MQTT client");

        match crate::mqtt_client::start(mqtt_config).await {
            Ok(handle) => {
                self.tenants.insert(
                    tenant_id,
                    TenantState {
                        handle,
                        config_hash: new_hash,
                    },
                );
            }
            Err(e) => {
                tracing::warn!(%tenant_id, error = ?e, "MQTT client startup failed");
            }
        }
    }
}

impl Default for TenantManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Build MqttConfig from wire protocol config.
fn build_config_from_wire(config: &MqttTenantConfig) -> MqttConfig {
    let transport = MqttTransport::parse(&config.transport).unwrap_or_default();
    let port = if config.port == 0 {
        transport.default_port()
    } else {
        config.port
    };

    MqttConfig {
        transport,
        host: config.host.clone(),
        port,
        path: config.path.clone(),
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
    config.path.hash(&mut hasher);
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

    #[test]
    fn build_config_from_wire_correct() {
        let config = MqttTenantConfig {
            tenant_id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            enabled: true,
            transport: "tls".to_string(),
            host: "broker.example.com".to_string(),
            port: 8883,
            path: Some("/mqtt".to_string()),
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
        assert_eq!(mqtt_config.path.as_deref(), Some("/mqtt"));
        assert_eq!(mqtt_config.client_id, "my-client");
        assert_eq!(mqtt_config.username.as_deref(), Some("user"));
        assert_eq!(mqtt_config.password.as_deref(), Some("pass"));
        assert_eq!(mqtt_config.topic_prefix, "home/uptrakit");
    }

    #[test]
    fn build_config_uses_default_port_when_zero() {
        let config = MqttTenantConfig {
            tenant_id: "test".to_string(),
            enabled: true,
            transport: "tls".to_string(),
            host: "broker.example.com".to_string(),
            port: 0,
            path: None,
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
            tenant_id: "test".to_string(),
            enabled: true,
            transport: "tls".to_string(),
            host: "broker1.example.com".to_string(),
            port: 8883,
            path: None,
            client_id: "client".to_string(),
            username: None,
            password: None,
            topic_prefix: "uptrakit".to_string(),
            updated_at: UtcDateTime::UNIX_EPOCH,
        };

        let config2 = MqttTenantConfig {
            tenant_id: "test".to_string(),
            enabled: true,
            transport: "tls".to_string(),
            host: "broker2.example.com".to_string(), // Different host
            port: 8883,
            path: None,
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
            tenant_id: "test".to_string(),
            enabled: true,
            transport: "tls".to_string(),
            host: "broker.example.com".to_string(),
            port: 8883,
            path: None,
            client_id: "client".to_string(),
            username: None,
            password: None,
            topic_prefix: "uptrakit".to_string(),
            updated_at: UtcDateTime::UNIX_EPOCH,
        };

        let config2 = MqttTenantConfig {
            tenant_id: "test".to_string(),
            enabled: true,
            transport: "tls".to_string(),
            host: "broker.example.com".to_string(),
            port: 8883,
            path: None,
            client_id: "client".to_string(),
            username: None,
            password: None,
            topic_prefix: "uptrakit".to_string(),
            updated_at: UtcDateTime::from_unix_timestamp(12345).unwrap(), // Different updated_at doesn't matter
        };

        assert_eq!(compute_config_hash(&config1), compute_config_hash(&config2));
    }
}
