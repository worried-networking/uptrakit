//! SSH agent UI extension: host management via the extensions framework.
//!
//! Provides:
//! - Extension manifest describing the `ssh-agent.hosts` data table
//! - Action library with `list-hosts`, `bootstrap`, `remove-host` definitions
//! - Action handlers for each action
//! - ECIES decryption of sensitive parameters (auth password, private key)

use base64::Engine as _;
use serde::Deserialize;
use serde_json::json;
use std::path::Path;

use uptrakit_crypto::ecies::sealed_box_decrypt;
use uptrakit_internal_wire::ServiceMessage;
use uptrakit_internal_wire::extension::{
    ExtensionManifest, ExtensionRegisterPayload, ExtensionRequestPayload, ExtensionResponsePayload,
};
use uptrakit_service_sdk::ControllerConnection;
use uptrakit_shared_types::Permission;

use uptrakit_plugin_infrastructure_proxmox::guest_exec::PveGuestType;

use crate::commands::bootstrap::{self, BootstrapParams};
use crate::commands::bootstrap_proxmox::{self, ProxmoxBootstrapParams};
use crate::host_ops;
use crate::ssh_target::SshTarget;

/// Extension ID for the SSH host management extension.
pub const EXTENSION_ID: &str = "ssh-agent.hosts";

// ── Manifest ─────────────────────────────────────────────────────────

/// Build the extension manifest for SSH host management.
///
/// Uses JSON deserialization because all extension types are `#[non_exhaustive]`
/// and cannot be constructed with struct literals from external crates.
///
/// Action fields (`row_actions`, `primary_actions`) contain action ID strings
/// that reference entries in the action library returned by [`build_actions_json`].
pub fn build_manifest() -> ExtensionManifest {
    let manage_hosts = Permission::ManageHosts.as_str();
    serde_json::from_value(json!({
        "id": EXTENSION_ID,
        "label": "SSH Hosts",
        "priority": 450,
        "placement": {
            "type": "page",
            "nav_section": "management",
            "icon": "server"
        },
        "required_permission": manage_hosts,
        "targeting": "targeted",
        "ui": {
            "type": "data_table",
            "columns": [
                { "key": "id", "label": "ID" },
                { "key": "name", "label": "Name", "sortable": true },
                { "key": "hostname", "label": "Hostname", "sortable": true },
                { "key": "port", "label": "Port" },
                { "key": "username", "label": "Username" }
            ],
            "data_action": "list-hosts",
            "row_actions": ["remove-host"],
            "primary_actions": ["bootstrap"]
        }
    }))
    .expect("SSH agent extension manifest JSON should be valid")
}

/// Build the register payload including the manifest and encryption key.
///
/// Uses JSON deserialization because `ExtensionRegisterPayload` is
/// `#[non_exhaustive]`.
pub fn build_register_payload(encryption_public_key: Option<String>) -> ExtensionRegisterPayload {
    let mut payload = json!({
        "manifests": [build_manifest()]
    });
    if let Some(key) = encryption_public_key {
        payload["encryption_public_key"] = serde_json::Value::String(key);
    }
    serde_json::from_value(payload).expect("register payload JSON should be valid")
}

/// Build the action library JSON for registration via `ExtensionActionsRegister`.
///
/// Returns a JSON value that can be deserialized into `ExtensionActionsPayload`.
pub fn build_actions_json() -> serde_json::Value {
    let manage_hosts = Permission::ManageHosts.as_str();
    json!({
        "actions": [
            {
                "action_id": "remove-host",
                "label": "Remove Host",
                "permission": manage_hosts,
                "destructive": true,
                "timeout_seconds": 30
            },
            {
                "action_id": "list-pve-hosts",
                "label": "List PVE Hosts",
                "permission": manage_hosts,
                "timeout_seconds": 10
            },
            {
                "action_id": "bootstrap",
                "label": "Bootstrap Host",
                "permission": manage_hosts,
                "timeout_seconds": 120,
                "ui": {
                    "type": "form",
                    "fields": [
                        {
                            "key": "target",
                            "label": "SSH Target",
                            "field_type": "text",
                            "required": true,
                            "placeholder": "[user@]host[:port]",
                            "help_text": "SSH target in [user@]host[:port] format. Default user: root, port: 22."
                        },
                        {
                            "key": "name",
                            "label": "Host Name",
                            "field_type": "text",
                            "required": true,
                            "placeholder": "my-server",
                            "help_text": "Friendly name for identification."
                        },
                        {
                            "key": "auth_method",
                            "label": "Auth Method",
                            "field_type": "select",
                            "required": true,
                            "default_value": "password",
                            "options": [
                                { "value": "password", "label": "Password" },
                                { "value": "private_key", "label": "Private Key" }
                            ]
                        },
                        {
                            "key": "auth_password",
                            "label": "SSH Password",
                            "field_type": "password",
                            "help_text": "Required when auth method is 'password'.",
                            "sensitive": true
                        },
                        {
                            "key": "auth_private_key",
                            "label": "SSH Private Key",
                            "field_type": "textarea",
                            "placeholder": "-----BEGIN OPENSSH PRIVATE KEY-----",
                            "help_text": "PEM-encoded private key. Required when auth method is 'private_key'.",
                            "sensitive": true
                        },
                        {
                            "key": "target_username",
                            "label": "Target Username",
                            "field_type": "text",
                            "help_text": "User to create/use on the remote host.",
                            "default_value": "uptrakit"
                        },
                        {
                            "key": "host_key_fingerprint",
                            "label": "Host Key Fingerprint",
                            "field_type": "text",
                            "placeholder": "SHA256:...",
                            "help_text": "Expected SHA-256 fingerprint of the host key."
                        },
                        {
                            "key": "strict_host_key_checking",
                            "label": "Strict Host Key Checking",
                            "field_type": "toggle",
                            "help_text": "Require fingerprint match (disables TOFU)."
                        },
                        {
                            "key": "allow_all",
                            "label": "Allow All (NOPASSWD: ALL)",
                            "field_type": "toggle",
                            "help_text": "Use NOPASSWD: ALL in sudoers (less secure)."
                        },
                        {
                            "key": "remove_stale_keys",
                            "label": "Remove Stale Keys",
                            "field_type": "toggle",
                            "help_text": "Remove existing Uptrakit-managed keys before writing new ones."
                        }
                    ]
                }
            },
            {
                "action_id": "bootstrap-proxmox",
                "label": "Bootstrap via Proxmox",
                "permission": manage_hosts,
                "timeout_seconds": 120,
                "ui": {
                    "type": "form",
                    "fields": [
                        {
                            "key": "pve_host_id",
                            "label": "PVE Host",
                            "field_type": "select",
                            "required": true,
                            "help_text": "PVE node to use as gateway.",
                            "dynamic_options": {
                                "action_id": "list-pve-hosts"
                            }
                        },
                        {
                            "key": "vmid",
                            "label": "Guest VMID",
                            "field_type": "text",
                            "required": true,
                            "placeholder": "100",
                            "help_text": "VMID of the target container or virtual machine."
                        },
                        {
                            "key": "guest_type",
                            "label": "Guest Type",
                            "field_type": "select",
                            "required": true,
                            "default_value": "lxc",
                            "options": [
                                { "value": "lxc", "label": "LXC Container" },
                                { "value": "qemu", "label": "QEMU VM" }
                            ]
                        },
                        {
                            "key": "name",
                            "label": "Host Name",
                            "field_type": "text",
                            "required": true,
                            "placeholder": "my-container",
                            "help_text": "Friendly name for identification."
                        },
                        {
                            "key": "target_username",
                            "label": "Target Username",
                            "field_type": "text",
                            "help_text": "User to create/use in the guest.",
                            "default_value": "uptrakit"
                        },
                        {
                            "key": "allow_all",
                            "label": "Allow All (NOPASSWD: ALL)",
                            "field_type": "toggle",
                            "help_text": "Use NOPASSWD: ALL in sudoers (less secure)."
                        },
                    ]
                }
            }
        ]
    })
}

// ── Action dispatch ──────────────────────────────────────────────────

/// Dispatch an extension request to the appropriate handler.
///
/// Actions that complete quickly (`list-hosts`, `remove-host`) respond inline.
/// Long-running actions (`bootstrap`) are spawned as background tasks via `bg_tx`.
#[tracing::instrument(skip_all, fields(
    request_id = %request.request_id,
    extension_id = %request.extension_id,
    action_id = %request.action_id,
))]
pub async fn handle_extension_request(
    request: ExtensionRequestPayload,
    db: &sea_orm::DatabaseConnection,
    state_dir: &Path,
    private_key_der: Option<&[u8]>,
    service_id: Option<uuid::Uuid>,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    conn: &mut ControllerConnection,
) {
    if request.extension_id != EXTENSION_ID {
        tracing::warn!(
            extension_id = %request.extension_id,
            "received extension request for unknown extension"
        );
        let response = make_error_response(&request.request_id, "unknown extension");
        send_response(conn, response).await;
        return;
    }

    match request.action_id.as_str() {
        "list-hosts" => {
            let response = handle_list_hosts(&request.request_id, db).await;
            send_response(conn, response).await;
        }
        "list-pve-hosts" => {
            let response = handle_list_pve_hosts(&request.request_id, db).await;
            send_response(conn, response).await;
        }
        "remove-host" => {
            let response = handle_remove_host(&request.request_id, &request.params, db).await;
            send_response(conn, response).await;
        }
        "bootstrap" => {
            spawn_bootstrap(request, state_dir, private_key_der, service_id, bg_tx);
        }
        "bootstrap-proxmox" => {
            spawn_bootstrap_proxmox(request, state_dir, service_id, bg_tx);
        }
        other => {
            tracing::warn!(action = %other, "unknown extension action");
            let response = make_error_response(&request.request_id, "unknown action");
            send_response(conn, response).await;
        }
    }
}

// ── Action handlers ──────────────────────────────────────────────────

/// List all SSH hosts from the local database.
async fn handle_list_hosts(
    request_id: &str,
    db: &sea_orm::DatabaseConnection,
) -> ExtensionResponsePayload {
    match host_ops::list_hosts(db).await {
        Ok(hosts) => {
            let rows: Vec<serde_json::Value> = hosts
                .into_iter()
                .map(|h| {
                    json!({
                        "id": h.id,
                        "name": h.name,
                        "hostname": h.hostname,
                        "port": h.port,
                        "username": h.username,
                    })
                })
                .collect();
            make_success_response(request_id, json!({ "rows": rows }))
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to list hosts");
            make_error_response(request_id, "failed to list hosts")
        }
    }
}

/// List PVE hosts for dynamic select options in bootstrap-proxmox.
async fn handle_list_pve_hosts(
    request_id: &str,
    db: &sea_orm::DatabaseConnection,
) -> ExtensionResponsePayload {
    match host_ops::find_pve_hosts(db).await {
        Ok(hosts) => {
            let options: Vec<serde_json::Value> = hosts
                .into_iter()
                .map(|h| {
                    json!({
                        "value": h.id,
                        "label": format!("{} ({})", h.name, h.hostname),
                    })
                })
                .collect();
            make_success_response(request_id, json!({ "options": options }))
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to list PVE hosts");
            make_error_response(request_id, "failed to list PVE hosts")
        }
    }
}

/// Remove a host from the local database.
async fn handle_remove_host(
    request_id: &str,
    params: &serde_json::Value,
    db: &sea_orm::DatabaseConnection,
) -> ExtensionResponsePayload {
    let host_id = match params.get("id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => return make_error_response(request_id, "missing required field 'id'"),
    };

    match host_ops::remove_host(db, host_id).await {
        Ok(true) => make_success_response(request_id, json!({ "removed": true })),
        Ok(false) => make_error_response(request_id, "host not found"),
        Err(e) => {
            tracing::error!(error = %e, host = %host_id, "failed to remove host");
            make_error_response(request_id, "failed to remove host")
        }
    }
}

// ── Background tasks ─────────────────────────────────────────────────

/// Spawn the bootstrap workflow as a background task.
fn spawn_bootstrap(
    request: ExtensionRequestPayload,
    state_dir: &Path,
    private_key_der: Option<&[u8]>,
    service_id: Option<uuid::Uuid>,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
) {
    let state_dir = state_dir.to_path_buf();
    let private_key_der = private_key_der.map(|k| k.to_vec());
    let bg_tx = bg_tx.clone();
    let request_id = request.request_id.clone();

    tokio::spawn(async move {
        let response = run_bootstrap_action(
            &request_id,
            &request.params,
            request.sensitive_params.as_ref().map(|s| s.expose_secret()),
            private_key_der.as_deref(),
            service_id,
            &state_dir,
            Some(&bg_tx),
        )
        .await;
        let msg = ServiceMessage::ExtensionResponse(response);
        if bg_tx.send(msg).await.is_err() {
            tracing::error!("failed to send bootstrap result via bg_tx");
        }
    });
}

/// Spawn the Proxmox guest bootstrap workflow as a background task.
fn spawn_bootstrap_proxmox(
    request: ExtensionRequestPayload,
    state_dir: &Path,
    service_id: Option<uuid::Uuid>,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
) {
    let state_dir = state_dir.to_path_buf();
    let bg_tx = bg_tx.clone();
    let request_id = request.request_id.clone();

    tokio::spawn(async move {
        let response =
            run_bootstrap_proxmox_action(&request_id, &request.params, service_id, &state_dir)
                .await;
        let msg = ServiceMessage::ExtensionResponse(response);
        if bg_tx.send(msg).await.is_err() {
            tracing::error!("failed to send proxmox bootstrap result via bg_tx");
        }
    });
}

/// The actual Proxmox guest bootstrap logic.
#[tracing::instrument(skip_all, fields(request_id = %request_id))]
async fn run_bootstrap_proxmox_action(
    request_id: &str,
    params: &serde_json::Value,
    service_id: Option<uuid::Uuid>,
    state_dir: &Path,
) -> ExtensionResponsePayload {
    let pve_host_id = match params.get("pve_host_id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => return make_error_response(request_id, "missing required field 'pve_host_id'"),
    };

    let vmid_str = match params.get("vmid").and_then(|v| v.as_str()) {
        Some(v) => v,
        None => return make_error_response(request_id, "missing required field 'vmid'"),
    };
    let vmid: u32 = match vmid_str.parse() {
        Ok(v) => v,
        Err(_) => return make_error_response(request_id, "vmid must be a number"),
    };

    let guest_type_str = params
        .get("guest_type")
        .and_then(|v| v.as_str())
        .unwrap_or("lxc");
    let guest_type = match guest_type_str {
        "lxc" => PveGuestType::Lxc,
        "qemu" => PveGuestType::Qemu,
        _ => return make_error_response(request_id, "guest_type must be 'lxc' or 'qemu'"),
    };

    let name = match params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => return make_error_response(request_id, "missing required field 'name'"),
    };

    let target_username = params
        .get("target_username")
        .and_then(|v| v.as_str())
        .unwrap_or("uptrakit")
        .to_string();

    let allow_all = param_bool(params, "allow_all");

    let host_id = uuid::Uuid::now_v7();

    let proxmox_params = ProxmoxBootstrapParams {
        pve_host_id,
        vmid,
        guest_type,
        name,
        target_username,
        allow_all,
        host_id,
        service_id,
    };

    match bootstrap_proxmox::run_proxmox_bootstrap(state_dir, proxmox_params).await {
        Ok(result) => {
            tracing::info!(
                %host_id,
                guest_ip = %result.guest_ip,
                "Proxmox guest bootstrap completed successfully"
            );
            make_success_response(
                request_id,
                json!({
                    "host_id": host_id.to_string(),
                    "guest_ip": result.guest_ip,
                }),
            )
        }
        Err(e) => {
            tracing::error!(error = %e, "Proxmox guest bootstrap failed");
            make_error_response(request_id, &format!("bootstrap failed: {e}"))
        }
    }
}

/// Extract a boolean parameter from an extension params object.
///
/// Accepts both JSON booleans (`true`/`false`) and the string representations
/// `"true"`/`"false"` that form-based UIs may emit when all field values are
/// carried as strings. Returns `false` for absent or unrecognised values.
fn param_bool(params: &serde_json::Value, key: &str) -> bool {
    match params.get(key) {
        Some(serde_json::Value::Bool(b)) => *b,
        Some(serde_json::Value::String(s)) => s == "true",
        _ => false,
    }
}

/// The actual bootstrap logic, run inside a spawned task.
#[tracing::instrument(skip_all, fields(request_id = %request_id))]
async fn run_bootstrap_action(
    request_id: &str,
    params: &serde_json::Value,
    sensitive_params_sealed: Option<&str>,
    private_key_der: Option<&[u8]>,
    service_id: Option<uuid::Uuid>,
    state_dir: &Path,
    bg_tx: Option<&tokio::sync::mpsc::Sender<ServiceMessage>>,
) -> ExtensionResponsePayload {
    // Decrypt sensitive params if present.
    let sensitive: Option<SensitiveBootstrapParams> =
        match decrypt_sensitive_params(sensitive_params_sealed, private_key_der) {
            Ok(s) => s,
            Err(msg) => return make_error_response(request_id, &msg),
        };

    // Parse the SSH target.
    let target_str = match params.get("target").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => return make_error_response(request_id, "missing required field 'target'"),
    };

    let parsed_target: SshTarget = match target_str.parse() {
        Ok(t) => t,
        Err(e) => return make_error_response(request_id, &format!("invalid target: {e}")),
    };

    let name = match params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => return make_error_response(request_id, "missing required field 'name'"),
    };

    let auth_method = params
        .get("auth_method")
        .and_then(|v| v.as_str())
        .unwrap_or("password");

    let auth_password = sensitive.as_ref().and_then(|s| s.auth_password.clone());
    let auth_private_key = sensitive.as_ref().and_then(|s| s.auth_private_key.clone());

    // Validate auth method matches provided credentials.
    match auth_method {
        "password" if auth_password.is_none() => {
            return make_error_response(
                request_id,
                "auth_method is 'password' but no password provided",
            );
        }
        "private_key" if auth_private_key.is_none() => {
            return make_error_response(
                request_id,
                "auth_method is 'private_key' but no private key provided",
            );
        }
        _ => {}
    }

    let target_username = params
        .get("target_username")
        .and_then(|v| v.as_str())
        .unwrap_or("uptrakit")
        .to_string();

    let host_key_fingerprint = params
        .get("host_key_fingerprint")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    let strict_host_key_checking = param_bool(params, "strict_host_key_checking");
    let allow_all = param_bool(params, "allow_all");
    let remove_stale_keys = param_bool(params, "remove_stale_keys");

    let host_id = uuid::Uuid::now_v7();

    let bootstrap_params = BootstrapParams {
        name,
        hostname: parsed_target.hostname,
        port: parsed_target.port.unwrap_or(22) as i32,
        auth_username: parsed_target.username.unwrap_or_else(|| "root".to_string()),
        auth_password,
        auth_private_key_pem: auth_private_key,
        use_ssh_agent: false, // Not available in daemon mode.
        target_username,
        target_private_key_pem: None,
        host_key_fingerprint,
        strict_host_key_checking,
        allow_all,
        host_id,
        service_id,
        remove_stale_keys,
    };

    match bootstrap::run_bootstrap(state_dir, bootstrap_params).await {
        Ok(result) => {
            tracing::info!(
                %host_id,
                is_pve_node = result.is_pve_node,
                "bootstrap completed successfully"
            );

            // If PVE credentials were obtained, send ReportPluginConfig to
            // the controller so it creates a Proxmox plugin config.
            if let Some(creds) = &result.pve_credentials
                && let Some(bg_tx) = bg_tx
            {
                let payload: uptrakit_internal_wire::ReportPluginConfigPayload =
                    serde_json::from_value(json!({
                        "request_id": uuid::Uuid::now_v7().to_string(),
                        "plugin_type": "infrastructure_proxmox",
                        "name": format!("pve-{}", host_id),
                        "config": {
                            "api_url": creds.api_url,
                            "api_token": creds.api_token,
                            "verify_ssl": true,
                        },
                    }))
                    .expect("ReportPluginConfigPayload JSON is always valid");
                let msg = ServiceMessage::ReportPluginConfig(payload);
                if bg_tx.send(msg).await.is_err() {
                    tracing::error!("failed to send ReportPluginConfig via bg_tx");
                }
            }

            let mut data = json!({ "host_id": host_id.to_string() });
            if result.is_pve_node {
                data["is_pve_node"] = json!(true);
            }
            make_success_response(request_id, data)
        }
        Err(e) => {
            tracing::error!(error = %e, "bootstrap failed");
            make_error_response(request_id, &format!("bootstrap failed: {e}"))
        }
    }
}

// ── Sensitive params decryption ──────────────────────────────────────

/// Sensitive bootstrap parameters extracted from the ECIES sealed box.
#[derive(Debug, Deserialize)]
struct SensitiveBootstrapParams {
    auth_password: Option<String>,
    auth_private_key: Option<String>,
}

/// Decrypt and deserialize the sealed sensitive params.
///
/// Returns `Ok(None)` when no sensitive params were provided.
/// Returns `Err(message)` on decryption or deserialization failure.
fn decrypt_sensitive_params(
    sealed_base64: Option<&str>,
    private_key_der: Option<&[u8]>,
) -> Result<Option<SensitiveBootstrapParams>, String> {
    let sealed_b64 = match sealed_base64 {
        Some(s) if !s.is_empty() => s,
        _ => return Ok(None),
    };

    let private_key = private_key_der
        .ok_or_else(|| "sensitive params received but no private key available".to_string())?;

    let sealed = base64::engine::general_purpose::STANDARD
        .decode(sealed_b64)
        .map_err(|e| format!("failed to decode sensitive params base64: {e}"))?;

    let plaintext = sealed_box_decrypt(&sealed, private_key)
        .map_err(|e| format!("failed to decrypt sensitive params: {e}"))?;

    let json_str = String::from_utf8(plaintext)
        .map_err(|e| format!("sensitive params plaintext is not valid UTF-8: {e}"))?;

    let params: SensitiveBootstrapParams = serde_json::from_str(&json_str)
        .map_err(|e| format!("failed to parse sensitive params JSON: {e}"))?;

    Ok(Some(params))
}

// ── Helpers ──────────────────────────────────────────────────────────

fn make_success_response(request_id: &str, data: serde_json::Value) -> ExtensionResponsePayload {
    ExtensionResponsePayload {
        request_id: request_id.to_string(),
        success: true,
        data,
        error: None,
    }
}

fn make_error_response(request_id: &str, message: &str) -> ExtensionResponsePayload {
    ExtensionResponsePayload {
        request_id: request_id.to_string(),
        success: false,
        data: serde_json::Value::Null,
        error: Some(message.to_string()),
    }
}

async fn send_response(conn: &mut ControllerConnection, response: ExtensionResponsePayload) {
    if let Err(e) = conn.send(ServiceMessage::ExtensionResponse(response)).await {
        tracing::error!(error = %e, "failed to send extension response");
    }
}
