// @generated — do not edit by hand. Run `cargo xtask sync-sdk` to regenerate.
#![allow(unreachable_patterns, clippy::wildcard_in_or_patterns)]
/// Re-export `PluginRole` so consumers can construct and match on role values
/// without a direct `uptrakit-shared-types` dependency.
pub use crate::generated::shared_types::PluginRole;
/// Re-export `SecretString` so consumers (web-api, CLI, openapi-client) can
/// use it without a direct `uptrakit-shared-types` dependency.
pub use crate::generated::shared_types::SecretString;
pub mod access_presets;
pub mod agents;
pub mod api_tokens;
pub mod audit_logs;
pub mod auth;
pub mod autodiscovery;
pub mod batch_actions;
pub mod command_validation;
pub mod device_auth;
pub mod discovery_allowlist;
pub mod enrollment_tokens;
pub mod error;
pub mod events;
pub mod host_tags;
pub mod hosts;
pub mod masked_url;
pub mod notifications;
pub mod oidc_auth;
pub mod oidc_providers;
pub mod pagination;
pub mod permissions;
pub mod plugin_config_test;
pub mod plugin_configs;
pub mod plugin_type_settings;
pub mod prelude;
pub mod profile;
pub mod registration;
pub mod roles;
pub mod scheduler;
pub mod server_cert;
pub mod services;
pub mod settings;
pub mod settings_agent_certs;
pub mod settings_auth;
pub mod settings_ca;
pub mod settings_combined;
pub mod settings_network;
pub mod settings_reset;
pub mod system_services;
pub use masked_url::MaskedUrl;
pub mod settings_nats;
pub mod settings_provider_github;
pub mod settings_zeroconf;
pub mod software_items;
pub mod surfaces;
pub mod system_alerts;
pub mod system_enrollment_tokens;
pub mod update_batches;
pub mod update_history;
pub mod users;
pub mod validation;
/// Default value for `enabled` fields in create-request types.
///
/// Used as `#[serde(default = "crate::generated::types::default_enabled")]` in
/// [`plugin_configs::CreatePluginConfigRequest`].
pub fn default_enabled() -> bool {
    true
}
/// Default value for the `featured` field in create-request types.
///
/// Used as `#[serde(default = "crate::generated::types::default_featured")]` in
/// [`software_items::CreateSoftwareItemRequest`].
pub fn default_featured() -> bool {
    true
}
