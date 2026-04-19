use std::sync::Once;

use sea_orm::{ActiveModelTrait, ConnectOptions, Database, Set};
use uuid::Uuid;

use super::tenant_id;

mod notifications;
mod proxmox;

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
