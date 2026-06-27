//! Audit-emission unit tests for the software_items route submodules.
#![expect(
    clippy::expect_used,
    reason = "test code: panics on failure are acceptable"
)]
#![expect(
    clippy::string_slice,
    reason = "test code: slice indexes are at validated boundaries"
)]

use super::version_check::load_agent_service;
use crate::tenant_db::TenantDb;
use crate::test_harness::TestApp;
use crate::test_harness::fixtures::{insert_host, link_service_host};
use sea_orm::{ActiveModelTrait, Set};
use uptrakit_shared_db::entity::service;

async fn insert_service_with_timestamps(
    app: &TestApp,
    id: uuid::Uuid,
    status: service::ServiceStatus,
    updated_at: time::OffsetDateTime,
    last_seen_at: Option<time::OffsetDateTime>,
) -> service::Model {
    service::ActiveModel {
        id: Set(id),
        tenant_id: Set(app.tenant_id),
        capabilities: Set("[]".to_string()),
        hostname: Set(format!("host-{}", &id.to_string()[..8])),
        friendly_name: Set(format!("Service {}", &id.to_string()[..8])),
        ip_address: Set(Some("10.0.0.1".to_string())),
        status: Set(status),
        enrollment_secret_hash: Set(format!("secret-{id}")),
        client_version: Set(None),
        last_seen_at: Set(last_seen_at),
        created_at: Set(updated_at),
        updated_at: Set(updated_at),
        deactivated_at: Set(None),
        ping_interval_seconds: Set(None),
        enrollment_token_id: Set(None),
        cert_lifetime_hours: Set(None),
        service_app_name: Set(None),
        is_embedded: Set(false),
        embedded_owner_key: Set(None),
    }
    .insert(&app.db)
    .await
    .expect("insert service")
}

#[tokio::test]
async fn load_agent_service_prefers_active_approved_service_when_host_has_stale_links() {
    let app = TestApp::new().await;
    let tenant_db = TenantDb::new_for_test(app.db.clone(), app.tenant_id);
    let host = insert_host(&app.db, app.tenant_id).await;

    let stale_updated_at = time::OffsetDateTime::now_utc() - time::Duration::days(1);
    let active_updated_at = time::OffsetDateTime::now_utc();

    let stale_service = insert_service_with_timestamps(
        &app,
        uuid::Uuid::now_v7(),
        service::ServiceStatus::Approved,
        stale_updated_at,
        Some(stale_updated_at),
    )
    .await;
    let active_service = insert_service_with_timestamps(
        &app,
        uuid::Uuid::now_v7(),
        service::ServiceStatus::Approved,
        active_updated_at,
        Some(active_updated_at),
    )
    .await;

    link_service_host(&app.db, stale_service.id, host.id).await;
    link_service_host(&app.db, active_service.id, host.id).await;

    service::ActiveModel {
        id: Set(stale_service.id),
        deactivated_at: Set(Some(time::OffsetDateTime::now_utc())),
        ..stale_service.into()
    }
    .update(&app.db)
    .await
    .expect("deactivate stale service");

    let agent = load_agent_service(&tenant_db, host.id)
        .await
        .expect("should select active approved service");

    assert_eq!(agent.id, active_service.id);
}
