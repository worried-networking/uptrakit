//! SSH agent UI extension: host management via the extensions framework.
//!
//! Provides:
//! - Extension manifest describing the `ssh-agent.hosts` data table
//! - Action library with `list-hosts`, `bootstrap`, `sync-host`, `remove-host` definitions
//! - Action handlers for each action
//! - ECIES decryption of sensitive parameters (auth password, private key)

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;

use uptrakit_internal_wire::ServiceMessage;
use uptrakit_internal_wire::extension::{
    ActionDef, ActionUi, ExtensionManifest, ExtensionPlacement, ExtensionRegisterPayload,
    ExtensionRequestPayload, ExtensionResponsePayload, ExtensionTargeting, ExtensionUi, FieldDef,
    FieldType, FormDef, SelectOption, TableColumn,
};
use uptrakit_plugin_infrastructure_core::agent_infra::{InfraActionInvoker, InfraPluginContext};
use uptrakit_plugin_infrastructure_registry::AgentInfraRegistry;
use uptrakit_service_sdk::ControllerConnection;
use uptrakit_shared_types::Permission;

use crate::commands::bootstrap::{self, BootstrapParams};
use crate::commands::bootstrap_proxmox::AgentGuestBootstrapExecutor;
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
/// `infra_primary_actions` are additional primary action IDs contributed by
/// infrastructure plugins (via [`AgentInfraRegistry::all_primary_action_ids`]).
pub fn build_manifest(infra_primary_actions: &[String]) -> ExtensionManifest {
    let mut primary_actions = vec!["bootstrap".to_string()];
    primary_actions.extend(infra_primary_actions.iter().cloned());

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
            primary_actions,
            context_selector: None,
            default_per_page: Some(50),
        },
    )
    .with_permission(Permission::UpdateHosts)
    .with_targeting(ExtensionTargeting::Targeted)
}

/// Build the register payload including the manifest and encryption key.
pub fn build_register_payload(
    encryption_public_key: Option<String>,
    infra_registry: &AgentInfraRegistry,
) -> ExtensionRegisterPayload {
    let infra_primary_actions = infra_registry.all_primary_action_ids();
    let infra_manifests = infra_registry.all_extension_manifests();

    let mut manifests = vec![build_manifest(&infra_primary_actions)];
    manifests.extend(infra_manifests);

    let payload = ExtensionRegisterPayload::new(manifests);
    match encryption_public_key {
        Some(key) => payload.with_encryption_public_key(key),
        None => payload,
    }
}

/// Build the action library for registration via `ExtensionActionsRegister`.
pub fn build_actions(infra_registry: &AgentInfraRegistry) -> Vec<ActionDef> {
    let mut actions = vec![
        ActionDef::new("remove-host", "Remove Host")
            .with_permission(Permission::UpdateHosts)
            .destructive()
            .with_confirm_entity_field("name")
            .with_timeout(30)
            .batch(),
        sync_host_action(),
        bootstrap_action(),
    ];
    actions.extend(infra_registry.all_extension_actions());
    actions
}

/// Build the sync-host action definition with optional auth override form.
fn sync_host_action() -> ActionDef {
    ActionDef::new("sync-host", "Sync Host")
        .with_permission(Permission::UpdateHosts)
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
        .batch()
}

/// Build the bootstrap host action definition with its form UI.
fn bootstrap_action() -> ActionDef {
    ActionDef::new("bootstrap", "Bootstrap Host")
        .with_permission(Permission::UpdateHosts)
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
    pub extension_proxy: &'a Arc<uptrakit_service_sdk::ServiceExtensionProxy>,
    pub infra_registry: Arc<AgentInfraRegistry>,
}

// ── InfraActionInvoker implementation ────────────────────────────────

/// [`InfraActionInvoker`] that routes calls through the `ServiceExtensionProxy`.
///
/// Wraps `invoke_proxy_action` so that infrastructure plugins can invoke
/// controller-side extension actions without depending on `uptrakit-service-sdk`.
pub struct InfraActionInvokerImpl<'a> {
    proxy: &'a uptrakit_service_sdk::ServiceExtensionProxy,
    bg_tx: &'a tokio::sync::mpsc::Sender<ServiceMessage>,
}

impl<'a> InfraActionInvokerImpl<'a> {
    pub fn new(
        proxy: &'a uptrakit_service_sdk::ServiceExtensionProxy,
        bg_tx: &'a tokio::sync::mpsc::Sender<ServiceMessage>,
    ) -> Self {
        Self { proxy, bg_tx }
    }
}

#[async_trait]
impl InfraActionInvoker for InfraActionInvokerImpl<'_> {
    async fn invoke(
        &self,
        extension_id: &str,
        action_id: &str,
        params: serde_json::Value,
    ) -> std::result::Result<uptrakit_internal_wire::extension::ExtensionResponsePayload, String>
    {
        invoke_proxy_action(self.proxy, self.bg_tx, extension_id, action_id, params)
            .await
            .map_err(|e| e.to_string())
    }
}

// ── Infra plugin action dispatch ─────────────────────────────────────

/// Spawn an infrastructure plugin action as a background task.
///
/// Iterates all registered infra plugins; the first one to return `Some`
/// wins. If no plugin handles the action, an error response is sent.
fn spawn_infra_plugin_action(request: ExtensionRequestPayload, ctx: &ExtensionContext<'_>) {
    let state_dir = ctx.state_dir.to_path_buf();
    let bg_tx = ctx.bg_tx.clone();
    let proxy = std::sync::Arc::clone(ctx.extension_proxy);
    let infra_registry = std::sync::Arc::clone(&ctx.infra_registry);
    let service_id = ctx.service_id;
    let tenant_id = ctx.tenant_id;
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());

    tokio::spawn(async move {
        let db = match crate::db::init_db(&state_dir).await {
            Ok(db) => db,
            Err(e) => {
                let resp = make_error_response(
                    &request.request_id,
                    &format!("failed to initialize database: {e}"),
                );
                let _ = bg_tx.send(ServiceMessage::ExtensionResponse(resp)).await;
                return;
            }
        };

        let tenant_id_str = tenant_id.map(|t| t.to_string());
        let action_invoker = InfraActionInvokerImpl::new(&proxy, &bg_tx);
        let guest_bootstrap = AgentGuestBootstrapExecutor {
            state_dir: state_dir.clone(),
            service_id,
        };
        let plugin_ctx = InfraPluginContext {
            db: &db,
            tenant_id: tenant_id_str.as_deref(),
            service_id,
            state_dir: &state_dir,
            private_key_der: private_key_der.as_deref(),
            action_invoker: &action_invoker,
            guest_bootstrap: &guest_bootstrap,
        };

        let mut response: Option<ExtensionResponsePayload> = None;
        for plugin in infra_registry.plugins() {
            if let Some(resp) = plugin.handle_extension_action(&plugin_ctx, &request).await {
                response = Some(resp);
                break;
            }
        }

        let resp = response.unwrap_or_else(|| {
            tracing::warn!(
                action_id = %request.action_id,
                extension_id = %request.extension_id,
                "no infrastructure plugin handled this action"
            );
            make_error_response(&request.request_id, "unknown action")
        });

        if bg_tx
            .send(ServiceMessage::ExtensionResponse(resp))
            .await
            .is_err()
        {
            tracing::error!("failed to send infra plugin action result via bg_tx");
        }
    });
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
            let response = handle_list_hosts(&request.request_id, &request.params, ctx.db).await;
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
        _ => {
            // Delegate to infrastructure plugins.
            spawn_infra_plugin_action(request, ctx);
        }
    }
}

// ── Action handlers ──────────────────────────────────────────────────

/// List SSH hosts from the local database with pagination.
async fn handle_list_hosts(
    request_id: &str,
    params: &serde_json::Value,
    db: &sea_orm::DatabaseConnection,
) -> ExtensionResponsePayload {
    let page = params
        .get("page")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .max(1);
    let per_page = params
        .get("per_page")
        .and_then(|v| v.as_u64())
        .unwrap_or(50)
        .clamp(1, 1000);

    match host_ops::list_hosts_paginated(db, page, per_page).await {
        Ok(result) => {
            let items: Vec<serde_json::Value> = result
                .items
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
            make_success_response(
                request_id,
                json!({
                    "items": items,
                    "total": result.total,
                    "page": result.page,
                    "per_page": result.per_page,
                    "total_pages": result.total_pages,
                }),
            )
        }
        Err(e) => {
            tracing::error!(error = %e, "failed to list hosts");
            make_error_response(request_id, "failed to list hosts")
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

/// Invoke an extension action on the controller via the proxy.
///
/// Sends the request via `bg_tx` (which flows through the event loop to
/// `conn.send()`), then waits for the controller's response via the proxy's
/// oneshot channel.
pub(crate) async fn invoke_proxy_action(
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
            tracing::info!(%host_id, "bootstrap completed successfully");

            // For each infra plugin that detected infrastructure, send
            // ReportPluginConfig if new credentials were created.
            for infra in &result.infra_results {
                if let Some(report) = &infra.report_plugin_config {
                    if let Some(bg_tx) = bg_tx {
                        let payload: uptrakit_internal_wire::ReportPluginConfigPayload =
                            serde_json::from_value(json!({
                                "request_id": uuid::Uuid::now_v7().to_string(),
                                "plugin_type": report.plugin_type,
                                "name": report.name,
                                "config": report.config,
                            }))
                            .expect("ReportPluginConfigPayload JSON is always valid");
                        let msg = ServiceMessage::ReportPluginConfig(payload);
                        if bg_tx.send(msg).await.is_err() {
                            tracing::error!("failed to send ReportPluginConfig via bg_tx");
                        }
                    }
                } else if let Some(config_id) = &infra.existing_plugin_config_id {
                    tracing::info!(
                        %host_id,
                        %config_id,
                        "reusing existing plugin config for cluster node"
                    );
                }
            }

            let any_infra = result.infra_results.iter().any(|r| r.detected);
            let mut data = json!({ "host_id": host_id.to_string() });
            if any_infra {
                data["has_infrastructure"] = json!(true);
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
