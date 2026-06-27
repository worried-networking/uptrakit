//! Common message handlers extracted from the authenticated loop.
//!
//! Each function corresponds to one match arm in the main dispatch and returns
//! a [`LoopAction`] plus an optional [`ControllerMessage`] reply. The main
//! loop is responsible for serializing and writing the reply to the WebSocket
//! sink.

#![expect(
    clippy::expect_used,
    reason = "expect used for infallible operations; message documents the invariant"
)]
#![expect(clippy::indexing_slicing, reason = "index is computed to be in bounds")]

use std::collections::HashSet;
use std::sync::Arc;

use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use uptrakit_shared_db::entity::{host, host_software_item, service, service_host, software_item};
use uptrakit_wire::report_tracker::{PageOutcome, ReportTracker};
use uptrakit_wire::{
    ControllerMessage, DiscoveryResultsPayload, ReportPagination, ReportPluginConfigPayload,
    ReportPluginConfigResponsePayload, VersionCheckResultsPayload,
};

use super::audit_service::{
    emit_service_certificate_renew_audit_event, ingest_service_audit_event,
};
use super::discovery::trigger_discovery_for_agent_host;
use super::message_processor::LoopAction;
use super::renewal::{sign_renewal_csr, sign_renewal_csr_system};
use super::shared_types::{ProcessorResponse, load_linked_host_ids};
use uptrakit_web_api_types::events::AdminEvent;

use crate::AppState;
use crate::notifications::events::{NotificationEvent, NotificationEventDetails};

mod certificate;
mod hosts;
mod shared;

pub(super) use certificate::handle_renew_certificate;
pub(super) use hosts::handle_report_hosts;
use shared::emit_service_inventory_audit;
pub(super) use shared::handle_ping;

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

    let mut builder = uptrakit_audit_log::AuditEntry::<uptrakit_audit_log::Event>::builder_event(
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
        Ok(entry) => ctx.state.audit_emitter.emit_event(entry),
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

/// Tri-state for the installed-display-version write path.
///
/// `UseAgentValue` preserves backward-compatible behaviour: the value comes
/// straight from `result.installed_display_version` (the wire payload).
/// `Override(Some(s))` writes the supplied display string. `Override(None)`
/// explicitly clears the column. The dispatcher in
/// `handle_version_check_results` constructs the value from enricher output:
/// no enricher applies → `UseAgentValue`; enricher ran and returned a string →
/// `Override(Some(...))`; enricher ran and returned `None` (miss / throttle /
/// out-of-window) → `Override(None)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum DisplayOverride {
    UseAgentValue,
    Override(Option<String>),
}

/// Build and execute the `update_many` query for a version check result.
async fn apply_version_update_to_db(
    db: &sea_orm::DatabaseConnection,
    result: &uptrakit_wire::VersionCheckResult,
    matching_ids: Vec<uuid::Uuid>,
    now: time::OffsetDateTime,
    installed_display_override: DisplayOverride,
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
                sea_orm::sea_query::Expr::value(match &installed_display_override {
                    DisplayOverride::UseAgentValue => result.installed_display_version.clone(),
                    DisplayOverride::Override(value) => value.clone(),
                }),
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

        {
            let mut event = NotificationEvent::new(
                tenant_id,
                NotificationEventDetails::UpdateAvailable {
                    installed_version: result.installed_version.clone(),
                    latest_version: latest_version.clone(),
                },
            );
            event.host_id = Some(host_id);
            event.host_name = host_name;
            event.software_item_id = Some(software_item_id);
            event.software_item_name = sw_name.clone();
            state.notification.notification_dispatcher.dispatch(event);
        }
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

/// Build the `host_software_item_id → DisplayOverride` map for a set of
/// `VersionCheckResult`s by dispatching to each capable plugin's
/// `InstalledVersionEnricher` slot.
///
/// Web-api stays plugin-agnostic — purely typed registry lookup (ADR-0018).
/// Tenant isolation flows through `TenantDb::find_via_tenant_join`. Items not
/// covered by an enricher are absent from the map; the caller treats absence
/// as `DisplayOverride::UseAgentValue`.
async fn build_enriched_display_overrides(
    state: &Arc<AppState>,
    svc_tenant_id: Option<uuid::Uuid>,
    payload: &VersionCheckResultsPayload,
) -> std::collections::HashMap<uuid::Uuid, DisplayOverride> {
    use std::collections::HashMap;

    use uptrakit_plugin_infrastructure_registry::{
        GlobalProviderLookup, HostCapabilities, InstalledVersionEnrichmentContext,
        InstalledVersionItem, PluginCapability, construct_host_runtime, get_descriptor,
    };

    let mut enriched: HashMap<uuid::Uuid, DisplayOverride> = HashMap::new();

    // 1. Source (plugin_type, package_identifier) per host_software_item_id
    //    for role `detect_version`.
    let hsi_ids: Vec<uuid::Uuid> = payload
        .results
        .iter()
        .filter_map(|r| r.host_software_item_id)
        .collect();
    if hsi_ids.is_empty() {
        return enriched;
    }
    let Some(tenant_id) = svc_tenant_id else {
        tracing::warn!("enrichment: tenant resolution failed; skipping");
        return enriched;
    };
    let tenant_db = uptrakit_web_api_queries::TenantDb::new(state.db().clone(), tenant_id);
    let assignments =
        match uptrakit_web_api_queries::queries::host_software_item_plugins::plugin_types_for_role(
            &tenant_db,
            &hsi_ids,
            "detect_version",
        )
        .await
        {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(error = %e, "enrichment: plugin_type lookup failed; skipping");
                return enriched;
            }
        };

    // 2. Group items by plugin_type. Items inherit `package_identifier` from
    //    the DB-side assignment (NOT from the wire payload, which doesn't
    //    carry it).
    #[derive(Default)]
    struct Group {
        hsi_ids: Vec<uuid::Uuid>,
        items: Vec<InstalledVersionItem>,
    }
    let mut by_plugin: HashMap<String, Group> = HashMap::new();
    for r in &payload.results {
        let Some(hsi_id) = r.host_software_item_id else {
            continue;
        };
        let Some(assignment) = assignments.get(&hsi_id) else {
            continue;
        };
        let g = by_plugin.entry(assignment.plugin_type.clone()).or_default();
        g.hsi_ids.push(hsi_id);
        g.items.push(InstalledVersionItem::new(
            assignment.package_identifier.clone(),
            r.installed_version.clone(),
        ));
    }

    // 3. For each plugin_type, look up descriptor + capability + slot, then
    //    invoke the enricher and merge results into the override map.
    let lookup: Arc<dyn GlobalProviderLookup> =
        state.plugin.global_providers.clone() as Arc<dyn GlobalProviderLookup>;
    for (pt, group) in by_plugin {
        let Some(desc) = get_descriptor(&pt) else {
            continue;
        };
        if !desc
            .capabilities
            .contains(&PluginCapability::EnrichInstalledVersion)
        {
            continue;
        }
        let Some(slot) = desc.roles.installed_version_enricher.as_ref() else {
            continue;
        };

        // Single positive `with_lookup` call — web-api always pulls
        // `core/catalog` transitively via the registry, so attaching the
        // lookup is unconditional here.
        let ctx = InstalledVersionEnrichmentContext::empty().with_lookup(lookup.clone());
        let runtime = construct_host_runtime(
            Arc::new(uptrakit_command::NoopCommandExecutor),
            HostCapabilities::default(),
        );
        // Empty per-plugin config: SkillsConfig has all-default fields and does not
        // need stored config to enrich. Future enrichers requiring stored config must
        // resolve it from `plugin_configs`/`plugin_type_settings` via the same path
        // `scheduler-runtime::fetch_releases` uses (`merged_plugin_config`) before
        // calling the factory. Tracked as a deferred follow-up in ADR-0021.
        let merged_cfg = serde_json::json!({});
        let enricher = match (slot.create)(&merged_cfg, runtime, &ctx) {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!(
                    plugin_type = %pt, error = %e, reason = "provider_error",
                    "enrichment: factory failed; collapsing group"
                );
                for hsi in &group.hsi_ids {
                    enriched.insert(*hsi, DisplayOverride::Override(None));
                }
                continue;
            }
        };
        let out = match enricher.enrich_installed_versions(&group.items).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(
                    plugin_type = %pt, error = %e, reason = "provider_error",
                    "enrichment: enricher returned Err; collapsing group"
                );
                for hsi in &group.hsi_ids {
                    enriched.insert(*hsi, DisplayOverride::Override(None));
                }
                continue;
            }
        };
        if out.len() != group.items.len() {
            tracing::warn!(
                plugin_type = %pt, expected = group.items.len(), got = out.len(),
                reason = "race_skipped",
                "enrichment: length mismatch; collapsing group"
            );
            for hsi in &group.hsi_ids {
                enriched.insert(*hsi, DisplayOverride::Override(None));
            }
            continue;
        }
        for (i, display) in out.into_iter().enumerate() {
            let hsi = group.hsi_ids[i];
            if display.installed_version_echo != group.items[i].installed_version {
                tracing::warn!(
                    plugin_type = %pt, %hsi, reason = "race_skipped",
                    "enrichment: installed_version_echo mismatch"
                );
                enriched.insert(hsi, DisplayOverride::Override(None));
                continue;
            }
            if display.package_identifier != group.items[i].package_identifier {
                tracing::warn!(
                    plugin_type = %pt, %hsi, reason = "race_skipped",
                    "enrichment: package_identifier echo mismatch"
                );
                enriched.insert(hsi, DisplayOverride::Override(None));
                continue;
            }
            enriched.insert(hsi, DisplayOverride::Override(display.display_version));
        }
    }

    enriched
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

    // ── Installed-version enrichment dispatch ────────────────────────────
    // Resolves a `host_software_item_id → DisplayOverride` map for every
    // result whose plugin_type declares `EnrichInstalledVersion`. Items not
    // covered by an enricher fall through to `DisplayOverride::UseAgentValue`
    // at the write site.
    //
    // Web-api stays plugin-agnostic — purely typed registry lookup. ADR-0018.
    let enriched = build_enriched_display_overrides(state, svc_tenant_id, payload).await;

    for result in &payload.results {
        // AwaitingRestart correlation: evaluated even for error results so that
        // an error=Some response correctly triggers the stay-or-fail rule in
        // apply_awaiting_restart_version_check.
        if let Some(hsi_id) = result.host_software_item_id {
            let terminal = crate::queries::update_batches::apply_awaiting_restart_version_check(
                state.db(),
                hsi_id,
                result.installed_version.clone(),
                result.not_ready,
                result.error.clone(),
            )
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(
                    error = %e,
                    host_software_item_id = %hsi_id,
                    "apply_awaiting_restart_version_check failed"
                );
                None
            });

            if terminal.is_some() {
                trigger_host_progression_after_awaiting_restart(state, hsi_id).await;
            }
        }

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

        let override_for_result = result
            .host_software_item_id
            .and_then(|hsi| enriched.get(&hsi).cloned())
            .unwrap_or(DisplayOverride::UseAgentValue);
        apply_version_update_to_db(state.db(), result, matching_ids, now, override_for_result)
            .await;

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

/// After an `AwaitingRestart` record transitions to `Completed` or `Failed`,
/// emit a per-item `BatchProgressEvent`, then promote the next queued update
/// for the same host (batch or standalone).  If the batch is now complete,
/// `handle_batch_completion` is called to emit the final summary and send
/// batch-completion notifications.
async fn trigger_host_progression_after_awaiting_restart(
    state: &Arc<AppState>,
    hsi_id: uuid::Uuid,
) {
    use sea_orm::QueryOrder;
    use uptrakit_shared_db::entity::update_history;

    // Load the record just transitioned from AwaitingRestart.
    // Filter on awaiting_restart_since IS NOT NULL to avoid picking up
    // unrelated records that happened to end up Completed/Failed.
    let record = match update_history::Entity::find()
        .filter(update_history::Column::HostSoftwareItemId.eq(hsi_id))
        .filter(update_history::Column::Status.is_in([
            update_history::UpdateStatus::Completed,
            update_history::UpdateStatus::Failed,
        ]))
        .filter(update_history::Column::AwaitingRestartSince.is_not_null())
        .order_by_desc(update_history::Column::CompletedAt)
        .one(state.db())
        .await
    {
        Ok(Some(r)) => r,
        Ok(None) => {
            tracing::warn!(
                host_software_item_id = %hsi_id,
                "no Completed/Failed record found after AwaitingRestart transition"
            );
            return;
        }
        Err(e) => {
            tracing::warn!(
                error = %e,
                host_software_item_id = %hsi_id,
                "failed to load update_history for post-AwaitingRestart dispatch"
            );
            return;
        }
    };

    let dispatch = crate::queries::update_dispatch::DispatchContext {
        notifier: &state.notification.notification_service,
        protection: state.controller_update_protection(),
        #[cfg(feature = "plugin-ops")]
        hook: state.controller_update_hook(),
        #[cfg(feature = "plugin-ops")]
        notification_ops: Some(state.plugin.plugin_ops.as_ref()),
    };

    if let Some(batch_id) = record.batch_id {
        // Emit per-item progress event before dispatching next — mirrors
        // what handle_update_result does in updates.rs.
        let event = match record.status {
            update_history::UpdateStatus::Completed => {
                crate::batch_progress_broadcaster::BatchProgressEvent::UpdateCompleted {
                    update_history_id: record.id,
                    software_item_name: super::updates::resolve_software_item_name(
                        state,
                        record.software_item_id,
                    )
                    .await,
                    host_name: super::updates::resolve_host_name(state, record.host_id).await,
                }
            }
            _ => crate::batch_progress_broadcaster::BatchProgressEvent::UpdateFailed {
                update_history_id: record.id,
                software_item_name: super::updates::resolve_software_item_name(
                    state,
                    record.software_item_id,
                )
                .await,
                host_name: super::updates::resolve_host_name(state, record.host_id).await,
                // The error detail is not stored on the AwaitingRestart record itself.
                error: None,
            },
        };
        super::updates::emit_batch_progress_event(state, batch_id, event).await;

        match crate::queries::update_batches::dispatch_next_in_batch(
            state.db(),
            dispatch,
            batch_id,
            record.host_id,
            record.tenant_id,
        )
        .await
        {
            Ok(Some(completion)) => {
                super::updates::handle_batch_completion(state, batch_id, &completion).await;
            }
            Ok(None) => {
                // Batch still in progress — emit updated progress summary.
                super::updates::emit_batch_progress_from_db(state, batch_id).await;
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    %batch_id,
                    host_id = %record.host_id,
                    "post-AwaitingRestart batch dispatch failed"
                );
            }
        }
    } else if let Err(e) = crate::queries::update_batches::dispatch_next_queued_for_host(
        state.db(),
        dispatch,
        record.host_id,
        record.tenant_id,
    )
    .await
    {
        tracing::warn!(
            error = %e,
            host_id = %record.host_id,
            "post-AwaitingRestart standalone dispatch failed"
        );
    }
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

                {
                    let mut event = NotificationEvent::new(
                        svc.tenant_id,
                        NotificationEventDetails::NewSoftwareDiscovered {
                            discovered_count: total_discovered,
                        },
                    );
                    event.host_id = Some(host_id);
                    event.host_name = host_name;
                    state.notification.notification_dispatcher.dispatch(event);
                }
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
        .plugin
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
        state.plugin.plugin_ops.as_ref(),
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
            .plugin
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
mod tests;
