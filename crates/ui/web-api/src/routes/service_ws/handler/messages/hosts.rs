use std::collections::HashSet;
use std::sync::Arc;

use sea_orm::{ActiveModelTrait, EntityTrait, Set};
use uptrakit_shared_db::entity::service;
use uptrakit_wire::{HostConnectivityUpdate, ReportHostsPayload};

use crate::AppState;
use crate::routes::agent_operations::find_or_create_host_and_link;

use super::emit_service_inventory_audit;
use super::trigger_discovery_for_agent_host;
use super::{ProcessorResponse, load_linked_host_ids};

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

/// Resolve the user-visible name for a reported host.
///
/// Prefers `host_info.ip_address` (the SSH-target string the operator typed
/// at bootstrap, populated by the SSH agent — see
/// `agent-ssh-runtime/src/client.rs:229,384`) over `host_info.hostname`
/// (what the remote machine reports for itself). For SSH-bootstrapped
/// hosts the operator-typed target is the canonical name; the remote-read
/// hostname (RouterOS identity, `hostname -f`, etc.) is not allowed to
/// override it.
///
/// Falls back to `host_info.hostname` only when no SSH target is present
/// (standalone agents do not populate `ip_address`).
///
/// Never reaches into `service_model.hostname`: on the embedded SSH agent
/// that is the controller's own hostname and would leak into every newly
/// created host row otherwise.
///
/// Returns `None` when the agent reported neither field — the caller skips
/// the host. Unreachable for real agents (standalone always sets `hostname`,
/// SSH always sets `ip_address`) so we do not synthesise a name.
fn resolve_host_hostname(host_info: &uptrakit_wire::HostInfo) -> Option<String> {
    host_info
        .ip_address
        .as_deref()
        .or(host_info.hostname.as_deref())
        .map(str::to_string)
}

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

        let Some(host_hostname) = resolve_host_hostname(host_info) else {
            tracing::warn!(
                %service_id,
                machine_id = %host_info.machine_id,
                "host reported with neither hostname nor ip_address; skipping. \
                 This indicates an agent bug — every real agent populates at least \
                 one of the two."
            );
            summary.failed_hosts = summary.failed_hosts.saturating_add(1);
            continue;
        };
        let host_ip = host_info
            .ip_address
            .as_deref()
            .or(service_model.ip_address.as_deref());
        match find_or_create_host_and_link(
            state.db(),
            service_model.tenant_id,
            service_id,
            host_info,
            &host_hostname,
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
pub(in super::super) async fn handle_report_hosts(
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
