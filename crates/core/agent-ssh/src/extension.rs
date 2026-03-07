//! SSH agent UI extension: host management via the extensions framework.
//!
//! Provides:
//! - Extension manifest describing the `ssh-agent.hosts` data table
//! - Action library with `list-hosts`, `bootstrap`, `sync-host`, `remove-host` definitions
//! - Action handlers for each action
//! - ECIES decryption of sensitive parameters (auth password, private key)

use serde::Deserialize;
use serde_json::json;
use std::path::Path;

use uptrakit_internal_wire::ServiceMessage;
use uptrakit_internal_wire::extension::{
    ActionDef, ActionUi, ExtensionManifest, ExtensionPlacement, ExtensionRegisterPayload,
    ExtensionRequestPayload, ExtensionResponsePayload, ExtensionTargeting, ExtensionUi, FieldDef,
    FieldType, FormDef, SelectOption, SelectSource, TableColumn,
};
use uptrakit_service_sdk::ControllerConnection;
use uptrakit_shared_types::Permission;

use uptrakit_plugin_infrastructure_proxmox::guest_exec::PveGuestType;

use crate::commands::bootstrap::{self, BootstrapParams};
use crate::commands::bootstrap_proxmox::{self, ProxmoxBootstrapParams};
use crate::commands::sync;
use crate::host_ops;
use crate::ssh_target::SshTarget;

/// Extension ID for the SSH host management extension.
pub const EXTENSION_ID: &str = "ssh-agent.hosts";

// ── Manifest ─────────────────────────────────────────────────────────

/// Build the extension manifest for SSH host management.
///
/// Action fields (`row_actions`, `primary_actions`) contain action ID strings
/// that reference entries in the action library returned by [`build_actions`].
pub fn build_manifest() -> ExtensionManifest {
    ExtensionManifest::new(
        EXTENSION_ID,
        "SSH Hosts",
        450,
        ExtensionPlacement::Page {
            nav_section: "management".to_string(),
            icon: Some("server".to_string()),
        },
        ExtensionUi::DataTable {
            columns: vec![
                TableColumn::new("id", "ID"),
                TableColumn::new("name", "Name").sortable(),
                TableColumn::new("hostname", "Hostname").sortable(),
                TableColumn::new("port", "Port"),
                TableColumn::new("username", "Username"),
            ],
            data_action: "list-hosts".to_string(),
            row_actions: vec!["sync-host".to_string(), "remove-host".to_string()],
            primary_actions: vec![
                "bootstrap".to_string(),
                "bootstrap-proxmox".to_string(),
                "bootstrap-proxmox-guest".to_string(),
            ],
            context_selector: None,
        },
    )
    .with_permission(Permission::ManageHosts)
    .with_targeting(ExtensionTargeting::Targeted)
}

/// Build the register payload including the manifest and encryption key.
pub fn build_register_payload(encryption_public_key: Option<String>) -> ExtensionRegisterPayload {
    let payload = ExtensionRegisterPayload::new(vec![build_manifest()]);
    match encryption_public_key {
        Some(key) => payload.with_encryption_public_key(key),
        None => payload,
    }
}

/// Build the action library for registration via `ExtensionActionsRegister`.
pub fn build_actions() -> Vec<ActionDef> {
    vec![
        ActionDef::new("remove-host", "Remove Host")
            .with_permission(Permission::ManageHosts)
            .destructive()
            .with_confirm_entity_field("name")
            .with_timeout(30),
        sync_host_action(),
        ActionDef::new("list-pve-hosts", "List PVE Hosts")
            .with_permission(Permission::ManageHosts)
            .with_timeout(10),
        ActionDef::new("list-discovered-guests", "List Discovered Guests")
            .with_permission(Permission::ManageHosts)
            .with_timeout(15),
        bootstrap_action(),
        bootstrap_proxmox_action(),
        bootstrap_proxmox_guest_action(),
    ]
}

/// Build the sync-host action definition with optional auth override form.
fn sync_host_action() -> ActionDef {
    ActionDef::new("sync-host", "Sync Host")
        .with_permission(Permission::ManageHosts)
        .with_timeout(120)
        .with_ui(ActionUi::Form(FormDef::new(vec![
            FieldDef::new("auth_method", "Auth Method")
                .with_type(FieldType::Select)
                .with_default_value("stored")
                .with_options(vec![
                    SelectOption::new("stored", "Stored Credentials"),
                    SelectOption::new("password", "Password"),
                    SelectOption::new("private_key", "Private Key"),
                ]),
            FieldDef::new("username", "SSH Username")
                .with_default_value("root")
                .with_help_text("User to connect as (e.g. root). Only used with custom auth.")
                .with_visible_when(
                    "auth_method",
                    vec!["password".to_string(), "private_key".to_string()],
                ),
            FieldDef::new("auth_password", "SSH Password")
                .with_type(FieldType::Password)
                .with_help_text("Required when auth method is 'password'.")
                .sensitive()
                .with_visible_when("auth_method", vec!["password".to_string()]),
            FieldDef::new("auth_private_key", "SSH Private Key")
                .with_type(FieldType::Textarea)
                .with_placeholder("-----BEGIN OPENSSH PRIVATE KEY-----")
                .with_help_text(
                    "PEM-encoded private key. Required when auth method is 'private_key'.",
                )
                .sensitive()
                .with_visible_when("auth_method", vec!["private_key".to_string()]),
            FieldDef::new("allow_all", "Allow All (NOPASSWD: ALL)")
                .with_type(FieldType::Toggle)
                .with_help_text("Use NOPASSWD: ALL in sudoers (less secure)."),
        ])))
}

/// Build the bootstrap host action definition with its form UI.
fn bootstrap_action() -> ActionDef {
    ActionDef::new("bootstrap", "Bootstrap Host")
        .with_permission(Permission::ManageHosts)
        .with_timeout(120)
        .with_ui(ActionUi::Form(FormDef::new(vec![
            FieldDef::new("target", "SSH Target")
                .required()
                .with_placeholder("[user@]host[:port]")
                .with_help_text(
                    "SSH target in [user@]host[:port] format. Default user: root, port: 22.",
                ),
            FieldDef::new("name", "Host Name")
                .required()
                .with_placeholder("my-server")
                .with_help_text("Friendly name for identification."),
            FieldDef::new("auth_method", "Auth Method")
                .with_type(FieldType::Select)
                .required()
                .with_default_value("password")
                .with_options(vec![
                    SelectOption::new("password", "Password"),
                    SelectOption::new("private_key", "Private Key"),
                ]),
            FieldDef::new("auth_password", "SSH Password")
                .with_type(FieldType::Password)
                .with_help_text("Required when auth method is 'password'.")
                .sensitive()
                .with_visible_when("auth_method", vec!["password".to_string()]),
            FieldDef::new("auth_private_key", "SSH Private Key")
                .with_type(FieldType::Textarea)
                .with_placeholder("-----BEGIN OPENSSH PRIVATE KEY-----")
                .with_help_text(
                    "PEM-encoded private key. Required when auth method is 'private_key'.",
                )
                .sensitive()
                .with_visible_when("auth_method", vec!["private_key".to_string()]),
            FieldDef::new("target_username", "Target Username")
                .with_help_text("User to create/use on the remote host.")
                .with_default_value("uptrakit"),
            FieldDef::new("host_key_fingerprint", "Host Key Fingerprint")
                .with_placeholder("SHA256:...")
                .with_help_text("Expected SHA-256 fingerprint of the host key."),
            FieldDef::new("strict_host_key_checking", "Strict Host Key Checking")
                .with_type(FieldType::Toggle)
                .with_help_text("Require fingerprint match (disables TOFU)."),
            FieldDef::new("allow_all", "Allow All (NOPASSWD: ALL)")
                .with_type(FieldType::Toggle)
                .with_help_text("Use NOPASSWD: ALL in sudoers (less secure)."),
            FieldDef::new("remove_stale_keys", "Remove Stale Keys")
                .with_type(FieldType::Toggle)
                .with_help_text("Remove existing Uptrakit-managed keys before writing new ones."),
        ])))
}

/// Build the Proxmox bootstrap action definition with its form UI.
fn bootstrap_proxmox_action() -> ActionDef {
    ActionDef::new("bootstrap-proxmox", "Bootstrap via Proxmox")
        .with_permission(Permission::ManageHosts)
        .with_timeout(120)
        .with_ui(ActionUi::Form(FormDef::new(vec![
            FieldDef::new("pve_host_id", "PVE Host")
                .with_type(FieldType::Select)
                .required()
                .with_help_text("PVE node to use as gateway.")
                .with_select_source(SelectSource::Action {
                    action_id: "list-pve-hosts".to_string(),
                }),
            FieldDef::new("vmid", "Guest VMID")
                .required()
                .with_placeholder("100")
                .with_help_text("VMID of the target container or virtual machine."),
            FieldDef::new("guest_type", "Guest Type")
                .with_type(FieldType::Select)
                .required()
                .with_default_value("lxc")
                .with_options(vec![
                    SelectOption::new("lxc", "LXC Container"),
                    SelectOption::new("qemu", "QEMU VM"),
                ]),
            FieldDef::new("name", "Host Name")
                .required()
                .with_placeholder("my-container")
                .with_help_text("Friendly name for identification."),
            FieldDef::new("target_username", "Target Username")
                .with_help_text("User to create/use in the guest.")
                .with_default_value("uptrakit"),
            FieldDef::new("allow_all", "Allow All (NOPASSWD: ALL)")
                .with_type(FieldType::Toggle)
                .with_help_text("Use NOPASSWD: ALL in sudoers (less secure)."),
            FieldDef::new("remove_stale_keys", "Remove Stale Keys")
                .with_type(FieldType::Toggle)
                .with_help_text(
                    "Remove existing Uptrakit-managed keys from authorized_keys before \
                     writing the new entry. Same-service keys are always removed regardless.",
                ),
        ])))
}

/// Build the "Bootstrap via Discovered Guest" action definition.
///
/// Presents a dropdown of unmatched Proxmox guests discovered by the Proxmox
/// plugin, allowing users to bootstrap a guest without manually specifying
/// VMID, node, or guest type.
fn bootstrap_proxmox_guest_action() -> ActionDef {
    ActionDef::new("bootstrap-proxmox-guest", "Bootstrap Discovered Guest")
        .with_permission(Permission::ManageHosts)
        .with_timeout(120)
        .with_ui(ActionUi::Form(FormDef::new(vec![
            FieldDef::new("discovered_guest", "Discovered Guest")
                .with_type(FieldType::Select)
                .required()
                .with_help_text("Select a Proxmox guest discovered by the Proxmox VE plugin.")
                .with_select_source(SelectSource::Action {
                    action_id: "list-discovered-guests".to_string(),
                }),
            FieldDef::new("name", "Host Name")
                .with_placeholder("Leave blank to use guest hostname")
                .with_help_text(
                    "Friendly name for identification. Defaults to the guest's \
                     Proxmox hostname if left blank.",
                ),
            FieldDef::new("target_username", "Target Username")
                .with_help_text("User to create/use in the guest.")
                .with_default_value("uptrakit"),
            FieldDef::new("allow_all", "Allow All (NOPASSWD: ALL)")
                .with_type(FieldType::Toggle)
                .with_help_text("Use NOPASSWD: ALL in sudoers (less secure)."),
            FieldDef::new("remove_stale_keys", "Remove Stale Keys")
                .with_type(FieldType::Toggle)
                .with_help_text(
                    "Remove existing Uptrakit-managed keys from authorized_keys before \
                     writing the new entry. Same-service keys are always removed regardless.",
                ),
        ])))
}

// ── Extension context ────────────────────────────────────────────────

/// Shared context for extension request handling.
///
/// Groups the handler-level state needed by action dispatch and background
/// bootstrap tasks, avoiding parameter-count explosion on public APIs.
pub struct ExtensionContext<'a> {
    pub db: &'a sea_orm::DatabaseConnection,
    pub state_dir: &'a Path,
    pub private_key_der: Option<&'a [u8]>,
    pub service_id: Option<uuid::Uuid>,
    pub tenant_id: Option<uuid::Uuid>,
    pub bg_tx: &'a tokio::sync::mpsc::Sender<ServiceMessage>,
    pub extension_proxy: &'a std::sync::Arc<uptrakit_service_sdk::ServiceExtensionProxy>,
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
    ctx: &ExtensionContext<'_>,
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
            let response = handle_list_hosts(&request.request_id, ctx.db).await;
            send_response(conn, response).await;
        }
        "list-pve-hosts" => {
            let response = handle_list_pve_hosts(&request.request_id, ctx.db).await;
            send_response(conn, response).await;
        }
        "remove-host" => {
            let response = handle_remove_host(&request.request_id, &request.params, ctx.db).await;
            send_response(conn, response).await;
        }
        "sync-host" => {
            spawn_sync_host(request, ctx);
        }
        "bootstrap" => {
            spawn_bootstrap(request, ctx);
        }
        "bootstrap-proxmox" => {
            spawn_bootstrap_proxmox(request, ctx.state_dir, ctx.service_id, ctx.bg_tx);
        }
        "list-discovered-guests" => {
            spawn_list_discovered_guests(request, ctx.extension_proxy, ctx.bg_tx);
        }
        "bootstrap-proxmox-guest" => {
            spawn_bootstrap_proxmox_guest(request, ctx);
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
fn spawn_bootstrap(request: ExtensionRequestPayload, ctx: &ExtensionContext<'_>) {
    let state_dir = ctx.state_dir.to_path_buf();
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());
    let bg_tx = ctx.bg_tx.clone();
    let service_id = ctx.service_id;
    let tenant_id = ctx.tenant_id;
    let request_id = request.request_id.clone();

    tokio::spawn(async move {
        let response = run_bootstrap_action(BootstrapActionArgs {
            request_id: &request_id,
            params: &request.params,
            sensitive_params_sealed: request.sensitive_params.as_ref().map(|s| s.expose_secret()),
            private_key_der: private_key_der.as_deref(),
            service_id,
            tenant_id,
            state_dir: &state_dir,
            bg_tx: Some(&bg_tx),
        })
        .await;
        let msg = ServiceMessage::ExtensionResponse(response);
        if bg_tx.send(msg).await.is_err() {
            tracing::error!("failed to send bootstrap result via bg_tx");
        }
    });
}

/// Spawn the sync-host workflow as a background task.
fn spawn_sync_host(request: ExtensionRequestPayload, ctx: &ExtensionContext<'_>) {
    let db_state_dir = ctx.state_dir.to_path_buf();
    let bg_tx = ctx.bg_tx.clone();
    let tenant_id = ctx.tenant_id;
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());
    let request_id = request.request_id.clone();

    tokio::spawn(async move {
        let host_id = match request.params.get("id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => {
                let resp = make_error_response(&request_id, "missing required field 'id'");
                let msg = ServiceMessage::ExtensionResponse(resp);
                let _ = bg_tx.send(msg).await;
                return;
            }
        };

        // Decrypt sensitive params (auth password / private key) if present.
        let sensitive: Option<SensitiveAuthParams> =
            match uptrakit_service_sdk::decrypt_sensitive_params(
                request.sensitive_params.as_ref().map(|s| s.expose_secret()),
                private_key_der.as_deref(),
            ) {
                Ok(s) => s,
                Err(msg) => {
                    let resp = make_error_response(&request_id, &msg);
                    let msg = ServiceMessage::ExtensionResponse(resp);
                    let _ = bg_tx.send(msg).await;
                    return;
                }
            };

        let auth_method = request
            .params
            .get("auth_method")
            .and_then(|v| v.as_str())
            .unwrap_or("stored");

        let auth_override = match auth_method {
            "stored" => None,
            "password" => {
                let password = sensitive.as_ref().and_then(|s| s.auth_password.as_deref());
                match password {
                    Some(pw) => Some(sync::SyncAuthOverride {
                        username: request
                            .params
                            .get("username")
                            .and_then(|v| v.as_str())
                            .unwrap_or("root")
                            .to_string(),
                        auth_password: Some(pw.to_string()),
                        auth_private_key_pem: None,
                    }),
                    None => {
                        let resp = make_error_response(
                            &request_id,
                            "auth_method is 'password' but no password provided",
                        );
                        let msg = ServiceMessage::ExtensionResponse(resp);
                        let _ = bg_tx.send(msg).await;
                        return;
                    }
                }
            }
            "private_key" => {
                let key = sensitive
                    .as_ref()
                    .and_then(|s| s.auth_private_key.as_deref());
                match key {
                    Some(pem) => Some(sync::SyncAuthOverride {
                        username: request
                            .params
                            .get("username")
                            .and_then(|v| v.as_str())
                            .unwrap_or("root")
                            .to_string(),
                        auth_password: None,
                        auth_private_key_pem: Some(pem.to_string()),
                    }),
                    None => {
                        let resp = make_error_response(
                            &request_id,
                            "auth_method is 'private_key' but no private key provided",
                        );
                        let msg = ServiceMessage::ExtensionResponse(resp);
                        let _ = bg_tx.send(msg).await;
                        return;
                    }
                }
            }
            other => {
                let resp =
                    make_error_response(&request_id, &format!("unknown auth_method '{other}'"));
                let msg = ServiceMessage::ExtensionResponse(resp);
                let _ = bg_tx.send(msg).await;
                return;
            }
        };

        let allow_all = param_bool(&request.params, "allow_all");

        let db = match crate::db::init_db(&db_state_dir).await {
            Ok(db) => db,
            Err(e) => {
                let resp = make_error_response(
                    &request_id,
                    &format!("failed to initialize database: {e}"),
                );
                let msg = ServiceMessage::ExtensionResponse(resp);
                let _ = bg_tx.send(msg).await;
                return;
            }
        };

        let response = match sync::run_for_extension(
            host_id,
            &db,
            tenant_id,
            auth_override.as_ref(),
            allow_all,
        )
        .await
        {
            Ok(summary) => {
                make_success_response(&request_id, serde_json::json!({ "summary": summary }))
            }
            Err(e) => make_error_response(&request_id, &e),
        };
        let msg = ServiceMessage::ExtensionResponse(response);
        if bg_tx.send(msg).await.is_err() {
            tracing::error!("failed to send sync-host result via bg_tx");
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

/// Spawn a task to list discovered Proxmox guests via the extension proxy.
fn spawn_list_discovered_guests(
    request: ExtensionRequestPayload,
    proxy: &std::sync::Arc<uptrakit_service_sdk::ServiceExtensionProxy>,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
) {
    let proxy = std::sync::Arc::clone(proxy);
    let bg_tx = bg_tx.clone();
    let request_id = request.request_id.clone();

    tokio::spawn(async move {
        let response = invoke_proxy_action(
            &proxy,
            &bg_tx,
            "proxmox.hosts",
            "list-all-unmatched",
            serde_json::Value::Object(serde_json::Map::new()),
        )
        .await;

        // Wrap the proxy response (or error) into our extension response.
        let ext_response = match response {
            Ok(proxy_resp) if proxy_resp.success => {
                make_success_response(&request_id, proxy_resp.data)
            }
            Ok(proxy_resp) => {
                // Proxmox plugin not installed or returned error — return empty options.
                tracing::debug!(
                    error = ?proxy_resp.error,
                    "list-all-unmatched returned error, returning empty options"
                );
                make_success_response(&request_id, json!({ "options": [] }))
            }
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "list-all-unmatched proxy call failed, returning empty options"
                );
                make_success_response(&request_id, json!({ "options": [] }))
            }
        };

        let msg = ServiceMessage::ExtensionResponse(ext_response);
        if bg_tx.send(msg).await.is_err() {
            tracing::error!("failed to send list-discovered-guests result via bg_tx");
        }
    });
}

/// Invoke an extension action on the controller via the proxy.
///
/// Sends the request via `bg_tx` (which flows through the event loop to
/// `conn.send()`), then waits for the controller's response via the proxy's
/// oneshot channel.
async fn invoke_proxy_action(
    proxy: &uptrakit_service_sdk::ServiceExtensionProxy,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    extension_id: &str,
    action_id: &str,
    params: serde_json::Value,
) -> Result<ExtensionResponsePayload, uptrakit_service_sdk::ServiceExtensionProxyError> {
    let pending = proxy.invoke(extension_id, action_id, params);

    // Send the request to the controller via bg_tx.
    if bg_tx.send(pending.message.clone()).await.is_err() {
        return Err(uptrakit_service_sdk::ServiceExtensionProxyError::SendFailed);
    }

    // Wait for the response (15s timeout for proxy calls).
    pending
        .wait(proxy, std::time::Duration::from_secs(15))
        .await
}

/// Spawn the "Bootstrap via Discovered Guest" workflow.
fn spawn_bootstrap_proxmox_guest(request: ExtensionRequestPayload, ctx: &ExtensionContext<'_>) {
    let state_dir = ctx.state_dir.to_path_buf();
    let bg_tx = ctx.bg_tx.clone();
    let service_id = ctx.service_id;
    let proxy = std::sync::Arc::clone(ctx.extension_proxy);
    let request_id = request.request_id.clone();

    tokio::spawn(async move {
        let response = run_bootstrap_proxmox_guest_action(
            &request_id,
            &request.params,
            service_id,
            &state_dir,
            &proxy,
            &bg_tx,
        )
        .await;
        let msg = ServiceMessage::ExtensionResponse(response);
        if bg_tx.send(msg).await.is_err() {
            tracing::error!("failed to send proxmox guest bootstrap result via bg_tx");
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
    let remove_stale_keys = param_bool(params, "remove_stale_keys");

    let host_id = uuid::Uuid::now_v7();

    let proxmox_params = ProxmoxBootstrapParams {
        pve_host_id,
        vmid,
        guest_type,
        name,
        target_username,
        allow_all,
        remove_stale_keys,
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

/// The "Bootstrap via Discovered Guest" action logic.
///
/// 1. Resolves the selected discovered guest's metadata (node, VMID, type).
/// 2. Auto-detects the PVE host from the guest's node name.
/// 3. Runs the Proxmox bootstrap workflow.
/// 4. On success, auto-matches the host to the Proxmox guest mapping via the
///    Proxmox plugin's `match` action.
#[tracing::instrument(skip_all, fields(request_id = %request_id))]
async fn run_bootstrap_proxmox_guest_action(
    request_id: &str,
    params: &serde_json::Value,
    service_id: Option<uuid::Uuid>,
    state_dir: &Path,
    proxy: &uptrakit_service_sdk::ServiceExtensionProxy,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
) -> ExtensionResponsePayload {
    // Get the selected discovered guest's mapping_id.
    let discovered_guest = match params.get("discovered_guest").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            return make_error_response(request_id, "missing required field 'discovered_guest'");
        }
    };

    // Look up the guest details from the Proxmox plugin.
    let guests = match invoke_proxy_action(
        proxy,
        bg_tx,
        "proxmox.hosts",
        "list-all-unmatched",
        serde_json::Value::Object(serde_json::Map::new()),
    )
    .await
    {
        Ok(resp) if resp.success => resp.data,
        Ok(resp) => {
            let err = resp
                .error
                .unwrap_or_else(|| "Proxmox plugin returned error".to_string());
            return make_error_response(request_id, &err);
        }
        Err(e) => {
            return make_error_response(
                request_id,
                &format!("failed to query Proxmox plugin (is it installed?): {e}"),
            );
        }
    };

    // Find the selected guest in the options.
    let options = guests
        .get("options")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let guest = options.iter().find(|o| {
        o.get("value")
            .and_then(|v| v.as_str())
            .is_some_and(|v| v == discovered_guest)
    });

    let guest = match guest {
        Some(g) => g,
        None => {
            return make_error_response(
                request_id,
                "selected guest not found in discovered guests list",
            );
        }
    };

    let vmid: u32 = match guest.get("proxmox_vmid").and_then(|v| v.as_i64()) {
        Some(v) if v > 0 => v as u32,
        _ => return make_error_response(request_id, "invalid VMID in discovered guest"),
    };

    let guest_type_str = guest
        .get("proxmox_type")
        .and_then(|v| v.as_str())
        .unwrap_or("lxc");

    let guest_type = match guest_type_str {
        "lxc" => PveGuestType::Lxc,
        "qemu" => PveGuestType::Qemu,
        _ => return make_error_response(request_id, "unknown guest type in discovered guest"),
    };

    // Auto-detect PVE host from the guest's node name + plugin config ID.
    let proxmox_node = match guest.get("proxmox_node").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => {
            return make_error_response(request_id, "missing proxmox_node in discovered guest");
        }
    };

    let plugin_config_id = match guest.get("plugin_config_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            return make_error_response(request_id, "missing plugin_config_id in discovered guest");
        }
    };

    let pve_host_id = match find_pve_host_for_node(state_dir, proxmox_node, plugin_config_id).await
    {
        Ok(id) => id,
        Err(msg) => return make_error_response(request_id, &msg),
    };

    // Auto-fill hostname: user override → guest hostname → proxmox_name → "unnamed".
    let name = match params
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        Some(n) => n.to_string(),
        None => guest
            .get("hostname")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                guest
                    .get("proxmox_name")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
            })
            .unwrap_or("unnamed")
            .to_string(),
    };

    let target_username = params
        .get("target_username")
        .and_then(|v| v.as_str())
        .unwrap_or("uptrakit")
        .to_string();

    let allow_all = param_bool(params, "allow_all");
    let remove_stale_keys = param_bool(params, "remove_stale_keys");
    let host_id = uuid::Uuid::now_v7();

    let proxmox_params = ProxmoxBootstrapParams {
        pve_host_id,
        vmid,
        guest_type,
        name,
        target_username,
        allow_all,
        remove_stale_keys,
        host_id,
        service_id,
    };

    match bootstrap_proxmox::run_proxmox_bootstrap(state_dir, proxmox_params).await {
        Ok(result) => {
            tracing::info!(
                %host_id,
                guest_ip = %result.guest_ip,
                "discovered guest bootstrap completed successfully"
            );

            // Auto-match: invoke the Proxmox plugin's "match" action.
            let match_params = json!({
                "mapping_id": discovered_guest,
                "host_id": host_id.to_string(),
            });
            match invoke_proxy_action(proxy, bg_tx, "proxmox.hosts", "match", match_params).await {
                Ok(resp) if resp.success => {
                    tracing::info!(
                        %host_id,
                        mapping_id = %discovered_guest,
                        "auto-matched host to Proxmox guest mapping"
                    );
                }
                Ok(resp) => {
                    tracing::warn!(
                        %host_id,
                        mapping_id = %discovered_guest,
                        error = ?resp.error,
                        "auto-match failed (bootstrap succeeded, match can be done manually)"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        %host_id,
                        mapping_id = %discovered_guest,
                        error = %e,
                        "auto-match proxy call failed (bootstrap succeeded)"
                    );
                }
            }

            make_success_response(
                request_id,
                json!({
                    "host_id": host_id.to_string(),
                    "guest_ip": result.guest_ip,
                    "auto_matched": true,
                }),
            )
        }
        Err(e) => {
            tracing::error!(error = %e, "discovered guest bootstrap failed");
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

/// Arguments for the bootstrap action, bundled to stay within the 7-arg limit.
struct BootstrapActionArgs<'a> {
    request_id: &'a str,
    params: &'a serde_json::Value,
    sensitive_params_sealed: Option<&'a str>,
    private_key_der: Option<&'a [u8]>,
    service_id: Option<uuid::Uuid>,
    tenant_id: Option<uuid::Uuid>,
    state_dir: &'a Path,
    bg_tx: Option<&'a tokio::sync::mpsc::Sender<ServiceMessage>>,
}

/// The actual bootstrap logic, run inside a spawned task.
#[tracing::instrument(skip_all, fields(request_id = %args.request_id))]
async fn run_bootstrap_action(args: BootstrapActionArgs<'_>) -> ExtensionResponsePayload {
    let request_id = args.request_id;
    let bg_tx = args.bg_tx;

    // Decrypt sensitive params if present.
    let sensitive: Option<SensitiveAuthParams> =
        match uptrakit_service_sdk::decrypt_sensitive_params(
            args.sensitive_params_sealed,
            args.private_key_der,
        ) {
            Ok(s) => s,
            Err(msg) => return make_error_response(request_id, &msg),
        };

    let params = args.params;

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
        service_id: args.service_id,
        tenant_id: args.tenant_id,
        remove_stale_keys,
    };

    match bootstrap::run_bootstrap(args.state_dir, bootstrap_params).await {
        Ok(result) => {
            tracing::info!(
                %host_id,
                is_pve_node = result.is_pve_node,
                "bootstrap completed successfully"
            );

            if let Some(creds) = &result.pve_credentials
                && let Some(bg_tx) = bg_tx
            {
                // New PVE cluster: send ReportPluginConfig to controller.
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
            } else if let Some(config_id) = &result.existing_pve_plugin_config_id {
                // Existing PVE cluster: reuse the plugin config from the
                // already-bootstrapped node. Update the host's PVE state
                // directly without sending ReportPluginConfig.
                tracing::info!(
                    %host_id,
                    %config_id,
                    "reusing existing PVE plugin config for cluster node"
                );
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

// ── Sensitive params ─────────────────────────────────────────────────

/// Sensitive authentication parameters extracted from the ECIES sealed box.
///
/// Used by both bootstrap and sync actions — any action that accepts SSH
/// credentials from the UI.
#[derive(Debug, Deserialize)]
struct SensitiveAuthParams {
    auth_password: Option<String>,
    auth_private_key: Option<String>,
}

/// Find the local PVE host matching a Proxmox node name and plugin config ID.
///
/// Matches exclusively on `(pve_node_name, pve_plugin_config_id)` — both fields
/// must be set on the host and match the given values. This disambiguates hosts
/// with the same short hostname across different Proxmox clusters.
async fn find_pve_host_for_node(
    state_dir: &Path,
    proxmox_node: &str,
    plugin_config_id: &str,
) -> Result<String, String> {
    let db = crate::db::init_db(state_dir)
        .await
        .map_err(|e| format!("failed to initialize local database: {e}"))?;

    let pve_hosts = host_ops::find_pve_hosts(&db)
        .await
        .map_err(|e| format!("failed to list PVE hosts: {e}"))?;

    if pve_hosts.is_empty() {
        return Err("no PVE hosts found; bootstrap a PVE node first".to_string());
    }

    let matched = pve_hosts.iter().find(|h| {
        h.pve_node_name.as_deref() == Some(proxmox_node)
            && h.pve_plugin_config_id.as_deref() == Some(plugin_config_id)
    });

    match matched {
        Some(host) => Ok(host.id.clone()),
        None => {
            let available: Vec<String> = pve_hosts
                .iter()
                .map(|h| {
                    format!(
                        "{} (node={}, config={})",
                        h.name,
                        h.pve_node_name.as_deref().unwrap_or("?"),
                        h.pve_plugin_config_id.as_deref().unwrap_or("?"),
                    )
                })
                .collect();
            Err(format!(
                "no PVE host found for node '{proxmox_node}' with plugin config '{plugin_config_id}'; \
                 run 'host sync' on PVE hosts to populate node names. \
                 Available PVE hosts: [{}]",
                available.join(", ")
            ))
        }
    }
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
