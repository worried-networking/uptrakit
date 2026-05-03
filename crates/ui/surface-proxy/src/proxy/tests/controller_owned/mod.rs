use std::sync::Once;

use sea_orm::{
    ActiveModelTrait, ColumnTrait, ConnectOptions, Database, EntityTrait, QueryFilter, QueryOrder,
    Set,
};
use uuid::Uuid;

use super::tenant_id;
use uptrakit_shared_db::entity::audit_log;

mod notifications;
mod proxmox;

mod docker;
mod notification_settings;
mod proxmox_update_protection;

fn ensure_master_key() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        uptrakit_crypto::enable_plaintext_mode();
        let _ = uptrakit_crypto::init_master_key(zeroize::Zeroizing::new([7_u8; 32]));
    });
}

async fn setup_notification_db() -> sea_orm::DatabaseConnection {
    let opt = ConnectOptions::new("sqlite::memory:".to_owned());
    let db = Database::connect(opt).await.expect("test db");
    uptrakit_shared_db::migration::run_migrations(&db)
        .await
        .expect("migrations should run");
    insert_tenant(&db, tenant_id()).await;
    db
}

async fn insert_tenant(db: &sea_orm::DatabaseConnection, id: Uuid) {
    let now = time::OffsetDateTime::now_utc();
    uptrakit_shared_db::entity::tenant::ActiveModel {
        id: Set(id),
        name: Set("Surface Test Tenant".to_string()),
        slug: Set(id.to_string()),
        is_default: Set(false),
        created_at: Set(now),
        updated_at: Set(now),
        deactivated_at: Set(None),
    }
    .insert(db)
    .await
    .expect("insert tenant");
}

pub(super) fn test_audit_emitter(
    db: sea_orm::DatabaseConnection,
) -> uptrakit_audit_log::AuditEmitter {
    use std::sync::Arc as StdArc;
    let backend = StdArc::new(uptrakit_audit_log::DatabaseBackend::new(db));
    let dispatcher = uptrakit_audit_log::AuditLogDispatcher::new(backend);
    uptrakit_audit_log::AuditEmitter::new(dispatcher)
}

pub(super) async fn latest_tenant_audit_row_for_action(
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

pub(super) async fn latest_tenant_audit_row_for_action_and_outcome(
    db: &sea_orm::DatabaseConnection,
    action_type: uptrakit_audit_log::RegisteredAuditAction,
    outcome: uptrakit_audit_log::AuditOutcome,
) -> audit_log::Model {
    for _ in 0..50 {
        if let Some(row) = audit_log::Entity::find()
            .filter(audit_log::Column::ActionType.eq(action_type))
            .filter(audit_log::Column::Outcome.eq(outcome.as_str()))
            .order_by_desc(audit_log::Column::OccurredAt)
            .one(db)
            .await
            .expect("query tenant audit rows by outcome")
        {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("expected tenant audit row for action {action_type} with outcome {outcome}");
}
