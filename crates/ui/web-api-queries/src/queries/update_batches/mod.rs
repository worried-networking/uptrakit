//! Query helpers for batch update operations.
//!
//! Provides functions for:
//! - Finding outdated items on a host (for host-wide batch updates)
//! - Finding outdated hosts for an item (for item-wide rollouts)
//! - Creating a batch with associated update_history records
//! - Querying batch status and child updates
//! - Dispatching the next pending update in a batch for a host

mod candidates;
mod dispatch;
mod queries;

pub use candidates::{
    BatchUpdateCandidate, find_outdated_hosts_for_item, find_outdated_items_for_host,
};
pub use dispatch::{
    BatchCompletionInfo, ClaimExecutionInfo, ClaimExecutionOutcome, FinalizeBatchItemIfOwnedArgs,
    FinalizeUpdateResultIfOwnedArgs, append_update_output_if_owned,
    claim_or_replay_update_start_db, dispatch_next_in_batch, dispatch_next_queued_for_host,
    fail_pending_unowned_update, finalize_batch_item_if_owned, finalize_update_result_if_owned,
    mark_all_in_progress_as_failed_for_rollout,
    mark_orchestrator_inprogress_as_failed_on_reconnect,
    mark_owned_in_progress_as_failed_on_reconnect, touch_stdin_attention_if_owned,
};
pub use queries::{get_batch_with_items, list_batches};

use rootcause::prelude::*;
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter, Set,
    TransactionTrait,
};
use std::collections::{HashMap, HashSet};
use time::OffsetDateTime;
use uptrakit_shared_db::entity::{update_batch, update_history};
use uptrakit_shared_types::BatchStatus;
use uptrakit_web_api_types::software_items::TriggerUpdateStatus;
use uptrakit_web_api_types::update_batches::{
    BatchSkippedItem, BatchUpdateItem, BatchUpdateResponse,
};
use uuid::Uuid;

use crate::queries::update_dispatch::{
    CreateUpdateRecordParams, DispatchContext, DispatchUpdateParams, PreUpdateProtectionOutcome,
    TriggerUpdateError, has_active_update_for_host, prepare_pre_update_protection,
};
use crate::queries::update_types::BatchType;
use crate::token_utils::generate_uuid;

type Result<T> = std::result::Result<T, rootcause::Report<TriggerUpdateError>>;

fn trigger_status_from_update_history_status(
    status: update_history::UpdateStatus,
) -> TriggerUpdateStatus {
    match status {
        update_history::UpdateStatus::Queued => TriggerUpdateStatus::Queued,
        update_history::UpdateStatus::Failed => TriggerUpdateStatus::Failed,
        update_history::UpdateStatus::Pending
        | update_history::UpdateStatus::InProgress
        | update_history::UpdateStatus::Completed => TriggerUpdateStatus::Pending,
        _ => TriggerUpdateStatus::Queued,
    }
}

// ---------------------------------------------------------------------------
// Batch creation
// ---------------------------------------------------------------------------

/// Parameters for creating a batch update.
pub struct CreateBatchParams<'a> {
    pub tenant_id: Uuid,
    /// The batch category.
    pub batch_type: BatchType,
    /// Who initiated the batch.
    pub actor_type: &'a str,
    pub actor_id: &'a str,
}

/// Create a batch with associated update_history records.
///
/// For each candidate, validates preconditions, creates an `update_history`
/// record, and dispatches the first pending update per host.
///
/// Returns the `BatchUpdateResponse`. If zero candidates are eligible, returns
/// a response with `batch_id: None` and `total_created: 0`.
#[tracing::instrument(skip_all)]
pub async fn create_batch(
    db: &DatabaseConnection,
    dispatch: DispatchContext<'_>,
    params: &CreateBatchParams<'_>,
    candidates: Vec<BatchUpdateCandidate>,
) -> Result<BatchUpdateResponse> {
    if candidates.is_empty() {
        return Ok(BatchUpdateResponse {
            batch_id: None,
            total_created: 0,
            updates: vec![],
            skipped: vec![],
        });
    }

    let now = OffsetDateTime::now_utc();
    let batch_id = generate_uuid();

    // Validate all candidates and partition into valid + skipped
    let mut validated: Vec<(
        BatchUpdateCandidate,
        super::update_dispatch::ValidatedUpdateTarget,
    )> = Vec::new();
    let mut skipped: Vec<BatchSkippedItem> = Vec::new();

    for candidate in candidates {
        match super::update_dispatch::validate_update_preconditions(
            db,
            params.tenant_id,
            candidate.host_id,
            candidate.software_item_id,
        )
        .await
        {
            Ok(target) => {
                validated.push((candidate, target));
            }
            Err(e) => {
                skipped.push(BatchSkippedItem {
                    software_item_id: candidate.software_item_id,
                    software_item_name: candidate.software_item_name,
                    host_id: candidate.host_id,
                    host_name: candidate.host_name,
                    reason: e.to_string(),
                });
            }
        }
    }

    if validated.is_empty() {
        return Ok(BatchUpdateResponse {
            batch_id: None,
            total_created: 0,
            updates: vec![],
            skipped,
        });
    }

    // Determine initial status per host before opening the transaction.
    // Doing this outside the transaction avoids a pool-exhaustion deadlock on
    // single-connection pools (e.g. SQLite in-memory): db.begin() takes the
    // sole connection, and a subsequent query on `db` would block forever
    // waiting for a connection that is never released.
    //
    // - If the host already has an active (Pending/InProgress) update outside
    //   this batch, ALL items on that host start as Queued.
    // - Among hosts that are free, only the first item per host is Pending;
    //   subsequent items on the same host are Queued.
    let mut externally_busy_hosts: HashSet<Uuid> = HashSet::new();
    for (candidate, _) in &validated {
        if !externally_busy_hosts.contains(&candidate.host_id) {
            let busy = has_active_update_for_host(db, candidate.host_id)
                .await
                .unwrap_or(false);
            if busy {
                externally_busy_hosts.insert(candidate.host_id);
            }
        }
    }

    // Insert the batch record and all update_history rows atomically so that a
    // mid-flight failure cannot leave a batch record with an incorrect total_count.
    // Dispatch (WebSocket sends) happens outside the transaction because it cannot
    // be rolled back.
    let txn = db.begin().await.context_to()?;

    let batch_record = update_batch::ActiveModel {
        id: Set(batch_id),
        tenant_id: Set(params.tenant_id),
        batch_type: Set(params.batch_type.as_str().to_string()),
        status: Set(BatchStatus::InProgress),
        total_count: Set(validated.len() as i32),
        actor_type: Set(params.actor_type.to_string()),
        actor_id: Set(params.actor_id.to_string()),
        output: Set(String::new()),
        output_bytes: Set(0),
        created_at: Set(now),
        completed_at: Set(None),
    };
    batch_record.insert(&txn).await.context_to()?;

    let mut first_per_free_host: HashSet<Uuid> = HashSet::new();

    // Collect (history_id, should_dispatch) pairs inside the transaction, then
    // dispatch eligible items after commit.
    let mut history_ids: Vec<(Uuid, bool)> = Vec::with_capacity(validated.len());
    for (candidate, target) in &validated {
        let (initial_status, should_dispatch) =
            if externally_busy_hosts.contains(&candidate.host_id) {
                (update_history::UpdateStatus::Queued, false)
            } else {
                let is_first = first_per_free_host.insert(candidate.host_id);
                if is_first {
                    (update_history::UpdateStatus::Pending, true)
                } else {
                    (update_history::UpdateStatus::Queued, false)
                }
            };
        let update_history_id = super::update_dispatch::create_update_history_record(
            &txn,
            &CreateUpdateRecordParams {
                tenant_id: params.tenant_id,
                host_id: candidate.host_id,
                item_id: candidate.software_item_id,
                host_software_item_id: Some(target.hsi_link.id),
                to_version: &candidate.latest_version,
                from_version: Some(candidate.installed_version.clone()),
                actor_type: params.actor_type,
                actor_id: params.actor_id,
                update_category: &candidate.update_category,
                batch_id: Some(batch_id),
                initial_status,
                interactive: false,
            },
        )
        .await?;
        history_ids.push((update_history_id, should_dispatch));
    }

    txn.commit().await.context_to()?;

    // Dispatch only Pending items -- Queued items wait for
    // dispatch_next_in_batch to promote them.
    let mut updates: Vec<BatchUpdateItem> = Vec::new();

    for ((candidate, target), (update_history_id, should_dispatch)) in
        validated.iter().zip(history_ids)
    {
        let trigger_status = if should_dispatch {
            let pre_update_outcome = prepare_pre_update_protection(
                db,
                dispatch.protection.clone(),
                target,
                update_history_id,
                None,
            )
            .await?;

            if matches!(pre_update_outcome, PreUpdateProtectionOutcome::Failed) {
                let _ = dispatch::dispatch_next_in_batch(
                    db,
                    DispatchContext {
                        notifier: dispatch.notifier,
                        protection: dispatch.protection.clone(),
                    },
                    batch_id,
                    candidate.host_id,
                    params.tenant_id,
                )
                .await?;
                updates.push(BatchUpdateItem {
                    update_history_id,
                    software_item_id: candidate.software_item_id,
                    software_item_name: candidate.software_item_name.clone(),
                    host_id: candidate.host_id,
                    host_name: candidate.host_name.clone(),
                    to_version: candidate.latest_version.clone(),
                    trigger_status: TriggerUpdateStatus::Failed,
                });
                continue;
            }

            let connected = super::update_dispatch::dispatch_update_to_agent(
                dispatch.notifier,
                target,
                DispatchUpdateParams {
                    update_history_id,
                    to_version: candidate.latest_version.clone(),
                    release_info: None,
                    interactive: false,
                },
            )
            .await?;
            if connected {
                TriggerUpdateStatus::Pending
            } else {
                TriggerUpdateStatus::Queued
            }
        } else {
            // Host busy or subsequent item -- queued for sequential dispatch.
            TriggerUpdateStatus::Queued
        };

        updates.push(BatchUpdateItem {
            update_history_id,
            software_item_id: candidate.software_item_id,
            software_item_name: candidate.software_item_name.clone(),
            host_id: candidate.host_id,
            host_name: candidate.host_name.clone(),
            to_version: candidate.latest_version.clone(),
            trigger_status,
        });
    }

    // Same-request progression can mutate sibling rows after this loop started
    // (e.g. first host item fails protection and immediately promotes/fails
    // the next sibling). Re-read persisted statuses so the response reflects
    // final row state, not provisional queue assumptions.
    let status_by_update_id: HashMap<Uuid, update_history::UpdateStatus> =
        update_history::Entity::find()
            .filter(
                update_history::Column::Id.is_in(
                    updates
                        .iter()
                        .map(|item| item.update_history_id)
                        .collect::<Vec<_>>(),
                ),
            )
            .all(db)
            .await
            .context_to()?
            .into_iter()
            .map(|row| (row.id, row.status))
            .collect();

    for item in &mut updates {
        if let Some(status) = status_by_update_id.get(&item.update_history_id) {
            item.trigger_status = trigger_status_from_update_history_status(*status);
        }
    }

    Ok(BatchUpdateResponse {
        batch_id: Some(batch_id),
        total_created: updates.len(),
        updates,
        skipped,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use sea_orm::{
        ActiveModelTrait, ColumnTrait, Database, DatabaseConnection, EntityTrait, QueryFilter, Set,
    };
    use time::OffsetDateTime;
    use uptrakit_internal_wire::ControllerMessage;
    use uptrakit_plugin_infrastructure_registry::{
        ControllerPostUpdateContext, ControllerProtectionContext, ControllerProtectionDecision,
        ControllerUpdateProtection, PluginError, PluginResult, PostUpdateOutcome,
    };
    use uptrakit_shared_db::entity::{
        host, host_software_item, host_software_item_plugin, plugin_config, prelude::*, service,
        service_host, software_item, tenant, update_history,
    };
    use uptrakit_shared_types::{PluginTypeId, ServiceStatus};
    use uuid::Uuid;

    use super::*;
    use crate::queries::update_types::ActorType;

    /// A no-op notifier for tests -- always returns `true` (agent locally connected).
    pub(crate) struct NoopNotifier;

    #[async_trait::async_trait]
    impl crate::notifier::ServiceNotifier for NoopNotifier {
        async fn send_to_service(&self, _service_id: &Uuid, _msg: ControllerMessage) -> bool {
            true
        }
    }

    pub(crate) struct FailFirstProtection {
        calls: AtomicUsize,
    }

    impl FailFirstProtection {
        pub(crate) fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
            }
        }

        pub(crate) fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    impl uptrakit_plugin_infrastructure_registry::PluginMeta for FailFirstProtection {
        fn plugin_type_id(&self) -> PluginTypeId {
            PluginTypeId::new("infra_test_controller_update_protection")
        }
    }

    #[async_trait]
    impl ControllerUpdateProtection for FailFirstProtection {
        async fn prepare_pre_update_protection(
            &self,
            _ctx: &ControllerProtectionContext<'_>,
        ) -> PluginResult<ControllerProtectionDecision> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(rootcause::report!(PluginError::PluginInternal(
                "controller protection failed".to_string()
            )))
        }

        async fn finalize_post_update(
            &self,
            _ctx: &ControllerPostUpdateContext<'_>,
        ) -> PluginResult<PostUpdateOutcome> {
            Ok(PostUpdateOutcome::default())
        }
    }

    pub(crate) async fn setup_db() -> DatabaseConnection {
        let db = Database::connect("sqlite::memory:").await.unwrap();
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .unwrap();
        db
    }

    pub(crate) struct Fixture {
        pub tenant_id: Uuid,
        pub item_id: Uuid,
        pub host_id: Uuid,
        pub service_id: Uuid,
    }

    /// Insert a minimal valid fixture: tenant, one software item, one host, one agent
    /// (Approved), service_host link, host_software_item (installed="1.0.0",
    /// latest="1.1.0"), plugin_config, and an execute_update plugin assignment.
    pub(crate) async fn insert_base_fixture(db: &DatabaseConnection) -> Fixture {
        let now = OffsetDateTime::now_utc();
        let tenant_id = Uuid::now_v7();
        let item_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let service_id = Uuid::now_v7();
        let plugin_config_id = Uuid::now_v7();

        tenant::ActiveModel {
            id: Set(tenant_id),
            name: Set("test-tenant".to_string()),
            slug: Set(format!("test-{tenant_id}")),
            is_default: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        software_item::ActiveModel {
            id: Set(item_id),
            tenant_id: Set(tenant_id),
            name: Set("test-app".to_string()),
            featured: Set(true),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        host::ActiveModel {
            id: Set(host_id),
            tenant_id: Set(tenant_id),
            machine_id: Set("machine-001".to_string()),
            hostname: Set("host-001".to_string()),
            friendly_name: Set("Host 001".to_string()),
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
        .unwrap();

        service::ActiveModel {
            id: Set(service_id),
            tenant_id: Set(tenant_id),
            capabilities: Set("[]".to_string()),
            hostname: Set("agent-host".to_string()),
            friendly_name: Set("Agent 001".to_string()),
            ip_address: Set(None),
            status: Set(ServiceStatus::Approved),
            enrollment_secret_hash: Set(format!("hash-{service_id}")),
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
        .unwrap();

        service_host::ActiveModel {
            service_id: Set(service_id),
            host_id: Set(host_id),
            linked_at: Set(now),
        }
        .insert(db)
        .await
        .unwrap();

        plugin_config::ActiveModel {
            id: Set(plugin_config_id),
            tenant_id: Set(tenant_id),
            name: Set("test-plugin".to_string()),
            plugin_type: Set("releases_github".to_string()),
            config: Set(serde_json::json!({})),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        let hsi_id = Uuid::now_v7();
        host_software_item::ActiveModel {
            id: Set(hsi_id),
            host_id: Set(host_id),
            software_item_id: Set(item_id),
            qualifier: Set(None),
            plugin_config_id: Set(Some(plugin_config_id)),
            package_identifier: Set(Some("test-pkg".to_string())),
            installed_version: Set(Some("1.0.0".to_string())),
            installed_version_detected_at: Set(None),
            installed_display_version: Set(None),
            latest_version: Set(Some("1.1.0".to_string())),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("security".to_string()),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        host_software_item_plugin::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(host_id),
            software_item_id: Set(item_id),
            host_software_item_id: Set(hsi_id),
            plugin_config_id: Set(Some(plugin_config_id)),
            plugin_type: Set("releases_github".to_string()),
            role: Set("execute_update".to_string()),
            ordinal: Set(0),
            package_identifier: Set("org/repo".to_string()),
            config: Set(None),
            execution_site: Set("auto".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .unwrap();

        let _ = plugin_config_id; // used in DB only; not needed by callers
        Fixture {
            tenant_id,
            item_id,
            host_id,
            service_id,
        }
    }

    /// Helper: insert a second software item + host_software_item + plugin assignment
    /// on the same host as the base fixture. Returns (item2_id).
    pub(crate) async fn insert_second_item(db: &DatabaseConnection, f: &Fixture) -> Uuid {
        let now = OffsetDateTime::now_utc();
        let item2_id = Uuid::now_v7();
        let pc2_id = Uuid::now_v7();

        software_item::ActiveModel {
            id: Set(item2_id),
            tenant_id: Set(f.tenant_id),
            name: Set("test-app-2".to_string()),
            featured: Set(true),
            icon_url: Set(None),
            last_checked_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        plugin_config::ActiveModel {
            id: Set(pc2_id),
            tenant_id: Set(f.tenant_id),
            name: Set("test-plugin-2".to_string()),
            plugin_type: Set("releases_github".to_string()),
            config: Set(serde_json::json!({})),
            enabled: Set(true),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        let hsi2_id = Uuid::now_v7();
        host_software_item::ActiveModel {
            id: Set(hsi2_id),
            host_id: Set(f.host_id),
            software_item_id: Set(item2_id),
            qualifier: Set(None),
            plugin_config_id: Set(Some(pc2_id)),
            package_identifier: Set(Some("test-app-2".to_string())),
            installed_version: Set(Some("2.0.0".to_string())),
            installed_version_detected_at: Set(None),
            installed_display_version: Set(None),
            latest_version: Set(Some("2.1.0".to_string())),
            latest_version_fetched_at: Set(None),
            latest_release_metadata: Set(None),
            last_updated_at: Set(None),
            linked_at: Set(now),
            update_category: Set("security".to_string()),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .unwrap();

        host_software_item_plugin::ActiveModel {
            id: Set(Uuid::now_v7()),
            host_id: Set(f.host_id),
            software_item_id: Set(item2_id),
            host_software_item_id: Set(hsi2_id),
            plugin_config_id: Set(Some(pc2_id)),
            plugin_type: Set("releases_github".to_string()),
            role: Set("execute_update".to_string()),
            ordinal: Set(0),
            package_identifier: Set("org/repo2".to_string()),
            config: Set(None),
            execution_site: Set("auto".to_string()),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(db)
        .await
        .unwrap();

        item2_id
    }

    // -- create_batch --

    /// When a batch contains two outdated items on the same host, the first must
    /// be inserted as `Pending` and the second as `Queued`.
    #[tokio::test]
    async fn create_batch_multiple_items_same_host_queued() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let item2_id = insert_second_item(&db, &f).await;

        let candidates = vec![
            BatchUpdateCandidate {
                software_item_id: f.item_id,
                software_item_name: "test-app".to_string(),
                host_id: f.host_id,
                host_name: "Host 001".to_string(),
                installed_version: "1.0.0".to_string(),
                latest_version: "1.1.0".to_string(),
                update_category: "security".to_string(),
            },
            BatchUpdateCandidate {
                software_item_id: item2_id,
                software_item_name: "test-app-2".to_string(),
                host_id: f.host_id,
                host_name: "Host 001".to_string(),
                installed_version: "2.0.0".to_string(),
                latest_version: "2.1.0".to_string(),
                update_category: "security".to_string(),
            },
        ];

        let resp = create_batch(
            &db,
            DispatchContext {
                notifier: &NoopNotifier,
                protection: None,
            },
            &CreateBatchParams {
                tenant_id: f.tenant_id,
                batch_type: BatchType::HostUpdate,
                actor_type: ActorType::User.as_str(),
                actor_id: "test-user",
            },
            candidates,
        )
        .await
        .unwrap();

        assert_eq!(resp.total_created, 2);
        assert!(resp.batch_id.is_some());

        // Verify DB: exactly one Pending and one Queued.
        let all_records = UpdateHistory::find()
            .filter(update_history::Column::BatchId.eq(resp.batch_id.unwrap()))
            .all(&db)
            .await
            .unwrap();

        assert_eq!(all_records.len(), 2);

        let pending_count = all_records
            .iter()
            .filter(|r| r.status == update_history::UpdateStatus::Pending)
            .count();
        let queued_count = all_records
            .iter()
            .filter(|r| r.status == update_history::UpdateStatus::Queued)
            .count();

        assert_eq!(pending_count, 1, "expected exactly one Pending item");
        assert_eq!(queued_count, 1, "expected exactly one Queued item");

        // The first item (by insertion order) must be Pending.
        let first = all_records
            .iter()
            .find(|r| r.software_item_id == f.item_id)
            .unwrap();
        assert_eq!(
            first.status,
            update_history::UpdateStatus::Pending,
            "first item must be Pending"
        );

        let second = all_records
            .iter()
            .find(|r| r.software_item_id == item2_id)
            .unwrap();
        assert_eq!(
            second.status,
            update_history::UpdateStatus::Queued,
            "second item must be Queued"
        );
    }

    #[tokio::test]
    async fn create_batch_initial_dispatch_continues_after_protection_failure() {
        let db = setup_db().await;
        let f = insert_base_fixture(&db).await;
        let item2_id = insert_second_item(&db, &f).await;
        let protection = Arc::new(FailFirstProtection::new());

        let candidates = vec![
            BatchUpdateCandidate {
                software_item_id: f.item_id,
                software_item_name: "test-app".to_string(),
                host_id: f.host_id,
                host_name: "Host 001".to_string(),
                installed_version: "1.0.0".to_string(),
                latest_version: "1.1.0".to_string(),
                update_category: "security".to_string(),
            },
            BatchUpdateCandidate {
                software_item_id: item2_id,
                software_item_name: "test-app-2".to_string(),
                host_id: f.host_id,
                host_name: "Host 001".to_string(),
                installed_version: "2.0.0".to_string(),
                latest_version: "2.1.0".to_string(),
                update_category: "security".to_string(),
            },
        ];

        let resp = create_batch(
            &db,
            DispatchContext {
                notifier: &NoopNotifier,
                protection: Some(protection.clone()),
            },
            &CreateBatchParams {
                tenant_id: f.tenant_id,
                batch_type: BatchType::HostUpdate,
                actor_type: ActorType::User.as_str(),
                actor_id: "test-user",
            },
            candidates,
        )
        .await
        .unwrap();

        let first_response = resp
            .updates
            .iter()
            .find(|item| item.software_item_id == f.item_id)
            .expect("first response item");
        let second_response = resp
            .updates
            .iter()
            .find(|item| item.software_item_id == item2_id)
            .expect("second response item");
        assert_eq!(
            first_response.trigger_status.to_string(),
            "failed",
            "initial candidate status must match failed persisted row"
        );
        assert_eq!(
            second_response.trigger_status.to_string(),
            "failed",
            "second sibling status must match failed persisted row after same-request progression"
        );

        let rows = UpdateHistory::find()
            .filter(update_history::Column::BatchId.eq(resp.batch_id))
            .all(&db)
            .await
            .unwrap();

        let first = rows
            .iter()
            .find(|row| row.software_item_id == f.item_id)
            .expect("first row");
        let second = rows
            .iter()
            .find(|row| row.software_item_id == item2_id)
            .expect("second row");

        assert_eq!(first.status, update_history::UpdateStatus::Failed);
        assert_eq!(second.status, update_history::UpdateStatus::Failed);
        assert!(
            protection.call_count() >= 2,
            "protection must be attempted for the next queued sibling"
        );
    }
}
