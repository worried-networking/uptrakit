//! Common message handlers extracted from the authenticated loop.
//!
//! Each function corresponds to one match arm in the main dispatch and returns
//! a [`LoopAction`] plus an optional [`ControllerMessage`] reply. The main
//! loop is responsible for serializing and writing the reply to the WebSocket
//! sink.

use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use sea_orm::sea_query::Expr;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, Set};

use uptrakit_shared_db::entity::{host, host_software_item, service, service_host, software_item};
use uptrakit_wire::report_tracker::{PageOutcome, ReportTracker};
use uptrakit_wire::{
    CertificatePayload, CloseReason, ControllerMessage, DiscoveryResultsPayload, ErrorCode,
    ErrorPayload, HostConnectivityUpdate, OutgoingSeq, ReportHostsPayload, ReportPagination,
    ReportPluginConfigPayload, ReportPluginConfigResponsePayload, RequestCrlRenewalPayload,
    VersionCheckResultsPayload,
};

use uptrakit_shared_db::entity::system_service as sys_svc_entity;

use super::LoopAction;
use super::discovery::trigger_discovery_for_agent_host;
use super::renewal::{sign_renewal_csr, sign_renewal_csr_system};
use super::shared_types::{ProcessorResponse, load_linked_host_ids};
use uptrakit_web_api_types::events::AdminEvent;

use crate::AppState;
use crate::notifications::events::{NotificationEvent, NotificationEventDetails};
use crate::routes::agent_operations::{
    find_or_create_host_and_link, revoke_certificate, revoke_system_certificate,
};
use crate::routes::service_ws::protocol::{
    CertIdentity, record_service_activity, record_system_service_activity, send_pong,
};

fn emit_service_inventory_audit(
    state: &AppState,
    service_model: &service::Model,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    outcome: uptrakit_audit_log::AuditOutcome,
    target: Option<(&str, String, Option<String>)>,
    details: serde_json::Value,
) {
    let mut builder = uptrakit_audit_log::AuditEntry::builder(action_type)
        .tenant_scope(service_model.tenant_id)
        .actor_service(service_model.id)
        .actor_display_opt(service_model.service_app_name.clone())
        .outcome(outcome)
        .details(details);
    if let Some((target_type, target_id, target_display)) = target {
        builder = builder.target(target_type, target_id, target_display);
    }
    match builder.build() {
        Ok(entry) => state.audit_emitter.emit_best_effort(entry),
        Err(error) => {
            tracing::warn!(
                service_id = %service_model.id,
                action_type = %action_type,
                error = %error,
                "failed to build service inventory audit entry"
            );
        }
    }
}

fn report_plugin_config_target_id(plugin_type: &str, config_name: &str) -> String {
    format!("service_reported:{plugin_type}:{config_name}")
}

struct PluginConfigReportAuditCtx<'a> {
    state: &'a AppState,
    service_id: uuid::Uuid,
    service_tenant_id: Option<uuid::Uuid>,
    service_app_name: Option<&'a str>,
}

fn emit_report_plugin_config_audit(
    ctx: &PluginConfigReportAuditCtx<'_>,
    request_id: &str,
    plugin_type: &str,
    config_name: &str,
    target_id: Option<String>,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: Option<&'static str>,
) {
    let mut details = serde_json::json!({
        "plugin_type": plugin_type,
        "config_name": config_name,
        "mutation_source": "service_ws.report_plugin_config",
    });
    if let Some(service_app_name) = ctx.service_app_name {
        details["service_app_name"] = serde_json::Value::String(service_app_name.to_string());
    }
    if let Some(reason_code) = reason_code {
        details["reason_code"] = serde_json::Value::String(reason_code.to_string());
    }

    let mut builder = uptrakit_audit_log::AuditEntry::builder(
        uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
    )
    .actor_service(ctx.service_id)
    .actor_display_opt(ctx.service_app_name.map(str::to_string))
    .target(
        "plugin_config",
        target_id.unwrap_or_else(|| report_plugin_config_target_id(plugin_type, config_name)),
        Some(config_name.to_string()),
    )
    .outcome(outcome)
    .details(details)
    .request_id_opt(Some(request_id.to_string()));
    builder = if let Some(tenant_id) = ctx.service_tenant_id {
        builder.tenant_scope(tenant_id)
    } else {
        builder.system_scope()
    };

    match builder.build() {
        Ok(entry) => ctx.state.audit_emitter.emit_best_effort(entry),
        Err(error) => tracing::warn!(
            error = %error,
            service_id = %ctx.service_id,
            plugin_type,
            config_name,
            outcome = outcome.as_str(),
            "failed to build ReportPluginConfig audit entry"
        ),
    }
}

async fn emit_service_certificate_renew_non_success_audit_event(
    state: &AppState,
    service_id: uuid::Uuid,
    is_system: bool,
    outcome: uptrakit_audit_log::AuditOutcome,
    reason_code: &'static str,
) {
    let payload = uptrakit_wire::AuditEventPayload {
        action_type: uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW.to_string(),
        tenant_id: None,
        target_type: Some("service".to_string()),
        target_id: Some(service_id.to_string()),
        target_display: None,
        outcome: outcome.as_str().to_string(),
        details_json: Some(
            serde_json::json!({
                "reason_code": reason_code,
            })
            .to_string(),
        ),
        request_id: None,
    };
    let _ =
        super::ingest_service_audit_event(state, service_id, is_system, None, None, payload).await;
}

#[derive(Default)]
struct ReportHostsSummary {
    reported_hosts: u32,
    unknown_hosts: u32,
    created_hosts: u32,
    updated_hosts: u32,
    failed_hosts: u32,
    discovery_triggered_hosts: u32,
}

impl ReportHostsSummary {
    fn linked_hosts(&self) -> u32 {
        self.created_hosts.saturating_add(self.updated_hosts)
    }

    fn should_emit_audit(&self) -> bool {
        self.linked_hosts() > 0 || self.failed_hosts > 0
    }

    fn outcome(&self) -> uptrakit_audit_log::AuditOutcome {
        if self.failed_hosts == 0 {
            uptrakit_audit_log::AuditOutcome::Success
        } else if self.linked_hosts() > 0 {
            uptrakit_audit_log::AuditOutcome::Partial
        } else {
            uptrakit_audit_log::AuditOutcome::Failed
        }
    }
}

// ---------------------------------------------------------------------------
// handle_ping (stays in main loop — not part of processor)
// ---------------------------------------------------------------------------

/// Handle a `Ping` message: send pong and record activity.
#[tracing::instrument(skip_all, fields(%service_id))]
pub(super) async fn handle_ping(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    out_seq: &mut OutgoingSeq,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    service_ts: i64,
    is_system: bool,
) -> LoopAction {
    let Ok(controller_ts) = send_pong(sink, out_seq, service_ts).await else {
        return LoopAction::Break;
    };
    tracing::trace!(service_ts, controller_ts, "ping/pong");
    let activity_result = if is_system {
        record_system_service_activity(state.db(), service_id, None).await
    } else {
        record_service_activity(state.db(), service_id, None).await
    };
    if let Err(e) = activity_result {
        tracing::warn!(
            error = %e,
            %service_id,
            "failed to record service activity"
        );
    }
    LoopAction::Continue
}

// ---------------------------------------------------------------------------
// handle_renew_certificate
// ---------------------------------------------------------------------------

/// Handle a `RenewCertificate` message: verify approved, sign renewal CSR,
/// revoke old cert.
///
/// Returns a [`ProcessorResponse`] with the reply message and action.
#[tracing::instrument(skip_all, fields(%service_id, is_system))]
pub(super) async fn handle_renew_certificate(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    cert: &CertIdentity,
    payload: &uptrakit_wire::RenewCertificatePayload,
    is_system: bool,
) -> ProcessorResponse {
    if is_system {
        // System service renewal path.
        let svc = match sys_svc_entity::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(s))
                if s.status == sys_svc_entity::SystemServiceStatus::Approved
                    && s.deactivated_at.is_none() =>
            {
                s
            }
            _ => {
                emit_service_certificate_renew_non_success_audit_event(
                    state,
                    service_id,
                    true,
                    uptrakit_audit_log::AuditOutcome::Denied,
                    "not_approved",
                )
                .await;
                return ProcessorResponse::reply_and_break(ControllerMessage::Error(
                    ErrorPayload {
                        code: ErrorCode::Forbidden,
                        message: "service is not approved".to_string(),
                    },
                ));
            }
        };

        match sign_renewal_csr_system(
            state.cert_signer.as_ref(),
            &state.settings,
            state.db(),
            svc,
            &payload.csr_pem,
        )
        .await
        {
            Ok(bundle) => {
                // Revoke old system service certificate.
                if let Err(e) =
                    revoke_system_certificate(state.db(), &cert.serial, &cert.ca_fingerprint).await
                {
                    tracing::error!(error = %e, "failed to revoke old system service certificate");
                }

                // Bump CRL and notify (system certs share the CRL).
                if let Err(e) = crate::settings_store::bump_revocation_version(
                    state.db(),
                    state.default_tenant_id,
                )
                .await
                {
                    tracing::warn!(error = ?e, "failed to bump revocation version counter");
                }
                state.cert.revocation_notify.notify_one();
                state
                    .notification
                    .notification_service
                    .publish_controller_event(ControllerMessage::RequestCrlRenewal(
                        RequestCrlRenewalPayload::default(),
                    ))
                    .await;
                tracing::info!(
                    %service_id,
                    old_serial = %cert.serial,
                    "system service certificate renewed, old cert revoked"
                );

                let cert_msg = ControllerMessage::Certificate(CertificatePayload {
                    cert_pem: bundle.cert_pem,
                    not_after: bundle.not_after,
                });
                super::emit_service_certificate_renew_audit_event(
                    state,
                    service_id,
                    true,
                    bundle.not_after.into(),
                )
                .await;
                ProcessorResponse::reply_and_close(cert_msg, CloseReason::CertificateRotated)
            }
            Err(e) => {
                emit_service_certificate_renew_non_success_audit_event(
                    state,
                    service_id,
                    true,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    "certificate_signing_failed",
                )
                .await;
                ProcessorResponse::reply_and_break(ControllerMessage::Error(ErrorPayload {
                    code: ErrorCode::CertificateError,
                    message: e.to_string(),
                }))
            }
        }
    } else {
        // Tenant service renewal path.
        let svc = match service::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
            Ok(Some(s))
                if s.status == service::ServiceStatus::Approved && s.deactivated_at.is_none() =>
            {
                s
            }
            _ => {
                emit_service_certificate_renew_non_success_audit_event(
                    state,
                    service_id,
                    false,
                    uptrakit_audit_log::AuditOutcome::Denied,
                    "not_approved",
                )
                .await;
                return ProcessorResponse::reply_and_break(ControllerMessage::Error(
                    ErrorPayload {
                        code: ErrorCode::Forbidden,
                        message: "service is not approved".to_string(),
                    },
                ));
            }
        };

        match sign_renewal_csr(
            state.cert_signer.as_ref(),
            &state.settings,
            state.db(),
            svc,
            &payload.csr_pem,
        )
        .await
        {
            Ok(bundle) => {
                // Revoke old certificate.
                if let Err(e) = revoke_certificate(
                    state.db(),
                    &cert.serial,
                    &cert.ca_fingerprint,
                    uptrakit_shared_db::entity::prelude::RevocationReason::CertificateRenewed,
                )
                .await
                {
                    tracing::error!(error = %e, "failed to revoke old certificate");
                }

                if let Err(e) = crate::settings_store::bump_revocation_version(
                    state.db(),
                    state.default_tenant_id,
                )
                .await
                {
                    tracing::warn!(error = ?e, "failed to bump revocation version counter");
                }
                state.cert.revocation_notify.notify_one();
                state
                    .notification
                    .notification_service
                    .publish_controller_event(ControllerMessage::RequestCrlRenewal(
                        RequestCrlRenewalPayload::default(),
                    ))
                    .await;
                tracing::info!(
                    %service_id,
                    old_serial = %cert.serial,
                    "certificate renewed, old cert revoked"
                );

                let cert_msg = ControllerMessage::Certificate(CertificatePayload {
                    cert_pem: bundle.cert_pem,
                    not_after: bundle.not_after,
                });
                super::emit_service_certificate_renew_audit_event(
                    state,
                    service_id,
                    false,
                    bundle.not_after.into(),
                )
                .await;
                ProcessorResponse::reply_and_close(cert_msg, CloseReason::CertificateRotated)
            }
            Err(e) => {
                emit_service_certificate_renew_non_success_audit_event(
                    state,
                    service_id,
                    false,
                    uptrakit_audit_log::AuditOutcome::Failed,
                    "certificate_signing_failed",
                )
                .await;
                ProcessorResponse::reply_and_break(ControllerMessage::Error(ErrorPayload {
                    code: ErrorCode::CertificateError,
                    message: e.to_string(),
                }))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// handle_report_hosts
// ---------------------------------------------------------------------------

/// Iterate reported hosts and find-or-create their DB entries, triggering
/// autodiscovery for any newly-seen hosts.
async fn link_reported_hosts(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    service_model: &service::Model,
    payload: &ReportHostsPayload,
) -> ReportHostsSummary {
    let mut summary = ReportHostsSummary {
        reported_hosts: payload.hosts.len() as u32,
        ..ReportHostsSummary::default()
    };

    for host_info in &payload.hosts {
        if host_info.machine_id == "unknown" {
            summary.unknown_hosts = summary.unknown_hosts.saturating_add(1);
            continue;
        }

        if !service_model.is_embedded
            && host_info.machine_id != "unknown"
            && let Some(ref notifier) = state.embedded_service_notifier
        {
            notifier.on_machine_id_reported(&service_id, &host_info.machine_id);
        }

        let host_hostname = host_info
            .hostname
            .as_deref()
            .unwrap_or(&service_model.hostname);
        let host_ip = host_info
            .ip_address
            .as_deref()
            .or(service_model.ip_address.as_deref());
        match find_or_create_host_and_link(
            state.db(),
            service_model.tenant_id,
            service_id,
            host_info,
            host_hostname,
            host_ip,
        )
        .await
        {
            Ok(Some((host_id, true))) => {
                summary.created_hosts = summary.created_hosts.saturating_add(1);
                // New host -- trigger autodiscovery.
                trigger_discovery_for_agent_host(
                    state,
                    service_id,
                    service_model.tenant_id,
                    host_id,
                    &host_info.machine_id,
                )
                .await;
                summary.discovery_triggered_hosts =
                    summary.discovery_triggered_hosts.saturating_add(1);
            }
            Ok(Some((_host_id, false))) => {
                summary.updated_hosts = summary.updated_hosts.saturating_add(1);
            }
            Ok(None) => {
                summary.unknown_hosts = summary.unknown_hosts.saturating_add(1);
            }
            Err(e) => {
                summary.failed_hosts = summary.failed_hosts.saturating_add(1);
                tracing::warn!(
                    error = %e,
                    machine_id = %host_info.machine_id,
                    "failed to link host"
                );
            }
        }
    }

    summary
}

/// Notify MQTT services that a service's linked hosts are online.
async fn notify_reported_hosts_online(
    state: &Arc<AppState>,
    service_model: &service::Model,
    linked_host_ids: &Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
    agent_version: &str,
) {
    let current_ids = linked_host_ids.lock().clone();
    if current_ids.is_empty() {
        return;
    }
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    let updates: Vec<HostConnectivityUpdate> = current_ids
        .iter()
        .map(|&host_id| {
            HostConnectivityUpdate::online(
                host_id,
                Some(now.clone()),
                Some(agent_version.to_string()),
            )
        })
        .collect();
    state
        .notification
        .notification_service
        .send_connectivity_update(service_model.tenant_id, updates)
        .await;
}

/// Handle a `ReportHosts` message: update `client_version`, find/create hosts,
/// trigger discovery, refresh `linked_host_ids`.
#[tracing::instrument(skip_all, fields(%service_id, host_count = payload.hosts.len()))]
pub(super) async fn handle_report_hosts(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &ReportHostsPayload,
    linked_host_ids: &Arc<parking_lot::Mutex<HashSet<uuid::Uuid>>>,
) -> ProcessorResponse {
    tracing::debug!(
        %service_id,
        capabilities = ?payload.capabilities,
        "received ReportHosts"
    );

    let service_model = match service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(s)) => s,
        _ => return ProcessorResponse::cont(),
    };

    // Update client_version.
    let mut active: service::ActiveModel = service_model.clone().into();
    active.client_version = Set(Some(payload.agent_version.clone()));
    active.updated_at = Set(time::OffsetDateTime::now_utc());
    if let Err(e) = active.update(state.db()).await {
        tracing::error!(
            error = %e,
            "failed to update client_version"
        );
    }

    let summary = link_reported_hosts(state, service_id, &service_model, payload).await;

    // Refresh cached host IDs.
    if let Ok(ids) = load_linked_host_ids(state.db(), service_id).await {
        *linked_host_ids.lock() = ids;
    }

    notify_reported_hosts_online(
        state,
        &service_model,
        linked_host_ids,
        &payload.agent_version,
    )
    .await;

    if summary.should_emit_audit() {
        emit_service_inventory_audit(
            state,
            &service_model,
            uptrakit_audit_log::AuditActionType::HOST_UPDATE,
            summary.outcome(),
            Some((
                "service",
                service_model.id.to_string(),
                Some(service_model.friendly_name.clone()),
            )),
            serde_json::json!({
                "reported_hosts": summary.reported_hosts,
                "linked_hosts": summary.linked_hosts(),
                "created_hosts": summary.created_hosts,
                "updated_hosts": summary.updated_hosts,
                "unknown_hosts": summary.unknown_hosts,
                "failed_hosts": summary.failed_hosts,
                "discovery_triggered_hosts": summary.discovery_triggered_hosts,
                "agent_version": payload.agent_version,
            }),
        );
    }

    ProcessorResponse::cont()
}

// ---------------------------------------------------------------------------
// handle_version_check_results
// ---------------------------------------------------------------------------

/// Resolve the `host_software_item` rows that a version check result targets.
///
/// Prefers the targeted path (`host_software_item_id` present) and falls back
/// to a host-ids scan for old agent versions that do not set the field.
async fn resolve_matching_host_software_items(
    db: &sea_orm::DatabaseConnection,
    service_id: uuid::Uuid,
    result: &uptrakit_wire::VersionCheckResult,
    host_ids: &[uuid::Uuid],
) -> Vec<host_software_item::Model> {
    let software_item_id = result.software_item_id;

    if let Some(hsi_id) = result.host_software_item_id {
        match host_software_item::Entity::find_by_id(hsi_id)
            .filter(host_software_item::Column::HostId.is_in(host_ids.to_vec()))
            .filter(host_software_item::Column::DeactivatedAt.is_null())
            .one(db)
            .await
        {
            Ok(Some(row)) => vec![row],
            Ok(None) => {
                tracing::debug!(
                    %software_item_id,
                    host_software_item_id = %hsi_id,
                    "targeted host_software_item not found or not owned by this service"
                );
                vec![]
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    %software_item_id,
                    host_software_item_id = %hsi_id,
                    "failed to look up targeted host_software_item"
                );
                vec![]
            }
        }
    } else {
        // Legacy path: scan all hosts linked to this service.
        tracing::warn!(
            %service_id,
            %software_item_id,
            "VersionCheckResult missing host_software_item_id; \
             falling back to host_ids scan (cross-host contamination risk)"
        );
        match host_software_item::Entity::find()
            .filter(host_software_item::Column::HostId.is_in(host_ids.to_vec()))
            .filter(host_software_item::Column::SoftwareItemId.eq(software_item_id))
            .filter(host_software_item::Column::DeactivatedAt.is_null())
            .all(db)
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    %software_item_id,
                    "failed to look up host_software_items"
                );
                vec![]
            }
        }
    }
}

/// Build and execute the `update_many` query for a version check result.
async fn apply_version_update_to_db(
    db: &sea_orm::DatabaseConnection,
    result: &uptrakit_wire::VersionCheckResult,
    matching_ids: Vec<uuid::Uuid>,
    now: time::OffsetDateTime,
) {
    debug_assert!(
        result.error.is_none(),
        "apply_version_update_to_db called with error-bearing VersionCheckResult; caller must skip DB writes for software_item_id={} host_software_item_id={:?}",
        result.software_item_id,
        result.host_software_item_id
    );
    let software_item_id = result.software_item_id;
    let mut update = host_software_item::Entity::update_many()
        .filter(host_software_item::Column::Id.is_in(matching_ids.clone()));
    update = update.col_expr(
        host_software_item::Column::UpdateCategory,
        sea_orm::sea_query::Expr::value(result.update_category.to_string()),
    );
    if let Some(ref installed_version) = result.installed_version {
        update = update
            .col_expr(
                host_software_item::Column::InstalledVersion,
                sea_orm::sea_query::Expr::value(Some(installed_version.clone())),
            )
            .col_expr(
                host_software_item::Column::InstalledVersionDetectedAt,
                sea_orm::sea_query::Expr::value(Some(now)),
            )
            .col_expr(
                host_software_item::Column::InstalledDisplayVersion,
                sea_orm::sea_query::Expr::value(result.installed_display_version.clone()),
            );
    }
    if let Some(ref latest_version) = result.latest_version {
        update = update
            .col_expr(
                host_software_item::Column::LatestVersion,
                sea_orm::sea_query::Expr::value(Some(latest_version.clone())),
            )
            .col_expr(
                host_software_item::Column::LatestVersionFetchedAt,
                sea_orm::sea_query::Expr::value(Some(now)),
            );
    }
    if let Err(e) = update.exec(db).await {
        tracing::warn!(
            error = %e,
            %software_item_id,
            row_count = matching_ids.len(),
            "failed to update host_software_items"
        );
    }
}

/// Dispatch update-available notifications for each matched host when a new
/// latest version is detected.
async fn dispatch_version_update_notification(
    state: &Arc<AppState>,
    tenant_id: uuid::Uuid,
    result: &uptrakit_wire::VersionCheckResult,
    matching_host_ids: Vec<uuid::Uuid>,
) {
    let Some(ref latest_version) = result.latest_version else {
        return;
    };
    let software_item_id = result.software_item_id;

    let sw_name = software_item::Entity::find_by_id(software_item_id)
        .one(state.db())
        .await
        .ok()
        .flatten()
        .map(|sw| sw.name.clone());

    for host_id in matching_host_ids {
        let host_name = host::Entity::find_by_id(host_id)
            .one(state.db())
            .await
            .ok()
            .flatten()
            .map(|h| h.hostname.clone());

        state
            .notification
            .notification_dispatcher
            .dispatch(NotificationEvent {
                tenant_id,
                host_id: Some(host_id),
                host_name,
                software_item_id: Some(software_item_id),
                software_item_name: sw_name.clone(),
                plugin_type: None,
                details: NotificationEventDetails::UpdateAvailable {
                    installed_version: result.installed_version.clone(),
                    latest_version: latest_version.clone(),
                },
            });
    }
}

/// Post-loop finalization: batch-update `last_checked_at`, push MQTT states,
/// and emit `VersionCheckCompleted` SSE events.
async fn finalize_version_check_results(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &VersionCheckResultsPayload,
    now: time::OffsetDateTime,
    svc_tenant_id: Option<uuid::Uuid>,
    completed_pairs: Vec<(uuid::Uuid, uuid::Uuid)>,
) {
    // Batch-update last_checked_at for successful results.
    let checked_ids: Vec<uuid::Uuid> = payload
        .results
        .iter()
        .filter(|r| r.error.is_none())
        .map(|r| r.software_item_id)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    if !checked_ids.is_empty()
        && let Err(e) = software_item::Entity::update_many()
            .filter(software_item::Column::Id.is_in(checked_ids))
            .col_expr(software_item::Column::LastCheckedAt, Expr::value(now))
            .exec(state.db())
            .await
    {
        tracing::warn!(
            error = %e,
            "failed to batch-update software_item last_checked_at"
        );
    }

    // Push updated software states to MQTT services.
    if let Ok(Some(svc)) = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        state
            .notification
            .notification_service
            .push_software_states_for_tenant(state.db(), svc.tenant_id)
            .await;
    }

    // Emit AdminEvent::VersionCheckCompleted for each (host, software_item) pair
    // so the /software page SSE subscribers can refresh.
    if let Some(tenant_id) = svc_tenant_id {
        for (host_id, software_item_id) in completed_pairs {
            state
                .notification
                .event_broadcaster
                .send(
                    tenant_id,
                    AdminEvent::VersionCheckCompleted {
                        host_id,
                        software_item_id,
                    },
                )
                .await;
        }
    }
}

#[derive(Default)]
struct VersionCheckAuditSummary {
    result_count: u32,
    success_count: u32,
    error_count: u32,
    unmatched_count: u32,
    rows_mutated: u32,
}

impl VersionCheckAuditSummary {
    fn outcome(&self) -> uptrakit_audit_log::AuditOutcome {
        if self.result_count == 0
            || (self.success_count == self.result_count
                && self.error_count == 0
                && self.unmatched_count == 0)
        {
            uptrakit_audit_log::AuditOutcome::Success
        } else if self.success_count > 0 {
            uptrakit_audit_log::AuditOutcome::Partial
        } else {
            uptrakit_audit_log::AuditOutcome::Failed
        }
    }
}

/// Handle a `VersionCheckResults` message: update installed versions, upsert
/// available versions, batch update `last_checked_at`, push software states.
#[tracing::instrument(skip_all, fields(%service_id, result_count = payload.results.len()))]
pub(super) async fn handle_version_check_results(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &VersionCheckResultsPayload,
) -> ProcessorResponse {
    tracing::debug!(
        %service_id,
        count = payload.results.len(),
        "received VersionCheckResults"
    );

    let host_ids: Vec<uuid::Uuid> = match load_linked_host_ids(state.db(), service_id).await {
        Ok(ids) => ids.into_iter().collect(),
        Err(e) => {
            tracing::warn!(
                error = %e,
                "failed to look up service hosts"
            );
            return ProcessorResponse::cont();
        }
    };

    if host_ids.is_empty() {
        tracing::debug!(
            %service_id,
            "no hosts linked, skipping version updates"
        );
        return ProcessorResponse::cont();
    }

    let now = time::OffsetDateTime::now_utc();

    // Look up service identity once; reused for notifications and audit scope.
    let service_model = match service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(svc)) => Some(svc),
        Ok(None) => {
            tracing::warn!(%service_id, "service not found for version check results");
            None
        }
        Err(e) => {
            tracing::warn!(%service_id, error = %e, "failed to look up service");
            None
        }
    };
    let svc_tenant_id = service_model.as_ref().map(|svc| svc.tenant_id);

    // Collect (host_id, software_item_id) pairs for successful results so we
    // can emit VersionCheckCompleted SSE events after the DB work is done.
    let mut completed_pairs: Vec<(uuid::Uuid, uuid::Uuid)> = Vec::new();
    let mut audit_summary = VersionCheckAuditSummary {
        result_count: payload.results.len() as u32,
        ..VersionCheckAuditSummary::default()
    };

    for result in &payload.results {
        if result.error.is_some() {
            tracing::debug!(
                software_item_id = %result.software_item_id,
                host_software_item_id = ?result.host_software_item_id,
                error = ?result.error,
                "skipping version result with error; existing DB state preserved"
            );
            audit_summary.error_count += 1;
            continue;
        }

        let matching_rows =
            resolve_matching_host_software_items(state.db(), service_id, result, &host_ids).await;

        if matching_rows.is_empty() {
            audit_summary.unmatched_count += 1;
            continue;
        }

        let matching_host_ids: Vec<uuid::Uuid> = matching_rows.iter().map(|r| r.host_id).collect();
        let matching_ids: Vec<uuid::Uuid> = matching_rows.iter().map(|r| r.id).collect();
        audit_summary.success_count += 1;
        audit_summary.rows_mutated += matching_ids.len() as u32;

        // Record one (host_id, software_item_id) pair per result
        // so we can emit VersionCheckCompleted events after DB writes complete.
        if let Some(&first_host_id) = matching_host_ids.first() {
            completed_pairs.push((first_host_id, result.software_item_id));
        }

        apply_version_update_to_db(state.db(), result, matching_ids, now).await;

        if let Some(tenant_id) = svc_tenant_id {
            dispatch_version_update_notification(state, tenant_id, result, matching_host_ids).await;
        }
    }

    finalize_version_check_results(
        state,
        service_id,
        payload,
        now,
        svc_tenant_id,
        completed_pairs,
    )
    .await;

    if let Some(svc) = service_model.as_ref() {
        emit_service_inventory_audit(
            state,
            svc,
            uptrakit_audit_log::AuditActionType::SOFTWARE_VERSION_CHECK_COMPLETED,
            audit_summary.outcome(),
            Some((
                "service",
                svc.id.to_string(),
                Some(svc.friendly_name.clone()),
            )),
            serde_json::json!({
                "result_count": audit_summary.result_count,
                "rows_mutated": audit_summary.rows_mutated,
                "success_count": audit_summary.success_count,
                "error_count": audit_summary.error_count,
                "unmatched_count": audit_summary.unmatched_count,
            }),
        );
    }

    ProcessorResponse::cont()
}

// ---------------------------------------------------------------------------
// handle_discovery_results
// ---------------------------------------------------------------------------

/// Find the host linked to a service that matches the given `machine_id`.
///
/// Iterates the provided service-host links and queries the DB for each
/// until a matching, non-deactivated host is found.
async fn find_linked_host_by_machine_id(
    db: &sea_orm::DatabaseConnection,
    links: &[service_host::Model],
    machine_id: &str,
) -> Option<uuid::Uuid> {
    for link in links {
        if let Ok(Some(h)) = host::Entity::find_by_id(link.host_id)
            .filter(host::Column::MachineId.eq(machine_id))
            .filter(host::Column::DeactivatedAt.is_null())
            .one(db)
            .await
        {
            return Some(h.id);
        }
    }
    None
}

/// Process a single discovery page for a known host.
///
/// Calls [`process_discovery_results`] and, on the final page, dispatches
/// a [`NotificationEventDetails::NewSoftwareDiscovered`] notification when
/// at least one item was found.
async fn process_discovery_page_for_host(
    state: &Arc<AppState>,
    svc: &service::Model,
    host_id: uuid::Uuid,
    payload: DiscoveryResultsPayload,
    page_outcome: PageOutcome,
    pagination: Option<&ReportPagination>,
    report_tracker: &mut ReportTracker,
) -> bool {
    let this_page_count: u32 = payload
        .results
        .iter()
        .filter(|r| r.error.is_none())
        .map(|r| r.discoveries.len() as u32)
        .sum();

    if let Err(e) = crate::queries::autodiscovery::process_discovery_results(
        state.db(),
        svc.id,
        svc.tenant_id,
        host_id,
        payload,
    )
    .await
    {
        tracing::warn!(
            error = %e,
            service_id = %svc.id,
            "failed to process discovery results"
        );
        return false;
    }

    // Fire software-item lifecycle plugins on newly discovered items that may
    // benefit from enrichment (e.g. icon assignment from Dashboard Icons).
    enrich_discovered_items(state, svc).await;

    match page_outcome {
        PageOutcome::Final {
            accumulated_discovered_count,
        } => {
            let total_discovered = accumulated_discovered_count.saturating_add(this_page_count);
            if total_discovered > 0 {
                let host_name = host::Entity::find_by_id(host_id)
                    .one(state.db())
                    .await
                    .ok()
                    .flatten()
                    .map(|h| h.hostname.clone());

                state
                    .notification
                    .notification_dispatcher
                    .dispatch(NotificationEvent {
                        tenant_id: svc.tenant_id,
                        host_id: Some(host_id),
                        host_name,
                        software_item_id: None,
                        software_item_name: None,
                        plugin_type: None,
                        details: NotificationEventDetails::NewSoftwareDiscovered {
                            discovered_count: total_discovered,
                        },
                    });
            }
        }
        PageOutcome::Pending => {
            if let Some(p) = pagination {
                report_tracker.add_discovered_count(p.report_id, this_page_count);
            }
        }
    }

    true
}

/// Handle a `DiscoveryResults` message: find host, process results.
#[tracing::instrument(skip_all, fields(%service_id, host_machine_id = %payload.host_machine_id))]
pub(super) async fn handle_discovery_results(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: DiscoveryResultsPayload,
    pagination: Option<&ReportPagination>,
    report_tracker: &mut ReportTracker,
) -> ProcessorResponse {
    tracing::debug!(
        %service_id,
        host_machine_id = %payload.host_machine_id,
        results = payload.results.len(),
        page = pagination.map(|p| p.page),
        total_pages = pagination.map(|p| p.total_pages),
        "received DiscoveryResults"
    );

    let service_model = match service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(svc)) => svc,
        Ok(None) => {
            tracing::warn!(
                %service_id,
                "service not found for DiscoveryResults"
            );
            return ProcessorResponse::cont();
        }
        Err(e) => {
            tracing::warn!(
                %service_id,
                error = %e,
                "failed to resolve service for DiscoveryResults"
            );
            return ProcessorResponse::cont();
        }
    };

    let plugin_results = payload.results.len() as u32;
    let discovered_items_reported: u32 = payload
        .results
        .iter()
        .filter(|result| result.error.is_none())
        .map(|result| result.discoveries.len() as u32)
        .sum();

    // Determine whether this is the final page (or a non-paginated message).
    let page_outcome = if let Some(p) = pagination {
        match report_tracker.register_page(p.report_id, p.page, p.total_pages) {
            Ok(outcome) => outcome,
            Err(e) => {
                tracing::warn!(
                    %service_id,
                    error = %e,
                    "invalid pagination for DiscoveryResults"
                );
                emit_service_inventory_audit(
                    state,
                    &service_model,
                    uptrakit_audit_log::AuditActionType::HOST_DISCOVER,
                    uptrakit_audit_log::AuditOutcome::ValidationFailed,
                    Some((
                        "service",
                        service_model.id.to_string(),
                        Some(service_model.friendly_name.clone()),
                    )),
                    serde_json::json!({
                        "reason_code": "invalid_pagination",
                        "host_machine_id": payload.host_machine_id,
                        "plugin_results": plugin_results,
                        "discovered_items_reported": discovered_items_reported,
                        "page": p.page,
                        "total_pages": p.total_pages,
                        "report_id": p.report_id,
                    }),
                );
                return ProcessorResponse::cont();
            }
        }
    } else {
        // Non-paginated: treat as final (and only) page.
        PageOutcome::Final {
            accumulated_discovered_count: 0,
        }
    };

    let links = service_host::Entity::find()
        .filter(service_host::Column::ServiceId.eq(service_id))
        .all(state.db())
        .await
        .unwrap_or_default();

    let host_machine_id = payload.host_machine_id.clone();
    match find_linked_host_by_machine_id(state.db(), &links, &host_machine_id).await {
        Some(host_id) => {
            let processed = process_discovery_page_for_host(
                state,
                &service_model,
                host_id,
                payload,
                page_outcome,
                pagination,
                report_tracker,
            )
            .await;
            let host_display = host::Entity::find_by_id(host_id)
                .one(state.db())
                .await
                .ok()
                .flatten()
                .map(|host| host.friendly_name);

            if !processed || discovered_items_reported > 0 {
                emit_service_inventory_audit(
                    state,
                    &service_model,
                    uptrakit_audit_log::AuditActionType::HOST_DISCOVER,
                    if processed {
                        uptrakit_audit_log::AuditOutcome::Success
                    } else {
                        uptrakit_audit_log::AuditOutcome::Failed
                    },
                    Some(("host", host_id.to_string(), host_display)),
                    serde_json::json!({
                        "host_machine_id": host_machine_id,
                        "plugin_results": plugin_results,
                        "discovered_items_reported": discovered_items_reported,
                        "paginated": pagination.is_some(),
                        "page": pagination.map(|p| p.page),
                        "total_pages": pagination.map(|p| p.total_pages),
                        "reason_code": if processed {
                            serde_json::Value::Null
                        } else {
                            serde_json::Value::String("process_discovery_results_failed".to_string())
                        },
                    }),
                );
            }
        }
        None => {
            tracing::warn!(
                %service_id,
                host_machine_id = %host_machine_id,
                "received DiscoveryResults for unknown host machine_id"
            );
            emit_service_inventory_audit(
                state,
                &service_model,
                uptrakit_audit_log::AuditActionType::HOST_DISCOVER,
                uptrakit_audit_log::AuditOutcome::Denied,
                Some((
                    "service",
                    service_model.id.to_string(),
                    Some(service_model.friendly_name.clone()),
                )),
                serde_json::json!({
                    "reason_code": "unknown_host_machine_id",
                    "host_machine_id": host_machine_id,
                    "plugin_results": plugin_results,
                    "discovered_items_reported": discovered_items_reported,
                    "paginated": pagination.is_some(),
                    "page": pagination.map(|p| p.page),
                    "total_pages": pagination.map(|p| p.total_pages),
                }),
            );
        }
    }

    ProcessorResponse::cont()
}

// ---------------------------------------------------------------------------
// handle_report_plugin_config
// ---------------------------------------------------------------------------

/// Handle a `ReportPluginConfig` message: find or create a plugin config and
/// return the response message.
///
/// Idempotent: if a config with the same `(tenant_id, plugin_type, name)`
/// already exists, the existing ID is returned without creating a duplicate.
pub(super) async fn handle_report_plugin_config(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &ReportPluginConfigPayload,
) -> ProcessorResponse {
    let request_id = payload.request_id.clone();

    let service_model = match service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(service_model)) => service_model,
        Ok(None) => {
            tracing::warn!(%service_id, "ReportPluginConfig: service not found");
            emit_report_plugin_config_audit(
                &PluginConfigReportAuditCtx {
                    state,
                    service_id,
                    service_tenant_id: None,
                    service_app_name: None,
                },
                &request_id,
                &payload.plugin_type,
                &payload.name,
                None,
                uptrakit_audit_log::AuditOutcome::Denied,
                Some("service_not_found"),
            );
            return ProcessorResponse::cont();
        }
        Err(e) => {
            tracing::warn!(%service_id, error = %e, "ReportPluginConfig: DB error");
            emit_report_plugin_config_audit(
                &PluginConfigReportAuditCtx {
                    state,
                    service_id,
                    service_tenant_id: None,
                    service_app_name: None,
                },
                &request_id,
                &payload.plugin_type,
                &payload.name,
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                Some("service_lookup_failed"),
            );
            return ProcessorResponse::cont();
        }
    };

    // Validate the plugin type is known
    let plugin_type_id = uptrakit_shared_types::PluginTypeId::new(&payload.plugin_type);
    if let Err(e) = state
        .plugin_ops
        .validate_config(&plugin_type_id, &payload.config)
    {
        tracing::warn!(
            %service_id,
            plugin_type = %payload.plugin_type,
            error = %e,
            "ReportPluginConfig: invalid config"
        );
        emit_report_plugin_config_audit(
            &PluginConfigReportAuditCtx {
                state,
                service_id,
                service_tenant_id: Some(service_model.tenant_id),
                service_app_name: service_model.service_app_name.as_deref(),
            },
            &request_id,
            &payload.plugin_type,
            &payload.name,
            None,
            uptrakit_audit_log::AuditOutcome::ValidationFailed,
            Some("invalid_plugin_config"),
        );
        let resp_payload: ReportPluginConfigResponsePayload =
            serde_json::from_value(serde_json::json!({
                "request_id": request_id,
                "success": false,
                "error": format!("invalid plugin config: {e}"),
            }))
            .expect("ReportPluginConfigResponsePayload JSON is always valid");
        return ProcessorResponse::reply(ControllerMessage::ReportPluginConfigResponse(
            resp_payload,
        ));
    }

    // Find or create the plugin config
    let result = crate::queries::autodiscovery::find_or_create_default_plugin_config(
        state.db(),
        service_model.tenant_id,
        &payload.plugin_type,
        &payload.config,
        &payload.name,
    )
    .await;

    let resp = match result {
        Ok(config_id) => {
            tracing::info!(
                %service_id,
                %config_id,
                plugin_type = %payload.plugin_type,
                name = %payload.name,
                "ReportPluginConfig: config created/found"
            );
            emit_report_plugin_config_audit(
                &PluginConfigReportAuditCtx {
                    state,
                    service_id,
                    service_tenant_id: Some(service_model.tenant_id),
                    service_app_name: service_model.service_app_name.as_deref(),
                },
                &request_id,
                &payload.plugin_type,
                &payload.name,
                Some(config_id.to_string()),
                uptrakit_audit_log::AuditOutcome::Success,
                None,
            );
            let resp_payload: ReportPluginConfigResponsePayload =
                serde_json::from_value(serde_json::json!({
                    "request_id": request_id,
                    "success": true,
                    "plugin_config_id": config_id,
                }))
                .expect("ReportPluginConfigResponsePayload JSON is always valid");
            ControllerMessage::ReportPluginConfigResponse(resp_payload)
        }
        Err(e) => {
            tracing::warn!(
                %service_id,
                error = %e,
                "ReportPluginConfig: failed to create/find config"
            );
            emit_report_plugin_config_audit(
                &PluginConfigReportAuditCtx {
                    state,
                    service_id,
                    service_tenant_id: Some(service_model.tenant_id),
                    service_app_name: service_model.service_app_name.as_deref(),
                },
                &request_id,
                &payload.plugin_type,
                &payload.name,
                None,
                uptrakit_audit_log::AuditOutcome::Failed,
                Some("create_or_find_failed"),
            );
            let resp_payload: ReportPluginConfigResponsePayload =
                serde_json::from_value(serde_json::json!({
                    "request_id": request_id,
                    "success": false,
                    "error": format!("failed to create plugin config: {e}"),
                }))
                .expect("ReportPluginConfigResponsePayload JSON is always valid");
            ControllerMessage::ReportPluginConfigResponse(resp_payload)
        }
    };

    ProcessorResponse::reply(resp)
}

// ---------------------------------------------------------------------------
// Software-item lifecycle enrichment (post-discovery)
// ---------------------------------------------------------------------------

/// After discovery results are processed, fire lifecycle plugins on featured
/// icon-less items. This is a best-effort operation — errors on individual
/// items are logged but never propagate.
async fn enrich_discovered_items(state: &AppState, service_model: &service::Model) {
    let tenant_id = service_model.tenant_id;
    let items =
        crate::queries::software_items::load_items_needing_enrichment(state.db(), tenant_id).await;

    let lifecycle_ctx = match crate::queries::plugin_type_settings::preload_lifecycle_type_settings(
        state.db(),
        tenant_id,
        state.plugin_ops.as_ref(),
    )
    .await
    {
        Ok(ctx) => ctx,
        Err(e) => {
            tracing::warn!(
                error = %e,
                %tenant_id,
                "failed to preload lifecycle type settings; using defaults"
            );
            uptrakit_plugin_infrastructure_registry::SoftwareItemLifecycleContext::default()
        }
    };

    tracing::debug!(%tenant_id, count = items.len(), "lifecycle enrichment loaded items");
    let examined_count = items.len() as u32;
    let mut patch_attempt_count = 0u32;
    let mut patched_count = 0u32;
    let mut patch_failed_count = 0u32;

    for item in items {
        let event = uptrakit_plugin_infrastructure_registry::SoftwareItemCreatedEvent::new(
            item.id,
            item.tenant_id,
            item.name.clone(),
            item.featured,
            item.icon_url.clone(),
        );
        match state
            .plugin_ops
            .on_software_item_created(&event, &lifecycle_ctx)
            .await
        {
            Some(patch) => {
                patch_attempt_count += 1;
                if let Err(e) = crate::queries::software_items::apply_software_item_patch(
                    state.db(),
                    item.id,
                    &patch,
                )
                .await
                {
                    tracing::warn!(
                        error = %e,
                        item_id = %item.id,
                        name = %item.name,
                        "lifecycle patch failed"
                    );
                    patch_failed_count += 1;
                } else {
                    tracing::trace!(item_id = %item.id, name = %item.name, "lifecycle patch applied");
                    patched_count += 1;
                }
            }
            None => {
                tracing::trace!(item_id = %item.id, name = %item.name, "lifecycle plugin produced no patch");
            }
        }
    }

    if patch_attempt_count > 0 {
        let outcome = if patch_failed_count == 0 {
            uptrakit_audit_log::AuditOutcome::Success
        } else if patched_count > 0 {
            uptrakit_audit_log::AuditOutcome::Partial
        } else {
            uptrakit_audit_log::AuditOutcome::Failed
        };
        emit_service_inventory_audit(
            state,
            service_model,
            uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_ENRICH,
            outcome,
            Some((
                "service",
                service_model.id.to_string(),
                Some(service_model.friendly_name.clone()),
            )),
            serde_json::json!({
                "examined_count": examined_count,
                "patch_attempt_count": patch_attempt_count,
                "patched_count": patched_count,
                "patch_failed_count": patch_failed_count,
            }),
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use sea_orm::{ActiveModelTrait, ColumnTrait, ConnectionTrait, QueryFilter, QueryOrder, Set};
    use serde::Deserialize;
    use std::sync::{Arc, OnceLock};
    use uptrakit_plugin_infrastructure_registry::{
        ControllerUpdateProtection, ControllerUpdateProtectionOps, NotificationOps,
        NotificationTransport, PluginConfigOps, PluginDescriptor, PluginMetadataOps, PluginOps,
        PluginSurfaceActionOps, PluginSurfaceOps, PluginTypeId, SoftwareItemCreatedEvent,
        SoftwareItemLifecycle, SoftwareItemLifecycleContext, SoftwareItemLifecycleOps,
        SoftwareItemPatch, SurfaceActionError, plugin_ids,
    };
    use uptrakit_shared_db::entity::{
        audit_log, ca_certificate, host, host_software_item, plugin_config, service, service_host,
        software_item, system_audit_log, system_service,
    };
    use uptrakit_wire::{
        Capability, DiscoveredSoftware, DiscoveryPluginResult, DiscoveryResultsPayload, HostInfo,
        RenewCertificatePayload, ReportHostsPayload, UpdateCategory, VersionCheckResult,
        VersionCheckResultsPayload,
    };
    use uuid::Uuid;

    use crate::embedded_support::EmbeddedServiceNotifier;
    use crate::test_harness::{
        build_test_state, build_test_state_with_plugin_ops, insert_default_tenant,
        setup_migrated_db,
    };

    const TEST_CA_FINGERPRINT: &str =
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    struct TestPluginOps;
    struct TestLifecyclePlugin;
    struct TestSuccessfulCertSigner;
    struct TestFailingCertSigner;
    #[derive(Default)]
    struct TestEmbeddedNotifier {
        machine_ids: parking_lot::Mutex<Vec<(Uuid, String)>>,
    }

    impl EmbeddedServiceNotifier for TestEmbeddedNotifier {
        fn on_external_connected(
            &self,
            _service_id: Uuid,
            _capabilities: &std::collections::BTreeSet<Capability>,
            _hostname: Option<&str>,
            _is_system: bool,
        ) {
        }

        fn on_external_disconnected(&self, _service_id: &Uuid) {}

        fn on_machine_id_reported(&self, service_id: &Uuid, machine_id: &str) {
            self.machine_ids
                .lock()
                .push((*service_id, machine_id.to_string()));
        }

        fn is_capability_yielded(&self, _capability: &Capability) -> bool {
            false
        }
    }

    #[derive(Debug, Deserialize)]
    struct TestLifecycleTypeSettings {
        #[serde(default = "default_lifecycle_enabled")]
        enabled: bool,
    }

    const fn default_lifecycle_enabled() -> bool {
        true
    }

    static TEST_LIFECYCLE_PLUGINS: OnceLock<Vec<Arc<dyn SoftwareItemLifecycle>>> = OnceLock::new();

    fn lifecycle_plugins() -> &'static [Arc<dyn SoftwareItemLifecycle>] {
        TEST_LIFECYCLE_PLUGINS
            .get_or_init(|| vec![Arc::new(TestLifecyclePlugin)])
            .as_slice()
    }

    async fn insert_embedded_service(
        db: &sea_orm::DatabaseConnection,
        tenant_id: Uuid,
        service_id: Uuid,
    ) {
        let now = time::OffsetDateTime::now_utc();
        service::ActiveModel {
            id: Set(service_id),
            tenant_id: Set(tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set("embedded-agent-host".to_string()),
            friendly_name: Set("Embedded Agent".to_string()),
            ip_address: Set(None),
            status: Set(uptrakit_shared_types::ServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("embedded-secret-{service_id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(Some("uptrakit-agent".to_string())),
            is_embedded: Set(true),
            embedded_owner_key: Set(None),
        }
        .insert(db)
        .await
        .expect("insert embedded service");
    }

    #[async_trait::async_trait]
    impl SoftwareItemLifecycle for TestLifecyclePlugin {
        async fn on_software_item_created(
            &self,
            _event: &SoftwareItemCreatedEvent,
            _ctx: &SoftwareItemLifecycleContext,
        ) -> std::result::Result<
            Option<SoftwareItemPatch>,
            uptrakit_plugin_infrastructure_registry::PluginError,
        > {
            Ok(None)
        }
    }

    impl uptrakit_plugin_infrastructure_registry::PluginMeta for TestLifecyclePlugin {
        fn plugin_type_id(&self) -> PluginTypeId {
            plugin_ids::ENHANCEMENT_DASHBOARD_ICONS
        }
    }

    impl PluginMetadataOps for TestPluginOps {
        fn get(&self, _id: &PluginTypeId) -> Option<&PluginDescriptor> {
            None
        }
        fn all(&self) -> Vec<&PluginDescriptor> {
            vec![]
        }
    }

    impl PluginConfigOps for TestPluginOps {}

    impl PluginSurfaceActionOps for TestPluginOps {
        fn handle_surface_action<'a>(
            &'a self,
            _ctx: &'a uptrakit_plugin_infrastructure_registry::SurfaceActionContext<'a>,
            _surface_id: &'a str,
            _action_id: &'a str,
            _params: serde_json::Value,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = std::result::Result<serde_json::Value, SurfaceActionError>,
                    > + Send
                    + 'a,
            >,
        > {
            Box::pin(async {
                Err(SurfaceActionError::PluginInternal(
                    "not implemented".to_string(),
                ))
            })
        }
    }

    impl PluginSurfaceOps for TestPluginOps {
        fn surface_registrations(&self) -> Vec<uptrakit_wire::surfaces::SurfaceRegistration> {
            Vec::new()
        }
    }

    impl NotificationOps for TestPluginOps {
        fn transport(
            &self,
            _id: &PluginTypeId,
        ) -> Option<std::sync::Arc<dyn NotificationTransport>> {
            None
        }
        fn notification_supported_types(&self) -> Vec<PluginTypeId> {
            vec![]
        }
    }

    impl SoftwareItemLifecycleOps for TestPluginOps {
        fn on_software_item_created<'a>(
            &'a self,
            event: &'a SoftwareItemCreatedEvent,
            ctx: &'a SoftwareItemLifecycleContext,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Option<SoftwareItemPatch>> + Send + 'a>,
        > {
            Box::pin(async move {
                let enabled = ctx
                    .typed_type_setting::<TestLifecycleTypeSettings>(
                        &plugin_ids::ENHANCEMENT_DASHBOARD_ICONS,
                    )
                    .map(|cfg| cfg.enabled)
                    .unwrap_or(true);

                if !enabled {
                    return None;
                }

                if event.name == "Actual Budget" {
                    Some(
                        SoftwareItemPatch::new().with_icon_url(Some(
                            "https://cdn.example.test/actual-budget.svg".into(),
                        )),
                    )
                } else {
                    None
                }
            })
        }

        fn software_item_lifecycle_plugins(&self) -> &[std::sync::Arc<dyn SoftwareItemLifecycle>] {
            lifecycle_plugins()
        }
    }

    impl ControllerUpdateProtectionOps for TestPluginOps {
        fn controller_update_protection(
            &self,
        ) -> Option<std::sync::Arc<dyn ControllerUpdateProtection>> {
            None
        }
    }

    #[async_trait::async_trait]
    impl crate::cert_signer::AgentCertSigner for TestSuccessfulCertSigner {
        async fn sign_agent_csr(
            &self,
            _csr_pem: &str,
            _agent_id: &Uuid,
            _lifetime: time::Duration,
        ) -> std::result::Result<
            crate::cert_signer::SignedCertBundle,
            rootcause::Report<crate::cert_signer::CertSignerError>,
        > {
            let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
                .expect("key generation should succeed");
            let cert = rcgen::CertificateParams::new(vec!["localhost".to_string()])
                .expect("certificate params should be valid")
                .self_signed(&key_pair)
                .expect("certificate should self-sign");
            let not_after = time::UtcDateTime::from_unix_timestamp(
                (time::OffsetDateTime::now_utc() + time::Duration::days(30)).unix_timestamp(),
            )
            .expect("valid not_after timestamp");

            Ok(crate::cert_signer::SignedCertBundle {
                cert_pem: cert.pem(),
                not_after,
            })
        }

        fn active_ca_fingerprint(&self) -> String {
            TEST_CA_FINGERPRINT.to_string()
        }
    }

    #[async_trait::async_trait]
    impl crate::cert_signer::AgentCertSigner for TestFailingCertSigner {
        async fn sign_agent_csr(
            &self,
            _csr_pem: &str,
            _agent_id: &Uuid,
            _lifetime: time::Duration,
        ) -> std::result::Result<
            crate::cert_signer::SignedCertBundle,
            rootcause::Report<crate::cert_signer::CertSignerError>,
        > {
            Err(rootcause::report!(
                crate::cert_signer::CertSignerError::Signing("forced renewal failure".to_string())
            ))
        }

        fn active_ca_fingerprint(&self) -> String {
            TEST_CA_FINGERPRINT.to_string()
        }
    }
    // ── Fixture helpers ───────────────────────────────────────────────────

    fn state_with_successful_cert_signer(state: &Arc<AppState>) -> Arc<AppState> {
        Arc::new(AppState {
            cert_signer: Arc::new(TestSuccessfulCertSigner),
            ..(**state).clone()
        })
    }

    fn state_with_failing_cert_signer(state: &Arc<AppState>) -> Arc<AppState> {
        Arc::new(AppState {
            cert_signer: Arc::new(TestFailingCertSigner),
            ..(**state).clone()
        })
    }

    fn test_renewal_csr_pem() -> String {
        let key_pair = rcgen::KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256)
            .expect("key generation should succeed");
        let mut params = rcgen::CertificateParams::default();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, Uuid::now_v7().to_string());
        let csr = params
            .serialize_request(&key_pair)
            .expect("csr serialization should succeed");
        csr.pem().expect("csr pem encoding should succeed")
    }

    async fn insert_ca_certificate(db: &sea_orm::DatabaseConnection) {
        let now = time::OffsetDateTime::now_utc();
        ca_certificate::ActiveModel {
            fingerprint: Set(TEST_CA_FINGERPRINT.to_string()),
            cert_pem: Set(
                "-----BEGIN CERTIFICATE-----\ntest\n-----END CERTIFICATE-----\n".to_string(),
            ),
            key_pem: Set(uptrakit_crypto::EncryptedString::new(
                "test-key".to_string(),
                "uptrakit:ca_certificates:key_pem",
            )
            .expect("encrypt test CA key")),
            not_before: Set(now - time::Duration::days(1)),
            not_after: Set(now + time::Duration::days(365)),
            activated_at: Set(now),
            deactivated_at: Set(None),
            created_at: Set(now),
        }
        .insert(db)
        .await
        .expect("insert ca certificate");
    }

    async fn insert_service(
        db: &sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
    ) -> service::Model {
        let id = uuid::Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        service::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set(format!("svc-{}", &id.to_string()[..8])),
            friendly_name: Set(format!("Service {}", &id.to_string()[..8])),
            ip_address: Set(None),
            status: Set(service::ServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("secret-{id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            enrollment_token_id: Set(None),
            cert_lifetime_hours: Set(None),
            service_app_name: Set(None),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(db)
        .await
        .expect("insert service")
    }

    async fn insert_system_service(db: &sea_orm::DatabaseConnection) -> system_service::Model {
        let id = Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        system_service::ActiveModel {
            id: Set(id),
            capabilities: Set("[]".to_string()),
            hostname: Set(format!("sys-{}", &id.to_string()[..8])),
            friendly_name: Set(format!("System Service {}", &id.to_string()[..8])),
            ip_address: Set(None),
            status: Set(system_service::SystemServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("secret-{id}")),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            cert_lifetime_hours: Set(None),
            system_enrollment_token_id: Set(None),
            service_app_name: Set(Some("uptrakit-scheduler".to_string())),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(db)
        .await
        .expect("insert system service")
    }

    async fn wait_for_tenant_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = audit_log::Entity::find()
                .filter(audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query tenant audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected tenant audit row for action {action_type}");
    }

    async fn wait_for_system_audit_row_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> system_audit_log::Model {
        for _ in 0..50 {
            if let Some(row) = system_audit_log::Entity::find()
                .filter(system_audit_log::Column::ActionType.eq(action_type))
                .order_by_desc(system_audit_log::Column::OccurredAt)
                .one(db)
                .await
                .expect("query system audit rows")
            {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        panic!("expected system audit row for action {action_type}");
    }

    async fn tenant_audit_count_for_action(
        db: &sea_orm::DatabaseConnection,
        action_type: uptrakit_audit_log::RegisteredAuditAction,
    ) -> usize {
        audit_log::Entity::find()
            .filter(audit_log::Column::ActionType.eq(action_type))
            .all(db)
            .await
            .expect("query tenant audit count")
            .len()
    }

    fn assert_report_plugin_config_reply(
        response: &ProcessorResponse,
    ) -> &ReportPluginConfigResponsePayload {
        let Some(reply) = response.replies.first() else {
            panic!("expected ReportPluginConfigResponse reply");
        };

        match reply {
            ControllerMessage::ReportPluginConfigResponse(payload) => payload,
            other => panic!("unexpected reply variant: {other:?}"),
        }
    }

    fn report_plugin_config_payload(
        request_id: &str,
        plugin_type: &str,
        name: &str,
        config: serde_json::Value,
    ) -> ReportPluginConfigPayload {
        serde_json::from_value(serde_json::json!({
            "request_id": request_id,
            "plugin_type": plugin_type,
            "name": name,
            "config": config,
        }))
        .expect("ReportPluginConfigPayload JSON is always valid")
    }

    fn assert_certificate_reply(response: &ProcessorResponse) {
        let Some(reply) = response.replies.first() else {
            panic!("expected certificate reply");
        };

        match reply {
            ControllerMessage::Certificate(_) => {}
            ControllerMessage::Error(err) => {
                panic!(
                    "renew response returned error: code={}, message={}",
                    err.code, err.message
                );
            }
            _ => panic!("unexpected renew response variant"),
        }
    }

    fn assert_error_reply(
        response: &ProcessorResponse,
        expected_code: ErrorCode,
        expected_message: &str,
    ) {
        let Some(reply) = response.replies.first() else {
            panic!("expected error reply");
        };

        match reply {
            ControllerMessage::Error(err) => {
                assert_eq!(err.code, expected_code);
                assert_eq!(err.message, expected_message);
            }
            other => panic!("unexpected reply variant: {other:?}"),
        }
    }

    fn assert_error_reply_contains(
        response: &ProcessorResponse,
        expected_code: ErrorCode,
        expected_message_fragment: &str,
    ) {
        let Some(reply) = response.replies.first() else {
            panic!("expected error reply");
        };

        match reply {
            ControllerMessage::Error(err) => {
                assert_eq!(err.code, expected_code);
                assert!(
                    err.message.contains(expected_message_fragment),
                    "expected error message to contain {expected_message_fragment:?}, got {:?}",
                    err.message
                );
            }
            other => panic!("unexpected reply variant: {other:?}"),
        }
    }

    async fn insert_host(db: &sea_orm::DatabaseConnection, tenant_id: uuid::Uuid) -> host::Model {
        let id = uuid::Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        host::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            machine_id: Set(format!("machine-{id}")),
            hostname: Set(format!("host-{id}")),
            friendly_name: Set(format!("Host {id}")),
            os_type: Set(None),
            os_version: Set(None),
            architecture: Set(None),
            ip_address: Set(None),
            host_features: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert host")
    }

    async fn link_service_host(
        db: &sea_orm::DatabaseConnection,
        service_id: uuid::Uuid,
        host_id: uuid::Uuid,
    ) {
        let now = time::OffsetDateTime::now_utc();
        service_host::ActiveModel {
            service_id: Set(service_id),
            host_id: Set(host_id),
            linked_at: Set(now),
        }
        .insert(db)
        .await
        .expect("link service_host");
    }

    async fn insert_software_item(
        db: &sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
    ) -> software_item::Model {
        insert_named_software_item(
            db,
            tenant_id,
            &format!("App-{}", &uuid::Uuid::now_v7().to_string()[..8]),
            false,
        )
        .await
    }

    async fn insert_named_software_item(
        db: &sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
        name: &str,
        featured: bool,
    ) -> software_item::Model {
        let id = uuid::Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        software_item::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            name: Set(name.to_string()),
            featured: Set(featured),
            icon_url: Set(None),
            last_checked_at: Set(None),
            awaiting_restart_timeout: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert software_item")
    }

    async fn insert_host_software_item(
        db: &sea_orm::DatabaseConnection,
        host_id: uuid::Uuid,
        software_item_id: uuid::Uuid,
    ) -> host_software_item::Model {
        let id = uuid::Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        host_software_item::ActiveModel {
            id: Set(id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            qualifier: Set(None),
            plugin_config_id: Set(None),
            package_identifier: Set(None),
            installed_version: Set(None),
            installed_version_detected_at: Set(None),
            installed_display_version: Set(None),
            latest_version: Set(None),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("unknown".to_string()),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert host_software_item")
    }

    async fn insert_plugin_config(
        db: &sea_orm::DatabaseConnection,
        tenant_id: uuid::Uuid,
        plugin_type: &str,
    ) -> plugin_config::Model {
        let id = uuid::Uuid::now_v7();
        let now = time::OffsetDateTime::now_utc();
        plugin_config::ActiveModel {
            id: Set(id),
            tenant_id: Set(tenant_id),
            name: Set(format!("Config-{id}")),
            plugin_type: Set(plugin_type.to_string()),
            config: Set(serde_json::json!({})),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert plugin_config")
    }

    // ── Tests ─────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn handle_renew_certificate_tenant_service_writes_tenant_semantic_audit_row() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (base_state, _jwt) = build_test_state(db.clone(), tenant_id).await;
        let state = state_with_successful_cert_signer(&base_state);
        insert_ca_certificate(&db).await;
        let svc = insert_service(&db, tenant_id).await;

        let response = handle_renew_certificate(
            &state,
            svc.id,
            &crate::routes::service_ws::protocol::CertIdentity {
                serial: "old-serial".to_string(),
                ca_fingerprint: TEST_CA_FINGERPRINT.to_string(),
            },
            &RenewCertificatePayload {
                csr_pem: test_renewal_csr_pem(),
            },
            false,
        )
        .await;
        assert_certificate_reply(&response);

        let row = wait_for_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(svc.id));
        assert_eq!(row.target_type.as_deref(), Some("service"));
        assert_eq!(row.target_id.as_deref(), Some(svc.id.to_string().as_str()));
    }

    #[tokio::test]
    async fn handle_renew_certificate_tenant_service_not_approved_emits_denied_tenant_audit_row() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (base_state, _jwt) = build_test_state(db.clone(), tenant_id).await;
        let state = state_with_successful_cert_signer(&base_state);
        let svc = insert_service(&db, tenant_id).await;

        service::ActiveModel {
            id: Set(svc.id),
            status: Set(service::ServiceStatus::Pending),
            ..Default::default()
        }
        .update(&db)
        .await
        .expect("downgrade tenant service approval");

        let response = handle_renew_certificate(
            &state,
            svc.id,
            &crate::routes::service_ws::protocol::CertIdentity {
                serial: "old-serial".to_string(),
                ca_fingerprint: TEST_CA_FINGERPRINT.to_string(),
            },
            &RenewCertificatePayload {
                csr_pem: test_renewal_csr_pem(),
            },
            false,
        )
        .await;
        assert_error_reply(&response, ErrorCode::Forbidden, "service is not approved");

        let row = wait_for_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("service.certificate_renew details");
        assert_eq!(details["reason_code"], serde_json::json!("not_approved"));
    }

    #[tokio::test]
    async fn handle_renew_certificate_tenant_signing_failure_emits_failed_tenant_audit_row() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (base_state, _jwt) = build_test_state(db.clone(), tenant_id).await;
        let state = state_with_failing_cert_signer(&base_state);
        insert_ca_certificate(&db).await;
        let svc = insert_service(&db, tenant_id).await;

        let response = handle_renew_certificate(
            &state,
            svc.id,
            &crate::routes::service_ws::protocol::CertIdentity {
                serial: "old-serial".to_string(),
                ca_fingerprint: TEST_CA_FINGERPRINT.to_string(),
            },
            &RenewCertificatePayload {
                csr_pem: test_renewal_csr_pem(),
            },
            false,
        )
        .await;
        assert_error_reply_contains(
            &response,
            ErrorCode::CertificateError,
            "forced renewal failure",
        );

        let row = wait_for_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("service.certificate_renew details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("certificate_signing_failed")
        );
    }

    #[tokio::test]
    async fn handle_report_plugin_config_emits_success_tenant_audit_row() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;
        let svc = insert_service(&db, tenant_id).await;

        let payload = report_plugin_config_payload(
            "req-plugin-config-success",
            "generic_shell",
            "Discovered Generic Shell",
            serde_json::json!({
                "version_command": "echo 1.2.3"
            }),
        );

        let response = handle_report_plugin_config(&state, svc.id, &payload).await;
        let reply = assert_report_plugin_config_reply(&response);
        assert!(reply.success);
        let config_id = reply.plugin_config_id.expect("plugin_config_id on success");

        let row = wait_for_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(svc.id));
        assert_eq!(row.target_type.as_deref(), Some("plugin_config"));
        assert_eq!(row.target_id, Some(config_id.to_string()));
        let details = row.details_json.expect("plugin_config.create details");
        assert_eq!(details["plugin_type"], serde_json::json!("generic_shell"));
        assert_eq!(
            details["config_name"],
            serde_json::json!("Discovered Generic Shell")
        );
        assert_eq!(
            details["mutation_source"],
            serde_json::json!("service_ws.report_plugin_config")
        );
        assert!(
            !details.to_string().contains("echo 1.2.3"),
            "semantic audit details must not store raw config content"
        );
    }

    #[tokio::test]
    async fn handle_report_plugin_config_emits_validation_failed_tenant_audit_row_for_invalid_config()
     {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;
        let svc = insert_service(&db, tenant_id).await;

        let payload = report_plugin_config_payload(
            "req-plugin-config-invalid",
            "generic_shell",
            "Invalid Generic Shell",
            serde_json::json!({}),
        );

        let response = handle_report_plugin_config(&state, svc.id, &payload).await;
        let reply = assert_report_plugin_config_reply(&response);
        assert!(!reply.success);
        assert_eq!(reply.plugin_config_id, None);

        let row = wait_for_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::ValidationFailed.as_str()
        );
        let details = row.details_json.expect("plugin_config.create details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("invalid_plugin_config")
        );
    }

    #[tokio::test]
    async fn handle_report_plugin_config_missing_service_emits_denied_system_audit_row() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;

        let payload = report_plugin_config_payload(
            "req-plugin-config-missing-service",
            "generic_shell",
            "Missing Service Config",
            serde_json::json!({
                "version_command": "echo 1.2.3"
            }),
        );

        let response = handle_report_plugin_config(&state, Uuid::now_v7(), &payload).await;
        assert!(response.replies.is_empty());

        let row = wait_for_system_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("plugin_config.create details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("service_not_found")
        );
        let tenant_rows = tenant_audit_count_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
        )
        .await;
        assert_eq!(tenant_rows, 0);
    }

    #[tokio::test]
    async fn handle_report_plugin_config_db_failure_emits_failed_tenant_audit_row() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;
        let svc = insert_service(&db, tenant_id).await;

        db.execute_unprepared("DROP TABLE plugin_configs")
            .await
            .expect("drop plugin_configs table");

        let payload = report_plugin_config_payload(
            "req-plugin-config-db-failure",
            "generic_shell",
            "Broken Storage Config",
            serde_json::json!({
                "version_command": "echo 1.2.3"
            }),
        );

        let response = handle_report_plugin_config(&state, svc.id, &payload).await;
        let reply = assert_report_plugin_config_reply(&response);
        assert!(!reply.success);
        assert_eq!(reply.plugin_config_id, None);

        let row = wait_for_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::PLUGIN_CONFIG_CREATE,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("plugin_config.create details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("create_or_find_failed")
        );
    }

    #[tokio::test]
    async fn handle_renew_certificate_system_service_keeps_writing_system_audit_row() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (base_state, _jwt) = build_test_state(db.clone(), tenant_id).await;
        let state = state_with_successful_cert_signer(&base_state);
        insert_ca_certificate(&db).await;
        let svc = insert_system_service(&db).await;

        let response = handle_renew_certificate(
            &state,
            svc.id,
            &crate::routes::service_ws::protocol::CertIdentity {
                serial: "old-serial".to_string(),
                ca_fingerprint: TEST_CA_FINGERPRINT.to_string(),
            },
            &RenewCertificatePayload {
                csr_pem: test_renewal_csr_pem(),
            },
            true,
        )
        .await;
        assert_certificate_reply(&response);

        let row = wait_for_system_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(svc.id));

        let tenant_rows = tenant_audit_count_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW,
        )
        .await;
        assert_eq!(tenant_rows, 0);
    }

    #[tokio::test]
    async fn handle_renew_certificate_system_service_not_approved_emits_denied_system_audit_row() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (base_state, _jwt) = build_test_state(db.clone(), tenant_id).await;
        let state = state_with_successful_cert_signer(&base_state);
        let svc = insert_system_service(&db).await;

        system_service::ActiveModel {
            id: Set(svc.id),
            status: Set(system_service::SystemServiceStatus::Pending),
            ..Default::default()
        }
        .update(&db)
        .await
        .expect("downgrade system service approval");

        let response = handle_renew_certificate(
            &state,
            svc.id,
            &crate::routes::service_ws::protocol::CertIdentity {
                serial: "old-serial".to_string(),
                ca_fingerprint: TEST_CA_FINGERPRINT.to_string(),
            },
            &RenewCertificatePayload {
                csr_pem: test_renewal_csr_pem(),
            },
            true,
        )
        .await;
        assert_error_reply(&response, ErrorCode::Forbidden, "service is not approved");

        let row = wait_for_system_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );
        let details = row.details_json.expect("service.certificate_renew details");
        assert_eq!(details["reason_code"], serde_json::json!("not_approved"));
    }

    #[tokio::test]
    async fn handle_renew_certificate_system_signing_failure_emits_failed_system_audit_row() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (base_state, _jwt) = build_test_state(db.clone(), tenant_id).await;
        let state = state_with_failing_cert_signer(&base_state);
        insert_ca_certificate(&db).await;
        let svc = insert_system_service(&db).await;

        let response = handle_renew_certificate(
            &state,
            svc.id,
            &crate::routes::service_ws::protocol::CertIdentity {
                serial: "old-serial".to_string(),
                ca_fingerprint: TEST_CA_FINGERPRINT.to_string(),
            },
            &RenewCertificatePayload {
                csr_pem: test_renewal_csr_pem(),
            },
            true,
        )
        .await;
        assert_error_reply_contains(
            &response,
            ErrorCode::CertificateError,
            "forced renewal failure",
        );

        let row = wait_for_system_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SERVICE_CERTIFICATE_RENEW,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Failed.as_str()
        );
        let details = row.details_json.expect("service.certificate_renew details");
        assert_eq!(
            details["reason_code"],
            serde_json::json!("certificate_signing_failed")
        );
    }

    /// When `host_software_item_id` is set in the result, only the targeted row
    /// is updated. The other host's row for the same software item is unchanged.
    #[tokio::test]
    async fn version_check_results_targeted_update_isolates_correct_row() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;

        let svc = insert_service(&db, tenant_id).await;
        let host1 = insert_host(&db, tenant_id).await;
        let host2 = insert_host(&db, tenant_id).await;
        link_service_host(&db, svc.id, host1.id).await;
        link_service_host(&db, svc.id, host2.id).await;

        let sw = insert_software_item(&db, tenant_id).await;
        let hsi1 = insert_host_software_item(&db, host1.id, sw.id).await;
        let hsi2 = insert_host_software_item(&db, host2.id, sw.id).await;

        // Send VersionCheckResults targeting hsi1 only.
        let payload = VersionCheckResultsPayload {
            results: vec![VersionCheckResult {
                software_item_id: sw.id,
                installed_version: Some("2.0.0".to_string()),
                installed_display_version: None,
                latest_version: None,
                error: None,
                update_category: Default::default(),
                host_software_item_id: Some(hsi1.id),
                not_ready: None,
            }],
        };

        handle_version_check_results(&state, svc.id, &payload).await;

        // hsi1 must reflect the new version.
        let updated = host_software_item::Entity::find_by_id(hsi1.id)
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert_eq!(updated.installed_version, Some("2.0.0".to_string()));

        // hsi2 must be unchanged (no cross-host contamination).
        let unchanged = host_software_item::Entity::find_by_id(hsi2.id)
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert_eq!(unchanged.installed_version, None);
    }

    /// When `host_software_item_id` points to a row belonging to a *different*
    /// service's host, the update must be rejected (security guard).
    #[tokio::test]
    async fn version_check_results_targeted_update_rejects_foreign_hsi_id() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;

        // Service A owns host_a; service B owns host_b.
        let svc_a = insert_service(&db, tenant_id).await;
        let svc_b = insert_service(&db, tenant_id).await;
        let host_a = insert_host(&db, tenant_id).await;
        let host_b = insert_host(&db, tenant_id).await;
        link_service_host(&db, svc_a.id, host_a.id).await;
        link_service_host(&db, svc_b.id, host_b.id).await;

        let sw = insert_software_item(&db, tenant_id).await;
        let hsi_a = insert_host_software_item(&db, host_a.id, sw.id).await;
        let hsi_b = insert_host_software_item(&db, host_b.id, sw.id).await;

        // Service A sends a result pointing at hsi_b (belongs to service B).
        let payload = VersionCheckResultsPayload {
            results: vec![VersionCheckResult {
                software_item_id: sw.id,
                installed_version: Some("evil".to_string()),
                installed_display_version: None,
                latest_version: None,
                error: None,
                update_category: Default::default(),
                host_software_item_id: Some(hsi_b.id),
                not_ready: None,
            }],
        };

        handle_version_check_results(&state, svc_a.id, &payload).await;

        // hsi_b must not be modified — the host_ids guard filters it out.
        let untouched = host_software_item::Entity::find_by_id(hsi_b.id)
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert_eq!(untouched.installed_version, None);

        // hsi_a is also untouched (wrong hsi_id was provided).
        let untouched_a = host_software_item::Entity::find_by_id(hsi_a.id)
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert_eq!(untouched_a.installed_version, None);
    }

    #[tokio::test]
    async fn version_check_results_error_result_preserves_targeted_row_while_success_updates_peer_row()
     {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;

        let svc = insert_service(&db, tenant_id).await;
        let host_error = insert_host(&db, tenant_id).await;
        let host_success = insert_host(&db, tenant_id).await;
        link_service_host(&db, svc.id, host_error.id).await;
        link_service_host(&db, svc.id, host_success.id).await;

        let sw = insert_software_item(&db, tenant_id).await;
        let hsi_error = insert_host_software_item(&db, host_error.id, sw.id).await;
        let hsi_success = insert_host_software_item(&db, host_success.id, sw.id).await;

        let preserved_detected_at =
            time::OffsetDateTime::from_unix_timestamp(1_700_000_000).expect("valid unix timestamp");
        let preserved_fetched_at =
            time::OffsetDateTime::from_unix_timestamp(1_700_000_100).expect("valid unix timestamp");
        let success_seed_detected_at =
            time::OffsetDateTime::from_unix_timestamp(1_700_000_200).expect("valid unix timestamp");
        let success_seed_fetched_at =
            time::OffsetDateTime::from_unix_timestamp(1_700_000_300).expect("valid unix timestamp");

        host_software_item::ActiveModel {
            id: Set(hsi_error.id),
            installed_version: Set(Some("1.0.0".to_string())),
            installed_version_detected_at: Set(Some(preserved_detected_at)),
            installed_display_version: Set(Some("1.0.0+baseline".to_string())),
            latest_version: Set(Some("1.0.1".to_string())),
            latest_version_fetched_at: Set(Some(preserved_fetched_at)),
            update_category: Set(UpdateCategory::Bugfix.to_string()),
            ..Default::default()
        }
        .update(&db)
        .await
        .expect("seed error-targeted host_software_item");

        let preserved_baseline = host_software_item::Entity::find_by_id(hsi_error.id)
            .one(&db)
            .await
            .expect("query")
            .expect("row");

        host_software_item::ActiveModel {
            id: Set(hsi_success.id),
            installed_version: Set(Some("0.9.0".to_string())),
            installed_version_detected_at: Set(Some(success_seed_detected_at)),
            installed_display_version: Set(Some("0.9.0+seed".to_string())),
            latest_version: Set(Some("0.9.9".to_string())),
            latest_version_fetched_at: Set(Some(success_seed_fetched_at)),
            update_category: Set(UpdateCategory::Unknown.to_string()),
            ..Default::default()
        }
        .update(&db)
        .await
        .expect("seed success-targeted host_software_item");

        let payload = VersionCheckResultsPayload {
            results: vec![
                VersionCheckResult {
                    software_item_id: sw.id,
                    installed_version: Some("9.9.9-should-not-apply".to_string()),
                    installed_display_version: Some("should-not-apply-display".to_string()),
                    latest_version: Some("10.0.0-should-not-apply".to_string()),
                    error: Some("registry unavailable".to_string()),
                    update_category: UpdateCategory::Feature,
                    host_software_item_id: Some(hsi_error.id),
                    not_ready: None,
                },
                VersionCheckResult {
                    software_item_id: sw.id,
                    installed_version: Some("2.0.0".to_string()),
                    installed_display_version: Some("2.0.0+stable".to_string()),
                    latest_version: Some("2.1.0".to_string()),
                    error: None,
                    update_category: UpdateCategory::Security,
                    host_software_item_id: Some(hsi_success.id),
                    not_ready: None,
                },
            ],
        };

        handle_version_check_results(&state, svc.id, &payload).await;

        let preserved_after = host_software_item::Entity::find_by_id(hsi_error.id)
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        let success_after = host_software_item::Entity::find_by_id(hsi_success.id)
            .one(&db)
            .await
            .expect("query")
            .expect("row");

        assert_eq!(
            preserved_after.installed_version,
            preserved_baseline.installed_version
        );
        assert_eq!(
            preserved_after.installed_display_version,
            preserved_baseline.installed_display_version
        );
        assert_eq!(
            preserved_after.installed_version_detected_at,
            preserved_baseline.installed_version_detected_at
        );
        assert_eq!(
            preserved_after.latest_version,
            preserved_baseline.latest_version
        );
        assert_eq!(
            preserved_after.latest_version_fetched_at,
            preserved_baseline.latest_version_fetched_at
        );
        assert_eq!(
            preserved_after.update_category,
            preserved_baseline.update_category
        );

        assert_eq!(success_after.installed_version, Some("2.0.0".to_string()));
        assert_eq!(
            success_after.installed_display_version,
            Some("2.0.0+stable".to_string())
        );
        assert!(success_after.installed_version_detected_at.is_some());
        assert_ne!(
            success_after.installed_version_detected_at,
            Some(success_seed_detected_at)
        );
        assert_eq!(success_after.latest_version, Some("2.1.0".to_string()));
        assert!(success_after.latest_version_fetched_at.is_some());
        assert_ne!(
            success_after.latest_version_fetched_at,
            Some(success_seed_fetched_at)
        );
        assert_eq!(
            success_after.update_category,
            UpdateCategory::Security.to_string()
        );
    }

    #[tokio::test]
    async fn version_check_results_targeted_update_skips_deactivated_host() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;

        let svc = insert_service(&db, tenant_id).await;
        let host = insert_host(&db, tenant_id).await;
        link_service_host(&db, svc.id, host.id).await;

        let sw = insert_software_item(&db, tenant_id).await;
        let hsi = insert_host_software_item(&db, host.id, sw.id).await;

        host::ActiveModel {
            id: Set(host.id),
            deactivated_at: Set(Some(time::OffsetDateTime::now_utc())),
            ..host.into()
        }
        .update(&db)
        .await
        .expect("deactivate host");

        let payload = VersionCheckResultsPayload {
            results: vec![VersionCheckResult {
                software_item_id: sw.id,
                installed_version: Some("2.0.0".to_string()),
                installed_display_version: None,
                latest_version: None,
                error: None,
                update_category: Default::default(),
                host_software_item_id: Some(hsi.id),
                not_ready: None,
            }],
        };

        handle_version_check_results(&state, svc.id, &payload).await;

        let unchanged = host_software_item::Entity::find_by_id(hsi.id)
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert_eq!(unchanged.installed_version, None);
    }

    #[tokio::test]
    async fn enrich_discovered_items_defaults_to_enabled_when_type_setting_missing() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let plugin_ops: Arc<dyn PluginOps> = Arc::new(TestPluginOps);
        let (state, _jwt) =
            build_test_state_with_plugin_ops(db.clone(), tenant_id, Some(plugin_ops)).await;
        let svc = insert_service(&db, tenant_id).await;

        let item = insert_named_software_item(&db, tenant_id, "Actual Budget", true).await;

        enrich_discovered_items(&state, &svc).await;

        let updated = software_item::Entity::find_by_id(item.id)
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert_eq!(
            updated.icon_url.as_deref(),
            Some("https://cdn.example.test/actual-budget.svg")
        );
    }

    #[tokio::test]
    async fn enrich_discovered_items_respects_explicit_disabled_lifecycle_setting() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let plugin_ops: Arc<dyn PluginOps> = Arc::new(TestPluginOps);
        let (state, _jwt) =
            build_test_state_with_plugin_ops(db.clone(), tenant_id, Some(plugin_ops)).await;
        let svc = insert_service(&db, tenant_id).await;

        crate::queries::plugin_type_settings::upsert_type_settings(
            &db,
            tenant_id,
            plugin_ids::ENHANCEMENT_DASHBOARD_ICONS.as_str(),
            serde_json::json!({ "enabled": false }),
        )
        .await
        .expect("save lifecycle type setting");

        let item = insert_named_software_item(&db, tenant_id, "Actual Budget", true).await;

        enrich_discovered_items(&state, &svc).await;

        let updated = software_item::Entity::find_by_id(item.id)
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert_eq!(updated.icon_url, None);
    }

    #[tokio::test]
    async fn handle_version_check_results_emits_version_check_completed_audit_summary() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;

        let svc = insert_service(&db, tenant_id).await;
        let host_error = insert_host(&db, tenant_id).await;
        let host_success = insert_host(&db, tenant_id).await;
        link_service_host(&db, svc.id, host_error.id).await;
        link_service_host(&db, svc.id, host_success.id).await;

        let sw = insert_software_item(&db, tenant_id).await;
        let hsi_error = insert_host_software_item(&db, host_error.id, sw.id).await;
        let hsi_success = insert_host_software_item(&db, host_success.id, sw.id).await;

        let payload = VersionCheckResultsPayload {
            results: vec![
                VersionCheckResult {
                    software_item_id: sw.id,
                    installed_version: Some("9.9.9-should-not-apply".to_string()),
                    installed_display_version: None,
                    latest_version: Some("10.0.0".to_string()),
                    error: Some("registry unavailable".to_string()),
                    update_category: Default::default(),
                    host_software_item_id: Some(hsi_error.id),
                    not_ready: None,
                },
                VersionCheckResult {
                    software_item_id: sw.id,
                    installed_version: Some("2.0.0".to_string()),
                    installed_display_version: None,
                    latest_version: Some("2.1.0".to_string()),
                    error: None,
                    update_category: Default::default(),
                    host_software_item_id: Some(hsi_success.id),
                    not_ready: None,
                },
            ],
        };

        handle_version_check_results(&state, svc.id, &payload).await;

        let row = wait_for_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_VERSION_CHECK_COMPLETED,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Partial.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(svc.id));
        assert_eq!(row.target_type.as_deref(), Some("service"));
        assert_eq!(row.target_id.as_deref(), Some(svc.id.to_string().as_str()));
        let details = row
            .details_json
            .expect("software.version_check.completed details");
        assert_eq!(details["result_count"], serde_json::json!(2));
        assert_eq!(details["success_count"], serde_json::json!(1));
        assert_eq!(details["error_count"], serde_json::json!(1));
        assert_eq!(details["rows_mutated"], serde_json::json!(1));
    }

    #[tokio::test]
    async fn enrich_discovered_items_emits_software_item_enrich_audit_summary() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let plugin_ops: Arc<dyn PluginOps> = Arc::new(TestPluginOps);
        let (state, _jwt) =
            build_test_state_with_plugin_ops(db.clone(), tenant_id, Some(plugin_ops)).await;

        let svc = insert_service(&db, tenant_id).await;
        let item = insert_named_software_item(&db, tenant_id, "Actual Budget", true).await;

        enrich_discovered_items(&state, &svc).await;

        let updated = software_item::Entity::find_by_id(item.id)
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert_eq!(
            updated.icon_url.as_deref(),
            Some("https://cdn.example.test/actual-budget.svg")
        );

        let row = wait_for_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::SOFTWARE_ITEM_ENRICH,
        )
        .await;
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(svc.id));
        assert_eq!(row.target_type.as_deref(), Some("service"));
        assert_eq!(row.target_id.as_deref(), Some(svc.id.to_string().as_str()));
        let details = row.details_json.expect("software_item.enrich details");
        assert_eq!(details["patched_count"], serde_json::json!(1));
        assert_eq!(details["patch_failed_count"], serde_json::json!(0));
        assert_eq!(details["examined_count"], serde_json::json!(1));
    }

    #[tokio::test]
    async fn handle_report_hosts_embedded_service_does_not_report_machine_id_to_notifier() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;
        let notifier = Arc::new(TestEmbeddedNotifier::default());
        let state = Arc::new(AppState {
            embedded_service_notifier: Some(notifier.clone()),
            ..(*state).clone()
        });

        let service_id = Uuid::now_v7();
        insert_embedded_service(&db, tenant_id, service_id).await;

        let payload = ReportHostsPayload {
            hosts: vec![HostInfo {
                machine_id: "embedded-machine".to_string(),
                os_type: Some("macos".to_string()),
                os_version: Some("macOS 26.2".to_string()),
                architecture: Some("aarch64".to_string()),
                hostname: Some("MacBook-Pro---Andrey.local".to_string()),
                ip_address: None,
                agent_host_id: None,
                features: None,
            }],
            agent_version: "0.0.1".to_string(),
            capabilities: [Capability::SoftwareDiscovery, Capability::UpdateHooks]
                .into_iter()
                .collect(),
        };
        let linked_host_ids = Arc::new(parking_lot::Mutex::new(HashSet::new()));

        handle_report_hosts(&state, service_id, &payload, &linked_host_ids).await;

        assert!(notifier.machine_ids.lock().is_empty());
    }

    #[tokio::test]
    async fn handle_report_hosts_emits_host_update_audit_summary_on_success() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;
        let svc = insert_service(&db, tenant_id).await;

        let payload = ReportHostsPayload {
            hosts: vec![HostInfo {
                machine_id: "machine-success".to_string(),
                os_type: Some("linux".to_string()),
                os_version: Some("6.8".to_string()),
                architecture: Some("x86_64".to_string()),
                hostname: Some("host-success".to_string()),
                ip_address: Some("192.0.2.10".to_string()),
                agent_host_id: None,
                features: None,
            }],
            agent_version: "1.2.3".to_string(),
            capabilities: [Capability::SoftwareDiscovery].into_iter().collect(),
        };
        let linked_host_ids = Arc::new(parking_lot::Mutex::new(HashSet::new()));
        handle_report_hosts(&state, svc.id, &payload, &linked_host_ids).await;

        let row = wait_for_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::HOST_UPDATE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(svc.id));
        assert_eq!(row.target_type.as_deref(), Some("service"));
        assert_eq!(row.target_id.as_deref(), Some(svc.id.to_string().as_str()));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );

        let details = row.details_json.expect("host.update details");
        assert_eq!(details["created_hosts"].as_u64(), Some(1));
        assert_eq!(details["updated_hosts"].as_u64(), Some(0));
        assert_eq!(details["failed_hosts"].as_u64(), Some(0));
    }

    #[tokio::test]
    async fn handle_report_hosts_emits_host_update_audit_summary_partial_when_some_hosts_fail() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;
        let svc = insert_service(&db, tenant_id).await;

        let duplicate_host_id = Uuid::now_v7();
        let payload = ReportHostsPayload {
            hosts: vec![
                HostInfo {
                    machine_id: "machine-partial-a".to_string(),
                    os_type: Some("linux".to_string()),
                    os_version: Some("6.8".to_string()),
                    architecture: Some("x86_64".to_string()),
                    hostname: Some("host-partial-a".to_string()),
                    ip_address: Some("192.0.2.20".to_string()),
                    agent_host_id: Some(duplicate_host_id),
                    features: None,
                },
                HostInfo {
                    machine_id: "machine-partial-b".to_string(),
                    os_type: Some("linux".to_string()),
                    os_version: Some("6.8".to_string()),
                    architecture: Some("x86_64".to_string()),
                    hostname: Some("host-partial-b".to_string()),
                    ip_address: Some("192.0.2.21".to_string()),
                    agent_host_id: Some(duplicate_host_id),
                    features: None,
                },
            ],
            agent_version: "1.2.3".to_string(),
            capabilities: [Capability::SoftwareDiscovery].into_iter().collect(),
        };
        let linked_host_ids = Arc::new(parking_lot::Mutex::new(HashSet::new()));
        handle_report_hosts(&state, svc.id, &payload, &linked_host_ids).await;

        let row = wait_for_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::HOST_UPDATE,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(svc.id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Partial.as_str()
        );

        let details = row.details_json.expect("host.update details");
        assert_eq!(details["created_hosts"].as_u64(), Some(1));
        assert_eq!(details["failed_hosts"].as_u64(), Some(1));
    }

    #[tokio::test]
    async fn handle_discovery_results_emits_host_discover_audit_summary_on_success() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;
        let svc = insert_service(&db, tenant_id).await;
        let host = insert_host(&db, tenant_id).await;
        link_service_host(&db, svc.id, host.id).await;
        let plugin_config = insert_plugin_config(
            &db,
            tenant_id,
            uptrakit_shared_types::plugin_ids::PACKAGE_MANAGER_HOMEBREW.as_str(),
        )
        .await;
        let mut report_tracker = ReportTracker::new();

        let payload = DiscoveryResultsPayload {
            host_machine_id: host.machine_id.clone(),
            results: vec![DiscoveryPluginResult {
                plugin_config_id: Some(plugin_config.id),
                plugin_type: uptrakit_shared_types::plugin_ids::PACKAGE_MANAGER_HOMEBREW.clone(),
                discoveries: vec![DiscoveredSoftware {
                    package_identifier: "wget".to_string(),
                    name: "Wget".to_string(),
                    installed_version: "1.0.0".to_string(),
                    targets: vec![],
                    extra: None,
                    qualifier: None,
                    plugin_package_identifier: None,
                    featured: false,
                    installed_display_version: None,
                }],
                error: None,
            }],
        };

        handle_discovery_results(&state, svc.id, payload, None, &mut report_tracker).await;

        let row = wait_for_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::HOST_DISCOVER,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(svc.id));
        assert_eq!(row.target_type.as_deref(), Some("host"));
        assert_eq!(row.target_id.as_deref(), Some(host.id.to_string().as_str()));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Success.as_str()
        );
        let details = row.details_json.expect("host.discover details");
        assert_eq!(details["plugin_results"].as_u64(), Some(1));
    }

    #[tokio::test]
    async fn handle_discovery_results_emits_host_discover_audit_summary_for_unknown_machine_id() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let (state, _jwt) = build_test_state(db.clone(), tenant_id).await;
        let svc = insert_service(&db, tenant_id).await;
        let mut report_tracker = ReportTracker::new();

        let payload = DiscoveryResultsPayload {
            host_machine_id: "missing-machine".to_string(),
            results: vec![],
        };

        handle_discovery_results(&state, svc.id, payload, None, &mut report_tracker).await;

        let row = wait_for_tenant_audit_row_for_action(
            &db,
            uptrakit_audit_log::AuditActionType::HOST_DISCOVER,
        )
        .await;
        assert_eq!(
            row.actor_type,
            uptrakit_audit_log::AuditActorType::Service.as_str()
        );
        assert_eq!(row.actor_id, Some(svc.id));
        assert_eq!(
            row.outcome,
            uptrakit_audit_log::AuditOutcome::Denied.as_str()
        );

        let details = row.details_json.expect("host.discover details");
        assert_eq!(
            details["reason_code"].as_str(),
            Some("unknown_host_machine_id")
        );
    }
}
