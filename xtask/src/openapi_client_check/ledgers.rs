//! Reviewed ledgers of intentional spec↔client divergences.

/// `operationId` -> client method name, for legitimate name divergences.
pub const RENAME_MAP: &[(&str, &str)] = &[
    // OIDC provider management: spec uses short names, client adds `oidc_` prefix for clarity
    ("activate_provider", "activate_oidc_provider"),
    (
        "add_tenant_discovery_allowlist_entry",
        "add_discovery_allowlist_entry",
    ),
    // Autodiscovery ignores: client uses "software_ignores" terminology
    ("batch_autodiscovery_ignores", "batch_software_ignores"),
    ("create_autodiscovery_ignore", "create_software_ignore"),
    // Notifications: spec uses short names, client adds `notification_` prefix
    ("create_channel", "create_notification_channel"),
    ("create_provider", "create_oidc_provider"),
    ("create_rule", "create_notification_rule"),
    ("deactivate_provider", "deactivate_oidc_provider"),
    // Services: spec says "deactivate", client says "remove" for UX clarity
    ("deactivate_service", "remove_service"),
    // System services: same rename rationale as deactivate_service above
    ("deactivate_system_service", "remove_system_service"),
    ("delete_autodiscovery_ignore", "delete_software_ignore"),
    ("delete_channel", "delete_notification_channel"),
    ("delete_provider", "delete_oidc_provider"),
    ("delete_rule", "delete_notification_rule"),
    // OAuth device authorization: client adds `oauth_` prefix
    ("device_authorization", "oauth_device_authorization"),
    // OAuth AS metadata: spec name is ambiguous, client is explicit
    ("get_as_metadata", "oauth_authorization_server_metadata"),
    ("get_batch", "get_update_batch"),
    ("get_channel", "get_notification_channel"),
    ("get_provider", "get_oidc_provider"),
    ("get_rule", "get_notification_rule"),
    // Surfaces: spec says get_surface_read, client keeps the read_surface verb
    ("get_surface_read", "read_surface"),
    ("list_autodiscovery_ignores", "list_software_ignores"),
    ("list_batches", "list_update_batches"),
    ("list_channels", "list_notification_channels"),
    ("list_log", "list_notification_log"),
    ("list_providers", "list_oidc_providers"),
    ("list_rules", "list_notification_rules"),
    (
        "list_tenant_discovery_allowlist",
        "list_discovery_allowlist",
    ),
    (
        "remove_tenant_discovery_allowlist_entry",
        "remove_discovery_allowlist_entry",
    ),
    ("test_channel", "test_notification_channel"),
    // OAuth token: client uses descriptive name with `oauth_` prefix
    ("token", "oauth_token"),
    ("update_channel", "update_notification_channel"),
    ("update_provider", "update_oidc_provider"),
    ("update_rule", "update_notification_rule"),
];

/// operationIds intentionally without a client method.
pub const SPEC_ONLY: &[&str] = &[
    // Email change — multi-step browser flow; CLI does not implement it
    "cancel_email_change",
    // Password and profile changes — interactive browser forms not exposed in CLI client
    "change_password",
    // Instance-admin operations not yet implemented in the typed client
    "clear_coordinator_degraded",
    // Email-change confirmation is a redirect link; not a typed API call
    "confirm_email_change",
    "get_config_state",
    "get_instance_plugin",
    // OAuth and Zeroconf global settings not yet wired in the CLI client
    "get_oauth_settings",
    "get_zeroconf_settings",
    "initiate_email_change",
    "list_instance_plugins",
    // MFA / 2FA flows are browser-interactive; not in the typed API client
    "mfa_send_email",
    "mfa_status",
    "mfa_verify",
    // OIDC callback is a browser redirect endpoint, not a typed client call
    "oidc_callback",
    "regenerate_recovery_codes",
    "set_instance_plugin_enabled",
    "totp_confirm",
    "totp_disable",
    "totp_enroll",
    "update_oauth_settings",
    "update_profile",
    "update_zeroconf_settings",
    "upsert_instance_plugin_config",
];

/// Client methods intentionally without a spec operation.
pub const CLIENT_ONLY: &[&str] = &[
    // PKI downloads: binary DER/PEM responses not described in the OpenAPI spec
    "ca_cert",
    "ca_crl",
    // Health check: intentionally outside the /api/v1 prefix, excluded from spec
    "healthz",
    // Raw helpers and streaming methods have no single spec operation counterpart
    "raw_request",
    "stream_batch_progress",
    "stream_events",
    "stream_update_output",
    // Composite helper: removes host assignment and creates autodiscovery ignore atomically
    "unassign_host_with_ignore",
];

/// Normalized path templates present in `paths.rs` but absent from the spec.
pub const PATHS_CLIENT_ONLY: &[&str] = &[
    // SSE event stream — not described in the OpenAPI spec
    "/api/v1/events/stream",
    // PKI binary downloads — outside the typed API spec
    "/api/v1/pki/ca.crt",
    "/api/v1/pki/ca.crl",
    // Health check — intentionally outside the /api/v1 versioned prefix
    "/healthz",
];

/// True if `method` is a `list_all_<x>` whose `list_<x>` sibling exists.
#[must_use]
pub fn is_list_all_companion(method: &str, all_methods: &[String]) -> bool {
    let Some(rest) = method.strip_prefix("list_all_") else {
        return false;
    };
    let sibling = format!("list_{rest}");
    all_methods.contains(&sibling)
}

/// Fail if any ledger entry contradicts another.
///
/// Two conditions are checked:
/// 1. A `CLIENT_ONLY` name also appears in `RENAME_MAP` values or `SPEC_ONLY`.
/// 2. A `RENAME_MAP` key also appears in `SPEC_ONLY` (rename says "map to method",
///    spec-only says "no method" — a direct contradiction).
///
/// # Errors
/// Returns an error string naming the first double-booked or contradicting entry.
pub fn validate_no_double_booking() -> Result<(), String> {
    for name in CLIENT_ONLY {
        if RENAME_MAP.iter().any(|(_, method)| method == name) || SPEC_ONLY.contains(name) {
            return Err(format!(
                "ledger double-booking: '{name}' is in CLIENT_ONLY and another ledger"
            ));
        }
    }
    for (id, _) in RENAME_MAP {
        if SPEC_ONLY.contains(id) {
            return Err(format!(
                "ledger contradiction: operationId '{id}' is in both RENAME_MAP and SPEC_ONLY"
            ));
        }
    }
    Ok(())
}
