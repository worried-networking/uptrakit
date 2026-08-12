//! Version-check handlers and precondition/agent-service helpers.

use std::sync::Arc;

use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, RelationTrait as _};
use uptrakit_shared_db::entity::{
    host, host_software_item_plugin, prelude::*, service, service_host,
};
use uptrakit_shared_types::PluginTypeId;
use uptrakit_web_api_types::software_items::TriggerVersionCheckResponse;
use uuid::Uuid;

use crate::AppState;
use crate::error_response::error_response;
use crate::middleware::action::CanTriggerChecks;
use crate::middleware::require_auth::AuthenticatedApiTokenId;
use crate::queries::plugin_configs::find_raw_active_config;
use crate::queries::software_items as item_queries;
use crate::tenant_db::TenantDb;

use super::audit::{AuditContext, emit_software_version_check_audit};
use super::controller_fetch::{
    ControllerFetchJob, is_controller_fetch_site, run_controller_fetch_jobs,
};
use super::version_check_dispatch::{
    collect_and_run_controller_fetches, dispatch_agent_version_checks, load_version_check_context,
};

fn version_check_dispatch_mode(agents_notified: u32, controller_checks_run: u32) -> &'static str {
    match (agents_notified > 0, controller_checks_run > 0) {
        (true, true) => "hybrid",
        (true, false) => "agent_only",
        (false, true) => "controller_only",
        (false, false) => "none",
    }
}

fn classify_version_check_context_load_failure(
    status: StatusCode,
) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
    match status {
        StatusCode::NOT_FOUND => (
            uptrakit_audit_log::AuditOutcome::Denied,
            "version_check.no_hosts_assigned",
        ),
        _ => (
            uptrakit_audit_log::AuditOutcome::Failed,
            "version_check.internal_error",
        ),
    }
}

// ---------------------------------------------------------------------------
// check_versions_host helpers
// ---------------------------------------------------------------------------

/// Shared contract for per-phase precondition errors in version-check handlers.
///
/// Each implementing type retains its own distinct HTTP status mapping and audit
/// reason codes — this trait only captures the method *shape*.
trait VersionCheckError {
    fn into_response(self) -> Response;
    fn audit(&self) -> (uptrakit_audit_log::AuditOutcome, &'static str);
}

/// Verify that a software item exists, a host exists and belongs to the tenant,
/// and the host is assigned to the software item.
///
/// Returns `(item, host_record, link)` on success, or an HTTP error response on
/// any failure.
#[derive(Debug)]
enum CheckVersionsHostPreconditionError {
    SoftwareItemNotFound,
    HostNotFound,
    HostNotAssigned,
    Internal,
}

impl VersionCheckError for CheckVersionsHostPreconditionError {
    fn into_response(self) -> Response {
        match self {
            Self::SoftwareItemNotFound => {
                error_response(StatusCode::NOT_FOUND, "Software item not found")
            }
            Self::HostNotFound => error_response(StatusCode::NOT_FOUND, "Host not found"),
            Self::HostNotAssigned => error_response(
                StatusCode::BAD_REQUEST,
                "Host is not assigned to this software item",
            ),
            Self::Internal => {
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
        }
    }

    fn audit(&self) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
        match self {
            Self::SoftwareItemNotFound => (
                uptrakit_audit_log::AuditOutcome::Denied,
                "version_check.software_item_not_found",
            ),
            Self::HostNotFound => (
                uptrakit_audit_log::AuditOutcome::Denied,
                "version_check.host_not_found",
            ),
            Self::HostNotAssigned => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "version_check.host_not_assigned",
            ),
            Self::Internal => (
                uptrakit_audit_log::AuditOutcome::Failed,
                "version_check.internal_error",
            ),
        }
    }
}

async fn verify_software_item_and_host(
    tenant_db: &TenantDb,
    item_id: Uuid,
    host_id: Uuid,
) -> Result<
    (
        uptrakit_shared_db::entity::software_item::Model,
        uptrakit_shared_db::entity::host::Model,
        uptrakit_shared_db::entity::host_software_item::Model,
    ),
    CheckVersionsHostPreconditionError,
> {
    let item = match item_queries::find_active_item(tenant_db.db(), tenant_db.tenant_id(), item_id)
        .await
    {
        Some(i) => i,
        None => return Err(CheckVersionsHostPreconditionError::SoftwareItemNotFound),
    };

    let host_record = match Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_db.tenant_id()))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(tenant_db.db())
        .await
    {
        Ok(Some(h)) => h,
        Ok(None) => return Err(CheckVersionsHostPreconditionError::HostNotFound),
        Err(e) => {
            tracing::error!("Failed to lookup host: {e}");
            return Err(CheckVersionsHostPreconditionError::Internal);
        }
    };

    let link = match item_queries::load_host_assignment(tenant_db.db(), host_id, item_id).await {
        Some(l) => l,
        None => return Err(CheckVersionsHostPreconditionError::HostNotAssigned),
    };

    Ok((item, host_record, link))
}

/// Load the approved agent service record for a given host.
///
/// Queries `service_host` (tenant-scoped via join on `service`) and then verifies
/// the linked service is active and approved.
///
/// Returns the `service::Model` on success, or an HTTP error response on any
/// failure.
#[derive(Debug)]
pub(super) enum LoadAgentServiceError {
    NoAgentLinked,
    AgentNotApproved,
    AgentNotFound,
    Internal,
}

impl VersionCheckError for LoadAgentServiceError {
    fn into_response(self) -> Response {
        match self {
            Self::NoAgentLinked => {
                error_response(StatusCode::NOT_FOUND, "No agent linked to this host")
            }
            Self::AgentNotApproved => {
                error_response(StatusCode::BAD_REQUEST, "Agent is not approved")
            }
            Self::AgentNotFound => {
                error_response(StatusCode::NOT_FOUND, "Agent not found or deactivated")
            }
            Self::Internal => {
                error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
            }
        }
    }

    fn audit(&self) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
        match self {
            Self::NoAgentLinked => (
                uptrakit_audit_log::AuditOutcome::Denied,
                "version_check.no_agent_linked",
            ),
            Self::AgentNotApproved => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "version_check.agent_not_approved",
            ),
            Self::AgentNotFound => (
                uptrakit_audit_log::AuditOutcome::Denied,
                "version_check.agent_not_found",
            ),
            Self::Internal => (
                uptrakit_audit_log::AuditOutcome::Failed,
                "version_check.internal_error",
            ),
        }
    }
}

pub(super) async fn load_agent_service(
    tenant_db: &TenantDb,
    host_id: Uuid,
) -> Result<uptrakit_shared_db::entity::service::Model, LoadAgentServiceError> {
    let agent_links = match tenant_db
        .find_via_tenant_join::<service_host::Entity, service::Entity>(
            service_host::Relation::Service.def(),
        )
        .filter(service_host::Column::HostId.eq(host_id))
        .all(tenant_db.db())
        .await
    {
        Ok(links) if links.is_empty() => return Err(LoadAgentServiceError::NoAgentLinked),
        Ok(links) => links,
        Err(e) => {
            tracing::error!("Failed to find agent for host: {e}");
            return Err(LoadAgentServiceError::Internal);
        }
    };

    let service_ids: Vec<Uuid> = agent_links
        .into_iter()
        .map(|link| link.service_id)
        .collect();

    let agents = match Service::find()
        .filter(service::Column::Id.is_in(service_ids))
        .filter(service::Column::TenantId.eq(tenant_db.tenant_id()))
        .filter(service::Column::DeactivatedAt.is_null())
        .all(tenant_db.db())
        .await
    {
        Ok(agents) => agents,
        Err(e) => {
            tracing::error!("Failed to lookup agent: {e}");
            return Err(LoadAgentServiceError::Internal);
        }
    };

    let agent = agents
        .iter()
        .filter(|svc| svc.status == service::ServiceStatus::Approved)
        .max_by_key(|svc| svc.last_seen_at.unwrap_or(svc.updated_at))
        .cloned()
        .or_else(|| {
            agents
                .iter()
                .max_by_key(|svc| svc.last_seen_at.unwrap_or(svc.updated_at))
                .cloned()
        });

    match agent {
        Some(a) if a.status != service::ServiceStatus::Approved => {
            Err(LoadAgentServiceError::AgentNotApproved)
        }
        Some(a) => Ok(a),
        None => Err(LoadAgentServiceError::AgentNotFound),
    }
}

/// Classify plugin rows into controller-side fetch jobs and agent-side
/// `detect_version` / `fetch_releases` assignments.
///
/// Returns `(controller_fetch_jobs, detect_version, fetch_releases)`.
async fn classify_role_assignments(
    tenant_db: &TenantDb,
    plugin_rows: &[uptrakit_shared_db::entity::host_software_item_plugin::Model],
    host_id: Uuid,
    item_id: Uuid,
) -> Result<
    (
        Vec<ControllerFetchJob>,
        Option<uptrakit_wire::PluginAssignment>,
        Option<uptrakit_wire::PluginAssignment>,
    ),
    Response,
> {
    let mut detect_version: Option<uptrakit_wire::PluginAssignment> = None;
    let mut fetch_releases: Option<uptrakit_wire::PluginAssignment> = None;
    let mut controller_fetch_jobs: Vec<ControllerFetchJob> = Vec::new();

    for plugin in plugin_rows {
        let config = match plugin.plugin_config_id {
            Some(pc_id) => match find_raw_active_config(tenant_db, pc_id).await {
                Ok(Some(c)) => Some(c),
                Ok(None) => {
                    tracing::warn!(
                        plugin_config_id = %pc_id,
                        "plugin config not found or deactivated, skipping role assignment"
                    );
                    continue;
                }
                Err(e) => {
                    tracing::error!(
                        plugin_config_id = %pc_id,
                        error = %e,
                        "DB error loading plugin config, skipping role assignment"
                    );
                    continue;
                }
            },
            None => None,
        };
        let plugin_type_str = config
            .as_ref()
            .map(|c| c.plugin_type.clone())
            .unwrap_or_else(|| plugin.plugin_type.clone());
        let plugin_type = PluginTypeId::new(plugin_type_str);
        let merged = uptrakit_config_merge::resolve_effective_config(
            None,
            config.as_ref().map(|c| c.config.as_json()),
            plugin.config.as_ref(),
        );
        let pa = uptrakit_wire::PluginAssignment {
            plugin_type: plugin_type.clone(),
            package_identifier: plugin.package_identifier.clone(),
            config: merged.clone(),
        };
        match plugin.role.as_str() {
            "detect_version" => detect_version = Some(pa),
            "fetch_releases" => {
                if is_controller_fetch_site(&plugin.execution_site, &plugin_type, &merged) {
                    controller_fetch_jobs.push(ControllerFetchJob {
                        plugin_type,
                        package_identifier: plugin.package_identifier.clone(),
                        merged_config: merged,
                        targets: vec![(host_id, item_id)],
                    });
                } else {
                    fetch_releases = Some(pa);
                }
            }
            _ => {}
        }
    }

    Ok((controller_fetch_jobs, detect_version, fetch_releases))
}

// --- Endpoints ---

/// Trigger a version check for a specific software item across all assigned hosts.
///
/// Each host receives a version-check message using its own per-host plugin config
/// and package identifier.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/{id}/check-versions",
    params(("id" = Uuid, Path, description = "Software item UUID")),
    responses(
        (status = 200, description = "Version check triggered", body = TriggerVersionCheckResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Software item not found or no agents")
    ),
    tag = "Software Items",
    security(("oauth2" = ["checks:trigger"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn check_versions(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanTriggerChecks(user): CanTriggerChecks,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path(item_id): Path<Uuid>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &state.audit_emitter,
        tenant_id: tenant_db.tenant_id(),
        user: &user,
        api_token_id,
    };

    // Verify software item exists and is active
    let item = match item_queries::find_active_item(tenant_db.db(), tenant_db.tenant_id(), item_id)
        .await
    {
        Some(i) => i,
        None => {
            emit_software_version_check_audit(
                &audit_ctx,
                item_id,
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                serde_json::json!({
                    "dispatch_scope": "all_hosts",
                    "reason_code": "version_check.software_item_not_found",
                }),
            );
            return error_response(StatusCode::NOT_FOUND, "Software item not found");
        }
    };

    // Phase 1: Load all data needed for version checks.
    let ctx = match load_version_check_context(&tenant_db, item_id).await {
        Ok(ctx) => ctx,
        Err(resp) => {
            let (outcome, reason_code) = classify_version_check_context_load_failure(resp.status());
            emit_software_version_check_audit(
                &audit_ctx,
                item_id,
                Some(&item.name),
                outcome,
                serde_json::json!({
                    "dispatch_scope": "all_hosts",
                    "reason_code": reason_code,
                }),
            );
            return resp;
        }
    };

    // Phase 2: Collect and run controller-side fetch_releases jobs.
    let controller_checks_run = collect_and_run_controller_fetches(&tenant_db, &state, &ctx).await;

    // Phase 3: Send CheckVersions messages to agents for agent-side assignments.
    let agents_notified = dispatch_agent_version_checks(&state, &ctx, item_id, &item.name).await;

    if agents_notified == 0 && controller_checks_run == 0 {
        emit_software_version_check_audit(
            &audit_ctx,
            item_id,
            Some(&item.name),
            uptrakit_audit_log::AuditOutcome::Denied,
            serde_json::json!({
                "agents_notified": agents_notified,
                "controller_checks_run": controller_checks_run,
                "dispatch_mode": version_check_dispatch_mode(agents_notified, controller_checks_run),
                "dispatch_scope": "all_hosts",
                "reason_code": "version_check.no_approved_agents",
            }),
        );
        return error_response(
            StatusCode::NOT_FOUND,
            "No approved agents found for assigned hosts",
        );
    }

    let message = match (agents_notified, controller_checks_run) {
        (a, 0) => format!(
            "Version check triggered for '{}' on {a} agent(s)",
            item.name
        ),
        (0, c) => format!(
            "Version check completed for '{}' ({c} controller-side fetch(es))",
            item.name
        ),
        (a, c) => format!(
            "Version check triggered for '{}' on {a} agent(s) and {c} controller-side fetch(es)",
            item.name
        ),
    };

    let resp = TriggerVersionCheckResponse {
        agents_notified,
        controller_checks_run,
        message,
    };
    emit_software_version_check_audit(
        &audit_ctx,
        item_id,
        Some(&item.name),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "agents_notified": agents_notified,
            "controller_checks_run": controller_checks_run,
            "dispatch_mode": version_check_dispatch_mode(agents_notified, controller_checks_run),
            "dispatch_scope": "all_hosts",
        }),
    );
    (StatusCode::OK, Json(resp)).into_response()
}

/// Trigger a version check for a specific software item on a specific host.
#[utoipa::path(
    post,
    path = "/api/v1/software-items/{id}/hosts/{host_id}/check-versions",
    params(
        ("id" = Uuid, Path, description = "Software item UUID"),
        ("host_id" = Uuid, Path, description = "Host UUID")
    ),
    responses(
        (status = 200, description = "Version check triggered", body = TriggerVersionCheckResponse),
        (status = 400, description = "Invalid input"),
        (status = 404, description = "Software item, host, or agent not found")
    ),
    tag = "Software Items",
    security(("oauth2" = ["checks:trigger"]), ("developer_token" = []))
)]
#[tracing::instrument(skip_all)]
pub async fn check_versions_host(
    State(state): State<Arc<AppState>>,
    tenant_db: TenantDb,
    CanTriggerChecks(user): CanTriggerChecks,
    api_token_id: Option<Extension<AuthenticatedApiTokenId>>,
    Path((item_id, host_id)): Path<(Uuid, Uuid)>,
) -> Response {
    let api_token_id = api_token_id.map(|value| value.0);
    let audit_ctx = AuditContext {
        audit_emitter: &state.audit_emitter,
        tenant_id: tenant_db.tenant_id(),
        user: &user,
        api_token_id,
    };

    // Phase 1–3: verify software item, host, and assignment.
    let (item, host_record, link) =
        match verify_software_item_and_host(&tenant_db, item_id, host_id).await {
            Ok(t) => t,
            Err(error) => {
                let (outcome, reason_code) = error.audit();
                emit_software_version_check_audit(
                    &audit_ctx,
                    item_id,
                    None,
                    outcome,
                    serde_json::json!({
                        "dispatch_scope": "single_host",
                        "host_id": host_id,
                        "reason_code": reason_code,
                    }),
                );
                return error.into_response();
            }
        };

    // Phase 4–5: load approved agent service for this host.
    let agent = match load_agent_service(&tenant_db, host_id).await {
        Ok(a) => a,
        Err(error) => {
            let (outcome, reason_code) = error.audit();
            emit_software_version_check_audit(
                &audit_ctx,
                item_id,
                Some(&item.name),
                outcome,
                serde_json::json!({
                    "dispatch_scope": "single_host",
                    "host_id": host_id,
                    "reason_code": reason_code,
                }),
            );
            return error.into_response();
        }
    };

    // Phase 6: load role-specific plugin assignments for this host.
    let role_plugins = match HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(item_id))
        .filter(host_software_item_plugin::Column::Role.is_in(["detect_version", "fetch_releases"]))
        .all(tenant_db.db())
        .await
    {
        Ok(ps) => ps,
        Err(e) => {
            tracing::error!("Failed to load role plugins: {e}");
            emit_software_version_check_audit(
                &audit_ctx,
                item_id,
                Some(&item.name),
                uptrakit_audit_log::AuditOutcome::Failed,
                serde_json::json!({
                    "dispatch_scope": "single_host",
                    "host_id": host_id,
                    "reason_code": "version_check.internal_error",
                }),
            );
            return error_response(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };

    // Phase 7: classify plugins into controller jobs vs agent assignments.
    let (controller_fetch_jobs, detect_version, fetch_releases) =
        match classify_role_assignments(&tenant_db, &role_plugins, host_id, item_id).await {
            Ok(result) => result,
            Err(resp) => return resp,
        };

    // Phase 8a: run controller-side fetch_releases (e.g. GitHub, Docker).
    let controller_checks_run = run_controller_fetch_jobs(
        tenant_db.db(),
        &state.notification.notification_service,
        &state.notification.event_broadcaster,
        tenant_db.tenant_id(),
        Some(state.global_providers()),
        controller_fetch_jobs,
    )
    .await;

    // Phase 8b: if no agent-side work is needed, return immediately.
    if detect_version.is_none() && fetch_releases.is_none() {
        if controller_checks_run > 0 {
            let resp = TriggerVersionCheckResponse {
                agents_notified: 0,
                controller_checks_run,
                message: format!(
                    "Version check completed for '{}' (controller-side)",
                    item.name
                ),
            };
            emit_software_version_check_audit(
                &audit_ctx,
                item_id,
                Some(&item.name),
                uptrakit_audit_log::AuditOutcome::Success,
                serde_json::json!({
                    "agents_notified": 0,
                    "controller_checks_run": controller_checks_run,
                    "dispatch_mode": version_check_dispatch_mode(0, controller_checks_run),
                    "dispatch_scope": "single_host",
                    "host_id": host_id,
                }),
            );
            return (StatusCode::OK, Json(resp)).into_response();
        }
        emit_software_version_check_audit(
            &audit_ctx,
            item_id,
            Some(&item.name),
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            serde_json::json!({
                "agents_notified": 0,
                "controller_checks_run": controller_checks_run,
                "dispatch_mode": version_check_dispatch_mode(0, controller_checks_run),
                "dispatch_scope": "single_host",
                "host_id": host_id,
                "reason_code": "version_check.no_role_assignments",
            }),
        );
        return error_response(
            StatusCode::BAD_REQUEST,
            "No detect_version or fetch_releases plugin assigned",
        );
    }

    // Phase 8c: dispatch CheckVersions to the agent.
    let assignment = uptrakit_wire::VersionCheckAssignment {
        software_item_id: item_id,
        name: item.name.clone(),
        detect_version,
        fetch_releases,
        host_software_item_id: Some(link.id),
    };

    let msg =
        uptrakit_wire::ControllerMessage::CheckVersions(uptrakit_wire::CheckVersionsPayload {
            host_machine_id: host_record.machine_id.clone(),
            assignments: vec![assignment],
        });
    state
        .notification
        .notification_service
        .send(&agent.id, msg)
        .await;
    emit_software_version_check_audit(
        &audit_ctx,
        item_id,
        Some(&item.name),
        uptrakit_audit_log::AuditOutcome::Success,
        serde_json::json!({
            "agents_notified": 1,
            "controller_checks_run": controller_checks_run,
            "dispatch_mode": version_check_dispatch_mode(1, controller_checks_run),
            "dispatch_scope": "single_host",
            "host_id": host_id,
        }),
    );

    let resp = TriggerVersionCheckResponse {
        agents_notified: 1,
        controller_checks_run,
        message: format!("Version check triggered for '{}' on 1 agent", item.name),
    };
    (StatusCode::OK, Json(resp)).into_response()
}
