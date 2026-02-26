pub(crate) mod agents;
pub mod api_tokens;
pub mod auth;
pub mod autodiscovery;
pub mod ca;
pub mod device_auth;
pub mod health;
pub mod hosts;
pub mod ocsp;
#[cfg(feature = "oidc")]
pub mod oidc_auth;
#[cfg(feature = "oidc")]
pub mod oidc_providers;
pub mod plugin_configs;
pub mod scheduler;
pub mod server_cert;
pub mod service_ws;
pub mod services;
pub mod settings;
pub mod settings_agent_certs;
pub mod settings_auth;
pub mod settings_ca;
pub mod settings_combined;
pub mod settings_mqtt;
pub mod settings_network;
pub mod software_items;
pub mod system_alerts;
pub mod update_history;

// Unified capability-gated WebSocket handler. Replaces the former per-service-type
// modules (agent_ws, mqtt_ws, ssh_agent_ws) with a single module that dispatches
// based on persisted capabilities.
pub(crate) mod service_handler;
