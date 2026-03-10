//! Common message handlers extracted from the authenticated loop.
//!
//! Each function corresponds to one match arm in the main dispatch and returns
//! a [`LoopAction`] to tell the caller whether to `continue` or `break`.

use std::collections::HashSet;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use futures_util::SinkExt;
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
use super::updates::load_linked_host_ids;
use crate::AppState;
use crate::mqtt_lease_coordinator::MqttLeaseCoordinator;
use crate::notifications::events::{NotificationEvent, NotificationEventDetails};
use crate::routes::agents::{
    find_or_create_host_and_link, revoke_certificate, revoke_system_certificate,
};
use crate::routes::service_ws::protocol::{
    CertIdentity, close_with_reason, record_service_activity, record_system_service_activity,
    send_pong, serialize_controller_msg,
};

// ---------------------------------------------------------------------------
// handle_ping
// ---------------------------------------------------------------------------

/// Handle a `Ping` message: send pong, record activity, optional MQTT heartbeat.
#[tracing::instrument(skip_all, fields(%service_id))]
pub(super) async fn handle_ping(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    out_seq: &mut OutgoingSeq,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    service_ts: i64,
    lease_coordinator: Option<&MqttLeaseCoordinator>,
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
    // MQTT heartbeat
    if let Some(lc) = lease_coordinator
        && let Err(e) = lc.record_heartbeat(&service_id).await
    {
        tracing::warn!(
            error = %e,
            "failed to record heartbeat"
        );
    }
    LoopAction::Continue
}

// ---------------------------------------------------------------------------
// handle_renew_certificate
// ---------------------------------------------------------------------------

/// Handle a `RenewCertificate` message: verify approved, sign renewal CSR,
/// revoke old cert.
#[tracing::instrument(skip_all, fields(%service_id, is_system))]
pub(super) async fn handle_renew_certificate(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    out_seq: &mut OutgoingSeq,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    cert: &CertIdentity,
    payload: &uptrakit_internal_wire::RenewCertificatePayload,
    is_system: bool,
) -> LoopAction {
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
                let err = ControllerMessage::Error(ErrorPayload {
                    code: ErrorCode::Forbidden,
                    message: "service is not approved".to_string(),
                });
                if let Some(json) = serialize_controller_msg(out_seq, err) {
                    let _ = sink.send(Message::Text(json.into())).await;
                }
                return LoopAction::Break;
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
                let cert_msg = ControllerMessage::Certificate(CertificatePayload {
                    cert_pem: bundle.cert_pem,
                    not_after: bundle.not_after,
                });
                if let Some(json) = serialize_controller_msg(out_seq, cert_msg) {
                    let _ = sink.send(Message::Text(json.into())).await;
                }

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
                state.revocation_notify.notify_one();
                state
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
                let _ = close_with_reason(sink, CloseReason::CertificateRotated).await;
                LoopAction::Break
            }
            Err(e) => {
                let err = ControllerMessage::Error(ErrorPayload {
                    code: ErrorCode::CertificateError,
                    message: e.to_string(),
                });
                if let Some(json) = serialize_controller_msg(out_seq, err) {
                    let _ = sink.send(Message::Text(json.into())).await;
                }
                LoopAction::Break
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
                let err = ControllerMessage::Error(ErrorPayload {
                    code: ErrorCode::Forbidden,
                    message: "service is not approved".to_string(),
                });
                if let Some(json) = serialize_controller_msg(out_seq, err) {
                    let _ = sink.send(Message::Text(json.into())).await;
                }
                return LoopAction::Break;
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
                let cert_msg = ControllerMessage::Certificate(CertificatePayload {
                    cert_pem: bundle.cert_pem,
                    not_after: bundle.not_after,
                });
                if let Some(json) = serialize_controller_msg(out_seq, cert_msg) {
                    let _ = sink.send(Message::Text(json.into())).await;
                }

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
                state.revocation_notify.notify_one();
                state
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
                let _ = close_with_reason(sink, CloseReason::CertificateRotated).await;
                LoopAction::Break
            }
            Err(e) => {
                let err = ControllerMessage::Error(ErrorPayload {
                    code: ErrorCode::CertificateError,
                    message: e.to_string(),
                });
                if let Some(json) = serialize_controller_msg(out_seq, err) {
                    let _ = sink.send(Message::Text(json.into())).await;
                }
                LoopAction::Break
            }
        }
    }
}

// ---------------------------------------------------------------------------
// handle_report_hosts
// ---------------------------------------------------------------------------

/// Handle a `ReportHosts` message: update `client_version`, find/create hosts,
/// trigger discovery, refresh `linked_host_ids`.
#[tracing::instrument(skip_all, fields(%service_id, host_count = payload.hosts.len()))]
pub(super) async fn handle_report_hosts(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    out_seq: &mut OutgoingSeq,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &ReportHostsPayload,
    linked_host_ids: &mut HashSet<uuid::Uuid>,
) -> LoopAction {
    // Suppress unused-variable warnings -- sink and out_seq are part of the
    // standard handler signature but not needed for ReportHosts.
    let _ = (sink, out_seq);

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
        _ => return LoopAction::Continue,
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

    for host_info in &payload.hosts {
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

    // Refresh cached host IDs.
    if let Ok(ids) = load_linked_host_ids(state.db(), service_id).await {
        *linked_host_ids = ids;
    }

    // Notify MQTT services that this agent's hosts are online.
    if !linked_host_ids.is_empty() {
        let now = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        let agent_version = payload.agent_version.clone();
        let updates: Vec<HostConnectivityUpdate> = linked_host_ids
            .iter()
            .map(|&host_id| {
                HostConnectivityUpdate::online(
                    host_id,
                    Some(now.clone()),
                    Some(agent_version.clone()),
                )
            })
            .collect();
        state
            .notification_service
            .send_connectivity_update(service_model.tenant_id, updates)
            .await;
    }

    LoopAction::Continue
}

// ---------------------------------------------------------------------------
// handle_version_check_results
// ---------------------------------------------------------------------------

/// Handle a `VersionCheckResults` message: update installed versions, upsert
/// available versions, batch update `last_checked_at`, push software states.
#[tracing::instrument(skip_all, fields(%service_id, result_count = payload.results.len()))]
pub(super) async fn handle_version_check_results(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &VersionCheckResultsPayload,
) -> LoopAction {
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
            return LoopAction::Continue;
        }
    };

    if host_ids.is_empty() {
        tracing::debug!(
            %service_id,
            "no hosts linked, skipping version updates"
        );
        return LoopAction::Continue;
    }

    let now = time::OffsetDateTime::now_utc();

    // Look up tenant_id and service details once; reused per-result for notifications.
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

    for result in &payload.results {
        if result.error.is_some() {
            tracing::debug!(
                software_item_id = %result.software_item_id,
                host_software_item_id = ?result.host_software_item_id,
                error = ?result.error,
                "skipping version result with error"
            );
            continue;
        }

        let software_item_id = result.software_item_id;

        // Query all host_software_item rows for this software item that belong to
        // this service's hosts in one round-trip. This replaces the old per-host
        // loop that issued one SELECT per host and logged "not found" for the
        // many hosts that don't have this software item.
        let matching_rows = host_software_item::Entity::find()
            .filter(host_software_item::Column::HostId.is_in(host_ids.clone()))
            .filter(host_software_item::Column::SoftwareItemId.eq(software_item_id))
            .filter(host_software_item::Column::DeactivatedAt.is_null())
            .all(state.db())
            .await;

        let matching_rows = match matching_rows {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    software_item_id = %software_item_id,
                    "failed to look up host_software_items"
                );
                continue;
            }
        };

        if matching_rows.is_empty() {
            continue;
        }

        let matching_host_ids: Vec<uuid::Uuid> = matching_rows.iter().map(|r| r.host_id).collect();

        // Build and run the update across all matched host_software_item rows.
        let mut update = host_software_item::Entity::update_many()
            .filter(host_software_item::Column::HostId.is_in(matching_host_ids.clone()))
            .filter(host_software_item::Column::SoftwareItemId.eq(software_item_id));
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
        if let Err(e) = update.exec(state.db()).await {
            tracing::warn!(
                error = %e,
                software_item_id = %software_item_id,
                host_count = matching_host_ids.len(),
                "failed to update host_software_items"
            );
        }

        // Dispatch notification per matched host when a new version is detected.
        if let (Some(latest_version), Some(tenant_id)) = (&result.latest_version, svc_tenant_id) {
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

                state.notification_dispatcher.dispatch(NotificationEvent {
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
    }

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
            .notification_service
            .push_software_states_for_tenant(state.db(), svc.tenant_id)
            .await;
    }

    LoopAction::Continue
}

// ---------------------------------------------------------------------------
// handle_discovery_results
// ---------------------------------------------------------------------------

/// Handle a `DiscoveryResults` message: find host, process results.
#[tracing::instrument(skip_all, fields(%service_id, host_machine_id = %payload.host_machine_id))]
pub(super) async fn handle_discovery_results(
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: DiscoveryResultsPayload,
    pagination: Option<&ReportPagination>,
    report_tracker: &mut ReportTracker,
) -> LoopAction {
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
                return LoopAction::Continue;
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

    let mut host_id_opt: Option<uuid::Uuid> = None;
    for link in &links {
        if let Ok(Some(h)) = host::Entity::find_by_id(link.host_id)
            .filter(host::Column::MachineId.eq(&payload.host_machine_id))
            .filter(host::Column::DeactivatedAt.is_null())
            .one(state.db())
            .await
        {
            host_id_opt = Some(h.id);
            break;
        }
    }

    if let Some(host_id) = host_id_opt {
        if let Ok(Some(svc)) = service::Entity::find_by_id(service_id)
            .one(state.db())
            .await
        {
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
            } else {
                // Emit the notification only on the final page (or for
                // non-paginated messages). For intermediate pages, accumulate
                // the count in the tracker.
                match page_outcome {
                    PageOutcome::Final {
                        accumulated_discovered_count,
                    } => {
                        let total_discovered =
                            accumulated_discovered_count.saturating_add(this_page_count);
                        if total_discovered > 0 {
                            let host_name = host::Entity::find_by_id(host_id)
                                .one(state.db())
                                .await
                                .ok()
                                .flatten()
                                .map(|h| h.hostname.clone());

                            state.notification_dispatcher.dispatch(NotificationEvent {
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
        }
    } else {
        tracing::warn!(
            %service_id,
            host_machine_id = %payload.host_machine_id,
            "received DiscoveryResults for unknown host machine_id"
        );
    }

    LoopAction::Continue
}

// ---------------------------------------------------------------------------
// handle_report_plugin_config
// ---------------------------------------------------------------------------

/// Handle a `ReportPluginConfig` message: find or create a plugin config and
/// send the response back to the service.
///
/// Idempotent: if a config with the same `(tenant_id, plugin_type, name)`
/// already exists, the existing ID is returned without creating a duplicate.
pub(super) async fn handle_report_plugin_config(
    sink: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    out_seq: &mut OutgoingSeq,
    state: &Arc<AppState>,
    service_id: uuid::Uuid,
    payload: &ReportPluginConfigPayload,
) -> LoopAction {
    let request_id = payload.request_id.clone();

    // Validate the plugin type is known
    if let Err(e) = state
        .plugin_ops
        .validate_config_str(&payload.plugin_type, &payload.config)
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
        let resp = ControllerMessage::ReportPluginConfigResponse(resp_payload);
        if let Some(json) = serialize_controller_msg(out_seq, resp) {
            let _ = sink.send(Message::Text(json.into())).await;
        }
        return LoopAction::Continue;
    }

    // Resolve tenant_id from the service
    let tenant_id = match service::Entity::find_by_id(service_id)
        .one(state.db())
        .await
    {
        Ok(Some(svc)) => svc.tenant_id,
        Ok(None) => {
            tracing::warn!(%service_id, "ReportPluginConfig: service not found");
            return LoopAction::Continue;
        }
        Err(e) => {
            tracing::warn!(%service_id, error = %e, "ReportPluginConfig: DB error");
            return LoopAction::Continue;
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
            // Use JSON deserialization because the payload is `#[non_exhaustive]`.
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

    if let Some(json) = serialize_controller_msg(out_seq, resp) {
        let _ = sink.send(Message::Text(json.into())).await;
    }

    LoopAction::Continue
}
