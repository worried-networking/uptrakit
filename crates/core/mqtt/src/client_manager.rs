use crate::mqtt_client::{MqttConfig, MqttHandle};
use uptrakit_internal_wire::MqttTenantConfig;

/// Tracks the cached state for an MQTT client.
pub(crate) struct ClientState {
    pub(crate) handle: MqttHandle,
    pub(crate) config_hash: u64,
    pub(crate) tenant_id: uuid::Uuid,
    pub(crate) topic_prefix: String,
    pub(crate) ha_discovery: bool,
    pub(crate) ha_discovery_prefix: String,
}

/// Build `MqttConfig` from wire protocol config.
pub(crate) fn build_config_from_wire(config: &MqttTenantConfig) -> MqttConfig {
    // Wire and MqttConfig now use the same canonical MqttTransport type.
    let transport = config.transport;
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
        ca_pem: config.ca_pem.clone(),
        topic_prefix: config.topic_prefix.clone(),
    }
}

/// Compute a hash of the config for change detection.
///
/// Uses `DefaultHasher` (SipHash with a per-process random seed), so
/// hashes are only valid within the same process lifetime. This is
/// correct for the intended use: detecting config changes between
/// consecutive `TenantAssignments` messages during a single service
/// run. Hashes are not persisted or compared across process restarts.
pub(crate) fn compute_config_hash(config: &MqttTenantConfig) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    config.transport.hash(&mut hasher);
    config.host.hash(&mut hasher);
    config.port.hash(&mut hasher);
    config.client_id.hash(&mut hasher);
    config.username.hash(&mut hasher);
    config.password.hash(&mut hasher);
    config.ca_pem.hash(&mut hasher);
    config.topic_prefix.hash(&mut hasher);
    config.ha_discovery.hash(&mut hasher);
    config.ha_discovery_prefix.hash(&mut hasher);
    hasher.finish()
}
