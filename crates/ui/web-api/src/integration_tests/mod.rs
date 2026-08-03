mod access_rest_enforcement;
mod auth_flow;
mod device_auth_http_roundtrip;
mod device_auth_oauth;
mod enrollment_tokens;
mod error_cases;
mod hosts;
mod if_match;
mod instance_plugins;
mod notifications;
mod oauth_boot_validation;
mod oauth_master_switch_off;
mod oauth_mcp_roundtrip;
#[cfg(feature = "oidc")]
mod oidc_callback;
mod openapi_spec;
mod plugin_configs;
mod plugin_type_settings;
#[cfg(all(feature = "oidc", feature = "nats", feature = "reset-data"))]
mod scope_map;
mod service_ws;
mod services_crud;
mod settings;
mod settings_access;
mod software_items_crud;
mod surface_visibility;
mod surfaces_method_routes;
mod surfaces_routes;
