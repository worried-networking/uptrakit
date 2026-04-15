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

use uptrakit_internal_wire::report_tracker::{PageOutcome, ReportTracker};
use uptrakit_internal_wire::{
    CertificatePayload, CloseReason, ControllerMessage, DiscoveryResultsPayload, ErrorCode,
    ErrorPayload, HostConnectivityUpdate, OutgoingSeq, ReportHostsPayload, ReportPagination,
    ReportPluginConfigPayload, ReportPluginConfigResponsePayload, RequestCrlRenewalPayload,
    VersionCheckResultsPayload,
};
use uptrakit_shared_db::entity::{host, host_software_item, service, service_host, software_item};

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
    payload: &uptrakit_internal_wire::RenewCertificatePayload,
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
                ProcessorResponse::reply_and_close(cert_msg, CloseReason::CertificateRotated)
            }
            Err(e) => ProcessorResponse::reply_and_break(ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::CertificateError,
                message: e.to_string(),
            })),
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
                ProcessorResponse::reply_and_close(cert_msg, CloseReason::CertificateRotated)
            }
            Err(e) => ProcessorResponse::reply_and_break(ControllerMessage::Error(ErrorPayload {
                code: ErrorCode::CertificateError,
                message: e.to_string(),
            })),
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
) {
    for host_info in &payload.hosts {
        if host_info.machine_id != "unknown"
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
                // New host -- trigger autodiscovery.
                trigger_discovery_for_agent_host(
                    state,
                    service_id,
                    service_model.tenant_id,
                    host_id,
                    &host_info.machine_id,
                )
                .await;
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    machine_id = %host_info.machine_id,
                    "failed to link host"
                );
            }
        }
    }
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

    link_reported_hosts(state, service_id, &service_model, payload).await;

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
    result: &uptrakit_internal_wire::VersionCheckResult,
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
    result: &uptrakit_internal_wire::VersionCheckResult,
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
    result: &uptrakit_internal_wire::VersionCheckResult,
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

    let host_ids: Vec<uuid::Uuid> = match service_host::Entity::find()
        .filter(service_host::Column::ServiceId.eq(service_id))
        .all(state.db())
        .await
    {
        Ok(links) => links.into_iter().map(|l| l.host_id).collect(),
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

    // Look up tenant_id once; reused per-result for notifications.
    let svc_tenant_id = match service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(svc)) => Some(svc.tenant_id),
        Ok(None) => {
            tracing::warn!(%service_id, "service not found for version check results");
            None
        }
        Err(e) => {
            tracing::warn!(%service_id, error = %e, "failed to look up service");
            None
        }
    };

    // Collect (host_id, software_item_id) pairs for successful results so we
    // can emit VersionCheckCompleted SSE events after the DB work is done.
    let mut completed_pairs: Vec<(uuid::Uuid, uuid::Uuid)> = Vec::new();

    for result in &payload.results {
        if result.error.is_some() {
            tracing::debug!(
                software_item_id = %result.software_item_id,
                host_software_item_id = ?result.host_software_item_id,
                error = ?result.error,
                "skipping version result with error; existing DB state preserved"
            );
            continue;
        }

        let matching_rows =
            resolve_matching_host_software_items(state.db(), service_id, result, &host_ids).await;

        if matching_rows.is_empty() {
            continue;
        }

        let matching_host_ids: Vec<uuid::Uuid> = matching_rows.iter().map(|r| r.host_id).collect();
        let matching_ids: Vec<uuid::Uuid> = matching_rows.iter().map(|r| r.id).collect();

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
    service_id: uuid::Uuid,
    host_id: uuid::Uuid,
    payload: DiscoveryResultsPayload,
    page_outcome: PageOutcome,
    pagination: Option<&ReportPagination>,
    report_tracker: &mut ReportTracker,
) {
    let Ok(Some(svc)) = service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    else {
        return;
    };

    let this_page_count: u32 = payload
        .results
        .iter()
        .filter(|r| r.error.is_none())
        .map(|r| r.discoveries.len() as u32)
        .sum();

    if let Err(e) = crate::queries::autodiscovery::process_discovery_results(
        state.db(),
        service_id,
        svc.tenant_id,
        host_id,
        payload,
    )
    .await
    {
        tracing::warn!(
            error = %e,
            %service_id,
            "failed to process discovery results"
        );
        return;
    }

    // Fire software-item lifecycle plugins on newly discovered items that may
    // benefit from enrichment (e.g. icon assignment from Dashboard Icons).
    enrich_discovered_items(state, svc.tenant_id).await;

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
            process_discovery_page_for_host(
                state,
                service_id,
                host_id,
                payload,
                page_outcome,
                pagination,
                report_tracker,
            )
            .await;
        }
        None => {
            tracing::warn!(
                %service_id,
                host_machine_id = %host_machine_id,
                "received DiscoveryResults for unknown host machine_id"
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

    // Resolve tenant_id from the service
    let tenant_id = match service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(svc)) => svc.tenant_id,
        Ok(None) => {
            tracing::warn!(%service_id, "ReportPluginConfig: service not found");
            return ProcessorResponse::cont();
        }
        Err(e) => {
            tracing::warn!(%service_id, error = %e, "ReportPluginConfig: DB error");
            return ProcessorResponse::cont();
        }
    };

    // Find or create the plugin config
    let result = crate::queries::autodiscovery::find_or_create_default_plugin_config(
        state.db(),
        tenant_id,
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
async fn enrich_discovered_items(state: &AppState, tenant_id: uuid::Uuid) {
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
                } else {
                    tracing::trace!(item_id = %item.id, name = %item.name, "lifecycle patch applied");
                }
            }
            None => {
                tracing::trace!(item_id = %item.id, name = %item.name, "lifecycle plugin produced no patch");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(all(test, feature = "db-sqlite"))]
mod tests {
    use super::*;
    use sea_orm::{ActiveModelTrait, Set};
    use serde::Deserialize;
    use std::sync::{Arc, OnceLock};
    use uptrakit_internal_wire::{UpdateCategory, VersionCheckResult, VersionCheckResultsPayload};
    use uptrakit_plugin_infrastructure_registry::{
        NotificationOps, NotificationTransport, PluginConfigOps, PluginDescriptor,
        PluginExtensionOps, PluginMetadataOps, PluginOps, PluginTypeId, SoftwareItemCreatedEvent,
        SoftwareItemLifecycle, SoftwareItemLifecycleContext, SoftwareItemLifecycleOps,
        SoftwareItemPatch, plugin_ids,
    };
    use uptrakit_shared_db::entity::{
        host, host_software_item, service, service_host, software_item,
    };

    use crate::test_harness::{
        build_test_state, build_test_state_with_plugin_ops, insert_default_tenant,
        setup_migrated_db,
    };

    struct TestPluginOps;
    struct TestLifecyclePlugin;

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

    impl PluginExtensionOps for TestPluginOps {
        fn extension_manifests_and_actions(
            &self,
        ) -> Vec<(
            uptrakit_extension_framework::ExtensionManifest,
            Vec<uptrakit_extension_framework::ActionDef>,
            Option<PluginTypeId>,
        )> {
            vec![]
        }

        fn handle_extension_action<'a>(
            &'a self,
            _ctx: &'a uptrakit_plugin_infrastructure_registry::ExtensionActionContext<'a>,
            _ext_id: &'a str,
            _action_id: &'a str,
            _params: serde_json::Value,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = std::result::Result<serde_json::Value, String>>
                    + Send
                    + 'a,
            >,
        > {
            Box::pin(async { Err("not implemented".to_string()) })
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

    // ── Fixture helpers ───────────────────────────────────────────────────

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

    // ── Tests ─────────────────────────────────────────────────────────────

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
                },
                VersionCheckResult {
                    software_item_id: sw.id,
                    installed_version: Some("2.0.0".to_string()),
                    installed_display_version: Some("2.0.0+stable".to_string()),
                    latest_version: Some("2.1.0".to_string()),
                    error: None,
                    update_category: UpdateCategory::Security,
                    host_software_item_id: Some(hsi_success.id),
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
    async fn enrich_discovered_items_defaults_to_enabled_when_type_setting_missing() {
        let db = setup_migrated_db().await;
        let tenant_id = insert_default_tenant(&db).await;
        let plugin_ops: Arc<dyn PluginOps> = Arc::new(TestPluginOps);
        let (state, _jwt) =
            build_test_state_with_plugin_ops(db.clone(), tenant_id, Some(plugin_ops)).await;

        let item = insert_named_software_item(&db, tenant_id, "Actual Budget", true).await;

        enrich_discovered_items(&state, tenant_id).await;

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

        crate::queries::plugin_type_settings::upsert_type_settings(
            &db,
            tenant_id,
            plugin_ids::ENHANCEMENT_DASHBOARD_ICONS.as_str(),
            serde_json::json!({ "enabled": false }),
        )
        .await
        .expect("save lifecycle type setting");

        let item = insert_named_software_item(&db, tenant_id, "Actual Budget", true).await;

        enrich_discovered_items(&state, tenant_id).await;

        let updated = software_item::Entity::find_by_id(item.id)
            .one(&db)
            .await
            .expect("query")
            .expect("row");
        assert_eq!(updated.icon_url, None);
    }
}
