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
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use uptrakit_internal_wire::extension::{
    ActionDef, ActionUi, ExtensionManifest, ExtensionPlacement, ExtensionRegisterPayload,
    ExtensionRequestPayload, ExtensionResponsePayload, ExtensionTargeting, ExtensionUi, FieldDef,
    FieldType, FormDef, SelectOption, TableColumn, WizardStep,
};
use uptrakit_internal_wire::{ServiceMessage, ServiceTransport};
use uptrakit_plugin_infrastructure_core::PluginBase;
use uptrakit_plugin_infrastructure_core::agent_infra::{InfraActionInvoker, InfraPluginContext};
use uptrakit_shared_types::{Permission, SecretString};

use crate::host_ops;
use crate::operations::bootstrap::{self, BootstrapParams};
use crate::operations::bootstrap_proxmox::AgentGuestBootstrapExecutor;
use crate::operations::sync;
use crate::ssh_target::SshTarget;

/// Extension ID for the SSH host management extension.
pub const EXTENSION_ID: &str = "ssh-agent.hosts";

// ── Manifest ─────────────────────────────────────────────────────────

/// Build the extension manifest for SSH host management.
///
/// Action fields (`row_actions`, `primary_actions`) contain action ID strings
/// that reference entries in the action library returned by [`build_actions`].
/// `infra_primary_actions` are additional primary action IDs contributed by
/// infrastructure plugins (via [`PluginBase::primary_action_ids`]).
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
    infra_plugins: &[Arc<dyn PluginBase>],
) -> ExtensionRegisterPayload {
    let infra_primary_actions: Vec<String> = infra_plugins
        .iter()
        .flat_map(|p| p.primary_action_ids())
        .collect();
    let infra_manifests =
        uptrakit_plugin_infrastructure_registry::agent_infra_extension_manifests();

    let mut manifests = vec![build_manifest(&infra_primary_actions)];
    manifests.extend(infra_manifests);

    let payload = ExtensionRegisterPayload::new(manifests);
    match encryption_public_key {
        Some(key) => payload.with_encryption_public_key(key),
        None => payload,
    }
}

/// Build the action library for registration via `ExtensionActionsRegister`.
pub fn build_actions() -> Vec<ActionDef> {
    let mut actions = vec![
        ActionDef::new("remove-host", "Remove Host")
            .with_permission(Permission::UpdateHosts)
            .destructive()
            .with_confirm_entity_field("name")
            .with_timeout(30)
            .batch(),
        sync_host_action(),
        bootstrap_action(),
        // Internal wizard-step actions (not shown in UI directly).
        ActionDef::new("bootstrap-connect", "Bootstrap Connect")
            .with_permission(Permission::UpdateHosts)
            .with_timeout(60),
        ActionDef::new("bootstrap-execute", "Bootstrap Execute")
            .with_permission(Permission::UpdateHosts)
            .with_timeout(120),
        ActionDef::new("sync-connect", "Sync Connect")
            .with_permission(Permission::UpdateHosts)
            .with_timeout(60),
        ActionDef::new("sync-execute", "Sync Execute")
            .with_permission(Permission::UpdateHosts)
            .with_timeout(120),
    ];
    actions.extend(uptrakit_plugin_infrastructure_registry::agent_infra_extension_actions());
    actions
}

/// Build the sync-host action definition as a 3-step wizard.
fn sync_host_action() -> ActionDef {
    let connect_step = WizardStep::new(
        "connect",
        "Connection & Authentication",
        FormDef::new(vec![
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
                .with_type(FieldType::SshPrivateKey)
                .with_placeholder("-----BEGIN OPENSSH PRIVATE KEY-----")
                .with_help_text(
                    "PEM-encoded private key. Required when auth method is 'private_key'.",
                )
                .sensitive()
                .with_visible_when("auth_method", vec!["private_key".to_string()]),
            FieldDef::new("allow_all", "Allow All (NOPASSWD: ALL)")
                .with_type(FieldType::Toggle)
                .with_help_text("Use NOPASSWD: ALL in sudoers (less secure)."),
            FieldDef::new("auto", "Auto")
                .with_type(FieldType::Toggle)
                .with_help_text("Skip review and execute immediately."),
        ]),
    )
    .with_submit_action("sync-connect");

    let review_step = WizardStep::new("review", "Review Plan", FormDef::new(vec![]))
        .with_render_previous_response();

    let execute_step = WizardStep::new("execute", "Execute", FormDef::new(vec![]))
        .with_submit_action("sync-execute");

    ActionDef::new("sync-host", "Sync Host")
        .with_permission(Permission::UpdateHosts)
        .with_timeout(120)
        .with_ui(ActionUi::Wizard {
            steps: vec![connect_step, review_step, execute_step],
        })
        .batch()
}

/// Build the bootstrap host action definition as a 3-step wizard.
fn bootstrap_action() -> ActionDef {
    let connect_step = WizardStep::new(
        "connect",
        "Connection & Authentication",
        FormDef::new(vec![
            FieldDef::new("target", "SSH Target")
                .required()
                .with_placeholder("[user@]host[:port]")
                .with_help_text(
                    "SSH target in [user@]host[:port] format. Default user: root, port: 22.",
                ),
            FieldDef::new("name", "Host Name")
                .with_placeholder("my-server")
                .with_help_text("Optional. Defaults to the hostname from the SSH target."),
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
                .with_type(FieldType::SshPrivateKey)
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
            FieldDef::new("auto", "Auto")
                .with_type(FieldType::Toggle)
                .with_help_text("Skip review and execute immediately."),
        ]),
    )
    .with_submit_action("bootstrap-connect");

    let review_step = WizardStep::new("review", "Review Plan", FormDef::new(vec![]))
        .with_render_previous_response();

    let execute_step = WizardStep::new("execute", "Execute", FormDef::new(vec![]))
        .with_submit_action("bootstrap-execute");

    ActionDef::new("bootstrap", "Bootstrap Host")
        .with_permission(Permission::UpdateHosts)
        .with_timeout(120)
        .with_ui(ActionUi::Wizard {
            steps: vec![connect_step, review_step, execute_step],
        })
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
    pub infra_plugins: Arc<Vec<Arc<dyn PluginBase>>>,
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
    let infra_plugins = std::sync::Arc::clone(&ctx.infra_plugins);
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
        for plugin in infra_plugins.iter() {
            if let Some(resp) = plugin
                .handle_service_extension_action(&plugin_ctx, &request)
                .await
            {
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
    conn: &mut impl ServiceTransport,
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
        "bootstrap-connect" => {
            spawn_bootstrap_connect(request, ctx);
        }
        "bootstrap-execute" => {
            spawn_bootstrap_execute(request, ctx);
        }
        "sync-connect" => {
            spawn_sync_connect(request, ctx);
        }
        "sync-execute" => {
            spawn_sync_execute(request, ctx);
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

/// Spawn the bootstrap-connect (plan) step as a background task.
fn spawn_bootstrap_connect(request: ExtensionRequestPayload, ctx: &ExtensionContext<'_>) {
    let state_dir = ctx.state_dir.to_path_buf();
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());
    let bg_tx = ctx.bg_tx.clone();
    let service_id = ctx.service_id;
    let tenant_id = ctx.tenant_id;
    let request_id = request.request_id.clone();

    tokio::spawn(async move {
        let response = run_bootstrap_connect(
            &request_id,
            &request.params,
            request.sensitive_params.as_ref().map(|s| s.expose_secret()),
            private_key_der.as_deref(),
            service_id,
            tenant_id,
            &state_dir,
        )
        .await;
        let msg = ServiceMessage::ExtensionResponse(response);
        if bg_tx.send(msg).await.is_err() {
            tracing::error!("failed to send bootstrap-connect result via bg_tx");
        }
    });
}

/// Spawn the bootstrap-execute step as a background task.
fn spawn_bootstrap_execute(request: ExtensionRequestPayload, ctx: &ExtensionContext<'_>) {
    let state_dir = ctx.state_dir.to_path_buf();
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());
    let bg_tx = ctx.bg_tx.clone();
    let service_id = ctx.service_id;
    let tenant_id = ctx.tenant_id;
    let request_id = request.request_id.clone();

    tokio::spawn(async move {
        let response = run_bootstrap_execute(BootstrapExecuteArgs {
            request_id: &request_id,
            params: &request.params,
            sensitive_params_sealed: request.sensitive_params.as_ref().map(|s| s.expose_secret()),
            private_key_der: private_key_der.as_deref(),
            service_id,
            tenant_id,
            state_dir: &state_dir,
            bg_tx: &bg_tx,
        })
        .await;
        let msg = ServiceMessage::ExtensionResponse(response);
        if bg_tx.send(msg).await.is_err() {
            tracing::error!("failed to send bootstrap-execute result via bg_tx");
        }
    });
}

/// Spawn the sync-connect (plan) step as a background task.
fn spawn_sync_connect(request: ExtensionRequestPayload, ctx: &ExtensionContext<'_>) {
    let db_state_dir = ctx.state_dir.to_path_buf();
    let bg_tx = ctx.bg_tx.clone();
    let tenant_id = ctx.tenant_id;
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());
    let request_id = request.request_id.clone();

    tokio::spawn(async move {
        let Some((host_id, auth_override)) =
            resolve_sync_auth(&request, &request_id, private_key_der.as_deref(), &bg_tx).await
        else {
            return;
        };

        let allow_all = param_bool(&request.params, "allow_all");

        let db = match crate::db::init_db(&db_state_dir).await {
            Ok(db) => db,
            Err(e) => {
                let resp = make_error_response(
                    &request_id,
                    &format!("failed to initialize database: {e}"),
                );
                let _ = bg_tx.send(ServiceMessage::ExtensionResponse(resp)).await;
                return;
            }
        };

        let response =
            match sync::sync_connect(&host_id, &db, tenant_id, auth_override.as_ref(), allow_all)
                .await
            {
                Ok(plan) => match serde_json::to_value(&plan) {
                    Ok(data) => make_success_response(&request_id, data),
                    Err(e) => {
                        make_error_response(&request_id, &format!("failed to serialize plan: {e}"))
                    }
                },
                Err(e) => make_error_response(&request_id, &e),
            };
        let _ = bg_tx
            .send(ServiceMessage::ExtensionResponse(response))
            .await;
    });
}

/// Spawn the sync-execute step as a background task.
fn spawn_sync_execute(request: ExtensionRequestPayload, ctx: &ExtensionContext<'_>) {
    let db_state_dir = ctx.state_dir.to_path_buf();
    let bg_tx = ctx.bg_tx.clone();
    let tenant_id = ctx.tenant_id;
    let private_key_der = ctx.private_key_der.map(|k| k.to_vec());
    let request_id = request.request_id.clone();

    tokio::spawn(async move {
        let Some((host_id, auth_override)) =
            resolve_sync_auth(&request, &request_id, private_key_der.as_deref(), &bg_tx).await
        else {
            return;
        };

        let allow_all = param_bool(&request.params, "allow_all");
        let skip_actions = parse_skip_actions(&request.params);

        let db = match crate::db::init_db(&db_state_dir).await {
            Ok(db) => db,
            Err(e) => {
                let resp = make_error_response(
                    &request_id,
                    &format!("failed to initialize database: {e}"),
                );
                let _ = bg_tx.send(ServiceMessage::ExtensionResponse(resp)).await;
                return;
            }
        };

        let response = match sync::sync_execute(
            &host_id,
            &db,
            tenant_id,
            auth_override.as_ref(),
            allow_all,
            &skip_actions,
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
            tracing::error!("failed to send sync-execute result via bg_tx");
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

/// Parse `BootstrapParams` from extension request params and decrypted sensitive params.
fn parse_bootstrap_params(
    params: &serde_json::Value,
    sensitive: Option<&SensitiveAuthParams>,
    service_id: Option<uuid::Uuid>,
    tenant_id: Option<uuid::Uuid>,
) -> Result<BootstrapParams, String> {
    let target_str = params
        .get("target")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing required field 'target'".to_string())?;

    let parsed_target: SshTarget = target_str
        .parse()
        .map_err(|e| format!("invalid target: {e}"))?;

    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| parsed_target.hostname.clone());

    let auth_method = params
        .get("auth_method")
        .and_then(|v| v.as_str())
        .unwrap_or("password");

    let auth_password = sensitive.and_then(|s| s.auth_password.clone().map(SecretString::new));
    let auth_private_key =
        sensitive.and_then(|s| s.auth_private_key.clone().map(SecretString::new));

    match auth_method {
        "password" if auth_password.is_none() => {
            return Err("auth_method is 'password' but no password provided".to_string());
        }
        "private_key" if auth_private_key.is_none() => {
            return Err("auth_method is 'private_key' but no private key provided".to_string());
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

    Ok(BootstrapParams {
        name,
        hostname: parsed_target.hostname,
        port: parsed_target.port.unwrap_or(22) as i32,
        auth_username: parsed_target.username.unwrap_or_else(|| "root".to_string()),
        auth_password,
        auth_private_key_pem: auth_private_key,
        use_ssh_agent: false,
        target_username,
        target_private_key_pem: None,
        host_key_fingerprint,
        strict_host_key_checking,
        allow_all,
        host_id,
        service_id,
        tenant_id,
        remove_stale_keys,
    })
}

/// The bootstrap-connect handler: probe the host and return a plan.
#[tracing::instrument(skip_all, fields(request_id = %request_id))]
async fn run_bootstrap_connect(
    request_id: &str,
    params: &serde_json::Value,
    sensitive_params_sealed: Option<&str>,
    private_key_der: Option<&[u8]>,
    service_id: Option<uuid::Uuid>,
    tenant_id: Option<uuid::Uuid>,
    state_dir: &Path,
) -> ExtensionResponsePayload {
    let sensitive: Option<SensitiveAuthParams> =
        match uptrakit_service_sdk::decrypt_sensitive_params(
            sensitive_params_sealed,
            private_key_der,
        ) {
            Ok(s) => s,
            Err(msg) => return make_error_response(request_id, &msg),
        };

    let bootstrap_params =
        match parse_bootstrap_params(params, sensitive.as_ref(), service_id, tenant_id) {
            Ok(p) => p,
            Err(msg) => return make_error_response(request_id, &msg),
        };

    match bootstrap::bootstrap_connect(state_dir, &bootstrap_params).await {
        Ok(plan) => match serde_json::to_value(&plan) {
            Ok(data) => make_success_response(request_id, data),
            Err(e) => make_error_response(request_id, &format!("failed to serialize plan: {e}")),
        },
        Err(e) => {
            tracing::error!(error = %e, "bootstrap-connect failed");
            make_error_response(request_id, &format!("bootstrap connect failed: {e}"))
        }
    }
}

/// Arguments for the bootstrap-execute handler, bundled to stay within the 7-arg clippy limit.
struct BootstrapExecuteArgs<'a> {
    request_id: &'a str,
    params: &'a serde_json::Value,
    sensitive_params_sealed: Option<&'a str>,
    private_key_der: Option<&'a [u8]>,
    service_id: Option<uuid::Uuid>,
    tenant_id: Option<uuid::Uuid>,
    state_dir: &'a Path,
    bg_tx: &'a tokio::sync::mpsc::Sender<ServiceMessage>,
}

/// The bootstrap-execute handler: execute the bootstrap with optional skip set.
#[tracing::instrument(skip_all, fields(request_id = %args.request_id))]
async fn run_bootstrap_execute(args: BootstrapExecuteArgs<'_>) -> ExtensionResponsePayload {
    let request_id = args.request_id;
    let bg_tx = args.bg_tx;
    let sensitive: Option<SensitiveAuthParams> =
        match uptrakit_service_sdk::decrypt_sensitive_params(
            args.sensitive_params_sealed,
            args.private_key_der,
        ) {
            Ok(s) => s,
            Err(msg) => return make_error_response(request_id, &msg),
        };

    let bootstrap_params = match parse_bootstrap_params(
        args.params,
        sensitive.as_ref(),
        args.service_id,
        args.tenant_id,
    ) {
        Ok(p) => p,
        Err(msg) => return make_error_response(request_id, &msg),
    };

    let host_id = bootstrap_params.host_id;
    let skip_actions = parse_skip_actions(args.params);

    match bootstrap::bootstrap_execute(args.state_dir, bootstrap_params, &skip_actions).await {
        Ok(result) => {
            tracing::info!(%host_id, "bootstrap completed successfully");

            // For each infra plugin that detected infrastructure, send
            // ReportPluginConfig if new credentials were created.
            send_infra_plugin_reports(bg_tx, host_id, &result.infra_results).await;

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

/// Build a `SyncAuthOverride` from extension params and decrypted sensitive params.
fn build_sync_auth_override(
    params: &serde_json::Value,
    sensitive: Option<&SensitiveAuthParams>,
) -> Result<Option<sync::SyncAuthOverride>, String> {
    let auth_method = params
        .get("auth_method")
        .and_then(|v| v.as_str())
        .unwrap_or("stored");

    match auth_method {
        "stored" => Ok(None),
        "password" => {
            let password = sensitive.and_then(|s| s.auth_password.as_deref());
            match password {
                Some(pw) => Ok(Some(sync::SyncAuthOverride {
                    username: params
                        .get("username")
                        .and_then(|v| v.as_str())
                        .unwrap_or("root")
                        .to_string(),
                    auth_password: Some(pw.to_string()),
                    auth_private_key_pem: None,
                })),
                None => Err("auth_method is 'password' but no password provided".to_string()),
            }
        }
        "private_key" => {
            let key = sensitive.and_then(|s| s.auth_private_key.as_deref());
            match key {
                Some(pem) => Ok(Some(sync::SyncAuthOverride {
                    username: params
                        .get("username")
                        .and_then(|v| v.as_str())
                        .unwrap_or("root")
                        .to_string(),
                    auth_password: None,
                    auth_private_key_pem: Some(pem.to_string()),
                })),
                None => Err("auth_method is 'private_key' but no private key provided".to_string()),
            }
        }
        other => Err(format!("unknown auth_method '{other}'")),
    }
}

/// Parse `skip_actions` from params as a `HashSet<String>`.
///
/// Expects a JSON array of strings at `params["skip_actions"]`.
/// Returns an empty set if the key is absent or not an array.
fn parse_skip_actions(params: &serde_json::Value) -> HashSet<String> {
    params
        .get("skip_actions")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
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

// ── Shared helpers ───────────────────────────────────────────────────

/// Send a `ReportPluginConfig` message for each infra result that produced one.
///
/// Iterates `infra_results` and, for any result that carries a
/// `report_plugin_config`, constructs the wire payload and sends it via
/// `bg_tx`.  Results that refer to an existing config are logged at `info`
/// level instead.  Send failures are logged at `error` level.
async fn send_infra_plugin_reports(
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
    host_id: uuid::Uuid,
    infra_results: &[uptrakit_plugin_infrastructure_core::agent_infra::BootstrapInfraResult],
) {
    for infra in infra_results {
        if let Some(report) = &infra.report_plugin_config {
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
        } else if let Some(config_id) = &infra.existing_plugin_config_id {
            tracing::info!(
                %host_id,
                %config_id,
                "reusing existing plugin config for cluster node"
            );
        }
    }
}

/// Resolve `host_id`, decrypt sensitive params, and build the auth override.
///
/// This is the common setup for both `spawn_sync_connect` and
/// `spawn_sync_execute`.  On any failure, an `ExtensionResponse` error is sent
/// via `bg_tx` and `None` is returned so the caller can bail early.
async fn resolve_sync_auth(
    request: &ExtensionRequestPayload,
    request_id: &str,
    private_key_der: Option<&[u8]>,
    bg_tx: &tokio::sync::mpsc::Sender<ServiceMessage>,
) -> Option<(String, Option<sync::SyncAuthOverride>)> {
    let host_id = match request.params.get("id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            let resp = make_error_response(request_id, "missing required field 'id'");
            let _ = bg_tx.send(ServiceMessage::ExtensionResponse(resp)).await;
            return None;
        }
    };

    let sensitive: Option<SensitiveAuthParams> =
        match uptrakit_service_sdk::decrypt_sensitive_params(
            request.sensitive_params.as_ref().map(|s| s.expose_secret()),
            private_key_der,
        ) {
            Ok(s) => s,
            Err(msg) => {
                let resp = make_error_response(request_id, &msg);
                let _ = bg_tx.send(ServiceMessage::ExtensionResponse(resp)).await;
                return None;
            }
        };

    let auth_override = match build_sync_auth_override(&request.params, sensitive.as_ref()) {
        Ok(ov) => ov,
        Err(msg) => {
            let resp = make_error_response(request_id, &msg);
            let _ = bg_tx.send(ServiceMessage::ExtensionResponse(resp)).await;
            return None;
        }
    };

    Some((host_id, auth_override))
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

async fn send_response(conn: &mut impl ServiceTransport, response: ExtensionResponsePayload) {
    if let Err(e) = conn
        .transport_send(ServiceMessage::ExtensionResponse(response))
        .await
    {
        tracing::error!(error = %e, "failed to send extension response");
    }
}
