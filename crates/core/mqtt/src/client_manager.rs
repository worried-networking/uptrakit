use serde::{Deserialize, Serialize};
use uptrakit_internal_wire::SecretString;
use uuid::Uuid;

use crate::mqtt_client::{MqttConfig, MqttHandle};
use crate::types::MqttTransport;

/// Parsed MQTT client configuration as delivered by the service config store.
///
/// Deserialized from the JSON value of a `"clients.{uuid}"` config entry.
/// The `mqtt_client_id` and `tenant_id` fields are injected after deserialization
/// (they come from the config key and `ServiceConfigEntry::tenant_id` respectively).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct ParsedMqttClientConfig {
    /// Deserialized from the config key suffix — set by the caller after parsing.
    #[serde(skip_deserializing, skip_serializing)]
    pub mqtt_client_id: Uuid,
    /// From `ServiceConfigEntry::tenant_id` — set by the caller after parsing.
    #[serde(skip_deserializing, skip_serializing)]
    pub tenant_id: Uuid,
    /// Whether this connection is enabled.
    #[serde(default = "bool_true")]
    pub enabled: bool,
    /// Transport protocol.
    pub transport: MqttTransport,
    /// Broker hostname.
    pub host: String,
    /// Broker port (0 = use transport default).
    #[serde(default)]
    pub port: u16,
    /// MQTT client ID.
    pub client_id: String,
    /// Optional username for broker authentication.
    #[serde(default)]
    pub username: Option<SecretString>,
    /// Optional password for broker authentication.
    #[serde(default)]
    pub password: Option<SecretString>,
    /// Optional custom CA certificate in PEM format.
    #[serde(default)]
    pub ca_pem: Option<SecretString>,
    /// Topic prefix.
    pub topic_prefix: String,
    /// Whether to publish Home Assistant MQTT discovery topics.
    #[serde(default)]
    pub ha_discovery: bool,
    /// HA discovery topic prefix.
    #[serde(default = "default_ha_discovery_prefix")]
    pub ha_discovery_prefix: String,
}

fn bool_true() -> bool {
    true
}

fn default_ha_discovery_prefix() -> String {
    "homeassistant".to_string()
}

/// Tracks the live state for an MQTT client connection.
pub(crate) struct ClientState {
    pub(crate) handle: MqttHandle,
    pub(crate) config_hash: u64,
    pub(crate) tenant_id: Uuid,
    pub(crate) topic_prefix: String,
    pub(crate) ha_discovery: bool,
    pub(crate) ha_discovery_prefix: String,
}

/// Build `MqttConfig` from a parsed config entry.
pub(crate) fn build_config_from_parsed(config: &ParsedMqttClientConfig) -> MqttConfig {
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
/// consecutive `ServiceConfigUpdated` messages during a single service
/// run. Hashes are not persisted or compared across process restarts.
pub(crate) fn compute_config_hash(config: &ParsedMqttClientConfig) -> u64 {
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
