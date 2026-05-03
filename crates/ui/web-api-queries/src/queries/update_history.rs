use sea_orm::sea_query::{Expr, ExprTrait, Query};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
use std::collections::{HashMap, HashSet};
use uptrakit_shared_db::entity::{
    host, prelude::*, service, software_item, system_service, update_history, update_output_line,
    user,
};
use uptrakit_web_api_types::pagination::PaginatedResponse;
use uptrakit_web_api_types::update_history::{
    UpdateHistoryQuery, UpdateHistoryResponse, UpdateStatus,
};
use uuid::Uuid;

use crate::tenant_db::TenantDb;

// --- Private helpers ---

fn db_status_to_api(status: &update_history::UpdateStatus) -> UpdateStatus {
    match status {
        update_history::UpdateStatus::Queued => UpdateStatus::Queued,
        update_history::UpdateStatus::Pending => UpdateStatus::Pending,
        update_history::UpdateStatus::InProgress => UpdateStatus::InProgress,
        update_history::UpdateStatus::Completed => UpdateStatus::Completed,
        update_history::UpdateStatus::Failed => UpdateStatus::Failed,
        _ => {
            tracing::warn!("Unknown update status encountered, defaulting to Pending");
            UpdateStatus::Pending
        }
    }
}

/// Maximum bytes of output to load and return via the API (50 MB).
///
/// Must match `MAX_UPDATE_OUTPUT_BYTES` in the WebSocket handler. This cap is
/// applied when assembling output from streaming lines; stored consolidated
/// output is returned as-is (it was already capped at write time).
const UPDATE_OUTPUT_BYTES_CAP: usize = 52_428_800;

#[expect(
    clippy::string_slice,
    reason = "boundary is walked back to a valid UTF-8 char boundary; slice is always valid"
)]
fn truncate_to_char_boundary(output: &str, max_bytes: usize) -> (&str, bool) {
    if output.len() <= max_bytes {
        return (output, false);
    }

    let mut boundary = max_bytes;
    while boundary > 0 && !output.is_char_boundary(boundary) {
        boundary -= 1;
    }
    (&output[..boundary], true)
}

fn append_output_with_cap(output: &mut String, line: &str, cap: usize) -> bool {
    if output.len() >= cap {
        return true;
    }

    let remaining = cap.saturating_sub(output.len());
    let (prefix, truncated) = truncate_to_char_boundary(line, remaining);
    output.push_str(prefix);
    truncated
}

fn build_response(
    record: &update_history::Model,
    host_name: String,
    software_item_name: String,
    output: String,
    actor_name: Option<String>,
) -> UpdateHistoryResponse {
    UpdateHistoryResponse::new(
        record.id,
        record.host_id,
        host_name,
        record.software_item_id,
        software_item_name,
        record.from_version.clone(),
        record.to_version.clone().unwrap_or_default(),
        db_status_to_api(&record.status),
        output,
        record.actor_type.clone(),
        record.actor_id.clone(),
        actor_name,
        record.started_at.unwrap_or(record.created_at),
        record.completed_at,
        record.created_at,
        record.update_category.clone(),
        record.interactive,
        record.output_truncated,
        record.pre_update_protection_status.clone(),
        record.pre_update_protection_summary.clone(),
        record.recovery_hint.clone(),
    )
}

fn user_display_name(user: &user::Model) -> Option<String> {
    let full = format!("{} {}", user.first_name.trim(), user.last_name.trim())
        .trim()
        .to_string();
    (!full.is_empty()).then_some(full)
}

async fn load_actor_names(
    tenant_db: &TenantDb,
    records: &[update_history::Model],
) -> Result<HashMap<String, String>, sea_orm::DbErr> {
    let actor_ids: Vec<Uuid> = records
        .iter()
        .filter_map(|record| Uuid::parse_str(&record.actor_id).ok())
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();

    if actor_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let user_entries = User::find()
        .filter(user::Column::Id.is_in(actor_ids.iter().copied()))
        .all(tenant_db.db())
        .await?
        .into_iter()
        .filter_map(|row| user_display_name(&row).map(|name| (row.id.to_string(), name)))
        .collect::<HashMap<_, _>>();

    let service_entries = Service::find()
        .filter(service::Column::TenantId.eq(tenant_db.tenant_id()))
        .filter(service::Column::Id.is_in(actor_ids.iter().copied()))
        .all(tenant_db.db())
        .await?
        .into_iter()
        .map(|row| (row.id.to_string(), row.friendly_name))
        .collect::<HashMap<_, _>>();

    let system_service_entries = SystemService::find()
        .filter(system_service::Column::Id.is_in(actor_ids))
        .all(tenant_db.db())
        .await?
        .into_iter()
        .map(|row| (row.id.to_string(), row.friendly_name))
        .collect::<HashMap<_, _>>();

    let mut presentation = HashMap::new();
    presentation.extend(system_service_entries);
    presentation.extend(service_entries);
    presentation.extend(user_entries);
    Ok(presentation)
}

async fn load_output_lines(
    db: &sea_orm::DatabaseConnection,
    update_history_id: Uuid,
) -> Result<String, sea_orm::DbErr> {
    let lines = update_output_line::Entity::find()
        .filter(update_output_line::Column::UpdateHistoryId.eq(update_history_id))
        .order_by_asc(update_output_line::Column::CreatedAt)
        .order_by_asc(update_output_line::Column::Id)
        .all(db)
        .await?;

    let mut output = String::new();
    for line in lines {
        if append_output_with_cap(&mut output, &line.output, UPDATE_OUTPUT_BYTES_CAP) {
            break;
        }
    }

    Ok(output)
}

// --- Public query functions ---

#[tracing::instrument(skip_all)]
pub async fn list_update_history(
    tenant_db: &TenantDb,
    query: &UpdateHistoryQuery,
) -> Result<PaginatedResponse<UpdateHistoryResponse>, sea_orm::DbErr> {
    let pagination = query.pagination().resolve();

    // Tenant-scoped subquery: filter update_history by host IDs belonging to this tenant.
    // This avoids loading all host IDs into application memory.
    let host_subquery = Query::select()
        .column(host::Column::Id)
        .from(host::Entity)
        .and_where(Expr::col(host::Column::TenantId).eq(tenant_db.tenant_id()))
        .to_owned();

    let mut q =
        UpdateHistory::find().filter(update_history::Column::HostId.in_subquery(host_subquery));

    if let Some(host_id) = query.host_id {
        q = q.filter(update_history::Column::HostId.eq(host_id));
    }
    if let Some(software_item_id) = query.software_item_id {
        q = q.filter(update_history::Column::SoftwareItemId.eq(software_item_id));
    }
    if let Some(ref status) = query.status {
        q = q.filter(update_history::Column::Status.eq(status.as_str()));
    }

    let base_query = q.order_by_desc(update_history::Column::CreatedAt);

    let total = base_query.clone().count(tenant_db.db()).await?;

    let records = base_query
        .offset(Some(pagination.offset()))
        .limit(Some(pagination.per_page))
        .all(tenant_db.db())
        .await?;

    if records.is_empty() {
        return Ok(PaginatedResponse::new(vec![], total, pagination));
    }

    // Batch-load actor names (users, services, system services) in three queries.
    let actor_names = load_actor_names(tenant_db, &records).await?;

    // Batch-load host names and software item names in two queries (no per-record lookups).
    let host_ids: Vec<uuid::Uuid> = records
        .iter()
        .map(|r| r.host_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let si_ids: Vec<uuid::Uuid> = records
        .iter()
        .map(|r| r.software_item_id)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();

    let host_names: HashMap<uuid::Uuid, String> = Host::find()
        .filter(host::Column::Id.is_in(host_ids))
        .all(tenant_db.db())
        .await?
        .into_iter()
        .map(|h| (h.id, h.friendly_name))
        .collect();

    let si_names: HashMap<uuid::Uuid, String> = SoftwareItem::find()
        .filter(software_item::Column::Id.is_in(si_ids))
        .all(tenant_db.db())
        .await?
        .into_iter()
        .map(|si| (si.id, si.name))
        .collect();

    // Batch-load output lines for records that used the streaming path
    // (inline `output` column is empty for those).  A single query covers all
    // such records instead of one query per record (N+1 avoidance).
    let streamed_ids: Vec<uuid::Uuid> = records
        .iter()
        .filter(|r| r.output.is_empty())
        .map(|r| r.id)
        .collect();

    let all_lines: HashMap<uuid::Uuid, String> = if streamed_ids.is_empty() {
        HashMap::new()
    } else {
        let rows = update_output_line::Entity::find()
            .filter(update_output_line::Column::UpdateHistoryId.is_in(streamed_ids))
            .order_by_asc(update_output_line::Column::CreatedAt)
            .order_by_asc(update_output_line::Column::Id)
            .all(tenant_db.db())
            .await?;

        let mut map: HashMap<uuid::Uuid, String> = HashMap::new();
        let mut truncated_ids = HashSet::new();
        for line in rows {
            if truncated_ids.contains(&line.update_history_id) {
                continue;
            }

            let entry = map.entry(line.update_history_id).or_default();
            if append_output_with_cap(entry, &line.output, UPDATE_OUTPUT_BYTES_CAP) {
                truncated_ids.insert(line.update_history_id);
            }
        }
        map
    };

    let items: Vec<UpdateHistoryResponse> = records
        .iter()
        .map(|record| {
            let host_name = host_names
                .get(&record.host_id)
                .cloned()
                .unwrap_or_else(|| "Unknown Host".to_string());
            let si_name = si_names
                .get(&record.software_item_id)
                .cloned()
                .unwrap_or_else(|| "Unknown Software Item".to_string());
            let output = if record.output.is_empty() {
                all_lines.get(&record.id).cloned().unwrap_or_default()
            } else {
                record.output.clone()
            };
            let actor_name = actor_names.get(&record.actor_id).cloned();
            build_response(record, host_name, si_name, output, actor_name)
        })
        .collect();

    Ok(PaginatedResponse::new(items, total, pagination))
}

/// Returns `None` if the record is not found or its host does not belong to this tenant.
#[tracing::instrument(skip_all)]
pub async fn get_update_history(
    tenant_db: &TenantDb,
    id: Uuid,
) -> Result<Option<UpdateHistoryResponse>, sea_orm::DbErr> {
    let Some(record) = UpdateHistory::find_by_id(id).one(tenant_db.db()).await? else {
        return Ok(None);
    };

    // Tenant scoping: verify the record's host belongs to this tenant.
    let Some(host) = tenant_db
        .find_by_id::<host::Entity, _>(record.host_id)
        .one(tenant_db.db())
        .await?
    else {
        return Ok(None);
    };

    let si_name = match SoftwareItem::find_by_id(record.software_item_id)
        .one(tenant_db.db())
        .await
    {
        Ok(Some(si)) => si.name,
        _ => "Unknown Software Item".to_string(),
    };
    let output = if record.output.is_empty() {
        match load_output_lines(tenant_db.db(), record.id).await {
            Ok(out) => out,
            Err(e) => {
                tracing::warn!(error = %e, "failed to load update output lines");
                String::new()
            }
        }
    } else {
        record.output.clone()
    };

    let actor_names = load_actor_names(tenant_db, std::slice::from_ref(&record)).await?;
    let actor_name = actor_names.get(&record.actor_id).cloned();

    Ok(Some(build_response(
        &record,
        host.friendly_name,
        si_name,
        output,
        actor_name,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ActiveModelTrait, ConnectOptions, Database, DatabaseConnection, Set};
    use time::OffsetDateTime;
    use uptrakit_shared_db::entity::{host, software_item, tenant};
    use uptrakit_shared_types::OutputStreamType;

    async fn setup_test_db() -> DatabaseConnection {
        let opt = ConnectOptions::new("sqlite::memory:");
        let db = Database::connect(opt).await.expect("test db");
        uptrakit_shared_db::migration::run_migrations(&db)
            .await
            .expect("migrations");
        db
    }

    async fn insert_tenant_record(db: &DatabaseConnection, tenant_id: Uuid) {
        let now = OffsetDateTime::now_utc();
        tenant::ActiveModel {
            id: Set(tenant_id),
            name: Set("Test Tenant".to_string()),
            slug: Set(tenant_id.to_string()),
            is_default: Set(false),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert tenant");
    }

    async fn insert_host_record(db: &DatabaseConnection, tenant_id: Uuid, host_id: Uuid) {
        let now = OffsetDateTime::now_utc();
        host::ActiveModel {
            id: Set(host_id),
            tenant_id: Set(tenant_id),
            machine_id: Set(format!("machine-{host_id}")),
            hostname: Set(format!("host-{host_id}")),
            friendly_name: Set("Boundary Host".to_string()),
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
        .expect("insert host");
    }

    async fn insert_software_item_record(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        software_item_id: Uuid,
    ) {
        let now = OffsetDateTime::now_utc();
        software_item::ActiveModel {
            id: Set(software_item_id),
            tenant_id: Set(tenant_id),
            name: Set("Boundary Software".to_string()),
            featured: Set(false),
            icon_url: Set(None),
            last_checked_at: Set(None),
            awaiting_restart_timeout: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
        }
        .insert(db)
        .await
        .expect("insert software item");
    }

    async fn insert_update_history_record(
        db: &DatabaseConnection,
        tenant_id: Uuid,
        host_id: Uuid,
        software_item_id: Uuid,
        update_history_id: Uuid,
    ) {
        let now = OffsetDateTime::now_utc();
        update_history::ActiveModel {
            id: Set(update_history_id),
            tenant_id: Set(tenant_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(None),
            from_version: Set(Some("1.0.0".to_string())),
            to_version: Set(Some("1.1.0".to_string())),
            status: Set(update_history::UpdateStatus::Completed),
            output: Set(String::new()),
            output_bytes: Set(0),
            actor_type: Set("user".to_string()),
            actor_id: Set(String::new()),
            execution_owner_service_id: Set(None),
            execution_owner_instance_id: Set(None),
            started_at: Set(Some(now)),
            completed_at: Set(Some(now)),
            awaiting_restart_since: Set(None),
            created_at: Set(now),
            update_category: Set("unknown".to_string()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(db)
        .await
        .expect("insert update history");
    }

    async fn insert_output_line_record(
        db: &DatabaseConnection,
        update_history_id: Uuid,
        output: String,
    ) {
        update_output_line::ActiveModel {
            id: Set(Uuid::now_v7()),
            update_history_id: Set(update_history_id),
            stream: Set(OutputStreamType::Stdout),
            output: Set(output),
            created_at: Set(OffsetDateTime::now_utc()),
        }
        .insert(db)
        .await
        .expect("insert output line");
    }

    #[test]
    fn build_response_completed_status() {
        let now = OffsetDateTime::now_utc();
        let record = update_history::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::now_v7(),
            host_id: uuid::Uuid::now_v7(),
            software_item_id: uuid::Uuid::now_v7(),
            host_software_item_id: None,
            from_version: Some("1.0.0".to_string()),
            to_version: Some("2.0.0".to_string()),
            status: update_history::UpdateStatus::Completed,
            output: "Update completed successfully".to_string(),
            output_bytes: 28,
            actor_type: "user".to_string(),
            actor_id: "user-123".to_string(),
            execution_owner_service_id: None,
            execution_owner_instance_id: None,
            started_at: Some(now),
            completed_at: Some(now),
            awaiting_restart_since: None,
            created_at: now,
            update_category: "unknown".to_string(),
            batch_id: None,
            interactive: false,
            output_truncated: false,
            pre_update_protection_status: Some("protected".to_string()),
            pre_update_protection_summary: Some("snapshot created".to_string()),
            recovery_hint: Some("rollback snapshot id abc123".to_string()),
        };

        let resp = build_response(
            &record,
            "Web Server".to_string(),
            "Node.js".to_string(),
            "Update completed successfully".to_string(),
            None,
        );

        assert_eq!(resp.host_name, "Web Server");
        assert_eq!(resp.software_item_name, "Node.js");
        assert_eq!(resp.from_version, Some("1.0.0".to_string()));
        assert_eq!(resp.to_version, "2.0.0");
        assert_eq!(resp.status, UpdateStatus::Completed);
        assert_eq!(resp.output, "Update completed successfully");
        assert_eq!(resp.actor_type, "user");
        assert_eq!(resp.actor_id, "user-123");
        assert!(resp.completed_at.is_some());
        assert_eq!(
            resp.pre_update_protection_status.as_deref(),
            Some("protected")
        );
        assert_eq!(
            resp.pre_update_protection_summary.as_deref(),
            Some("snapshot created")
        );
        assert_eq!(
            resp.recovery_hint.as_deref(),
            Some("rollback snapshot id abc123")
        );
    }

    #[test]
    fn build_response_failed_status() {
        let now = OffsetDateTime::now_utc();
        let record = update_history::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::now_v7(),
            host_id: uuid::Uuid::now_v7(),
            software_item_id: uuid::Uuid::now_v7(),
            host_software_item_id: None,
            from_version: None,
            to_version: Some("3.0.0".to_string()),
            status: update_history::UpdateStatus::Failed,
            output: "Error: package not found".to_string(),
            output_bytes: 25,
            actor_type: "scheduler".to_string(),
            actor_id: "".to_string(),
            execution_owner_service_id: None,
            execution_owner_instance_id: None,
            started_at: Some(now),
            completed_at: Some(now),
            awaiting_restart_since: None,
            created_at: now,
            update_category: "security".to_string(),
            batch_id: None,
            interactive: false,
            output_truncated: false,
            pre_update_protection_status: None,
            pre_update_protection_summary: None,
            recovery_hint: None,
        };

        let resp = build_response(
            &record,
            "DB Server".to_string(),
            "PostgreSQL".to_string(),
            "Error: package not found".to_string(),
            None,
        );

        assert_eq!(resp.host_name, "DB Server");
        assert_eq!(resp.software_item_name, "PostgreSQL");
        assert!(resp.from_version.is_none());
        assert_eq!(resp.to_version, "3.0.0");
        assert_eq!(resp.status, UpdateStatus::Failed);
        assert_eq!(resp.output, "Error: package not found");
        assert_eq!(resp.actor_type, "scheduler");
    }

    #[test]
    fn build_response_pending_no_completed_at() {
        let now = OffsetDateTime::now_utc();
        let record = update_history::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::now_v7(),
            host_id: uuid::Uuid::now_v7(),
            software_item_id: uuid::Uuid::now_v7(),
            host_software_item_id: None,
            from_version: Some("1.0.0".to_string()),
            to_version: Some("1.1.0".to_string()),
            status: update_history::UpdateStatus::Pending,
            output: String::new(),
            output_bytes: 0,
            actor_type: "uptrakit-mqtt".to_string(),
            actor_id: "".to_string(),
            execution_owner_service_id: None,
            execution_owner_instance_id: None,
            started_at: Some(now),
            completed_at: None,
            awaiting_restart_since: None,
            created_at: now,
            update_category: "unknown".to_string(),
            batch_id: None,
            interactive: false,
            output_truncated: false,
            pre_update_protection_status: None,
            pre_update_protection_summary: None,
            recovery_hint: None,
        };

        let resp = build_response(
            &record,
            "App Host".to_string(),
            "Redis".to_string(),
            String::new(),
            None,
        );

        assert_eq!(resp.status, UpdateStatus::Pending);
        assert!(resp.completed_at.is_none());
        assert_eq!(resp.actor_type, "uptrakit-mqtt");
    }

    #[test]
    fn build_response_queued_status() {
        let now = OffsetDateTime::now_utc();
        let record = update_history::Model {
            id: uuid::Uuid::now_v7(),
            tenant_id: uuid::Uuid::now_v7(),
            host_id: uuid::Uuid::now_v7(),
            software_item_id: uuid::Uuid::now_v7(),
            host_software_item_id: None,
            from_version: Some("1.17.8".to_string()),
            to_version: Some("1.17.9".to_string()),
            status: update_history::UpdateStatus::Queued,
            output: String::new(),
            output_bytes: 0,
            actor_type: "user".to_string(),
            actor_id: "user-123".to_string(),
            execution_owner_service_id: None,
            execution_owner_instance_id: None,
            started_at: Some(now),
            completed_at: None,
            awaiting_restart_since: None,
            created_at: now,
            update_category: "unknown".to_string(),
            batch_id: None,
            interactive: false,
            output_truncated: false,
            pre_update_protection_status: None,
            pre_update_protection_summary: None,
            recovery_hint: None,
        };

        let resp = build_response(
            &record,
            "App Host".to_string(),
            "cargo-binstall".to_string(),
            String::new(),
            None,
        );

        assert_eq!(resp.status, UpdateStatus::Queued);
        assert!(resp.completed_at.is_none());
    }

    #[test]
    fn db_status_to_api_maps_all_variants() {
        assert_eq!(
            db_status_to_api(&update_history::UpdateStatus::Queued),
            UpdateStatus::Queued
        );
        assert_eq!(
            db_status_to_api(&update_history::UpdateStatus::Pending),
            UpdateStatus::Pending
        );
        assert_eq!(
            db_status_to_api(&update_history::UpdateStatus::InProgress),
            UpdateStatus::InProgress
        );
        assert_eq!(
            db_status_to_api(&update_history::UpdateStatus::Completed),
            UpdateStatus::Completed
        );
        assert_eq!(
            db_status_to_api(&update_history::UpdateStatus::Failed),
            UpdateStatus::Failed
        );
    }

    #[tokio::test]
    async fn load_output_lines_truncates_on_utf8_boundary() {
        let db = setup_test_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let update_history_id = Uuid::now_v7();

        insert_tenant_record(&db, tenant_id).await;
        insert_host_record(&db, tenant_id, host_id).await;
        insert_software_item_record(&db, tenant_id, software_item_id).await;
        insert_update_history_record(&db, tenant_id, host_id, software_item_id, update_history_id)
            .await;
        insert_output_line_record(
            &db,
            update_history_id,
            format!("{}étail", "a".repeat(UPDATE_OUTPUT_BYTES_CAP - 1)),
        )
        .await;

        let output = load_output_lines(&db, update_history_id).await.unwrap();

        assert_eq!(output, "a".repeat(UPDATE_OUTPUT_BYTES_CAP - 1));
    }

    #[tokio::test]
    async fn list_update_history_truncates_streamed_output_on_utf8_boundary() {
        let db = setup_test_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let update_history_id = Uuid::now_v7();

        insert_tenant_record(&db, tenant_id).await;
        insert_host_record(&db, tenant_id, host_id).await;
        insert_software_item_record(&db, tenant_id, software_item_id).await;
        insert_update_history_record(&db, tenant_id, host_id, software_item_id, update_history_id)
            .await;
        insert_output_line_record(
            &db,
            update_history_id,
            format!("{}étail", "a".repeat(UPDATE_OUTPUT_BYTES_CAP - 1)),
        )
        .await;

        let tenant_db = TenantDb::new(db, tenant_id);
        let response = list_update_history(
            &tenant_db,
            &UpdateHistoryQuery::new(None, None, None, Some(1), Some(20)),
        )
        .await
        .unwrap();

        assert_eq!(response.items.len(), 1);
        assert_eq!(
            response.items[0].output,
            "a".repeat(UPDATE_OUTPUT_BYTES_CAP - 1)
        );
    }

    #[test]
    fn build_response_includes_actor_name() {
        let now = OffsetDateTime::now_utc();
        let record = update_history::Model {
            id: Uuid::now_v7(),
            tenant_id: Uuid::now_v7(),
            host_id: Uuid::now_v7(),
            software_item_id: Uuid::now_v7(),
            host_software_item_id: None,
            from_version: Some("1.0.0".into()),
            to_version: Some("1.1.0".into()),
            status: update_history::UpdateStatus::Completed,
            output: "done".into(),
            output_bytes: 4,
            actor_type: "user".into(),
            actor_id: "11111111-1111-1111-1111-111111111111".into(),
            execution_owner_service_id: None,
            execution_owner_instance_id: None,
            started_at: Some(now),
            completed_at: Some(now),
            awaiting_restart_since: None,
            created_at: now,
            update_category: "unknown".into(),
            batch_id: None,
            interactive: false,
            output_truncated: false,
            pre_update_protection_status: None,
            pre_update_protection_summary: None,
            recovery_hint: None,
        };

        let resp = build_response(
            &record,
            "Web Server".into(),
            "Node.js".into(),
            "done".into(),
            Some("Alice Smith".into()),
        );

        assert_eq!(resp.actor_name.as_deref(), Some("Alice Smith"));
    }

    #[tokio::test]
    async fn list_update_history_resolves_user_actor_name() {
        use uptrakit_shared_db::entity::user;
        use uptrakit_shared_types::MaskedEmail;

        let db = setup_test_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let update_history_id = Uuid::now_v7();
        let user_id = Uuid::now_v7();

        insert_tenant_record(&db, tenant_id).await;
        insert_host_record(&db, tenant_id, host_id).await;
        insert_software_item_record(&db, tenant_id, software_item_id).await;

        let now = OffsetDateTime::now_utc();
        user::ActiveModel {
            id: Set(user_id),
            email: Set("alice@example.com".parse::<MaskedEmail>().unwrap()),
            first_name: Set("Alice".into()),
            last_name: Set("Smith".into()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert user");

        update_history::ActiveModel {
            id: Set(update_history_id),
            tenant_id: Set(tenant_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(None),
            from_version: Set(Some("1.0.0".into())),
            to_version: Set(Some("1.1.0".into())),
            status: Set(update_history::UpdateStatus::Completed),
            output: Set("done".into()),
            output_bytes: Set(4),
            actor_type: Set("user".into()),
            actor_id: Set(user_id.to_string()),
            execution_owner_service_id: Set(None),
            execution_owner_instance_id: Set(None),
            started_at: Set(Some(now)),
            completed_at: Set(Some(now)),
            awaiting_restart_since: Set(None),
            created_at: Set(now),
            update_category: Set("unknown".into()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert update history");

        let tenant_db = TenantDb::new(db.clone(), tenant_id);
        let resp = list_update_history(
            &tenant_db,
            &UpdateHistoryQuery::new(None, None, None, Some(1), Some(20)),
        )
        .await
        .expect("list update history");

        assert_eq!(resp.items[0].actor_name.as_deref(), Some("Alice Smith"));
    }

    #[tokio::test]
    async fn get_update_history_resolves_user_actor_name() {
        use uptrakit_shared_db::entity::user;
        use uptrakit_shared_types::MaskedEmail;

        let db = setup_test_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let update_history_id = Uuid::now_v7();
        let user_id = Uuid::now_v7();

        insert_tenant_record(&db, tenant_id).await;
        insert_host_record(&db, tenant_id, host_id).await;
        insert_software_item_record(&db, tenant_id, software_item_id).await;

        let now = OffsetDateTime::now_utc();
        user::ActiveModel {
            id: Set(user_id),
            email: Set("alice@example.com".parse::<MaskedEmail>().unwrap()),
            first_name: Set("Alice".into()),
            last_name: Set("Smith".into()),
            password_hash: Set(None),
            is_active: Set(true),
            deactivated_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&db)
        .await
        .expect("insert user");

        update_history::ActiveModel {
            id: Set(update_history_id),
            tenant_id: Set(tenant_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(None),
            from_version: Set(Some("1.0.0".into())),
            to_version: Set(Some("1.1.0".into())),
            status: Set(update_history::UpdateStatus::Completed),
            output: Set("done".into()),
            output_bytes: Set(4),
            actor_type: Set("user".into()),
            actor_id: Set(user_id.to_string()),
            execution_owner_service_id: Set(None),
            execution_owner_instance_id: Set(None),
            started_at: Set(Some(now)),
            completed_at: Set(Some(now)),
            awaiting_restart_since: Set(None),
            created_at: Set(now),
            update_category: Set("unknown".into()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert update history");

        let tenant_db = TenantDb::new(db.clone(), tenant_id);
        let resp = get_update_history(&tenant_db, update_history_id)
            .await
            .expect("get update history")
            .expect("history item");

        assert_eq!(resp.actor_name.as_deref(), Some("Alice Smith"));
    }

    #[tokio::test]
    async fn list_update_history_resolves_service_actor_name() {
        use uptrakit_shared_db::entity::service;
        use uptrakit_shared_types::ServiceStatus;

        let db = setup_test_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let update_history_id = Uuid::now_v7();
        let service_id = Uuid::now_v7();

        insert_tenant_record(&db, tenant_id).await;
        insert_host_record(&db, tenant_id, host_id).await;
        insert_software_item_record(&db, tenant_id, software_item_id).await;

        let now = OffsetDateTime::now_utc();
        service::ActiveModel {
            id: Set(service_id),
            tenant_id: Set(tenant_id),
            capabilities: Set(String::new()),
            hostname: Set("agent-host".into()),
            friendly_name: Set("My Agent Service".into()),
            ip_address: Set(None),
            status: Set(ServiceStatus::Approved),
            enrollment_secret_hash: Set("hash".into()),
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
        .insert(&db)
        .await
        .expect("insert service");

        update_history::ActiveModel {
            id: Set(update_history_id),
            tenant_id: Set(tenant_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(None),
            from_version: Set(Some("2.0.0".into())),
            to_version: Set(Some("2.1.0".into())),
            status: Set(update_history::UpdateStatus::Completed),
            output: Set("done".into()),
            output_bytes: Set(4),
            actor_type: Set("service".into()),
            actor_id: Set(service_id.to_string()),
            execution_owner_service_id: Set(None),
            execution_owner_instance_id: Set(None),
            started_at: Set(Some(now)),
            completed_at: Set(Some(now)),
            awaiting_restart_since: Set(None),
            created_at: Set(now),
            update_category: Set("unknown".into()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert update history");

        let tenant_db = TenantDb::new(db.clone(), tenant_id);
        let resp = list_update_history(
            &tenant_db,
            &UpdateHistoryQuery::new(None, None, None, Some(1), Some(20)),
        )
        .await
        .expect("list update history");

        assert_eq!(
            resp.items[0].actor_name.as_deref(),
            Some("My Agent Service")
        );
    }

    #[tokio::test]
    async fn get_update_history_resolves_system_service_actor_name() {
        use uptrakit_shared_db::entity::system_service;

        let db = setup_test_db().await;
        let tenant_id = Uuid::now_v7();
        let host_id = Uuid::now_v7();
        let software_item_id = Uuid::now_v7();
        let update_history_id = Uuid::now_v7();
        let system_service_id = Uuid::now_v7();

        insert_tenant_record(&db, tenant_id).await;
        insert_host_record(&db, tenant_id, host_id).await;
        insert_software_item_record(&db, tenant_id, software_item_id).await;

        let now = OffsetDateTime::now_utc();
        system_service::ActiveModel {
            id: Set(system_service_id),
            capabilities: Set(String::new()),
            hostname: Set("sys-host".into()),
            friendly_name: Set("MQTT Bridge".into()),
            ip_address: Set(None),
            status: Set(system_service::SystemServiceStatus::Approved),
            enrollment_secret_hash: Set("syshash".into()),
            client_version: Set(None),
            last_seen_at: Set(None),
            created_at: Set(now),
            updated_at: Set(now),
            deactivated_at: Set(None),
            ping_interval_seconds: Set(None),
            cert_lifetime_hours: Set(None),
            system_enrollment_token_id: Set(None),
            service_app_name: Set(None),
            is_embedded: Set(false),
            embedded_owner_key: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert system service");

        update_history::ActiveModel {
            id: Set(update_history_id),
            tenant_id: Set(tenant_id),
            host_id: Set(host_id),
            software_item_id: Set(software_item_id),
            host_software_item_id: Set(None),
            from_version: Set(Some("3.0.0".into())),
            to_version: Set(Some("3.1.0".into())),
            status: Set(update_history::UpdateStatus::Completed),
            output: Set("done".into()),
            output_bytes: Set(4),
            actor_type: Set("system_service".into()),
            actor_id: Set(system_service_id.to_string()),
            execution_owner_service_id: Set(None),
            execution_owner_instance_id: Set(None),
            started_at: Set(Some(now)),
            completed_at: Set(Some(now)),
            awaiting_restart_since: Set(None),
            created_at: Set(now),
            update_category: Set("unknown".into()),
            batch_id: Set(None),
            interactive: Set(false),
            output_truncated: Set(false),
            pre_update_protection_status: Set(None),
            pre_update_protection_summary: Set(None),
            recovery_hint: Set(None),
        }
        .insert(&db)
        .await
        .expect("insert update history");

        let tenant_db = TenantDb::new(db.clone(), tenant_id);
        let resp = get_update_history(&tenant_db, update_history_id)
            .await
            .expect("get update history")
            .expect("history item");

        assert_eq!(resp.actor_name.as_deref(), Some("MQTT Bridge"));
    }
}
