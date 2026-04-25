//! Shared update-dispatch primitives used by both `update_triggers` and
//! `update_batches`.
//!
//! This module contains the types, error definitions, and core functions that
//! form the composable building blocks of the update pipeline. By housing them
//! in a neutral module, `update_triggers` and `update_batches` can each depend
//! on `update_dispatch` without creating a circular dependency between each
//! other.

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectionTrait, DatabaseConnection, EntityTrait,
    PaginatorTrait, QueryFilter, QueryOrder, RelationTrait, Set, sea_query::Expr,
};
use std::{sync::Arc, time::Duration};
use time::OffsetDateTime;
use tokio::time::timeout;
use uptrakit_internal_wire::{
    AttestationStatus, ControllerMessage, PluginAssignment, ReleaseAsset, ReleaseInfo,
};
use uptrakit_plugin_infrastructure_registry::{
    ControllerPostUpdateContext, ControllerProtectionContext, ControllerUpdateProtection,
    PluginError, PluginResult, ProxmoxHostMappingRecord, ProxmoxProtectionAuditRecord,
    ProxmoxProtectionMode, ProxmoxProtectionPolicyRecord, ProxmoxProtectionStore,
    UpdateProtectionController, is_interactive_dispatch_plugin,
};
use uptrakit_shared_db::entity::{
    host, host_software_item, host_software_item_plugin, plugin_config, prelude::*,
    proxmox_backup_target_cache, proxmox_host_mapping, proxmox_protection_audit,
    proxmox_protection_default, proxmox_protection_item_override, service, service_host,
    software_item, update_history,
};
use uptrakit_shared_macros::impl_report_conversion;
use uuid::Uuid;

use crate::notifier::ServiceNotifier;
use crate::queries::software_items::find_active_item;
use crate::token_utils::generate_uuid;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Error returned by the update-trigger pipeline.
#[derive(Debug, thiserror::Error)]
pub enum TriggerUpdateError {
    /// Software item not found or deactivated.
    #[error("software item not found")]
    SoftwareItemNotFound,
    /// Host not found, deactivated, or not scoped to the tenant.
    #[error("host not found")]
    HostNotFound,
    /// The host does not have an assignment record for this software item.
    #[error("host is not assigned to this software item")]
    HostNotAssigned,
    /// No execute_update role plugin assigned for this host-software pair.
    #[error("no execute_update plugin assigned")]
    NoExecuteUpdatePlugin,
    /// No `service_host` link exists for the host.
    #[error("no agent linked to host")]
    NoAgent,
    /// The linked agent exists but is not in `Approved` status.
    #[error("agent is not approved")]
    AgentNotApproved,
    /// A `Pending` or `InProgress` update already exists for this
    /// (host_id, software_item_id) pair. This is the belt-and-suspenders path
    /// when the partial unique DB index rejects a concurrent INSERT.
    #[error("an update is already pending or in progress")]
    UpdateAlreadyActive,
    /// The plugin config referenced by the role assignment was not found.
    #[error("plugin config not found")]
    PluginConfigNotFound,
    /// The plugin type stored in the config could not be deserialized.
    #[error("unknown plugin type: {0}")]
    UnknownPluginType(String),
    /// A database error occurred.
    #[error("database error: {0}")]
    Database(sea_orm::DbErr),
    /// Controller-side pre-update protection rejected dispatch.
    #[error("controller-side pre-update protection failed: {0}")]
    PreUpdateProtection(String),
    /// Controller-side post-update finalization failed.
    #[error("controller-side post-update finalization failed: {0}")]
    PostUpdateFinalization(String),
    /// Controller-side post-update finalization timed out.
    #[error("controller-side post-update finalization timed out")]
    PostUpdateFinalizationTimeout,
}

pub type Result<T> = std::result::Result<T, rootcause::Report<TriggerUpdateError>>;
impl_report_conversion!(sea_orm::DbErr => TriggerUpdateError::Database);

impl TriggerUpdateError {
    /// Returns the audit classification `(outcome, reason_code)` for a single-host
    /// trigger update failure.
    pub fn trigger_audit_classification(&self) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
        match self {
            Self::SoftwareItemNotFound => (
                uptrakit_audit_log::AuditOutcome::Denied,
                "trigger_update.software_item_not_found",
            ),
            Self::HostNotFound => (
                uptrakit_audit_log::AuditOutcome::Denied,
                "trigger_update.host_not_found",
            ),
            Self::UpdateAlreadyActive => (
                uptrakit_audit_log::AuditOutcome::Denied,
                "trigger_update.update_already_active",
            ),
            Self::HostNotAssigned => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "trigger_update.host_not_assigned",
            ),
            Self::NoExecuteUpdatePlugin => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "trigger_update.no_execute_update_plugin",
            ),
            Self::NoAgent => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "trigger_update.no_agent",
            ),
            Self::AgentNotApproved => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "trigger_update.agent_not_approved",
            ),
            Self::PluginConfigNotFound => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "trigger_update.plugin_config_not_found",
            ),
            Self::UnknownPluginType(_) => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "trigger_update.unknown_plugin_type",
            ),
            Self::PreUpdateProtection(_) => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "trigger_update.pre_update_protection_failed",
            ),
            Self::Database(_) => (
                uptrakit_audit_log::AuditOutcome::Failed,
                "trigger_update.database_error",
            ),
            Self::PostUpdateFinalization(_) | Self::PostUpdateFinalizationTimeout => (
                uptrakit_audit_log::AuditOutcome::Failed,
                "trigger_update.post_update_finalization_failed",
            ),
        }
    }

    /// Returns the audit classification `(outcome, reason_code)` for a batch
    /// trigger update failure.
    pub fn batch_trigger_audit_classification(
        &self,
    ) -> (uptrakit_audit_log::AuditOutcome, &'static str) {
        match self {
            Self::SoftwareItemNotFound => (
                uptrakit_audit_log::AuditOutcome::Denied,
                "trigger_batch_update.software_item_not_found",
            ),
            Self::HostNotFound => (
                uptrakit_audit_log::AuditOutcome::Denied,
                "trigger_batch_update.host_not_found",
            ),
            Self::UpdateAlreadyActive => (
                uptrakit_audit_log::AuditOutcome::Denied,
                "trigger_batch_update.update_already_active",
            ),
            Self::HostNotAssigned => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "trigger_batch_update.host_not_assigned",
            ),
            Self::NoExecuteUpdatePlugin => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "trigger_batch_update.no_execute_update_plugin",
            ),
            Self::NoAgent => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "trigger_batch_update.no_agent",
            ),
            Self::AgentNotApproved => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "trigger_batch_update.agent_not_approved",
            ),
            Self::PluginConfigNotFound => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "trigger_batch_update.plugin_config_not_found",
            ),
            Self::UnknownPluginType(_) => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "trigger_batch_update.unknown_plugin_type",
            ),
            Self::PreUpdateProtection(_) => (
                uptrakit_audit_log::AuditOutcome::ValidationFailed,
                "trigger_batch_update.pre_update_protection_failed",
            ),
            Self::Database(_) => (
                uptrakit_audit_log::AuditOutcome::Failed,
                "trigger_batch_update.database_error",
            ),
            Self::PostUpdateFinalization(_) | Self::PostUpdateFinalizationTimeout => (
                uptrakit_audit_log::AuditOutcome::Failed,
                "trigger_batch_update.post_update_finalization_failed",
            ),
        }
    }
}

fn merged_plugin_config(
    assignment: &host_software_item_plugin::Model,
    config: Option<&plugin_config::Model>,
) -> serde_json::Value {
    uptrakit_config_merge::resolve_effective_config(
        None,
        config.map(|c| &c.config),
        assignment.config.as_ref(),
    )
}

// ---------------------------------------------------------------------------
// Public structs
// ---------------------------------------------------------------------------

/// All data loaded and validated during [`validate_update_preconditions`].
///
/// Carries everything needed for record creation and dispatch so that
/// callers do not need to repeat any DB lookups.
#[derive(Clone, Debug)]
pub struct ValidatedUpdateTarget {
    pub item: software_item::Model,
    pub host: host::Model,
    pub hsi_link: host_software_item::Model,
    pub agent: service::Model,
    pub execute_update_data: (
        host_software_item_plugin::Model,
        Option<plugin_config::Model>,
    ),
    pub detect_version_data: Option<(
        host_software_item_plugin::Model,
        Option<plugin_config::Model>,
    )>,
    /// The merged config for the `fetch_releases` role plugin, if assigned.
    ///
    /// Used to extract `require_attestation` at dispatch time without a
    /// hard dependency on any specific plugin crate.
    pub fetch_releases_config: Option<serde_json::Value>,
    /// Pre-update hook plugin assignments, ordered by `ordinal`.
    pub pre_update_hook_plugins: Vec<PluginAssignment>,
    /// Post-update hook plugin assignments, ordered by `ordinal`.
    pub post_update_hook_plugins: Vec<PluginAssignment>,
}

/// Dispatch dependencies threaded through immediate, queued, and cleanup flows.
pub struct DispatchContext<'a> {
    pub notifier: &'a dyn ServiceNotifier,
    pub protection: Option<Arc<dyn ControllerUpdateProtection>>,
}

/// Parameters for [`create_update_history_record`].
pub struct CreateUpdateRecordParams<'a> {
    pub tenant_id: Uuid,
    pub host_id: Uuid,
    pub item_id: Uuid,
    pub host_software_item_id: Option<Uuid>,
    pub to_version: &'a str,
    /// The currently installed version at the time the update was triggered.
    ///
    /// Populated from `host_software_items.installed_version` so the history
    /// record shows the "before" version even while the update is still
    /// pending or in progress.
    pub from_version: Option<String>,
    /// Who initiated the update.
    pub actor_type: &'a str,
    pub actor_id: &'a str,
    pub update_category: &'a str,
    /// Set when the update belongs to a batch.
    pub batch_id: Option<Uuid>,
    /// Initial status of the record. Non-batch callers always use
    /// [`update_history::UpdateStatus::Pending`]. Batch callers use
    /// [`update_history::UpdateStatus::Queued`] for non-first items on a
    /// host so that only one active record exists per host at a time.
    pub initial_status: update_history::UpdateStatus,
    /// Whether the update is dispatched in interactive mode (PTY allocated).
    ///
    /// Must reflect the fully-resolved value (i.e. `params.interactive ||
    /// config_prefers_interactive(...)`) so the persisted column accurately
    /// records how the agent was instructed to run.
    pub interactive: bool,
}

/// Parameters for [`dispatch_update_to_agent`].
pub struct DispatchUpdateParams {
    pub update_history_id: Uuid,
    pub to_version: String,
    pub release_info: Option<ReleaseInfo>,
    /// When true, the agent allocates a PTY and keeps stdin open.
    pub interactive: bool,
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// Load a role-specific plugin assignment for a host-software pair.
///
/// Returns `None` if no assignment with the given role exists.
async fn load_role_plugin(
    db: &DatabaseConnection,
    host_id: Uuid,
    software_item_id: Uuid,
    role: &str,
) -> Result<
    Option<(
        host_software_item_plugin::Model,
        Option<plugin_config::Model>,
    )>,
> {
    let assignment = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(software_item_id))
        .filter(host_software_item_plugin::Column::Role.eq(role))
        .one(db)
        .await
        .context_to()?;

    let Some(assignment) = assignment else {
        return Ok(None);
    };

    let config = if let Some(pc_id) = assignment.plugin_config_id {
        Some(
            PluginConfig::find_by_id(pc_id)
                .filter(plugin_config::Column::DeactivatedAt.is_null())
                .one(db)
                .await
                .context_to()?
                .ok_or_else(|| report!(TriggerUpdateError::PluginConfigNotFound))?,
        )
    } else {
        None
    };

    Ok(Some((assignment, config)))
}

/// Load all plugin assignments for a given role, ordered by `ordinal ASC`.
///
/// Used for hook roles (`pre_update_hook`, `post_update_hook`) where multiple
/// plugins can be assigned with ordering semantics.
async fn load_role_plugins_ordered(
    db: &DatabaseConnection,
    host_id: Uuid,
    software_item_id: Uuid,
    role: &str,
) -> Result<Vec<PluginAssignment>> {
    let assignments = HostSoftwareItemPlugin::find()
        .filter(host_software_item_plugin::Column::HostId.eq(host_id))
        .filter(host_software_item_plugin::Column::SoftwareItemId.eq(software_item_id))
        .filter(host_software_item_plugin::Column::Role.eq(role))
        .order_by_asc(host_software_item_plugin::Column::Ordinal)
        .all(db)
        .await
        .context_to()?;

    let mut result = Vec::with_capacity(assignments.len());
    for assignment in &assignments {
        let config = if let Some(pc_id) = assignment.plugin_config_id {
            PluginConfig::find_by_id(pc_id)
                .filter(plugin_config::Column::DeactivatedAt.is_null())
                .one(db)
                .await
                .context_to()?
        } else {
            None
        };
        result.push(build_plugin_assignment(assignment, config.as_ref())?);
    }

    Ok(result)
}

/// Build a [`PluginAssignment`] from a role plugin row and its optional config.
///
/// When `config` is `None` (no `plugin_config` linked to the assignment), the
/// plugin type is read from `assignment.plugin_type` and the effective config
/// is built from the assignment-level override alone.
pub(crate) fn build_plugin_assignment(
    assignment: &host_software_item_plugin::Model,
    config: Option<&plugin_config::Model>,
) -> Result<PluginAssignment> {
    let plugin_type = uptrakit_internal_wire::PluginTypeId::new(&assignment.plugin_type);
    let merged_config = merged_plugin_config(assignment, config);

    Ok(PluginAssignment {
        plugin_type,
        package_identifier: assignment.package_identifier.clone(),
        config: merged_config,
    })
}

// ---------------------------------------------------------------------------
// Controller-side protection helpers
// ---------------------------------------------------------------------------

const PRE_UPDATE_PROTECTION_FAILURE_SUMMARY: &str =
    "Controller pre-update protection failed before dispatch.";
const PRE_UPDATE_PROTECTION_FAILURE_OUTPUT: &str =
    "Update failed before agent dispatch: controller pre-update protection failed.";
/// Proxmox infrastructure plugin type identifier — used as a DB filter value
/// when scoping plugin-config queries to proxmox configs. Defined here to avoid
/// appearing as a literal inside the filter-expression identity context (where
/// `Column::PluginType` would trigger the plugin-type boundary check).
const PROXMOX_INFRA_CONFIG_TYPE: &str = "infrastructure_proxmox";

/// Outcome of attempting controller-side pre-update protection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreUpdateProtectionOutcome {
    Proceed,
    Failed,
}

fn plugin_internal_error(error: impl std::fmt::Display) -> rootcause::Report<PluginError> {
    report!(PluginError::PluginInternal(error.to_string()))
}

fn proxmox_mode_from_db(value: &str) -> ProxmoxProtectionMode {
    match value {
        "snapshot" => ProxmoxProtectionMode::Snapshot,
        "backup" => ProxmoxProtectionMode::Backup,
        _ => ProxmoxProtectionMode::DoNothing,
    }
}

fn proxmox_mode_to_db(value: ProxmoxProtectionMode) -> &'static str {
    match value {
        ProxmoxProtectionMode::DoNothing => "do_nothing",
        ProxmoxProtectionMode::Snapshot => "snapshot",
        ProxmoxProtectionMode::Backup => "backup",
    }
}

struct QueryUpdateProtectionController<'a> {
    proxmox_store: QueryProxmoxProtectionStore<'a>,
}

impl<'a> QueryUpdateProtectionController<'a> {
    fn new(db: &'a DatabaseConnection) -> Self {
        Self {
            proxmox_store: QueryProxmoxProtectionStore { db },
        }
    }
}

impl UpdateProtectionController for QueryUpdateProtectionController<'_> {
    fn proxmox_protection_store(&self) -> Option<&dyn ProxmoxProtectionStore> {
        Some(&self.proxmox_store)
    }
}

struct QueryProxmoxProtectionStore<'a> {
    db: &'a DatabaseConnection,
}

#[async_trait::async_trait]
impl ProxmoxProtectionStore for QueryProxmoxProtectionStore<'_> {
    async fn load_host_mapping(
        &self,
        tenant_id: Uuid,
        host_id: Uuid,
    ) -> PluginResult<Option<ProxmoxHostMappingRecord>> {
        let mut mappings = ProxmoxHostMapping::find()
            .filter(proxmox_host_mapping::Column::TenantId.eq(tenant_id))
            .filter(proxmox_host_mapping::Column::HostId.eq(Some(host_id)))
            .all(self.db)
            .await
            .map_err(plugin_internal_error)?;

        if mappings.len() > 1 {
            return Err(plugin_internal_error(format!(
                "multiple proxmox host mappings found for tenant={tenant_id}, host_id={host_id}"
            )));
        }

        Ok(mappings.pop().map(|row| ProxmoxHostMappingRecord {
            id: row.id,
            tenant_id: row.tenant_id,
            host_id: row.host_id,
            plugin_config_id: row.plugin_config_id,
            proxmox_node: row.proxmox_node,
            proxmox_vmid: i64::from(row.proxmox_vmid),
            proxmox_type: row.proxmox_type,
        }))
    }

    async fn load_plugin_config_payload(
        &self,
        tenant_id: Uuid,
        plugin_config_id: Uuid,
    ) -> PluginResult<serde_json::Value> {
        let config = PluginConfig::find_by_id(plugin_config_id)
            .filter(plugin_config::Column::TenantId.eq(tenant_id))
            .filter(plugin_config::Column::PluginType.eq(PROXMOX_INFRA_CONFIG_TYPE))
            .one(self.db)
            .await
            .map_err(plugin_internal_error)?
            .ok_or_else(|| {
                plugin_internal_error(format!(
                    "proxmox plugin config not found for tenant={tenant_id}, plugin_config_id={plugin_config_id}"
                ))
            })?;

        if config.plugin_type != PROXMOX_INFRA_CONFIG_TYPE {
            return Err(plugin_internal_error(format!(
                "plugin config {plugin_config_id} is not an {PROXMOX_INFRA_CONFIG_TYPE} config"
            )));
        }

        Ok(config.config)
    }

    async fn load_effective_policy(
        &self,
        tenant_id: Uuid,
        software_item_id: Uuid,
        plugin_config_id: Uuid,
    ) -> PluginResult<ProxmoxProtectionPolicyRecord> {
        let item_override = ProxmoxProtectionItemOverride::find()
            .filter(proxmox_protection_item_override::Column::SoftwareItemId.eq(software_item_id))
            .filter(proxmox_protection_item_override::Column::PluginConfigId.eq(plugin_config_id))
            .one(self.db)
            .await
            .map_err(plugin_internal_error)?;

        let global_default = ProxmoxProtectionDefault::find()
            .filter(proxmox_protection_default::Column::TenantId.eq(tenant_id))
            .filter(proxmox_protection_default::Column::PluginConfigId.eq(plugin_config_id))
            .one(self.db)
            .await
            .map_err(plugin_internal_error)?;

        let effective = item_override
            .map(|row| ProxmoxProtectionPolicyRecord {
                mode: proxmox_mode_from_db(&row.mode),
                backup_target_key: row.backup_target_key,
            })
            .or_else(|| {
                global_default.map(|row| ProxmoxProtectionPolicyRecord {
                    mode: proxmox_mode_from_db(&row.mode),
                    backup_target_key: row.backup_target_key,
                })
            })
            .unwrap_or(ProxmoxProtectionPolicyRecord {
                mode: ProxmoxProtectionMode::DoNothing,
                backup_target_key: None,
            });

        Ok(effective)
    }

    async fn load_audit(
        &self,
        update_history_id: Uuid,
    ) -> PluginResult<Option<ProxmoxProtectionAuditRecord>> {
        let row = ProxmoxProtectionAudit::find_by_id(update_history_id)
            .one(self.db)
            .await
            .map_err(plugin_internal_error)?;

        Ok(row.map(|row| ProxmoxProtectionAuditRecord {
            update_history_id: row.update_history_id,
            tenant_id: row.tenant_id,
            host_id: row.host_id,
            software_item_id: row.software_item_id,
            plugin_config_id: row.plugin_config_id,
            mapping_id: row.mapping_id,
            mode: proxmox_mode_from_db(&row.mode),
            status: row.status,
            artifact_kind: row.artifact_kind,
            artifact_ref: row.artifact_ref,
            backup_target_key: row.backup_target_key,
            detail: row.detail,
            error_message: row.error_message,
        }))
    }

    async fn upsert_audit(&self, audit: &ProxmoxProtectionAuditRecord) -> PluginResult<()> {
        let now = OffsetDateTime::now_utc();
        let existing = ProxmoxProtectionAudit::find_by_id(audit.update_history_id)
            .one(self.db)
            .await
            .map_err(plugin_internal_error)?;

        if let Some(existing) = existing {
            let mut active: proxmox_protection_audit::ActiveModel = existing.into();
            active.tenant_id = Set(audit.tenant_id);
            active.host_id = Set(audit.host_id);
            active.software_item_id = Set(audit.software_item_id);
            active.plugin_config_id = Set(audit.plugin_config_id);
            active.mapping_id = Set(audit.mapping_id);
            active.mode = Set(proxmox_mode_to_db(audit.mode).to_string());
            active.status = Set(audit.status.clone());
            active.artifact_kind = Set(audit.artifact_kind.clone());
            active.artifact_ref = Set(audit.artifact_ref.clone());
            active.backup_target_key = Set(audit.backup_target_key.clone());
            active.detail = Set(audit.detail.clone());
            active.error_message = Set(audit.error_message.clone());
            active.updated_at = Set(now);
            active
                .update(self.db)
                .await
                .map_err(plugin_internal_error)?;
        } else {
            let active = proxmox_protection_audit::ActiveModel {
                update_history_id: Set(audit.update_history_id),
                tenant_id: Set(audit.tenant_id),
                host_id: Set(audit.host_id),
                software_item_id: Set(audit.software_item_id),
                plugin_config_id: Set(audit.plugin_config_id),
                mapping_id: Set(audit.mapping_id),
                mode: Set(proxmox_mode_to_db(audit.mode).to_string()),
                status: Set(audit.status.clone()),
                artifact_kind: Set(audit.artifact_kind.clone()),
                artifact_ref: Set(audit.artifact_ref.clone()),
                backup_target_key: Set(audit.backup_target_key.clone()),
                detail: Set(audit.detail.clone()),
                error_message: Set(audit.error_message.clone()),
                created_at: Set(now),
                updated_at: Set(now),
            };
            active
                .insert(self.db)
                .await
                .map_err(plugin_internal_error)?;
        }

        Ok(())
    }

    async fn find_cached_backup_target(
        &self,
        plugin_config_id: Uuid,
        target_key: &str,
    ) -> PluginResult<Option<String>> {
        let row = ProxmoxBackupTargetCache::find()
            .filter(proxmox_backup_target_cache::Column::PluginConfigId.eq(plugin_config_id))
            .filter(proxmox_backup_target_cache::Column::TargetKey.eq(target_key))
            .one(self.db)
            .await
            .map_err(plugin_internal_error)?;

        Ok(row.map(|row| row.storage_id))
    }
}

/// Build a [`ControllerProtectionContext`] for pre-update protection.
pub fn build_controller_protection_context<'a>(
    controller: &'a dyn UpdateProtectionController,
    target: &'a ValidatedUpdateTarget,
    update_history_id: Uuid,
) -> ControllerProtectionContext<'a> {
    ControllerProtectionContext::new(
        controller,
        target.item.tenant_id,
        target.host.id,
        target.item.id,
        update_history_id,
    )
}

/// Build a [`ControllerPostUpdateContext`] for post-update finalization.
pub fn build_controller_post_update_context<'a>(
    controller: &'a dyn UpdateProtectionController,
    record: &'a update_history::Model,
) -> ControllerPostUpdateContext<'a> {
    ControllerPostUpdateContext::new(
        controller,
        record.tenant_id,
        record.host_id,
        record.software_item_id,
        record.id,
        record.status,
    )
}

/// Atomically transition a `Pending` record to `InProgress` for orchestrator ownership.
///
/// Sets `status = InProgress`, `pre_update_protection_status = "in_progress"`,
/// `execution_owner_service_id = NULL`, and `started_at = now()`.
///
/// CAS guard: only updates if `status = Pending`. Returns the number of rows
/// affected (1 = success, 0 = raced or record gone).
pub async fn set_inprogress_for_orchestrator(
    db: &DatabaseConnection,
    update_history_id: Uuid,
) -> Result<u64> {
    let now = OffsetDateTime::now_utc();
    let result = UpdateHistory::update_many()
        .filter(update_history::Column::Id.eq(update_history_id))
        .filter(update_history::Column::Status.eq(update_history::UpdateStatus::Pending))
        .col_expr(
            update_history::Column::Status,
            Expr::value(update_history::UpdateStatus::InProgress),
        )
        .col_expr(
            update_history::Column::PreUpdateProtectionStatus,
            Expr::value(Some("in_progress".to_string())),
        )
        .col_expr(
            update_history::Column::ExecutionOwnerServiceId,
            Expr::value(Option::<Uuid>::None),
        )
        .col_expr(update_history::Column::StartedAt, Expr::value(Some(now)))
        .exec(db)
        .await
        .context_to()?;
    Ok(result.rows_affected)
}

/// Insert one protection output line into `update_output_line`.
///
/// No ownership check — called from the orchestrator's `forward_protection_output`
/// task which already knows the record belongs to the orchestrator.
pub async fn insert_protection_output_line(
    db: &DatabaseConnection,
    update_history_id: Uuid,
    line_id: Uuid,
    text: String,
    stream: uptrakit_shared_types::OutputStreamType,
    timestamp: time::OffsetDateTime,
) -> Result<()> {
    use uptrakit_shared_db::entity::update_output_line;
    UpdateOutputLine::insert(update_output_line::ActiveModel {
        id: Set(line_id),
        update_history_id: Set(update_history_id),
        stream: Set(stream),
        output: Set(text),
        created_at: Set(timestamp),
    })
    .exec(db)
    .await
    .context_to()?;
    Ok(())
}

async fn write_pre_update_protection_status(
    db: &DatabaseConnection,
    update_history_id: Uuid,
    status: Option<String>,
    summary: Option<String>,
) -> Result<()> {
    UpdateHistory::update_many()
        .filter(update_history::Column::Id.eq(update_history_id))
        .col_expr(
            update_history::Column::PreUpdateProtectionStatus,
            Expr::value(status),
        )
        .col_expr(
            update_history::Column::PreUpdateProtectionSummary,
            Expr::value(summary),
        )
        .exec(db)
        .await
        .context_to()?;
    Ok(())
}

pub async fn fail_before_agent_dispatch(
    db: &DatabaseConnection,
    update_history_id: Uuid,
    protection_status: Option<String>,
) -> Result<()> {
    let now = OffsetDateTime::now_utc();
    let output = PRE_UPDATE_PROTECTION_FAILURE_OUTPUT.to_string();
    UpdateHistory::update_many()
        .filter(update_history::Column::Id.eq(update_history_id))
        .col_expr(
            update_history::Column::Status,
            Expr::value(update_history::UpdateStatus::Failed),
        )
        .col_expr(update_history::Column::CompletedAt, Expr::value(Some(now)))
        .col_expr(update_history::Column::Output, Expr::value(output.clone()))
        .col_expr(
            update_history::Column::OutputBytes,
            Expr::value(output.len() as i64),
        )
        .col_expr(update_history::Column::OutputTruncated, Expr::value(false))
        .col_expr(
            update_history::Column::PreUpdateProtectionStatus,
            Expr::value(protection_status.or_else(|| Some("failed".to_string()))),
        )
        .col_expr(
            update_history::Column::PreUpdateProtectionSummary,
            Expr::value(Some(PRE_UPDATE_PROTECTION_FAILURE_SUMMARY.to_string())),
        )
        .exec(db)
        .await
        .context_to()?;
    Ok(())
}

/// Run controller-side pre-update protection before dispatch.
///
/// Contract:
/// - `protection == None` -> leaves protection fields `NULL`
/// - plugin do-nothing (`attempted = false`) -> writes status `skipped`
/// - success -> writes plugin-selected status + summary
/// - controller-side failure -> marks row `Failed` with generic summary/output
pub async fn prepare_pre_update_protection(
    db: &DatabaseConnection,
    protection: Option<Arc<dyn ControllerUpdateProtection>>,
    target: &ValidatedUpdateTarget,
    update_history_id: Uuid,
) -> Result<PreUpdateProtectionOutcome> {
    let Some(protection) = protection else {
        return Ok(PreUpdateProtectionOutcome::Proceed);
    };

    let controller = QueryUpdateProtectionController::new(db);
    let ctx = build_controller_protection_context(&controller, target, update_history_id);
    let decision = match protection.prepare_pre_update_protection(&ctx).await {
        Ok(decision) => decision,
        Err(error) => {
            fail_before_agent_dispatch(db, update_history_id, None).await?;
            tracing::warn!(
                update_id = %update_history_id,
                error = %error,
                "controller pre-update protection returned an error"
            );
            return Ok(PreUpdateProtectionOutcome::Failed);
        }
    };

    if !decision.attempted {
        write_pre_update_protection_status(
            db,
            update_history_id,
            Some("skipped".to_string()),
            decision.protection_summary.clone(),
        )
        .await?;
        return Ok(PreUpdateProtectionOutcome::Proceed);
    }

    if decision.succeeded {
        write_pre_update_protection_status(
            db,
            update_history_id,
            decision.protection_status.clone(),
            decision.protection_summary.clone(),
        )
        .await?;
        return Ok(PreUpdateProtectionOutcome::Proceed);
    }

    fail_before_agent_dispatch(db, update_history_id, decision.protection_status.clone()).await?;
    Ok(PreUpdateProtectionOutcome::Failed)
}

async fn finalize_post_update_inner(
    db: &DatabaseConnection,
    protection: Option<Arc<dyn ControllerUpdateProtection>>,
    record: &update_history::Model,
    per_row_timeout: Option<Duration>,
) -> Result<()> {
    let Some(protection) = protection else {
        return Ok(());
    };

    let controller = QueryUpdateProtectionController::new(db);
    let ctx = build_controller_post_update_context(&controller, record);
    let outcome = match per_row_timeout {
        Some(deadline) => match timeout(deadline, protection.finalize_post_update(&ctx)).await {
            Ok(result) => result
                .context_transform(|e| TriggerUpdateError::PostUpdateFinalization(e.to_string()))?,
            Err(_) => bail!(TriggerUpdateError::PostUpdateFinalizationTimeout),
        },
        None => protection
            .finalize_post_update(&ctx)
            .await
            .context_transform(|e| TriggerUpdateError::PostUpdateFinalization(e.to_string()))?,
    };

    UpdateHistory::update_many()
        .filter(update_history::Column::Id.eq(record.id))
        .col_expr(
            update_history::Column::RecoveryHint,
            Expr::value(outcome.recovery_hint),
        )
        .exec(db)
        .await
        .context_to()?;

    Ok(())
}

/// Finalize controller-side post-update state and persist `recovery_hint`.
pub async fn finalize_post_update(
    db: &DatabaseConnection,
    protection: Option<Arc<dyn ControllerUpdateProtection>>,
    record: &update_history::Model,
) -> Result<()> {
    finalize_post_update_inner(db, protection, record, None).await
}

/// Same as [`finalize_post_update`] but with a per-row timeout.
pub async fn finalize_post_update_with_timeout(
    db: &DatabaseConnection,
    protection: Option<Arc<dyn ControllerUpdateProtection>>,
    record: &update_history::Model,
    per_row_timeout: Duration,
) -> Result<()> {
    finalize_post_update_inner(db, protection, record, Some(per_row_timeout)).await
}

// ---------------------------------------------------------------------------
// Layer 1: Validate preconditions
// ---------------------------------------------------------------------------

/// Loads all data needed for dispatch without performing the per-host lock check.
///
/// Used by [`dispatch_next_in_batch`](super::update_batches::dispatch_next_in_batch)
/// which performs the CAS transition instead, and by
/// [`validate_update_preconditions`] which calls this and then adds the lock check.
///
/// Steps:
/// 1. Verify software item exists and is active.
/// 2. Verify host exists, is active, and belongs to the tenant.
/// 3. Verify host is assigned to the software item.
/// 4. Find the agent linked to this host.
/// 5. Verify agent exists, belongs to the tenant, and is approved.
/// 6. Load role-specific plugin assignments (`execute_update`, `detect_version`,
///    `fetch_releases`).
#[tracing::instrument(skip_all, fields(%tenant_id, %host_id))]
pub(crate) async fn load_target_for_dispatch(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    item_id: Uuid,
) -> Result<ValidatedUpdateTarget> {
    // 1. Verify software item exists and is active.
    let item = find_active_item(db, tenant_id, item_id)
        .await
        .ok_or_else(|| report!(TriggerUpdateError::SoftwareItemNotFound))?;

    // 2. Verify host exists, is active, and belongs to the tenant.
    let host_record = Host::find_by_id(host_id)
        .filter(host::Column::TenantId.eq(tenant_id))
        .filter(host::Column::DeactivatedAt.is_null())
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(TriggerUpdateError::HostNotFound))?;

    // 3. Verify host is assigned to the software item.
    let hsi_link = HostSoftwareItem::find()
        .filter(host_software_item::Column::HostId.eq(host_id))
        .filter(host_software_item::Column::SoftwareItemId.eq(item_id))
        .one(db)
        .await
        .context_to()?
        .ok_or_else(|| report!(TriggerUpdateError::HostNotAssigned))?;

    // 4. Find services linked to this host (tenant-scoped via join on service).
    let tenant_db_local = crate::TenantDb::new(db.clone(), tenant_id);
    let agent_links = tenant_db_local
        .find_via_tenant_join::<service_host::Entity, service::Entity>(
            service_host::Relation::Service.def(),
        )
        .filter(service_host::Column::HostId.eq(host_id))
        .all(db)
        .await
        .context_to()?
        .into_iter()
        .map(|link| link.service_id)
        .collect::<Vec<_>>();

    if agent_links.is_empty() {
        bail!(TriggerUpdateError::NoAgent);
    }

    // 5. Prefer a non-deactivated approved service when multiple historical
    // links exist for the same host (for example after re-enrolling agent-ssh).
    let agents = Service::find()
        .filter(service::Column::Id.is_in(agent_links))
        .filter(service::Column::TenantId.eq(tenant_id))
        .filter(service::Column::DeactivatedAt.is_null())
        .all(db)
        .await
        .context_to()?;

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
        })
        .ok_or_else(|| report!(TriggerUpdateError::NoAgent))?;

    if agent.status != service::ServiceStatus::Approved {
        bail!(TriggerUpdateError::AgentNotApproved);
    }

    // 6. Load role-specific plugin assignments.
    let execute_update_data = load_role_plugin(db, host_id, item_id, "execute_update")
        .await?
        .ok_or_else(|| report!(TriggerUpdateError::NoExecuteUpdatePlugin))?;

    let detect_version_data = load_role_plugin(db, host_id, item_id, "detect_version").await?;

    // Load fetch_releases plugin config (optional). Used at dispatch time to
    // extract `require_attestation` from the GitHub plugin config.
    let fetch_releases_config = load_role_plugin(db, host_id, item_id, "fetch_releases")
        .await?
        .map(|(assignment, config)| merged_plugin_config(&assignment, config.as_ref()));

    // Load hook plugin assignments (ordered by ordinal).
    let pre_update_hook_plugins =
        load_role_plugins_ordered(db, host_id, item_id, "pre_update_hook").await?;
    let post_update_hook_plugins =
        load_role_plugins_ordered(db, host_id, item_id, "post_update_hook").await?;

    Ok(ValidatedUpdateTarget {
        item,
        host: host_record,
        hsi_link,
        agent,
        execute_update_data,
        detect_version_data,
        fetch_releases_config,
        pre_update_hook_plugins,
        post_update_hook_plugins,
    })
}

/// Validates all preconditions for triggering an update on a single
/// (host, software_item) pair:
///
/// - Software item exists and is active.
/// - Host exists, is active, and belongs to the tenant.
/// - Host is assigned to the software item.
/// - An agent is linked to the host and is approved.
/// - The execute_update role plugin is assigned.
///
/// Returns a [`ValidatedUpdateTarget`] containing all loaded data needed for
/// the subsequent record creation and dispatch steps.
#[tracing::instrument(skip_all, fields(%tenant_id, %host_id))]
pub async fn validate_update_preconditions(
    db: &DatabaseConnection,
    tenant_id: Uuid,
    host_id: Uuid,
    item_id: Uuid,
) -> Result<ValidatedUpdateTarget> {
    load_target_for_dispatch(db, tenant_id, host_id, item_id).await
}

/// Returns `true` if a `Pending` or `InProgress` update already exists for
/// the given host (across all software items and batches).
pub async fn has_active_update_for_host(db: &DatabaseConnection, host_id: Uuid) -> Result<bool> {
    let count = UpdateHistory::find()
        .filter(update_history::Column::HostId.eq(host_id))
        .filter(update_history::Column::Status.is_in([
            update_history::UpdateStatus::Pending,
            update_history::UpdateStatus::InProgress,
        ]))
        .count(db)
        .await
        .context_to()?;
    Ok(count > 0)
}

// ---------------------------------------------------------------------------
// Layer 2: Create update_history record
// ---------------------------------------------------------------------------

/// Inserts an `update_history` row with the given `initial_status` and returns its ID.
///
/// If `batch_id` is `Some`, the record is associated with a batch for
/// sequential per-host dispatch.
///
/// Non-batch callers pass `initial_status: UpdateStatus::Pending`. Batch
/// callers pass `UpdateStatus::Queued` for non-first items on a host.
///
/// Accepts any `ConnectionTrait` implementor (bare `DatabaseConnection` or a
/// SeaORM transaction) so callers can run this inside or outside a transaction.
#[tracing::instrument(skip_all)]
pub async fn create_update_history_record<C: ConnectionTrait>(
    db: &C,
    params: &CreateUpdateRecordParams<'_>,
) -> Result<Uuid> {
    let now = OffsetDateTime::now_utc();
    let update_history_id = generate_uuid();
    let record = update_history::ActiveModel {
        id: Set(update_history_id),
        tenant_id: Set(params.tenant_id),
        host_id: Set(params.host_id),
        software_item_id: Set(params.item_id),
        host_software_item_id: Set(params.host_software_item_id),
        from_version: Set(params.from_version.clone()),
        to_version: Set(Some(params.to_version.to_string())),
        status: Set(params.initial_status),
        output: Set(String::new()),
        output_bytes: Set(0),
        actor_type: Set(params.actor_type.to_string()),
        actor_id: Set(params.actor_id.to_string()),
        execution_owner_service_id: Set(None),
        execution_owner_instance_id: Set(None),
        started_at: Set(Some(now)),
        completed_at: Set(None),
        created_at: Set(now),
        update_category: Set(params.update_category.to_string()),
        batch_id: Set(params.batch_id),
        interactive: Set(params.interactive),
        output_truncated: Set(false),
        pre_update_protection_status: Set(None),
        pre_update_protection_summary: Set(None),
        recovery_hint: Set(None),
    };

    record.insert(db).await.context_to()?;

    Ok(update_history_id)
}

// ---------------------------------------------------------------------------
// Layer 3: Dispatch to agent
// ---------------------------------------------------------------------------

/// Returns `true` when the execute-update plugin config opts into interactive
/// PTY mode via the `"prefer_interactive": true` field.
///
/// The plugin-type gate is registry-backed via an explicit registry-owned
/// classification, so dispatch semantics are decoupled from UI schema metadata.
pub(crate) fn config_prefers_interactive(
    plugin_type: &uptrakit_internal_wire::PluginTypeId,
    config: &serde_json::Value,
) -> bool {
    is_interactive_dispatch_plugin(plugin_type)
        && config
            .get("prefer_interactive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
}

/// Builds the `ExecuteUpdate` payload from the validated target and sends it
/// to the agent via `NotificationService`.
///
/// The `interactive` flag in the payload is set to `true` when either:
/// - The caller explicitly requests it via [`DispatchUpdateParams::interactive`], or
/// - The execute-update plugin config carries `"prefer_interactive": true`
///   (set automatically for Proxmox Helper Scripts targets so that the PHS
///   `/usr/bin/update` script can read from `/dev/tty` during updates).
///
/// Returns `true` if the agent was locally connected at dispatch time.
#[tracing::instrument(skip_all)]
pub async fn dispatch_update_to_agent(
    notifier: &dyn ServiceNotifier,
    target: &ValidatedUpdateTarget,
    params: DispatchUpdateParams,
) -> Result<bool> {
    let execute_update_plugin = build_plugin_assignment(
        &target.execute_update_data.0,
        target.execute_update_data.1.as_ref(),
    )?;

    let detect_version_plugin = target
        .detect_version_data
        .as_ref()
        .map(|(a, c)| build_plugin_assignment(a, c.as_ref()))
        .transpose()?;

    let enriched_release_info = enrich_release_info_with_attestation(
        params.release_info,
        target.hsi_link.latest_release_metadata.as_ref(),
        target.fetch_releases_config.as_ref(),
    );

    // Auto-enable interactive mode when the plugin config opts in, regardless
    // of whether the caller explicitly requested it.
    let interactive = params.interactive
        || config_prefers_interactive(
            &execute_update_plugin.plugin_type,
            &execute_update_plugin.config,
        );

    let execute_payload = uptrakit_internal_wire::ExecuteUpdatePayload {
        host_machine_id: target.host.machine_id.clone(),
        update_history_id: params.update_history_id,
        software_item_id: target.item.id,
        software_item_name: target.item.name.clone(),
        to_version: params.to_version,
        detect_version_plugin,
        execute_update_plugin,
        pre_update_hook_plugins: target.pre_update_hook_plugins.clone(),
        post_update_hook_plugins: target.post_update_hook_plugins.clone(),
        release_info: enriched_release_info,
        timeout: uptrakit_internal_wire::DEFAULT_UPDATE_TIMEOUT,
        interactive,
    };

    let msg = ControllerMessage::ExecuteUpdate(Box::new(execute_payload));
    let agent_connected = notifier.send_to_service(&target.agent.id, msg).await;

    if agent_connected {
        tracing::info!(
            update_id = %params.update_history_id,
            agent_id = %target.agent.id,
            host = %target.host.friendly_name,
            software = %target.item.name,
            "update sent to connected agent"
        );
    } else {
        tracing::info!(
            update_id = %params.update_history_id,
            agent_id = %target.agent.id,
            host = %target.host.friendly_name,
            software = %target.item.name,
            "agent not connected locally, update queued (outbox written for cross-controller delivery)"
        );
    }

    Ok(agent_connected)
}

// ---------------------------------------------------------------------------
// Attestation enrichment
// ---------------------------------------------------------------------------

/// Projection of `latest_release_metadata` covering all fields needed for
/// both reconstruction and attestation enrichment.
///
/// Matches the shape of `UpstreamRelease` (and indirectly `ReleaseInfo`) as
/// stored by `FetchReleasesExecutor` after a controller-side `fetch_releases` run.
#[derive(serde::Deserialize)]
struct MetadataRelease {
    #[serde(default)]
    tag: String,
    #[serde(default)]
    release_url: String,
    #[serde(default)]
    attestation_status: Option<AttestationStatus>,
    #[serde(default)]
    assets: Vec<MetadataAsset>,
}

#[derive(serde::Deserialize)]
struct MetadataAsset {
    name: String,
    /// Required for reconstructing a complete `ReleaseAsset` when `release_info`
    /// is absent. Empty string indicates the field was not present in older metadata.
    #[serde(default)]
    download_url: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    content_type: Option<String>,
    #[serde(default)]
    sha256_digest: Option<String>,
}

/// Build or enrich a `ReleaseInfo` using data sourced from the DB.
///
/// **Reconstruction path** (`release_info = None`): when the caller does not
/// supply `release_info` (e.g. the frontend omits it, or a reconnect replay
/// occurs), the function reconstructs `ReleaseInfo` from
/// `latest_release_metadata` stored after the last `fetch_releases` run. This
/// makes GitHub-asset-based updates work even when the client sends only
/// `to_version`.
///
/// **Enrichment path** (`release_info = Some`): when the caller supplies
/// `release_info` (e.g. CLI `--release-tag`), the function enriches it with
/// `attestation_status` and per-asset `sha256_digest` from `metadata`.
///
/// In both cases, `require_attestation` is extracted from the `fetch_releases`
/// plugin config JSON so that this crate does not need to depend on any
/// specific plugin crate.
///
/// Returns `None` only when both `release_info` is `None` and metadata cannot
/// be deserialized into a usable `ReleaseInfo` (e.g. `tag` is absent).
pub fn enrich_release_info_with_attestation(
    release_info: Option<ReleaseInfo>,
    metadata: Option<&serde_json::Value>,
    fetch_config: Option<&serde_json::Value>,
) -> Option<ReleaseInfo> {
    let mut ri = match release_info {
        Some(mut ri) => {
            // Enrichment path: apply attestation_status and sha256_digest from
            // the last fetch_releases run.
            if let Some(meta) = metadata
                && let Ok(meta_ri) = serde_json::from_value::<MetadataRelease>(meta.clone())
            {
                ri.attestation_status = meta_ri.attestation_status;
                for asset in &mut ri.assets {
                    if let Some(ma) = meta_ri.assets.iter().find(|a| a.name == asset.name) {
                        asset.sha256_digest = ma.sha256_digest.clone();
                    }
                }
            }
            ri
        }
        None => {
            // Reconstruction path: build ReleaseInfo entirely from the stored
            // metadata when the trigger request did not include release_info.
            let meta = metadata?;
            let meta_ri = serde_json::from_value::<MetadataRelease>(meta.clone()).ok()?;
            if meta_ri.tag.is_empty() {
                return None;
            }
            ReleaseInfo {
                tag: meta_ri.tag,
                release_url: meta_ri.release_url,
                assets: meta_ri
                    .assets
                    .into_iter()
                    .filter(|a| !a.download_url.is_empty())
                    .map(|a| ReleaseAsset {
                        name: a.name,
                        download_url: a.download_url,
                        size: a.size,
                        content_type: a.content_type,
                        sha256_digest: a.sha256_digest,
                    })
                    .collect(),
                attestation_status: meta_ri.attestation_status,
                require_attestation: false,
            }
        }
    };

    // Apply require_attestation from the fetch_releases plugin config.
    if let Some(config) = fetch_config {
        #[derive(serde::Deserialize, Default)]
        struct RequireAttestation {
            #[serde(default)]
            require_attestation: bool,
        }
        if let Ok(parsed) = serde_json::from_value::<RequireAttestation>(config.clone()) {
            ri.require_attestation = parsed.require_attestation;
        }
    }

    Some(ri)
}

#[cfg(test)]
mod tests {
    use super::QueryProxmoxProtectionStore;
    use sea_orm::{DatabaseConnection, DbBackend, MockDatabase};
    use time::OffsetDateTime;
    use uptrakit_plugin_infrastructure_registry::ProxmoxProtectionStore;
    use uptrakit_shared_db::entity::{plugin_config, proxmox_host_mapping};
    use uuid::Uuid;

    fn store_with_db(db: DatabaseConnection) -> QueryProxmoxProtectionStore<'static> {
        let leaked = Box::leak(Box::new(db));
        QueryProxmoxProtectionStore { db: leaked }
    }

    fn sample_mapping(
        tenant_id: Uuid,
        host_id: Uuid,
        plugin_config_id: Uuid,
    ) -> proxmox_host_mapping::Model {
        proxmox_host_mapping::Model {
            id: Uuid::now_v7(),
            tenant_id,
            plugin_config_id,
            host_id: Some(host_id),
            proxmox_node: "pve1".to_string(),
            proxmox_vmid: 101,
            proxmox_type: "qemu".to_string(),
            proxmox_name: Some("vm-101".to_string()),
            proxmox_status: "running".to_string(),
            hostname: Some("vm-101".to_string()),
            ip_addresses: None,
            machine_id: None,
            match_method: Some("manual".to_string()),
            discovered_at: OffsetDateTime::now_utc(),
            updated_at: OffsetDateTime::now_utc(),
        }
    }

    fn sample_plugin_config(
        tenant_id: Uuid,
        plugin_config_id: Uuid,
        plugin_type: &str,
    ) -> plugin_config::Model {
        plugin_config::Model {
            id: plugin_config_id,
            tenant_id,
            name: "test".to_string(),
            plugin_type: plugin_type.to_string(),
            config: serde_json::json!({"api_url":"https://pve.local:8006"}),
            enabled: true,
            created_at: OffsetDateTime::now_utc(),
            updated_at: OffsetDateTime::now_utc(),
            deactivated_at: Some(OffsetDateTime::now_utc()),
        }
    }

    /// Insert the minimum parent rows required by the `update_history` FK constraints.
    ///
    /// Returns `(tenant_id, host_id, software_item_id)`.
    async fn insert_update_history_parents(
        db: &DatabaseConnection,
    ) -> (uuid::Uuid, uuid::Uuid, uuid::Uuid) {
        use sea_orm::ActiveModelTrait;
        use uptrakit_shared_db::entity::{host, software_item, tenant};
        let now = time::OffsetDateTime::now_utc();
        let tenant_id = uuid::Uuid::now_v7();
        let host_id = uuid::Uuid::now_v7();
        let software_item_id = uuid::Uuid::now_v7();

        tenant::ActiveModel {
            id: sea_orm::Set(tenant_id),
            name: sea_orm::Set("test-tenant".to_string()),
            slug: sea_orm::Set(format!("t-{tenant_id}")),
            is_default: sea_orm::Set(false),
            created_at: sea_orm::Set(now),
            updated_at: sea_orm::Set(now),
            deactivated_at: sea_orm::Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        host::ActiveModel {
            id: sea_orm::Set(host_id),
            tenant_id: sea_orm::Set(tenant_id),
            machine_id: sea_orm::Set(format!("machine-{host_id}")),
            hostname: sea_orm::Set("test-host".to_string()),
            friendly_name: sea_orm::Set("Test Host".to_string()),
            os_type: sea_orm::Set(None),
            os_version: sea_orm::Set(None),
            architecture: sea_orm::Set(None),
            ip_address: sea_orm::Set(None),
            host_features: sea_orm::Set(None),
            last_seen_at: sea_orm::Set(None),
            created_at: sea_orm::Set(now),
            updated_at: sea_orm::Set(now),
            deactivated_at: sea_orm::Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        software_item::ActiveModel {
            id: sea_orm::Set(software_item_id),
            tenant_id: sea_orm::Set(tenant_id),
            name: sea_orm::Set("test-item".to_string()),
            featured: sea_orm::Set(false),
            icon_url: sea_orm::Set(None),
            last_checked_at: sea_orm::Set(None),
            created_at: sea_orm::Set(now),
            updated_at: sea_orm::Set(now),
            deactivated_at: sea_orm::Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        (tenant_id, host_id, software_item_id)
    }

    async fn make_sqlite_db() -> DatabaseConnection {
        use sea_orm::Database;
        let db = Database::connect("sqlite::memory:").await.unwrap();
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .unwrap();
        db
    }

    #[tokio::test]
    async fn set_inprogress_for_orchestrator_transitions_pending_sets_started_at() {
        use sea_orm::{ActiveModelTrait, EntityTrait};
        use uptrakit_shared_db::entity::update_history;
        let db = make_sqlite_db().await;
        let (tenant_id, host_id, software_item_id) = insert_update_history_parents(&db).await;
        let now = time::OffsetDateTime::now_utc();
        let id = uuid::Uuid::now_v7();
        update_history::ActiveModel {
            id: sea_orm::Set(id),
            tenant_id: sea_orm::Set(tenant_id),
            host_id: sea_orm::Set(host_id),
            software_item_id: sea_orm::Set(software_item_id),
            host_software_item_id: sea_orm::Set(None),
            from_version: sea_orm::Set(None),
            to_version: sea_orm::Set(Some("1.0.0".to_string())),
            status: sea_orm::Set(update_history::UpdateStatus::Pending),
            output: sea_orm::Set(String::new()),
            output_bytes: sea_orm::Set(0),
            actor_type: sea_orm::Set("user".to_string()),
            actor_id: sea_orm::Set(String::new()),
            execution_owner_service_id: sea_orm::Set(None),
            execution_owner_instance_id: sea_orm::Set(None),
            started_at: sea_orm::Set(None),
            completed_at: sea_orm::Set(None),
            created_at: sea_orm::Set(now),
            update_category: sea_orm::Set("security".to_string()),
            batch_id: sea_orm::Set(None),
            interactive: sea_orm::Set(false),
            output_truncated: sea_orm::Set(false),
            pre_update_protection_status: sea_orm::Set(None),
            pre_update_protection_summary: sea_orm::Set(None),
            recovery_hint: sea_orm::Set(None),
        }
        .insert(&db)
        .await
        .unwrap();

        let rows = super::set_inprogress_for_orchestrator(&db, id)
            .await
            .unwrap();
        assert_eq!(rows, 1, "CAS must affect exactly one row");

        let row = update_history::Entity::find_by_id(id)
            .one(&db)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, update_history::UpdateStatus::InProgress);
        assert_eq!(
            row.pre_update_protection_status.as_deref(),
            Some("in_progress")
        );
        assert!(
            row.execution_owner_service_id.is_none(),
            "orchestrator sentinel: owner must be NULL"
        );
        assert!(row.started_at.is_some(), "started_at must be set");
    }

    #[tokio::test]
    async fn set_inprogress_for_orchestrator_returns_zero_when_not_pending() {
        use sea_orm::ActiveModelTrait;
        use uptrakit_shared_db::entity::update_history;
        let db = make_sqlite_db().await;
        let (tenant_id, host_id, software_item_id) = insert_update_history_parents(&db).await;
        let now = time::OffsetDateTime::now_utc();
        let id = uuid::Uuid::now_v7();
        update_history::ActiveModel {
            id: sea_orm::Set(id),
            tenant_id: sea_orm::Set(tenant_id),
            host_id: sea_orm::Set(host_id),
            software_item_id: sea_orm::Set(software_item_id),
            host_software_item_id: sea_orm::Set(None),
            from_version: sea_orm::Set(None),
            to_version: sea_orm::Set(Some("1.0.0".to_string())),
            status: sea_orm::Set(update_history::UpdateStatus::InProgress),
            output: sea_orm::Set(String::new()),
            output_bytes: sea_orm::Set(0),
            actor_type: sea_orm::Set("user".to_string()),
            actor_id: sea_orm::Set(String::new()),
            execution_owner_service_id: sea_orm::Set(Some(uuid::Uuid::now_v7())),
            execution_owner_instance_id: sea_orm::Set(None),
            started_at: sea_orm::Set(Some(now)),
            completed_at: sea_orm::Set(None),
            created_at: sea_orm::Set(now),
            update_category: sea_orm::Set("security".to_string()),
            batch_id: sea_orm::Set(None),
            interactive: sea_orm::Set(false),
            output_truncated: sea_orm::Set(false),
            pre_update_protection_status: sea_orm::Set(None),
            pre_update_protection_summary: sea_orm::Set(None),
            recovery_hint: sea_orm::Set(None),
        }
        .insert(&db)
        .await
        .unwrap();

        let rows = super::set_inprogress_for_orchestrator(&db, id)
            .await
            .unwrap();
        assert_eq!(rows, 0, "CAS must not affect an already-InProgress row");
    }

    #[tokio::test]
    async fn proxmox_protection_store_rejects_duplicate_host_mappings() {
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let plugin_config_id = Uuid::now_v7();
        let db = MockDatabase::new(DbBackend::MySql)
            .append_query_results([vec![
                sample_mapping(tenant_id, host_id, plugin_config_id),
                sample_mapping(tenant_id, host_id, plugin_config_id),
            ]])
            .into_connection();
        let store = store_with_db(db);

        let error = store
            .load_host_mapping(tenant_id, host_id)
            .await
            .expect_err("duplicate mappings must be rejected");

        assert!(
            error.to_string().contains("multiple proxmox host mappings"),
            "unexpected error: {error}"
        );
    }

    #[tokio::test]
    async fn proxmox_protection_store_rejects_non_proxmox_plugin_config_payload() {
        let tenant_id = Uuid::now_v7();
        let plugin_config_id = Uuid::now_v7();
        let db = MockDatabase::new(DbBackend::MySql)
            .append_query_results([vec![sample_plugin_config(
                tenant_id,
                plugin_config_id,
                "notifications_email",
            )]])
            .into_connection();
        let store = store_with_db(db);

        let error = store
            .load_plugin_config_payload(tenant_id, plugin_config_id)
            .await
            .expect_err("non-proxmox plugin config must be rejected");

        assert!(
            error.to_string().contains("infrastructure_proxmox"),
            "unexpected error: {error}"
        );
    }
}
