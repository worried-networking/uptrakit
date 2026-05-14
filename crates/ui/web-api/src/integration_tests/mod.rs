mod auth_flow;
mod device_auth_oauth;
mod enrollment_tokens;
mod error_cases;
mod hosts;
mod if_match;
mod instance_plugins;
mod notifications;
mod oauth_boot_validation;
mod oauth_master_switch_off;
#[cfg(feature = "oidc")]
mod oidc_callback;
mod plugin_configs;
mod plugin_type_settings;
mod service_ws;
mod services_crud;
mod settings;
mod software_items_crud;
